//! Intermediate representations shared by the compiler and the runtime.
//!
//! [`package`] is the compiled application: bundles, pages, styles and assets,
//! appended to the runtime executable. [`native`] is the platform-independent
//! native UI tree that separates web semantics from the Windows backend.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::boxed::Box;
    pub use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
    pub use alloc::string::String;
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod canvas;
pub mod json;
pub mod native;
#[cfg(feature = "std")]
pub mod package;
pub mod scrollbar;

pub use canvas::{CanvasCommand, CanvasPaint, CanvasSource, TextAnchor};
pub use native::{
    Change, Control, ControlKind, ControlState, Corner, DecorationLine, DisplayItem, Font,
    NativeChild, NativeTree, PathSegment, Radii, Rect, ScrollbarAxis, ScrollbarPaint,
    ScrollbarPart, Scrollbars, Stroke, Surface, TextDecoration, control_for, diff,
};
#[cfg(feature = "std")]
pub use package::{AppPackage, Asset, Metadata, Page, Script, Style};
