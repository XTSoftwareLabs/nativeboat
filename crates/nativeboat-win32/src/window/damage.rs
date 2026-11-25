//! The part of a surface a new display list changes, so a repaint covers
//! only that part: typing changes one line, not the window.

#[allow(unused_imports)]
use crate::prelude::*;
use nativeboat_ir::{DisplayItem, PathSegment, Rect};

/// What a new display list changes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Damage {
    /// Nothing.
    None,
    /// At most these areas, in content coordinates.
    Areas(Vec<Rect>),
    /// Possibly anything.
    All,
}

/// The damage of replacing `old` with `new`. The items both share at the
/// start and the end paint the same; the others are bounded by their
/// rectangles, unless a transform, filter, opacity group, path or native
/// child is involved, which repaints all.
pub(crate) fn between(old: &[DisplayItem], new: &[DisplayItem]) -> Damage {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(old, new)| old == new)
        .count();
    if prefix == old.len() && prefix == new.len() {
        return Damage::None;
    }
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    // Changed items inside an open transform or filter paint elsewhere or
    // spread; a whole repaint is simplest.
    let mut open = 0_i32;
    for item in &old[..prefix] {
        match item {
            DisplayItem::PushTransform(_) | DisplayItem::PushFilter(_) => open += 1,
            DisplayItem::PopTransform | DisplayItem::PopFilter => open -= 1,
            _ => {}
        }
    }
    if open > 0 {
        return Damage::All;
    }
    let (old, new) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );
    // Lists that changed item for item, as typing changes a line and a word
    // count far apart, repaint just the items that differ.
    if old.len() == new.len() {
        let mut areas: Vec<Rect> = Vec::new();
        for (before, after) in old.iter().zip(new) {
            if before == after {
                continue;
            }
            let (Some(before), Some(after)) = (bounds(before), bounds(after)) else {
                return Damage::All;
            };
            let mut area = union(before, after);
            if area.width <= 0.0 || area.height <= 0.0 {
                continue;
            }
            // Overlapping areas paint as one.
            while let Some(index) = areas.iter().position(|other| overlap(*other, area)) {
                area = union(area, areas.swap_remove(index));
            }
            areas.push(area);
        }
        if areas.len() > MOST_AREAS {
            let all = areas.into_iter().reduce(union).unwrap_or_default();
            return Damage::Areas(vec![all]);
        }
        return if areas.is_empty() {
            Damage::None
        } else {
            Damage::Areas(areas)
        };
    }
    let mut area: Option<Rect> = None;
    for item in old.iter().chain(new) {
        let Some(bounds) = bounds(item) else {
            return Damage::All;
        };
        area = Some(area.map_or(bounds, |area| union(area, bounds)));
    }
    area.map_or(Damage::None, |area| Damage::Areas(vec![area]))
}

/// More separate areas than this repaint the rectangle around them.
const MOST_AREAS: usize = 4;

/// The area an item paints, or `None` when that is not bounded by its own
/// rectangle.
fn bounds(item: &DisplayItem) -> Option<Rect> {
    let grown = |rect: &Rect, by: f32| Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + 2.0 * by,
        height: rect.height + 2.0 * by,
    };
    match item {
        DisplayItem::Fill { rect, .. }
        | DisplayItem::Gradient { rect, .. }
        | DisplayItem::Border { rect, .. }
        | DisplayItem::Canvas { rect, .. }
        | DisplayItem::Image { rect, .. }
        | DisplayItem::Caret { rect, .. } => Some(grown(rect, 1.0)),
        // Glyphs reach past their line box: italics, accents, descenders.
        DisplayItem::Text { rect, .. } => Some(grown(rect, rect.height * 0.5 + 1.0)),
        DisplayItem::Shadow {
            rect, blur, clip, ..
        } => Some(union(grown(rect, blur * 2.0 + 1.0), grown(clip, 1.0))),
        // Everything inside a clip paints within it.
        DisplayItem::PushClip { rect, .. } => Some(grown(rect, 1.0)),
        DisplayItem::PushClipPath { segments, .. } => {
            let mut points = segments.iter().flat_map(|segment| match *segment {
                PathSegment::Move(x, y) | PathSegment::Line(x, y) => vec![(x, y)],
                PathSegment::Cubic(x1, y1, x2, y2, x, y) => vec![(x1, y1), (x2, y2), (x, y)],
                PathSegment::Close => Vec::new(),
            });
            let first = points.next()?;
            let (left, top, right, bottom) = points.fold(
                (first.0, first.1, first.0, first.1),
                |(left, top, right, bottom), (x, y)| {
                    (left.min(x), top.min(y), right.max(x), bottom.max(y))
                },
            );
            Some(grown(
                &Rect::new(left, top, right - left, bottom - top),
                1.0,
            ))
        }
        DisplayItem::PopClip => Some(Rect::default()),
        DisplayItem::Child(_)
        | DisplayItem::Path { .. }
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform
        | DisplayItem::PushOpacity(_)
        | DisplayItem::PopOpacity
        | DisplayItem::PushFilter(_)
        | DisplayItem::PopFilter
        | DisplayItem::PushMask(_)
        | DisplayItem::PopMask
        | DisplayItem::Beneath
        | DisplayItem::Backdrop { .. } => None,
    }
}

/// Whether two rectangles share some area.
fn overlap(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width
        && right.x < left.x + left.width
        && left.y < right.y + right.height
        && right.y < left.y + left.height
}

/// The smallest rectangle around both; an empty one adds nothing.
fn union(left: Rect, right: Rect) -> Rect {
    let empty = |rect: &Rect| rect.width <= 0.0 || rect.height <= 0.0;
    if empty(&left) {
        return right;
    }
    if empty(&right) {
        return left;
    }
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    Rect {
        x,
        y,
        width: (left.x + left.width).max(right.x + right.width) - x,
        height: (left.y + left.height).max(right.y + right.height) - y,
    }
}

#[cfg(test)]
mod tests;
