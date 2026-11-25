//! `clip-path`: the shape that clips an element and its descendants.

use super::{fit_radii, radii};
use crate::prelude::*;
use crate::svg;
use crate::tree::BoxGeometry;
use nativeboat_css::ComputedStyle;
use nativeboat_css::types::{
    BackgroundPosition, BasicShape, ClipPath, FillRule, GeometryBox, ShapeRadius,
};
use nativeboat_dom::Dom;
use nativeboat_ir::{Corner, PathSegment, Radii, Rect};

/// What a clip clips to.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ClipShape {
    /// A rectangle with rounded corners.
    Rounded(Rect, Radii),
    /// Any outline, with the rule that decides which points it covers.
    Path(Vec<PathSegment>, FillRule),
}

/// The shape an element's `clip-path` clips it to, where `border`, its
/// border box as painted, is; `None` when nothing clips, as when a `url()`
/// names no `<clipPath>` in the document.
pub(super) fn clip_path(
    style: &ComputedStyle,
    geometry: &BoxGeometry,
    border: Rect,
    dom: &Dom,
) -> Option<ClipShape> {
    let reference = |kind: GeometryBox| {
        let (dx, dy) = (border.x - geometry.rect.x, border.y - geometry.rect.y);
        let moved = |rect: Rect| Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height);
        match kind {
            GeometryBox::PaddingBox => moved(geometry.padding_box()),
            GeometryBox::ContentBox => moved(geometry.content_box()),
            GeometryBox::MarginBox => {
                let [top, right, bottom, left] = geometry.margin;
                Rect::new(
                    border.x - left,
                    border.y - top,
                    border.width + left + right,
                    border.height + top + bottom,
                )
            }
            // SVG's boxes are the border box of other elements.
            _ => border,
        }
    };
    match &style.clip_path {
        ClipPath::None => None,
        // Only clip paths in the document itself.
        ClipPath::Url(url) => {
            let (segments, rule) = svg::clip_path_shape(dom, url.strip_prefix('#')?, border)?;
            Some(ClipShape::Path(segments, rule))
        }
        ClipPath::Box(kind) => {
            let rect = reference(*kind);
            // The border box's rounded corners, shrunk or grown with it.
            let outer = radii(style, border);
            let insets = [
                rect.y - border.y,
                border.x + border.width - rect.x - rect.width,
                border.y + border.height - rect.y - rect.height,
                rect.x - border.x,
            ];
            // Each corner's sides, top left first: the one down from it,
            // then the one across.
            const SIDES: [(usize, usize); 4] = [(0, 3), (0, 1), (2, 1), (2, 3)];
            let radii = core::array::from_fn(|corner| {
                let (down, across) = SIDES[corner];
                if outer[corner].is_square() {
                    Corner::SQUARE
                } else {
                    outer[corner].less(insets[across], insets[down])
                }
            });
            Some(ClipShape::Rounded(rect, radii))
        }
        ClipPath::Shape(shape, kind) => Some(shape_clip(shape, reference(*kind))),
    }
}

/// A basic shape in its reference box.
fn shape_clip(shape: &BasicShape, rect: Rect) -> ClipShape {
    let center = |at: &BackgroundPosition| {
        (
            rect.x + at.x.resolve(rect.width),
            rect.y + at.y.resolve(rect.height),
        )
    };
    // A radius along one axis, from the center at distances `near` and
    // `far` from the box's sides on that axis.
    let along = |radius: &ShapeRadius, near: f32, far: f32, basis: f32| match radius {
        ShapeRadius::Length(length) => length.resolve(basis).max(0.0),
        ShapeRadius::ClosestSide => near.min(far),
        ShapeRadius::FarthestSide => near.max(far),
    };
    match shape {
        BasicShape::Inset(offsets, corners) => {
            let top = offsets[0].resolve(rect.height);
            let right = offsets[1].resolve(rect.width);
            let bottom = offsets[2].resolve(rect.height);
            let left = offsets[3].resolve(rect.width);
            let inner = Rect::new(
                rect.x + left,
                rect.y + top,
                (rect.width - left - right).max(0.0),
                (rect.height - top - bottom).max(0.0),
            );
            let radii = fit_radii([&corners[0], &corners[1], &corners[2], &corners[3]], inner);
            ClipShape::Rounded(inner, radii)
        }
        BasicShape::Circle(radius, at) => {
            let (cx, cy) = center(at);
            let (left, right) = ((cx - rect.x).abs(), (rect.x + rect.width - cx).abs());
            let (top, bottom) = ((cy - rect.y).abs(), (rect.y + rect.height - cy).abs());
            // Percentages are of the box's normalized diagonal.
            let diagonal =
                crate::math::sqrt((rect.width * rect.width + rect.height * rect.height) / 2.0);
            let r = match radius {
                ShapeRadius::Length(length) => length.resolve(diagonal).max(0.0),
                ShapeRadius::ClosestSide => left.min(right).min(top).min(bottom),
                ShapeRadius::FarthestSide => left.max(right).max(top).max(bottom),
            };
            ClipShape::Rounded(
                Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r),
                [Corner::round(r); 4],
            )
        }
        BasicShape::Ellipse(rx, ry, at) => {
            let (cx, cy) = center(at);
            let rx = along(
                rx,
                (cx - rect.x).abs(),
                (rect.x + rect.width - cx).abs(),
                rect.width,
            );
            let ry = along(
                ry,
                (cy - rect.y).abs(),
                (rect.y + rect.height - cy).abs(),
                rect.height,
            );
            if rx == ry {
                ClipShape::Rounded(
                    Rect::new(cx - rx, cy - ry, 2.0 * rx, 2.0 * ry),
                    [Corner::round(rx); 4],
                )
            } else {
                ClipShape::Path(svg::ellipse(cx, cy, rx, ry), FillRule::NonZero)
            }
        }
        BasicShape::Polygon(even_odd, points) => {
            let mut segments: Vec<PathSegment> = points
                .iter()
                .enumerate()
                .map(|(index, (x, y))| {
                    let (x, y) = (
                        rect.x + x.resolve(rect.width),
                        rect.y + y.resolve(rect.height),
                    );
                    if index == 0 {
                        PathSegment::Move(x, y)
                    } else {
                        PathSegment::Line(x, y)
                    }
                })
                .collect();
            segments.push(PathSegment::Close);
            ClipShape::Path(segments, rule(*even_odd))
        }
        BasicShape::Path(even_odd, data) => ClipShape::Path(
            svg::transform_segments([1.0, 0.0, 0.0, 1.0, rect.x, rect.y], &svg::path_data(data)),
            rule(*even_odd),
        ),
    }
}

fn rule(even_odd: bool) -> FillRule {
    if even_odd {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

/// The smallest rectangle around a path's points and control points.
pub(super) fn path_bounds(segments: &[PathSegment]) -> Rect {
    let mut bounds = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    let mut add = |x: f32, y: f32| {
        bounds = (
            bounds.0.min(x),
            bounds.1.min(y),
            bounds.2.max(x),
            bounds.3.max(y),
        );
    };
    for segment in segments {
        match *segment {
            PathSegment::Move(x, y) | PathSegment::Line(x, y) => add(x, y),
            PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                add(x1, y1);
                add(x2, y2);
                add(x, y);
            }
            PathSegment::Close => {}
        }
    }
    if bounds.0 > bounds.2 {
        return Rect::default();
    }
    Rect::new(bounds.0, bounds.1, bounds.2 - bounds.0, bounds.3 - bounds.1)
}
