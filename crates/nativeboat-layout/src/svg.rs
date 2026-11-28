//! Inline SVG: shapes, paths and text of an `<svg>` element become vector
//! path and text display items in the element's content box, and their
//! geometry answers `getBBox()`.
//!
//! Fill and stroke come from computed styles, where SVG presentation
//! attributes take part in the cascade. Markers are drawn at the vertices
//! of paths, lines and polylines. Gradients, patterns, clipping paths,
//! masks and filters are not drawn.

use crate::prelude::*;

use crate::styles::Styles;
use crate::text::{FontMetrics, TextMeasurer, font_of};
use core::f32::consts::{FRAC_PI_2, TAU};
use nativeboat_css::types::{Display, FillRule, SvgBaseline, TextAnchor, Visibility};
use nativeboat_css::{
    ComputedStyle, Origin, SheetRef, StyleEnvironment, Stylesheet, parse_stylesheet,
    user_agent_stylesheet,
};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::{Corner, DisplayItem, PathSegment, Rect, Stroke};

/// An affine map `[a, b, c, d, e, f]`: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
pub type Matrix = [f32; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// How deeply `<use>` may nest, which also stops reference cycles.
const MAX_DEPTH: usize = 16;

/// Applies `inner` first, then `outer`.
pub fn multiply(outer: Matrix, inner: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = outer;
    let [g, h, i, j, k, l] = inner;
    [
        a * g + c * h,
        b * g + d * h,
        a * i + c * j,
        b * i + d * j,
        a * k + c * l + e,
        b * k + d * l + f,
    ]
}

fn translate(x: f32, y: f32) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// Reads the numbers of SVG attribute syntax.
struct Numbers<'a> {
    text: &'a [u8],
    index: usize,
}

impl<'a> Numbers<'a> {
    fn new(text: &'a str) -> Numbers<'a> {
        Numbers {
            text: text.as_bytes(),
            index: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.index).copied()
    }

    /// Skips whitespace and commas.
    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r' | b',')) {
            self.index += 1;
        }
    }

    fn digits(&mut self) -> bool {
        let start = self.index;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.index += 1;
        }
        self.index > start
    }

    /// The next number, such as `-1.5e3` or `.5`.
    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.index;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.index += 1;
        }
        let whole = self.digits();
        let fraction = self.peek() == Some(b'.') && {
            self.index += 1;
            self.digits()
        };
        if !whole && !fraction {
            self.index = start;
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mark = self.index;
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !self.digits() {
                self.index = mark;
            }
        }
        core::str::from_utf8(&self.text[start..self.index])
            .ok()?
            .parse()
            .ok()
    }

    fn pair(&mut self) -> Option<(f32, f32)> {
        Some((self.number()?, self.number()?))
    }

    /// An arc flag, which may be written without separators.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let flag = match self.peek()? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.index += 1;
        Some(flag)
    }

    /// Every number of a list such as `points` or `viewBox`.
    fn all(text: &str) -> Vec<f32> {
        let mut numbers = Numbers::new(text);
        core::iter::from_fn(|| numbers.number()).collect()
    }
}

/// Every number of an attribute such as `viewBox` or `points`.
pub fn numbers(text: &str) -> Vec<f32> {
    Numbers::all(text)
}

/// Appends an elliptical arc as cubic curves (SVG implementation notes F.6).
fn arc(
    segments: &mut Vec<PathSegment>,
    from: (f32, f32),
    radii: (f32, f32),
    rotation: f32,
    large: bool,
    sweep: bool,
    to: (f32, f32),
) {
    if from == to {
        return;
    }
    let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
    if rx == 0.0 || ry == 0.0 {
        segments.push(PathSegment::Line(to.0, to.1));
        return;
    }
    let (sin, cos) = crate::math::sin_cos(rotation.to_radians());
    let (dx, dy) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= crate::math::sqrt(lambda);
        ry *= crate::math::sqrt(lambda);
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coefficient = crate::math::sqrt((numerator / denominator).max(0.0));
    if large == sweep {
        coefficient = -coefficient;
    }
    let (cx1, cy1) = (coefficient * rx * y1 / ry, -coefficient * ry * x1 / rx);
    let center = (
        cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0,
        sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0,
    );
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let cosine =
            (ux * vx + uy * vy) / crate::math::sqrt((ux * ux + uy * uy) * (vx * vx + vy * vy));
        let angle = crate::math::acos(cosine.clamp(-1.0, 1.0));
        if ux * vy - uy * vx < 0.0 {
            -angle
        } else {
            angle
        }
    };
    let (ux, uy) = ((x1 - cx1) / rx, (y1 - cy1) / ry);
    let (vx, vy) = ((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let start = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= TAU;
    } else if sweep && delta < 0.0 {
        delta += TAU;
    }
    let count = crate::math::ceil(delta.abs() / FRAC_PI_2).max(1.0) as usize;
    let step = delta / count as f32;
    let k = 4.0 / 3.0 * crate::math::tan(step / 4.0);
    let point = |x: f32, y: f32| {
        (
            center.0 + rx * x * cos - ry * y * sin,
            center.1 + rx * x * sin + ry * y * cos,
        )
    };
    for index in 0..count {
        let first = start + step * index as f32;
        let (s1, c1) = crate::math::sin_cos(first);
        let (s2, c2) = crate::math::sin_cos(first + step);
        let one = point(c1 - k * s1, s1 + k * c1);
        let two = point(c2 + k * s2, s2 - k * c2);
        let end = if index + 1 == count {
            to
        } else {
            point(c2, s2)
        };
        segments.push(PathSegment::Cubic(one.0, one.1, two.0, two.1, end.0, end.1));
    }
}

