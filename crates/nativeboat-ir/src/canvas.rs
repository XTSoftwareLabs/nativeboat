//! Drawing commands of `<canvas>` 2D contexts.
//!
//! Scripts keep the context state (transform, styles, paths) and send
//! self-contained commands: paths are in canvas pixels with the transform
//! already applied, and every command carries the paint and opacity it
//! uses. The platform draws them into the canvas bitmap in order.

use crate::native::{Font, PathSegment, Rect};
use crate::prelude::*;
use nativeboat_css::Color;
use nativeboat_css::types::{FillRule, LineCap, LineJoin};
use serde::{Deserialize, Serialize};

/// What fills or outlines a shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CanvasPaint {
    /// A plain color.
    Color(Color),
    /// A linear gradient from `start` to `end` with `(offset, color)` stops.
    Linear {
        /// Start point.
        start: (f32, f32),
        /// End point.
        end: (f32, f32),
        /// Stops by offset between 0 and 1.
        stops: Vec<(f32, Color)>,
    },
    /// A radial gradient between two circles `(x, y, radius)`.
    Radial {
        /// The start circle.
        start: (f32, f32, f32),
        /// The end circle.
        end: (f32, f32, f32),
        /// Stops by offset between 0 and 1.
        stops: Vec<(f32, Color)>,
    },
}

/// Where `drawImage` takes pixels from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CanvasSource {
    /// An image asset by URL.
    Url(String),
    /// Another canvas, by the platform key of its bitmap.
    Canvas(u64),
}

/// How a canvas text run is aligned at its anchor point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAnchor {
    /// The anchor is the left end of the text.
    Left,
    /// The anchor is the middle of the text.
    Center,
    /// The anchor is the right end of the text.
    Right,
}

/// One drawing operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CanvasCommand {
    /// Fills a path.
    Fill {
        /// The path in canvas pixels.
        path: Vec<PathSegment>,
        /// The paint.
        paint: CanvasPaint,
        /// The fill rule.
        rule: FillRule,
        /// `globalAlpha`.
        alpha: f32,
        /// Whether the shape replaces the pixels under it instead of
        /// blending (`globalCompositeOperation = 'copy'`).
        copy: bool,
    },
    /// Outlines a path.
    Stroke {
        /// The path in canvas pixels.
        path: Vec<PathSegment>,
        /// The paint.
        paint: CanvasPaint,
        /// Line width in the units of `transform`.
        width: f32,
        /// Line ends.
        cap: LineCap,
        /// Corners.
        join: LineJoin,
        /// The miter limit.
        miter: f32,
        /// Dash lengths in the units of `transform`; empty for solid lines.
        dash: Vec<f32>,
        /// The dash offset.
        offset: f32,
        /// The transform when the path was stroked, which shapes the pen.
        transform: [f32; 6],
        /// `globalAlpha`.
        alpha: f32,
        /// Whether the outline replaces the pixels under it.
        copy: bool,
    },
    /// Makes the pixels inside a path transparent.
    Clear {
        /// The path in canvas pixels.
        path: Vec<PathSegment>,
    },
    /// Fills or outlines text.
    Text {
        /// The text.
        text: String,
        /// The anchor point on the alphabetic baseline, before `transform`.
        x: f32,
        /// The baseline.
        y: f32,
        /// How the text is aligned at the anchor.
        anchor: TextAnchor,
        /// The font.
        font: Font,
        /// The paint.
        paint: CanvasPaint,
        /// The outline width, for `strokeText`.
        stroke: Option<f32>,
        /// The current transform.
        transform: [f32; 6],
        /// `globalAlpha`.
        alpha: f32,
    },
    /// Draws part of an image into a rectangle.
    Image {
        /// The image.
        source: CanvasSource,
        /// The source rectangle in image pixels.
        from: Rect,
        /// The destination rectangle, before `transform`.
        to: Rect,
        /// The current transform.
        transform: [f32; 6],
        /// `globalAlpha`.
        alpha: f32,
    },
    /// Saves the clip region (`save`).
    Save,
    /// Restores the last saved clip region (`restore`).
    Restore,
    /// Intersects the clip region with a path in canvas pixels.
    Clip {
        /// The path.
        path: Vec<PathSegment>,
        /// The fill rule.
        rule: FillRule,
    },
}
