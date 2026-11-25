//! JavaScript front end of the native compiler.
//!
//! Application sources (TypeScript, JSX, ES modules and CommonJS) are
//! transpiled with oxc and rewritten into a single module format that the
//! runtime's JavaScript engine executes without a browser.

pub mod module;
pub mod transpile;

pub use module::{Dependency, DependencyKind, RewrittenModule, rewrite};
pub use transpile::{TranspileOptions, transpile};
pub mod bundle;
pub mod emit;
pub mod resolve;

pub use bundle::{
    Asset, Bundle, BundleOptions, Entry, Module, Resolution, Style, bundle, bundle_page,
};
pub use emit::{Emit, PathMapping};
pub use resolve::{Alias, AliasKind, Resolved, Resolver, Target, builtin};

#[cfg(test)]
mod tests;