/// Parses SVG path data; drawing stops at the first error, as in browsers.
pub fn path_data(data: &str) -> Vec<PathSegment> {
    let mut numbers = Numbers::new(data);
    let mut segments = Vec::new();
    let (mut current, mut start) = ((0.0_f32, 0.0_f32), (0.0_f32, 0.0_f32));
    // The second control point of the previous cubic or quadratic curve.
    let mut cubic: Option<(f32, f32)> = None;
    let mut quadratic: Option<(f32, f32)> = None;
    let mut command: Option<u8> = None;
    loop {
        numbers.skip();
        let Some(byte) = numbers.peek() else {
            break;
        };
        if byte.is_ascii_alphabetic() {
            command = Some(byte);
            numbers.index += 1;
        }
        let Some(letter) = command else {
            break;
        };
        let base = if letter.is_ascii_lowercase() {
            current
        } else {
            (0.0, 0.0)
        };
        let offset = |(x, y): (f32, f32)| (base.0 + x, base.1 + y);
        let reflect = |control: Option<(f32, f32)>| {
            control.map_or(current, |(x, y)| (2.0 * current.0 - x, 2.0 * current.1 - y))
        };
        let (mut next_cubic, mut next_quadratic) = (None, None);
        match letter.to_ascii_uppercase() {
            b'M' => {
                let Some(point) = numbers.pair().map(offset) else {
                    break;
                };
                segments.push(PathSegment::Move(point.0, point.1));
                (current, start) = (point, point);
                // Further pairs are lines.
                command = Some(if letter == b'm' { b'l' } else { b'L' });
            }
            b'L' => {
                let Some(point) = numbers.pair().map(offset) else {
                    break;
                };
                segments.push(PathSegment::Line(point.0, point.1));
                current = point;
            }
            b'H' => {
                let Some(x) = numbers.number() else {
                    break;
                };
                current = (base.0 + x, current.1);
                segments.push(PathSegment::Line(current.0, current.1));
            }
            b'V' => {
                let Some(y) = numbers.number() else {
                    break;
                };
                current = (current.0, base.1 + y);
                segments.push(PathSegment::Line(current.0, current.1));
            }
            b'C' | b'S' => {
                let first = if letter.eq_ignore_ascii_case(&b'C') {
                    let Some(point) = numbers.pair().map(offset) else {
                        break;
                    };
                    point
                } else {
                    reflect(cubic)
                };
                let (Some(second), Some(end)) =
                    (numbers.pair().map(offset), numbers.pair().map(offset))
                else {
                    break;
                };
                segments.push(PathSegment::Cubic(
                    first.0, first.1, second.0, second.1, end.0, end.1,
                ));
                (current, next_cubic) = (end, Some(second));
            }
            b'Q' | b'T' => {
                let control = if letter.eq_ignore_ascii_case(&b'Q') {
                    let Some(point) = numbers.pair().map(offset) else {
                        break;
                    };
                    point
                } else {
                    reflect(quadratic)
                };
                let Some(end) = numbers.pair().map(offset) else {
                    break;
                };
                let third = |from: (f32, f32)| {
                    (
                        from.0 + 2.0 / 3.0 * (control.0 - from.0),
                        from.1 + 2.0 / 3.0 * (control.1 - from.1),
                    )
                };
                let (one, two) = (third(current), third(end));
                segments.push(PathSegment::Cubic(one.0, one.1, two.0, two.1, end.0, end.1));
                (current, next_quadratic) = (end, Some(control));
            }
            b'A' => {
                let (Some(radii), Some(rotation), Some(large), Some(sweep), Some(end)) = (
                    numbers.pair(),
                    numbers.number(),
                    numbers.flag(),
                    numbers.flag(),
                    numbers.pair().map(offset),
                ) else {
                    break;
                };
                arc(&mut segments, current, radii, rotation, large, sweep, end);
                current = end;
            }
            b'Z' => {
                segments.push(PathSegment::Close);
                current = start;
                command = None;
            }
            _ => break,
        }
        (cubic, quadratic) = (next_cubic, next_quadratic);
    }
    segments
}

/// Parses a `transform` attribute; an invalid list is ignored.
pub fn transform_list(text: &str) -> Matrix {
    let mut matrix = IDENTITY;
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some((name, after)) = rest.split_once('(') else {
            return IDENTITY;
        };
        let Some((arguments, after)) = after.split_once(')') else {
            return IDENTITY;
        };
        let values = Numbers::all(arguments);
        let tangent = |degrees: f32| crate::math::tan(degrees.to_radians());
        let next = match (name.trim(), values.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => [*a, *b, *c, *d, *e, *f],
            ("translate", [x]) => translate(*x, 0.0),
            ("translate", [x, y]) => translate(*x, *y),
            ("scale", [x]) => [*x, 0.0, 0.0, *x, 0.0, 0.0],
            ("scale", [x, y]) => [*x, 0.0, 0.0, *y, 0.0, 0.0],
            ("rotate", [angle, around @ ..]) if matches!(around.len(), 0 | 2) => {
                let (sin, cos) = crate::math::sin_cos(angle.to_radians());
                let (x, y) = around
                    .first()
                    .zip(around.get(1))
                    .map_or((0.0, 0.0), |(x, y)| (*x, *y));
                multiply(
                    multiply(translate(x, y), [cos, sin, -sin, cos, 0.0, 0.0]),
                    translate(-x, -y),
                )
            }
            ("skewX", [angle]) => [1.0, 0.0, tangent(*angle), 1.0, 0.0, 0.0],
            ("skewY", [angle]) => [1.0, tangent(*angle), 0.0, 1.0, 0.0, 0.0],
            _ => return IDENTITY,
        };
        matrix = multiply(matrix, next);
        rest = after.trim_start_matches([' ', ',', '\t', '\n', '\r']);
    }
    matrix
}

/// Maps a `viewBox` into a viewport, honoring `preserveAspectRatio`.
pub fn view_box(view_box: Option<&str>, aspect: Option<&str>, viewport: Rect) -> Matrix {
    let origin = translate(viewport.x, viewport.y);
    let Some([x, y, width, height]) = view_box
        .map(Numbers::all)
        .and_then(|values| <[f32; 4]>::try_from(values).ok())
        .filter(|[_, _, width, height]| *width > 0.0 && *height > 0.0)
    else {
        return origin;
    };
    let (scale_x, scale_y) = (viewport.width / width, viewport.height / height);
    let mut words = aspect.unwrap_or("").split_whitespace();
    let align = words.next().unwrap_or("xMidYMid");
    if align == "none" {
        return multiply(
            origin,
            [scale_x, 0.0, 0.0, scale_y, -x * scale_x, -y * scale_y],
        );
    }
    let scale = if words.next() == Some("slice") {
        scale_x.max(scale_y)
    } else {
        scale_x.min(scale_y)
    };
    let place = |axis: &str, free: f32| {
        if align.contains(&format!("{axis}Min")) {
            0.0
        } else if align.contains(&format!("{axis}Max")) {
            free
        } else {
            free / 2.0
        }
    };
    let dx = place("x", viewport.width - width * scale);
    let dy = place("Y", viewport.height - height * scale);
    multiply(
        origin,
        [scale, 0.0, 0.0, scale, dx - x * scale, dy - y * scale],
    )
}

