//! Ahead-of-time compilation of application behavior into native code.
//!
//! This backend accepts checked scalar functions and emits standalone Rust for
//! a native toolchain. It has no JavaScript engine or compatibility runtime.
//! Unsupported syntax is a compilation error, never an execution fallback.

mod dynamic;
mod ir;
mod lower;

pub use dynamic::{
    CompileOptions, CompileValue, DynamicModule, Jsx, ModuleKind, ModuleSource, RequestKind,
    compile_dynamic, compile_project, compile_sources, compile_sources_with_entries,
    compile_sources_with_entries_and_options, compile_sources_with_options,
};

/// The imports every crate of generated code starts with, after its
/// `extern crate` lines.
pub const GENERATED_IMPORTS: &str = dynamic::PRELUDE;
pub use ir::NativeModule;

use oxc::allocator::Allocator;
use oxc::diagnostics::Diagnostics;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::{SourceType, Span};
use std::fmt;
use std::path::{Path, PathBuf};

/// A source-located native compilation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Original source path.
    pub path: PathBuf,
    /// Byte range in the original UTF-8 source.
    pub span: Span,
    /// One-based line.
    pub line: usize,
    /// One-based Unicode character column.
    pub column: usize,
    /// Unsupported operation or invalid scalar typing.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

pub(crate) struct Source<'a> {
    path: &'a Path,
    text: &'a str,
}

impl Source<'_> {
    pub(crate) fn error(&self, span: Span, message: &str) -> Diagnostic {
        let prefix = &self.text[..span.start as usize];
        Diagnostic {
            path: self.path.to_path_buf(),
            span,
            line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
            message: message.to_owned(),
        }
    }

    fn diagnostics(&self, diagnostics: Diagnostics) -> Vec<Diagnostic> {
        diagnostics
            .into_vec()
            .into_iter()
            .map(|diagnostic| {
                let span = diagnostic.labels.first().map_or(Span::default(), |label| {
                    Span::new(label.offset(), label.offset() + label.len())
                });
                self.error(span, &diagnostic.message)
            })
            .collect()
    }
}

/// Compiles a complete module in the bounded native scalar subset.
///
/// Parameters must explicitly declare `number` or `boolean`; returned scalar
/// paths are inferred and checked against any return annotation. Direct native
/// calls, initialized const/let locals, scalar assignments and structured
/// branches are supported. Recursive cycles are rejected until native stack
/// limits and exception semantics are implemented. All top-level code is checked.
/// Objects, imports, unknown calls and other unsupported behavior are rejected
/// rather than omitted.
pub fn compile(source: &str, path: &Path) -> Result<NativeModule, Vec<Diagnostic>> {
    let source = Source { path, text: source };
    let source_type = SourceType::from_path(path)
        .map_err(|error| vec![source.error(Span::default(), &error.to_string())])?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source.text, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(source.diagnostics(parsed.diagnostics));
    }
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if semantic.diagnostics.has_errors() {
        return Err(source.diagnostics(semantic.diagnostics));
    }
    lower::module(&parsed.program, semantic.semantic.scoping(), &source)
        .map_err(|error| vec![error])
}

#[cfg(test)]
mod tests;
