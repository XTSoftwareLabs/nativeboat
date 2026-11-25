//! TypeScript and JSX transpilation with compile-time defines.

use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{HelperLoaderMode, JsxRuntime, TransformOptions, Transformer};
use oxc::transformer_plugins::{ReplaceGlobalDefines, ReplaceGlobalDefinesConfig};
use std::path::Path;

/// How a source file is transpiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranspileOptions {
    /// Module that provides the automatic JSX runtime, usually `react`.
    pub jsx_import_source: String,
    /// Whether `.js` files may contain JSX.
    pub jsx_in_js: bool,
    /// Expressions replaced at compile time, such as `process.env.NODE_ENV`.
    pub defines: Vec<(String, String)>,
    /// Whether TypeScript's `experimentalDecorators` are compiled.
    pub legacy_decorators: bool,
    /// Whether decorated declarations also get `emitDecoratorMetadata`.
    pub decorator_metadata: bool,
}

impl Default for TranspileOptions {
    fn default() -> Self {
        TranspileOptions {
            jsx_import_source: "react".to_owned(),
            jsx_in_js: true,
            defines: vec![(
                "process.env.NODE_ENV".to_owned(),
                "\"production\"".to_owned(),
            )],
            legacy_decorators: false,
            decorator_metadata: false,
        }
    }
}

/// Determines how to parse a file from its extension.
pub fn source_type(path: &Path, jsx_in_js: bool) -> Result<SourceType, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(match extension.as_str() {
        "js" => SourceType::unambiguous().with_jsx(jsx_in_js),
        "jsx" => SourceType::unambiguous().with_jsx(true),
        "mjs" => SourceType::mjs(),
        "cjs" => SourceType::cjs(),
        "ts" | "mts" | "cts" => SourceType::ts().with_unambiguous(true),
        "tsx" => SourceType::tsx().with_unambiguous(true),
        _ => return Err(format!("{}: unsupported source file type", path.display())),
    })
}

/// Transpiles TypeScript and JSX to JavaScript and applies defines.
///
/// The result keeps ES module syntax; [`crate::module::rewrite`] converts it.
pub fn transpile(
    source: &str,
    path: &Path,
    options: &TranspileOptions,
) -> Result<String, Vec<String>> {
    let source_type = source_type(path, options.jsx_in_js).map_err(|error| vec![error])?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(render(path, parsed.diagnostics));
    }
    let mut program = parsed.program;
    let mut scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    if source_type.is_typescript() || source_type.is_jsx() {
        let mut transform = TransformOptions::default();
        transform.jsx.runtime = JsxRuntime::Automatic;
        transform.jsx.import_source = Some(options.jsx_import_source.clone());
        transform.decorator.legacy = options.legacy_decorators;
        transform.decorator.emit_decorator_metadata = options.decorator_metadata;
        // Helpers come from the runtime's `babelHelpers`, not from a package.
        transform.helper_loader.mode = HelperLoaderMode::External;
        let transformed = Transformer::new(&allocator, path, &transform)
            .build_with_scoping(scoping, &mut program);
        if transformed.diagnostics.has_errors() || transformed.diagnostics.has_warnings() {
            return Err(render(path, transformed.diagnostics));
        }
        scoping = transformed.scoping;
    }
    if !options.defines.is_empty() {
        let config = ReplaceGlobalDefinesConfig::new(&options.defines)
            .map_err(|errors| render(path, errors))?;
        let _ = ReplaceGlobalDefines::new(&allocator, config).build(scoping, &mut program);
    }
    Ok(Codegen::new().build(&program).code)
}

fn render(path: &Path, diagnostics: oxc::diagnostics::Diagnostics) -> Vec<String> {
    diagnostics
        .into_vec()
        .into_iter()
        .map(|diagnostic| format!("{}: {}", path.display(), diagnostic.message))
        .collect()
}

#[cfg(test)]
mod tests;