/// The size of the viewport that percentages refer to, in user units.
#[derive(Debug, Clone, Copy)]
struct Viewport(f32, f32);

/// A length attribute in user units; percentages refer to the viewport and
/// missing or invalid values are zero.
fn length(dom: &Dom, node: NodeId, name: &str, viewport: Viewport) -> f32 {
    let Some(value) = dom.get_attribute(node, name).map(str::trim) else {
        return 0.0;
    };
    let number = Numbers::all(value).first().copied().unwrap_or(0.0);
    if !value.ends_with('%') {
        return number;
    }
    let Viewport(width, height) = viewport;
    let basis = match name {
        "x" | "width" | "cx" | "rx" | "x1" | "x2" => width,
        "y" | "height" | "cy" | "ry" | "y1" | "y2" => height,
        _ => crate::math::sqrt((width * width + height * height) / 2.0),
    };
    number / 100.0 * basis
}

/// The size of a `viewBox`, or of the viewport when there is none.
fn view_box_size(view_box: Option<&str>, viewport: Viewport) -> Viewport {
    match view_box.map(Numbers::all).as_deref() {
        Some([_, _, width, height]) if *width > 0.0 && *height > 0.0 => Viewport(*width, *height),
        _ => viewport,
    }
}

/// A path's segments through an affine map.
pub fn transform_segments(matrix: Matrix, segments: &[PathSegment]) -> Vec<PathSegment> {
    let [a, b, c, d, e, f] = matrix;
    let point = |x: f32, y: f32| (a * x + c * y + e, b * x + d * y + f);
    segments
        .iter()
        .map(|segment| match *segment {
            PathSegment::Move(x, y) => {
                let (x, y) = point(x, y);
                PathSegment::Move(x, y)
            }
            PathSegment::Line(x, y) => {
                let (x, y) = point(x, y);
                PathSegment::Line(x, y)
            }
            PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                let ((x1, y1), (x2, y2), (x, y)) = (point(x1, y1), point(x2, y2), point(x, y));
                PathSegment::Cubic(x1, y1, x2, y2, x, y)
            }
            PathSegment::Close => PathSegment::Close,
        })
        .collect()
}

/// The shape of the `<clipPath>` with the id `id` for an element whose
/// reference box is `reference`: its shapes' outlines through their
/// transforms, with `userSpaceOnUse` units from the box's corner and
/// `objectBoundingBox` units scaled to the box. Even-odd when every shape
/// asks for it. `None` when there is no such element.
pub fn clip_path_shape(
    dom: &Dom,
    id: &str,
    reference: Rect,
) -> Option<(Vec<PathSegment>, FillRule)> {
    let clip = dom.get_element_by_id(Dom::DOCUMENT, id)?;
    if dom.local_name(clip) != Some("clipPath") {
        return None;
    }
    let bounding = dom.get_attribute(clip, "clipPathUnits") == Some("objectBoundingBox");
    let units = if bounding {
        [
            reference.width,
            0.0,
            0.0,
            reference.height,
            reference.x,
            reference.y,
        ]
    } else {
        translate(reference.x, reference.y)
    };
    let outer = multiply(
        units,
        transform_list(dom.get_attribute(clip, "transform").unwrap_or("")),
    );
    let viewport = if bounding {
        Viewport(1.0, 1.0)
    } else {
        Viewport(reference.width, reference.height)
    };
    let mut segments = Vec::new();
    let mut even_odd = true;
    for child in dom.child_elements(clip) {
        let name = dom.local_name(child).unwrap_or("");
        let outline = shape(dom, child, name, viewport);
        if outline.is_empty() {
            continue;
        }
        let matrix = multiply(
            outer,
            transform_list(dom.get_attribute(child, "transform").unwrap_or("")),
        );
        segments.extend(transform_segments(matrix, &outline));
        even_odd &= dom.get_attribute(child, "clip-rule") == Some("evenodd");
    }
    let rule = if even_odd && !segments.is_empty() {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    };
    Some((segments, rule))
}

/// The closed outline of an ellipse.
pub(crate) fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<PathSegment> {
    if rx <= 0.0 || ry <= 0.0 {
        return Vec::new();
    }
    let k = 0.552_284_8;
    let (kx, ky) = (rx * k, ry * k);
    vec![
        PathSegment::Move(cx + rx, cy),
        PathSegment::Cubic(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry),
        PathSegment::Cubic(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy),
        PathSegment::Cubic(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry),
        PathSegment::Cubic(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy),
        PathSegment::Close,
    ]
}

/// The outline of a `<rect>`, with corners rounded by `rx` and `ry`.
fn rectangle(dom: &Dom, node: NodeId, viewport: Viewport) -> Vec<PathSegment> {
    let length = |name| length(dom, node, name, viewport);
    let (x, y, width, height) = (length("x"), length("y"), length("width"), length("height"));
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let given = |name| dom.get_attribute(node, name).map(|_| length(name));
    let (rx, ry) = match (given("rx"), given("ry")) {
        (Some(rx), Some(ry)) => (rx, ry),
        (Some(radius), None) | (None, Some(radius)) => (radius, radius),
        (None, None) => (0.0, 0.0),
    };
    let (rx, ry) = (rx.clamp(0.0, width / 2.0), ry.clamp(0.0, height / 2.0));
    let (right, bottom) = (x + width, y + height);
    if rx == 0.0 || ry == 0.0 {
        return vec![
            PathSegment::Move(x, y),
            PathSegment::Line(right, y),
            PathSegment::Line(right, bottom),
            PathSegment::Line(x, bottom),
            PathSegment::Close,
        ];
    }
    let (kx, ky) = (rx * 0.552_284_8, ry * 0.552_284_8);
    vec![
        PathSegment::Move(x + rx, y),
        PathSegment::Line(right - rx, y),
        PathSegment::Cubic(right - rx + kx, y, right, y + ry - ky, right, y + ry),
        PathSegment::Line(right, bottom - ry),
        PathSegment::Cubic(
            right,
            bottom - ry + ky,
            right - rx + kx,
            bottom,
            right - rx,
            bottom,
        ),
        PathSegment::Line(x + rx, bottom),
        PathSegment::Cubic(x + rx - kx, bottom, x, bottom - ry + ky, x, bottom - ry),
        PathSegment::Line(x, y + ry),
        PathSegment::Cubic(x, y + ry - ky, x + rx - kx, y, x + rx, y),
        PathSegment::Close,
    ]
}

