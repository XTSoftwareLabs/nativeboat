//! Where display items can paint, so that groups painted off-screen use
//! buffers no larger than their content.

use super::filter;
#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_ir::{DisplayItem, PathSegment, Rect};
use nativeboat_layout::svg::multiply;

/// Left, top, right and bottom.
pub(super) type Bounds = [f32; 4];

#[derive(Debug, PartialEq)]
pub(super) enum Extent {
    Unknown,
    Empty,
    Known(Bounds),
}

/// Opacity must include every changed pixel. Text and shadows reach past
/// their rectangles by bounded amounts that [`bounds`] adds; images, canvases
/// and backdrops retain the complete backdrop until their drawing is
/// measured, rather than relying on estimates.
pub(super) fn opacity_extent(items: &[DisplayItem], transform: [f32; 6]) -> Extent {
    if items.iter().any(|item| {
        matches!(
            item,
            DisplayItem::Image { .. } | DisplayItem::Canvas { .. } | DisplayItem::Backdrop { .. }
        )
    }) {
        return Extent::Unknown;
    }
    match bounds(items, transform) {
        Some(bounds) => Extent::Known(bounds),
        None => Extent::Empty,
    }
}

fn union(a: Option<Bounds>, b: Bounds) -> Option<Bounds> {
    Some(match a {
        Some(a) => [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ],
        None => b,
    })
}

fn grow(bounds: Bounds, x: f32, y: f32) -> Bounds {
    [bounds[0] - x, bounds[1] - y, bounds[2] + x, bounds[3] + y]
}

fn points(transform: [f32; 6], points: impl IntoIterator<Item = (f32, f32)>) -> Option<Bounds> {
    let [a, b, c, d, e, f] = transform;
    points.into_iter().fold(None, |bounds, (x, y)| {
        let (x, y) = (a * x + c * y + e, b * x + d * y + f);
        union(bounds, [x, y, x, y])
    })
}

/// The bounds of a rectangle grown by `spread` after `transform` maps it.
pub(super) fn area(transform: [f32; 6], rect: Rect, spread: f32) -> Bounds {
    let (left, top) = (rect.x - spread, rect.y - spread);
    let (right, bottom) = (rect.x + rect.width + spread, rect.y + rect.height + spread);
    points(
        transform,
        [(left, top), (right, top), (right, bottom), (left, bottom)],
    )
    .unwrap_or_default()
}

/// The bounds of what items paint after `transform` maps them, or `None`
/// when they paint nothing. Clips are ignored, so the bounds may be larger
/// than what shows.
pub(super) fn bounds(items: &[DisplayItem], transform: [f32; 6]) -> Option<Bounds> {
    let mut transforms = vec![transform];
    let mut bounds = None;
    let mut index = 0;
    while index < items.len() {
        let current = *transforms.last().unwrap_or(&transform);
        let extent = match &items[index] {
            DisplayItem::Fill { rect: area, .. }
            | DisplayItem::Gradient { rect: area, .. }
            | DisplayItem::Border { rect: area, .. }
            | DisplayItem::Canvas { rect: area, .. }
            | DisplayItem::Image { rect: area, .. }
            | DisplayItem::Caret { rect: area, .. }
            | DisplayItem::Backdrop { rect: area, .. } => Some(self::area(current, *area, 0.0)),
            // Glyphs may overhang their run, for example in italics, and a
            // line box shorter than the font leaves ascenders and
            // descenders outside it: up to an em on every side.
            DisplayItem::Text {
                rect: area, font, ..
            } => Some(self::area(
                current,
                *area,
                (area.height / 2.0).max(font.size),
            )),
            DisplayItem::Shadow {
                rect: area,
                blur,
                inset,
                clip,
                ..
            } => Some(if *inset {
                self::area(current, *clip, 0.0)
            } else {
                self::area(current, *area, blur * 1.5)
            }),
            DisplayItem::Path {
                segments,
                transform: own,
                stroke,
                ..
            } => {
                let corners = segments.iter().flat_map(|segment| match *segment {
                    PathSegment::Move(x, y) | PathSegment::Line(x, y) => vec![(x, y)],
                    PathSegment::Cubic(x1, y1, x2, y2, x, y) => vec![(x1, y1), (x2, y2), (x, y)],
                    PathSegment::Close => Vec::new(),
                });
                let [a, b, c, d, ..] = multiply(current, *own);
                // Miter joins reach up to five stroke widths out.
                let spread = stroke.as_ref().map_or(0.0, |stroke| stroke.width * 5.0);
                points(multiply(current, *own), corners).map(|bounds| {
                    grow(
                        bounds,
                        spread * (a.abs() + c.abs()),
                        spread * (b.abs() + d.abs()),
                    )
                })
            }
            DisplayItem::PushTransform(own) => {
                transforms.push(multiply(current, *own));
                None
            }
            DisplayItem::PopTransform => {
                transforms.pop();
                None
            }
            DisplayItem::PushFilter(filters) => {
                let end = group_end(items, index);
                let inner = self::bounds(&items[index + 1..end], current);
                index = end;
                let (x, y) = filter::reach(filters, [1.0, 0.0, 0.0, 1.0]);
                let [a, b, c, d, ..] = current;
                inner.map(|inner| {
                    let (x, y) = (x as f32, y as f32);
                    grow(inner, x * a.abs() + y * c.abs(), x * b.abs() + y * d.abs())
                })
            }
            DisplayItem::PushClip { .. }
            | DisplayItem::PushClipPath { .. }
            | DisplayItem::PushMask(_)
            | DisplayItem::PopMask
            | DisplayItem::PopClip
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PopOpacity
            | DisplayItem::PopFilter
            | DisplayItem::Beneath
            | DisplayItem::Child(_) => None,
        };
        if let Some(extent) = extent {
            bounds = union(bounds, extent);
        }
        index += 1;
    }
    bounds
}

/// Whether the items leave clipping and transforms as they found them, so
/// skipping them changes nothing that follows.
pub(super) fn balanced(items: &[DisplayItem]) -> bool {
    let (mut clips, mut transforms) = (0usize, 0usize);
    for item in items {
        match item {
            DisplayItem::PushClip { .. } | DisplayItem::PushClipPath { .. } => clips += 1,
            DisplayItem::PushTransform(_) => transforms += 1,
            DisplayItem::PopClip => match clips.checked_sub(1) {
                Some(depth) => clips = depth,
                None => return false,
            },
            DisplayItem::PopTransform => match transforms.checked_sub(1) {
                Some(depth) => transforms = depth,
                None => return false,
            },
            _ => {}
        }
    }
    clips == 0 && transforms == 0
}

/// The matching opacity/filter pop, or the end of an unclosed group.
pub(super) fn group_end(items: &[DisplayItem], start: usize) -> usize {
    // 0: filters, 1: opacity, 2: masks.
    let kind = match items.get(start) {
        Some(DisplayItem::PushOpacity(_)) => 1,
        Some(DisplayItem::PushMask(_)) => 2,
        _ => 0,
    };
    let mut depth = 0;
    for (index, item) in items.iter().enumerate().skip(start) {
        match (item, kind) {
            (DisplayItem::PushFilter(_), 0)
            | (DisplayItem::PushOpacity(_), 1)
            | (DisplayItem::PushMask(_), 2) => depth += 1,
            (DisplayItem::PopFilter, 0)
            | (DisplayItem::PopOpacity, 1)
            | (DisplayItem::PopMask, 2) => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    items.len()
}

#[cfg(test)]
mod tests;
