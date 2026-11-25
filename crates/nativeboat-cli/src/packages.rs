//! Replacements for package files that generate JavaScript at run time. A
//! compiled program has no JavaScript engine, so a package that builds
//! functions from source text, such as Ajv's schema compiler or fflate's
//! workers, cannot run unchanged. Its entry files are replaced by sources that interpret the same
//! inputs with the same API. A replacement is served beside the file it
//! replaces, so its relative requests resolve to the package's own files.

use nativeboat_compiler::{ModuleKind, ModuleSource};
use std::path::{Path, PathBuf};

/// The suffix of a replacement's virtual path.
const SUFFIX: &str = ".nativeboat-package.cjs";

/// The JSON Schema interpreter the Ajv entry files share, as they request it.
const JSON_SCHEMA: &str = "./nativeboat-json-schema.js";

/// Replaced files by their path inside `node_modules`, with the replacement
/// source.
const REPLACED: &[(&str, crate::Text)] = &[
    (
        "ajv/dist/ajv.js",
        nativeboat_conceal::text!("../node/packages/ajv/ajv.js"),
    ),
    (
        "ajv/dist/2019.js",
        nativeboat_conceal::text!("../node/packages/ajv/2019.js"),
    ),
    (
        "ajv/dist/2020.js",
        nativeboat_conceal::text!("../node/packages/ajv/2020.js"),
    ),
    (
        "ajv/dist/nativeboat-json-schema.js",
        nativeboat_conceal::text!("../node/packages/ajv/json-schema.js"),
    ),
    (
        "ajv-formats/dist/index.js",
        nativeboat_conceal::text!("../node/packages/ajv-formats/index.js"),
    ),
    (
        "fflate/lib/node.cjs",
        nativeboat_conceal::text!("../node/packages/fflate/node.js"),
    ),
];

/// The replacement source for a package file path, if it is replaced.
fn replaced(path: &Path) -> Option<&'static str> {
    let path = path.to_str()?.replace('\\', "/");
    REPLACED.iter().find_map(|(file, source)| {
        path.ends_with(&format!("/node_modules/{file}"))
            .then(source)
    })
}

/// The module that stands for a resolved package file: its replacement's
/// virtual path, or `None` when the file is compiled unchanged.
pub(crate) fn replacement(path: &Path) -> Option<PathBuf> {
    replaced(path)?;
    Some(PathBuf::from(format!("{}{SUFFIX}", path.display())))
}

/// A request a replacement makes for a module that exists only as a
/// replacement: the Ajv entry files' shared interpreter.
pub(crate) fn request(from: &Path, name: &str) -> Option<PathBuf> {
    if name != JSON_SCHEMA {
        return None;
    }
    let original = original(from)?;
    replaced(&original)?;
    replacement(&original.with_file_name(&JSON_SCHEMA[2..]))
}

/// The replaced file a replacement's virtual path stands for.
fn original(path: &Path) -> Option<PathBuf> {
    path.to_str()?.strip_suffix(SUFFIX).map(PathBuf::from)
}

/// The source of a replacement's virtual path.
pub(crate) fn source(path: &Path) -> Option<ModuleSource> {
    let text = replaced(&original(path)?)?;
    Some(ModuleSource {
        text: text.to_owned(),
        kind: ModuleKind::CommonJs,
    })
}

#[cfg(test)]
mod tests;