/// The outline of `<polyline>` and `<polygon>` points.
fn polyline(dom: &Dom, node: NodeId, closed: bool) -> Vec<PathSegment> {
    let values = Numbers::all(dom.get_attribute(node, "points").unwrap_or(""));
    let (points, _) = values.as_chunks::<2>();
    let mut segments: Vec<PathSegment> = points
        .iter()
        .enumerate()
        .map(|(index, &[x, y])| {
            if index == 0 {
                PathSegment::Move(x, y)
            } else {
                PathSegment::Line(x, y)
            }
        })
        .collect();
    if closed && !segments.is_empty() {
        segments.push(PathSegment::Close);
    }
    segments
}

/// The segments of a basic shape or path element.
fn shape(dom: &Dom, node: NodeId, name: &str, viewport: Viewport) -> Vec<PathSegment> {
    let length = |name| length(dom, node, name, viewport);
    match name {
        "path" => path_data(dom.get_attribute(node, "d").unwrap_or("")),
        "rect" => rectangle(dom, node, viewport),
        "circle" => ellipse(length("cx"), length("cy"), length("r"), length("r")),
        "ellipse" => ellipse(length("cx"), length("cy"), length("rx"), length("ry")),
        "line" => vec![
            PathSegment::Move(length("x1"), length("y1")),
            PathSegment::Line(length("x2"), length("y2")),
        ],
        "polyline" => polyline(dom, node, false),
        "polygon" => polyline(dom, node, true),
        _ => Vec::new(),
    }
}

/// A vertex of a path where markers are drawn, with the directions of the
/// path into and out of it, in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vertex {
    point: (f32, f32),
    into: Option<f32>,
    out: Option<f32>,
}

impl Vertex {
    /// The direction `orient="auto"` turns a marker to: along the path at
    /// its ends, halfway between the directions at other vertices.
    fn angle(&self, first: bool, last: bool) -> f32 {
        match (self.into, self.out) {
            (Some(into), _) if last => into,
            (_, Some(out)) if first => out,
            (Some(into), Some(out)) => {
                let ((into_sin, into_cos), (out_sin, out_cos)) =
                    (crate::math::sin_cos(into), crate::math::sin_cos(out));
                crate::math::atan2(into_sin + out_sin, into_cos + out_cos)
            }
            (Some(direction), None) | (None, Some(direction)) => direction,
            (None, None) => 0.0,
        }
    }
}

/// The dash pattern of a `stroke-dasharray`: an odd list repeats to make
/// it even, and one with no length draws solid.
fn dashes(lengths: &[f32]) -> Vec<f32> {
    if lengths.iter().sum::<f32>() <= 0.0 {
        Vec::new()
    } else if lengths.len() % 2 == 1 {
        lengths.repeat(2)
    } else {
        lengths.to_vec()
    }
}

/// The vertices of a path: the start of each subpath and the end of each
/// segment.
fn vertices(segments: &[PathSegment]) -> Vec<Vertex> {
    let direction = |from: (f32, f32), to: (f32, f32)| {
        (from != to).then(|| crate::math::atan2(to.1 - from.1, to.0 - from.0))
    };
    let mut found: Vec<Vertex> = Vec::new();
    let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
    let leave = |found: &mut Vec<Vertex>, out: Option<f32>| {
        if let Some(last) = found.last_mut()
            && last.out.is_none()
        {
            last.out = out;
        }
    };
    for segment in segments {
        let (to, out, into) = match *segment {
            PathSegment::Move(x, y) => {
                found.push(Vertex {
                    point: (x, y),
                    into: None,
                    out: None,
                });
                current = (x, y);
                start = (x, y);
                continue;
            }
            PathSegment::Line(x, y) => {
                let direction = direction(current, (x, y));
                ((x, y), direction, direction)
            }
            PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                let out = direction(current, (x1, y1))
                    .or_else(|| direction(current, (x2, y2)))
                    .or_else(|| direction(current, (x, y)));
                let into = direction((x2, y2), (x, y))
                    .or_else(|| direction((x1, y1), (x, y)))
                    .or_else(|| direction(current, (x, y)));
                ((x, y), out, into)
            }
            PathSegment::Close => {
                let direction = direction(current, start);
                (start, direction, direction)
            }
        };
        leave(&mut found, out);
        found.push(Vertex {
            point: to,
            into,
            out: None,
        });
        current = to;
    }
    found
}

/// Walks an SVG subtree and collects its painted shapes, or with
/// `geometry` the outlines of every shape whether painted or not.
struct Painter<'a> {
    dom: &'a Dom,
    styles: &'a Styles,
    items: &'a mut Vec<DisplayItem>,
    viewport: Viewport,
    measurer: Option<&'a dyn TextMeasurer>,
    geometry: bool,
}

/// A piece of SVG text at its baseline position, in user units.
struct Run {
    text: String,
    node: NodeId,
    x: f32,
    y: f32,
    width: f32,
}

