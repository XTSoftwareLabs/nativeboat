//! CSS front end of the native compiler.
//!
//! Stylesheets are tokenized and parsed into a style IR of rules, selectors
//! and declarations. Selectors match against any DOM through the
//! [`selector::Element`] trait, media queries evaluate against the window
//! environment, and the cascade turns matching declarations into a
//! [`ComputedStyle`] with typed values for layout and painting.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub(crate) mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::boxed::Box;
    pub use alloc::collections::BTreeMap as HashMap;
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod cascade;
pub mod color;
pub mod computed;
mod math;
pub mod media;
pub mod page;
pub mod parser;
pub mod parsers;
pub mod selector;
pub mod shorthand;
pub mod tokenizer;
pub mod types;
pub mod ua;
pub mod values;

pub use cascade::{Origin, RuleMap, SheetRef, compute_pseudo_style, compute_style};
pub use color::{Color, SystemColors};
pub use computed::{ComputedStyle, PropertyId, StyleEnvironment, UsedBox};
pub use media::{MediaEnvironment, MediaQueryList};
pub use parser::{
    Declaration, Diagnostic, Keyframe, Position, Rule, StyleRule, Stylesheet,
    parse_declaration_list, parse_stylesheet,
};
pub use selector::{Element, PseudoElement, Selector, SelectorList, Specificity};
pub use ua::user_agent_stylesheet;
pub use values::{Calc, LengthPercentage};

#[cfg(test)]
mod tests;
