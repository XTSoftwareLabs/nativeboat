//! Style resolution, layout and painting for the native renderer.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub(crate) mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::boxed::Box;
    pub use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
    pub use alloc::format;
    pub use alloc::rc::Rc;
    pub use alloc::string::String;
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod bidi;
pub mod convert;
mod counters;
pub mod document;
pub mod inline;
mod line_break;
mod math;
pub mod motion;
pub mod paint;
pub mod resize;
pub mod scrollbars;
pub mod styles;
pub mod svg;
pub mod text;
pub mod tree;

pub use paint::{Caret, Highlight, Scene, Span, TextQuery, scene, scene_over, scene_with};
pub use styles::Styles;
pub use text::{FontMetrics, TextMeasurer};
pub use tree::{
    BoxGeometry, LayoutCache, LayoutOptions, LayoutResult, TextRun, VIDEO_FRAME, layout,
    layout_cached, media_url, native_control, video_picture,
};