impl Painter<'_> {
    fn children(&mut self, node: NodeId, transform: Matrix, opacity: f32, depth: usize) {
        for child in self.dom.child_elements(node) {
            self.element(child, transform, opacity, depth);
        }
    }

    fn element(&mut self, node: NodeId, transform: Matrix, opacity: f32, depth: usize) {
        let Some(style) = self.styles.get(node) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        let transform = self
            .dom
            .get_attribute(node, "transform")
            .map_or(transform, |text| multiply(transform, transform_list(text)));
        self.contents(node, style, transform, opacity * style.opacity, depth);
    }

    /// Draws an element in user space mapped by `transform`, which already
    /// includes the element's own transform.
    fn contents(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        transform: Matrix,
        opacity: f32,
        depth: usize,
    ) {
        let dom = self.dom;
        let name = dom.local_name(node).unwrap_or("");
        match name {
            "g" | "a" | "switch" => self.children(node, transform, opacity, depth),
            "text" => self.text(node, transform, opacity),
            "foreignObject" | "image" if self.geometry => {
                let viewport = self.viewport;
                let length = |name| length(dom, node, name, viewport);
                let (x, y) = (length("x"), length("y"));
                let (width, height) = (length("width"), length("height"));
                // A zero width or height disables rendering, and Chromium's
                // bounding boxes leave such elements out.
                if width <= 0.0 || height <= 0.0 {
                    return;
                }
                let (right, bottom) = (x + width, y + height);
                self.items.push(DisplayItem::Path {
                    segments: vec![
                        PathSegment::Move(x, y),
                        PathSegment::Line(right, y),
                        PathSegment::Line(right, bottom),
                        PathSegment::Line(x, bottom),
                        PathSegment::Close,
                    ],
                    transform,
                    fill: None,
                    fill_rule: style.fill_rule,
                    stroke: None,
                });
            }
            "svg" => {
                let viewport = self.viewport;
                let length = |name| length(dom, node, name, viewport);
                let area = Rect::new(
                    length("x"),
                    length("y"),
                    self.size(node, "width", viewport.0),
                    self.size(node, "height", viewport.1),
                );
                self.nested(node, area, transform, opacity, depth);
            }
            "use" => self.reference(node, transform, opacity, depth),
            _ => {
                let segments = shape(dom, node, name, self.viewport);
                // Rectangles, circles and ellipses without area render
                // nothing and are left out of bounding boxes, as Chromium's
                // empty shapes are; lines and paths count.
                if self.geometry
                    && matches!(name, "rect" | "circle" | "ellipse")
                    && outline_bounds(&[DisplayItem::Path {
                        segments: segments.clone(),
                        transform: IDENTITY,
                        fill: None,
                        fill_rule: style.fill_rule,
                        stroke: None,
                    }])
                    .is_none_or(|bounds| bounds.width <= 0.0 || bounds.height <= 0.0)
                {
                    return;
                }
                let markers = (!self.geometry
                    && matches!(name, "path" | "line" | "polyline" | "polygon"))
                .then(|| vertices(&segments));
                self.draw(style, segments, transform, opacity);
                if let Some(vertices) = markers {
                    self.markers(style, &vertices, transform, opacity, depth);
                }
            }
        }
    }

    /// Draws the markers an element's style names at its vertices.
    fn markers(
        &mut self,
        style: &ComputedStyle,
        vertices: &[Vertex],
        transform: Matrix,
        opacity: f32,
        depth: usize,
    ) {
        let Some(last) = vertices.len().checked_sub(1) else {
            return;
        };
        if depth >= MAX_DEPTH {
            return;
        }
        let dom = self.dom;
        for (url, range) in [
            (&style.marker_start, 0..1.min(last + 1)),
            (&style.marker_mid, 1.min(last)..last),
            (&style.marker_end, last..last + 1),
        ] {
            let marker = url
                .0
                .as_deref()
                .and_then(|value| value.trim().strip_prefix('#'))
                .and_then(|id| dom.get_element_by_id(nativeboat_dom::Dom::DOCUMENT, id))
                .filter(|marker| dom.local_name(*marker) == Some("marker"));
            let Some(marker) = marker else {
                continue;
            };
            for index in range {
                let vertex = &vertices[index];
                let angle = match dom.get_attribute(marker, "orient").map(str::trim) {
                    Some("auto") => vertex.angle(index == 0, index == last),
                    Some("auto-start-reverse") => {
                        let angle = vertex.angle(index == 0, index == last);
                        if index == 0 {
                            angle + core::f32::consts::PI
                        } else {
                            angle
                        }
                    }
                    Some(degrees) => numbers(degrees)
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                        .to_radians(),
                    None => 0.0,
                };
                self.marker(
                    marker,
                    vertex.point,
                    angle,
                    style.stroke_width.0,
                    transform,
                    opacity,
                    depth,
                );
            }
        }
    }

    /// Draws a marker's content at a point, turned by an angle.
    #[allow(clippy::too_many_arguments)]
    fn marker(
        &mut self,
        marker: NodeId,
        (x, y): (f32, f32),
        angle: f32,
        stroke_width: f32,
        transform: Matrix,
        opacity: f32,
        depth: usize,
    ) {
        let dom = self.dom;
        let number = |name: &str, default: f32| {
            dom.get_attribute(marker, name)
                .and_then(|value| numbers(value).first().copied())
                .unwrap_or(default)
        };
        let (width, height) = (number("markerWidth", 3.0), number("markerHeight", 3.0));
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let units =
            if dom.get_attribute(marker, "markerUnits").map(str::trim) == Some("userSpaceOnUse") {
                1.0
            } else {
                stroke_width
            };
        let view_box_text = dom.get_attribute(marker, "viewBox");
        let content = view_box(
            view_box_text,
            dom.get_attribute(marker, "preserveAspectRatio"),
            Rect::new(0.0, 0.0, width, height),
        );
        // The reference point, in the marker's content coordinates, sits on
        // the vertex.
        let (ref_x, ref_y) = (number("refX", 0.0), number("refY", 0.0));
        let [a, b, c, d, e, f] = content;
        let reference = (a * ref_x + c * ref_y + e, b * ref_x + d * ref_y + f);
        let (sin, cos) = crate::math::sin_cos(angle);
        let placed = multiply(
            transform,
            multiply(
                [cos * units, sin * units, -sin * units, cos * units, x, y],
                multiply(translate(-reference.0, -reference.1), content),
            ),
        );
        let outer = self.viewport;
        self.viewport = view_box_size(view_box_text, Viewport(width, height));
        self.children(marker, placed, opacity, depth + 1);
        self.viewport = outer;
    }

    /// A `width` or `height` attribute, which defaults to the viewport's.
    fn size(&self, node: NodeId, name: &str, viewport_size: f32) -> f32 {
        if self.dom.get_attribute(node, name).is_some() {
            length(self.dom, node, name, self.viewport)
        } else {
            viewport_size
        }
    }

    /// Paints the children of `node` in a new viewport mapped by its
    /// `viewBox` and `preserveAspectRatio`.
    fn nested(&mut self, node: NodeId, area: Rect, transform: Matrix, opacity: f32, depth: usize) {
        let view_box_text = self.dom.get_attribute(node, "viewBox");
        let inner = view_box(
            view_box_text,
            self.dom.get_attribute(node, "preserveAspectRatio"),
            area,
        );
        let outer = self.viewport;
        self.viewport = view_box_size(view_box_text, Viewport(area.width, area.height));
        self.children(node, multiply(transform, inner), opacity, depth);
        self.viewport = outer;
    }

    /// Draws the element a `<use>` refers to, at the `<use>` position.
    fn reference(&mut self, node: NodeId, transform: Matrix, opacity: f32, depth: usize) {
        let dom = self.dom;
        let target = dom
            .get_attribute(node, "href")
            .or_else(|| dom.get_attribute(node, "xlink:href"))
            .and_then(|href| href.strip_prefix('#'))
            .and_then(|id| dom.get_element_by_id(nativeboat_dom::Dom::DOCUMENT, id));
        let Some(target) = target.filter(|_| depth < MAX_DEPTH) else {
            return;
        };
        let viewport = self.viewport;
        let length = |name| length(dom, node, name, viewport);
        let transform = multiply(transform, translate(length("x"), length("y")));
        if dom.local_name(target) == Some("symbol") {
            let area = Rect::new(
                0.0,
                0.0,
                self.size(node, "width", viewport.0),
                self.size(node, "height", viewport.1),
            );
            self.nested(target, area, transform, opacity, depth + 1);
        } else {
            self.element(target, transform, opacity, depth + 1);
        }
    }

    fn draw(
        &mut self,
        style: &ComputedStyle,
        segments: Vec<PathSegment>,
        transform: Matrix,
        opacity: f32,
    ) {
        if self.geometry && !segments.is_empty() {
            self.items.push(DisplayItem::Path {
                segments,
                transform,
                fill: None,
                fill_rule: style.fill_rule,
                stroke: None,
            });
            return;
        }
        if segments.is_empty() || style.visibility != Visibility::Visible {
            return;
        }
        let faded = |mut color: nativeboat_css::Color, alpha: f32| {
            color.a *= alpha * opacity;
            color
        };
        let fill = style
            .fill
            .resolve(style.color)
            .map(|color| faded(color, style.fill_opacity));
        let stroke = style
            .stroke
            .resolve(style.color)
            .filter(|_| style.stroke_width.0 > 0.0)
            .map(|color| Stroke {
                color: faded(color, style.stroke_opacity),
                width: style.stroke_width.0,
                cap: style.stroke_linecap,
                join: style.stroke_linejoin,
                dashes: dashes(&style.stroke_dasharray.0),
                dash_offset: style.stroke_dashoffset.0,
            });
        if fill.is_none() && stroke.is_none() {
            return;
        }
        self.items.push(DisplayItem::Path {
            segments,
            transform,
            fill,
            fill_rule: style.fill_rule,
            stroke,
        });
    }
}

