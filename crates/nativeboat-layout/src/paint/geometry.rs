//! Affine regions retain their shape for hits and expose axis-aligned bounds
//! for DOM client rectangles.

use nativeboat_css::types::FillRule;
use nativeboat_ir::{PathSegment, Rect};

pub(super) const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Geometry {
    pub rect: Rect,
    pub transform: [f32; 6],
}

impl Geometry {
    pub fn bounds(self) -> Rect {
        let [a, b, c, d, e, f] = self.transform;
        let rect = self.rect;
        let points = [
            (rect.x, rect.y),
            (rect.x + rect.width, rect.y),
            (rect.x, rect.y + rect.height),
            (rect.x + rect.width, rect.y + rect.height),
        ];
        let mut bounds = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for (x, y) in points {
            let (x, y) = (a * x + c * y + e, b * x + d * y + f);
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        }
        Rect::new(bounds.0, bounds.1, bounds.2 - bounds.0, bounds.3 - bounds.1)
    }

    /// Maps a window point back through the transform; `None` when the
    /// transform is singular.
    fn inverse(self, x: f32, y: f32) -> Option<(f64, f64)> {
        // Double precision avoids underflow for small but invertible scales.
        let [a, b, c, d, e, f] = self.transform.map(f64::from);
        let determinant = a * d - b * c;
        if determinant == 0.0 || !determinant.is_finite() {
            return None;
        }
        let (x, y) = (f64::from(x) - e, f64::from(y) - f);
        Some(((d * x - c * y) / determinant, (a * y - b * x) / determinant))
    }

    /// A window point in the untransformed coordinates of the rectangle.
    pub fn local(self, x: f32, y: f32) -> Option<(f32, f32)> {
        self.inverse(x, y).map(|(x, y)| (x as f32, y as f32))
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        self.contains_local(x, y).is_some()
    }

    /// The untransformed point, when a window point lies in the rectangle.
    pub fn contains_local(self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (x, y) = self.inverse(x, y)?;
        let rect = self.rect;
        (x >= f64::from(rect.x)
            && y >= f64::from(rect.y)
            && x < f64::from(rect.x + rect.width)
            && y < f64::from(rect.y + rect.height))
        .then_some((x as f32, y as f32))
    }
}

/// Whether a path covers a point, by its fill rule; curves are followed in
/// short straight steps.
pub(super) fn path_contains(segments: &[PathSegment], rule: FillRule, x: f32, y: f32) -> bool {
    let mut winding = 0i32;
    let mut crossings = 0u32;
    let mut edge = |from: (f32, f32), to: (f32, f32)| {
        let ((x0, y0), (x1, y1)) = (from, to);
        if (y0 <= y) == (y1 <= y) {
            return;
        }
        let at = x0 + (y - y0) / (y1 - y0) * (x1 - x0);
        if at > x {
            crossings += 1;
            winding += if y1 > y0 { 1 } else { -1 };
        }
    };
    let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
    for segment in segments {
        match *segment {
            PathSegment::Move(x, y) => {
                edge(current, start);
                (current, start) = ((x, y), (x, y));
            }
            PathSegment::Line(x, y) => {
                edge(current, (x, y));
                current = (x, y);
            }
            PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                const STEPS: usize = 16;
                let (x0, y0) = current;
                let mut previous = current;
                for step in 1..=STEPS {
                    let t = step as f32 / STEPS as f32;
                    let u = 1.0 - t;
                    let point = (
                        u * u * u * x0
                            + 3.0 * u * u * t * x1
                            + 3.0 * u * t * t * x2
                            + t * t * t * x,
                        u * u * u * y0
                            + 3.0 * u * u * t * y1
                            + 3.0 * u * t * t * y2
                            + t * t * t * y,
                    );
                    edge(previous, point);
                    previous = point;
                }
                current = (x, y);
            }
            PathSegment::Close => {
                edge(current, start);
                current = start;
            }
        }
    }
    // An unclosed last subpath fills as if closed.
    edge(current, start);
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => crossings % 2 == 1,
    }
}