impl Painter<'_> {
    /// The vertical metrics of a font, measured or estimated.
    fn metrics(&self, font: &nativeboat_ir::Font) -> FontMetrics {
        self.measurer.map_or(
            FontMetrics {
                ascent: font.size * 0.8,
                descent: font.size * 0.2,
            },
            |measurer| measurer.metrics(font),
        )
    }

    /// Draws a `<text>` element: its text and `<tspan>`s laid out from their
    /// positions, each chunk anchored as `text-anchor` says and shifted to
    /// the baseline `dominant-baseline` names.
    fn text(&mut self, node: NodeId, transform: Matrix, opacity: f32) {
        let mut chunks: Vec<(TextAnchor, Vec<Run>)> = Vec::new();
        let mut cursor = (0.0, 0.0);
        let mut space = true;
        self.text_runs(node, &mut cursor, &mut space, &mut chunks);
        // Trailing white space is not drawn.
        if let Some(run) = chunks.last_mut().and_then(|(_, runs)| runs.last_mut())
            && run.text.ends_with(' ')
        {
            run.text.pop();
            run.width = self.width(run.node, &run.text);
        }
        let mut drawn = Vec::new();
        for (anchor, mut runs) in chunks {
            // The chunk's extent, with any `dx` between its runs.
            let width = match (runs.first(), runs.last()) {
                (Some(first), Some(last)) => last.x + last.width - first.x,
                _ => 0.0,
            };
            let shift = match anchor {
                TextAnchor::Start => 0.0,
                TextAnchor::Middle => width / 2.0,
                TextAnchor::End => width,
            };
            for run in &mut runs {
                run.x -= shift;
            }
            drawn.extend(runs);
        }
        let mut pushed = false;
        for run in drawn {
            let Some(style) = self.styles.get(run.node) else {
                continue;
            };
            if run.text.is_empty() || style.display == Display::None {
                continue;
            }
            let font = font_of(style);
            let metrics = self.metrics(&font);
            let baseline = match style.alignment_baseline {
                SvgBaseline::Auto => style.dominant_baseline,
                other => other,
            };
            let lowered = match baseline {
                SvgBaseline::Middle | SvgBaseline::Mathematical => font.size / 4.0,
                SvgBaseline::Central => (metrics.ascent - metrics.descent) / 2.0,
                SvgBaseline::Hanging => metrics.ascent * 0.8,
                SvgBaseline::TextBeforeEdge => metrics.ascent,
                SvgBaseline::TextAfterEdge | SvgBaseline::Ideographic => -metrics.descent,
                SvgBaseline::Auto | SvgBaseline::Alphabetic => 0.0,
            };
            let rect = Rect::new(
                run.x,
                run.y + lowered - metrics.ascent,
                run.width,
                metrics.ascent + metrics.descent,
            );
            if self.geometry {
                let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
                self.items.push(DisplayItem::Path {
                    segments: vec![
                        PathSegment::Move(rect.x, rect.y),
                        PathSegment::Line(right, rect.y),
                        PathSegment::Line(right, bottom),
                        PathSegment::Line(rect.x, bottom),
                        PathSegment::Close,
                    ],
                    transform,
                    fill: None,
                    fill_rule: style.fill_rule,
                    stroke: None,
                });
                continue;
            }
            let Some(mut color) = style.fill.resolve(style.color) else {
                continue;
            };
            if style.visibility != Visibility::Visible {
                continue;
            }
            color.a *= style.fill_opacity * opacity;
            if !pushed {
                self.items.push(DisplayItem::PushTransform(transform));
                pushed = true;
            }
            self.items.push(DisplayItem::Text {
                rect,
                text: run.text,
                font,
                color,
                underline: style.text_decoration_line.underline,
                line_through: style.text_decoration_line.line_through,
                letter_spacing: 0.0,
                rtl: false,
                grayscale: false,
                underline_offset: None,
                decorations: Vec::new(),
            });
        }
        if pushed {
            self.items.push(DisplayItem::PopTransform);
        }
    }

    /// The advance width of text in an element's font.
    fn width(&self, node: NodeId, text: &str) -> f32 {
        let Some(style) = self.styles.get(node) else {
            return 0.0;
        };
        let font = font_of(style);
        let spacing = style.letter_spacing.0 * text.chars().count() as f32;
        self.measurer
            .map_or(font.size * 0.6 * text.chars().count() as f32, |measurer| {
                measurer.width(text, &font)
            })
            + spacing
    }

    /// Collects the runs of a `<text>` or `<tspan>`: an `x` or `y` starts a
    /// new chunk at that position, `dx` and `dy` move the pen, and white
    /// space collapses as SVG's default `xml:space` says.
    fn text_runs(
        &self,
        node: NodeId,
        cursor: &mut (f32, f32),
        space: &mut bool,
        chunks: &mut Vec<(TextAnchor, Vec<Run>)>,
    ) {
        let dom = self.dom;
        let Some(style) = self.styles.get(node) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        // The first of a list of positions, in user units or `em`s.
        let font_size = style.font_size.0;
        let first = |name: &str| {
            let value = dom.get_attribute(node, name)?;
            let first = value
                .split(|character: char| character == ',' || character.is_whitespace())
                .find(|part| !part.is_empty())?;
            let number = Numbers::all(first).first().copied()?;
            Some(if first.ends_with("em") {
                number * font_size
            } else {
                number
            })
        };
        let (x, y) = (first("x"), first("y"));
        if x.is_some() || y.is_some() || chunks.is_empty() {
            cursor.0 = x.unwrap_or(cursor.0);
            cursor.1 = y.unwrap_or(cursor.1);
            chunks.push((style.text_anchor, Vec::new()));
        }
        cursor.0 += first("dx").unwrap_or(0.0);
        cursor.1 += first("dy").unwrap_or(0.0);
        for child in dom.children(node) {
            if dom.is_element(child) {
                if dom.local_name(child) == Some("tspan") {
                    self.text_runs(child, cursor, space, chunks);
                }
                continue;
            }
            let raw = dom.text_content(child).unwrap_or_default();
            let mut text = String::new();
            for character in raw
                .chars()
                .filter(|character| *character != '\n' && *character != '\r')
            {
                let blank = character == ' ' || character == '\t';
                if blank && *space {
                    continue;
                }
                text.push(if blank { ' ' } else { character });
                *space = blank;
            }
            if text.is_empty() {
                continue;
            }
            let width = self.width(node, &text);
            if let Some((_, runs)) = chunks.last_mut() {
                runs.push(Run {
                    text,
                    node,
                    x: cursor.0,
                    y: cursor.1,
                    width,
                });
            }
            cursor.0 += width;
        }
    }
}

/// The bounds of the outlines in display items, mapped by their transforms;
/// curves count by points along them.
fn outline_bounds(items: &[DisplayItem]) -> Option<Rect> {
    let mut extent: Option<(f32, f32, f32, f32)> = None;
    let mut include = |transform: &Matrix, x: f32, y: f32| {
        let [a, b, c, d, e, f] = *transform;
        let (x, y) = (a * x + c * y + e, b * x + d * y + f);
        extent = Some(match extent {
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
            None => (x, y, x, y),
        });
    };
    for item in items {
        let DisplayItem::Path {
            segments,
            transform,
            ..
        } = item
        else {
            continue;
        };
        let mut pen = (0.0, 0.0);
        for segment in segments {
            match *segment {
                PathSegment::Move(x, y) | PathSegment::Line(x, y) => {
                    include(transform, x, y);
                    pen = (x, y);
                }
                PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                    for step in 1..=16 {
                        let t = step as f32 / 16.0;
                        let u = 1.0 - t;
                        let point = |p0: f32, p1: f32, p2: f32, p3: f32| {
                            u * u * u * p0
                                + 3.0 * u * u * t * p1
                                + 3.0 * u * t * t * p2
                                + t * t * t * p3
                        };
                        include(transform, point(pen.0, x1, x2, x), point(pen.1, y1, y2, y));
                    }
                    pen = (x, y);
                }
                PathSegment::Close => {}
            }
        }
    }
    extent.map(|(left, top, right, bottom)| Rect::new(left, top, right - left, bottom - top))
}

/// The bounding box of an SVG element's geometry in its own user space, as
/// `getBBox()` gives it: its shapes, text and descendants, whether painted
/// or not, without its own transform.
pub fn bbox(dom: &Dom, styles: &Styles, node: NodeId, measurer: Option<&dyn TextMeasurer>) -> Rect {
    let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
    let Some(style) = styles
        .get(node)
        .filter(|style| style.display != Display::None)
    else {
        return empty;
    };
    // Percentages refer to the nearest viewport.
    let viewport =
        core::iter::successors(dom.parent_element(node), |node| dom.parent_element(*node))
            .find(|ancestor| dom.local_name(*ancestor) == Some("svg"))
            .map_or(Viewport(300.0, 150.0), |svg| {
                let given = |name: &str| {
                    dom.get_attribute(svg, name)
                        .filter(|value| !value.trim().ends_with('%'))
                        .and_then(|value| Numbers::all(value).first().copied())
                };
                view_box_size(
                    dom.get_attribute(svg, "viewBox"),
                    Viewport(
                        given("width").unwrap_or(300.0),
                        given("height").unwrap_or(150.0),
                    ),
                )
            });
    let mut items = Vec::new();
    let mut painter = Painter {
        dom,
        styles,
        items: &mut items,
        viewport,
        measurer,
        geometry: true,
    };
    if dom.local_name(node) == Some("svg") {
        painter.children(node, IDENTITY, 1.0, 0);
    } else {
        painter.contents(node, style, IDENTITY, 1.0, 0);
    }
    outline_bounds(&items).unwrap_or(empty)
}

/// The `<foreignObject>` elements of an outermost `<svg>` whose content box
/// has a size, with their areas in that box: their `x`, `y`, `width` and
/// `height` mapped by the `viewBox` and the transforms around them.
pub fn foreign_objects(
    dom: &Dom,
    styles: &Styles,
    svg: NodeId,
    size: (f32, f32),
) -> Vec<(NodeId, Rect)> {
    fn walk(
        dom: &Dom,
        styles: &Styles,
        node: NodeId,
        transform: Matrix,
        viewport: Viewport,
        found: &mut Vec<(NodeId, Rect)>,
    ) {
        for child in dom.child_elements(node) {
            if styles
                .get(child)
                .is_none_or(|style| style.display == Display::None)
            {
                continue;
            }
            let transform = dom
                .get_attribute(child, "transform")
                .map_or(transform, |text| multiply(transform, transform_list(text)));
            let length = |name| length(dom, child, name, viewport);
            match dom.local_name(child).unwrap_or("") {
                "foreignObject" => {
                    let (x, y) = (length("x"), length("y"));
                    let (right, bottom) = (x + length("width"), y + length("height"));
                    let [a, b, c, d, e, f] = transform;
                    let points = [(x, y), (right, y), (right, bottom), (x, bottom)]
                        .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
                    let left = points.iter().map(|point| point.0).fold(f32::MAX, f32::min);
                    let top = points.iter().map(|point| point.1).fold(f32::MAX, f32::min);
                    let right = points.iter().map(|point| point.0).fold(f32::MIN, f32::max);
                    let bottom = points.iter().map(|point| point.1).fold(f32::MIN, f32::max);
                    found.push((child, Rect::new(left, top, right - left, bottom - top)));
                }
                "svg" => {
                    let width = dom
                        .get_attribute(child, "width")
                        .map_or(viewport.0, |_| length("width"));
                    let height = dom
                        .get_attribute(child, "height")
                        .map_or(viewport.1, |_| length("height"));
                    let area = Rect::new(length("x"), length("y"), width, height);
                    let view_box_text = dom.get_attribute(child, "viewBox");
                    let inner = view_box(
                        view_box_text,
                        dom.get_attribute(child, "preserveAspectRatio"),
                        area,
                    );
                    let viewport = view_box_size(view_box_text, Viewport(width, height));
                    walk(
                        dom,
                        styles,
                        child,
                        multiply(transform, inner),
                        viewport,
                        found,
                    );
                }
                _ => walk(dom, styles, child, transform, viewport, found),
            }
        }
    }
    let area = Rect::new(0.0, 0.0, size.0, size.1);
    let view_box_text = dom.get_attribute(svg, "viewBox");
    let transform = view_box(
        view_box_text,
        dom.get_attribute(svg, "preserveAspectRatio"),
        area,
    );
    let viewport = view_box_size(view_box_text, Viewport(size.0, size.1));
    let mut found = Vec::new();
    walk(dom, styles, svg, transform, viewport, &mut found);
    found
}

/// Paints an outermost `<svg>` element into its content box.
pub fn paint(
    dom: &Dom,
    styles: &Styles,
    svg: NodeId,
    content: Rect,
    items: &mut Vec<DisplayItem>,
    measurer: Option<&dyn TextMeasurer>,
) {
    items.push(DisplayItem::PushClip {
        rect: content,
        radii: [Corner::SQUARE; 4],
    });
    let area = Rect::new(0.0, 0.0, content.width, content.height);
    let mut painter = Painter {
        dom,
        styles,
        items,
        viewport: Viewport(content.width, content.height),
        measurer,
        geometry: false,
    };
    painter.nested(svg, area, translate(content.x, content.y), 1.0, 0);
    items.push(DisplayItem::PopClip);
}

/// Turns an SVG file into its natural size and the display items that
/// draw it at that size. The file's own `<style>` sheets apply.
pub fn image(source: &str) -> Option<((f32, f32), Vec<DisplayItem>)> {
    let dom = Dom::from_document(&nativeboat_html::parse_document(source));
    let svg = *dom.get_elements_by_tag_name(Dom::DOCUMENT, "svg").first()?;
    let author: Vec<Stylesheet> = dom
        .get_elements_by_tag_name(Dom::DOCUMENT, "style")
        .into_iter()
        .map(|style| parse_stylesheet(&dom.text_content(style).unwrap_or_default()).0)
        .collect();
    let mut sheets = vec![SheetRef {
        sheet: user_agent_stylesheet(),
        origin: Origin::UserAgent,
    }];
    sheets.extend(author.iter().map(|sheet| SheetRef {
        sheet,
        origin: Origin::Author,
    }));
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    // Without a size, the view box gives it; one given side keeps its ratio.
    let Viewport(width, height) =
        view_box_size(dom.get_attribute(svg, "viewBox"), Viewport(300.0, 150.0));
    let absolute = |name| {
        dom.get_attribute(svg, name)
            .filter(|value| !value.trim().ends_with('%'))
            .map(|_| length(&dom, svg, name, Viewport(width, height)))
            .filter(|size| *size > 0.0)
    };
    let size = match (absolute("width"), absolute("height")) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * height / width),
        (None, Some(h)) => (h * width / height, h),
        (None, None) => (width, height),
    };
    let mut items = Vec::new();
    paint(
        &dom,
        &styles,
        svg,
        Rect::new(0.0, 0.0, size.0, size.1),
        &mut items,
        None,
    );
    Some((size, items))
}

#[cfg(test)]
mod tests;
