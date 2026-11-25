//! Where TypeScript writes compiled files.
//!
//! `tsc` mirrors the source root (`rootDir`) into the output directory
//! (`outDir`) and changes script extensions: `src/main/main.ts` becomes
//! `dist/src/main/main.js` with `rootDir: "."` and `outDir: "dist"`. Electron
//! loads the emitted files, so `package.json` `main`, preload paths and page
//! paths name them; this mapping leads back to the sources, which are compiled
//! instead. Build scripts copy other files, such as HTML pages and
//! stylesheets, to the same place under the output directory.

use std::path::{Path, PathBuf};

/// Prepared files may live outside the project while retaining portable
/// application paths for modules, CSS and assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMapping {
    /// Directory containing files produced by a build tool.
    pub source_root: PathBuf,
    /// Corresponding directory inside resources/app, using `/` separators.
    pub runtime_root: String,
}

impl PathMapping {
    /// Maps only descendants of this prepared directory.
    pub fn map(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.source_root).ok()?;
        Some(
            Path::new(&self.runtime_root)
                .join(relative)
                .to_string_lossy()
                .replace('\\', "/"),
        )
    }
}

/// The output layout of a TypeScript program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emit {
    /// The source root relative to the project root, with `/` separators;
    /// empty for the project root itself.
    pub root_dir: String,
    /// The output directory relative to the project root; the same as
    /// `root_dir` when files are emitted beside their sources.
    pub out_dir: String,
}

/// The extension `tsc` emits for a script extension.
fn emitted_extension(extension: &str) -> Option<&'static str> {
    match extension {
        "ts" | "tsx" | "js" | "jsx" => Some("js"),
        "mts" | "mjs" => Some("mjs"),
        "cts" | "cjs" => Some("cjs"),
        _ => None,
    }
}

/// The source extensions that compile to an emitted script extension, in the
/// order TypeScript looks for them; empty for other extensions.
pub fn source_extensions(extension: &str) -> &'static [&'static str] {
    match extension {
        "js" => &["ts", "tsx", "js", "jsx"],
        "jsx" => &["tsx", "jsx"],
        "mjs" => &["mts", "mjs"],
        "cjs" => &["cts", "cjs"],
        _ => &[],
    }
}

/// `path` relative to `directory`, both relative to the project root.
fn within<'p>(path: &'p str, directory: &str) -> Option<&'p str> {
    if directory.is_empty() {
        return Some(path);
    }
    let rest = path.strip_prefix(directory)?;
    if rest.is_empty() {
        Some(rest)
    } else {
        rest.strip_prefix('/')
    }
}

fn join(directory: &str, path: &str) -> String {
    match (directory.is_empty(), path.is_empty()) {
        (true, _) => path.to_owned(),
        (false, true) => directory.to_owned(),
        (false, false) => format!("{directory}/{path}"),
    }
}

impl Emit {
    /// The file emitted for a source file, both relative to the project root;
    /// `None` for declarations, files that are not scripts and files outside
    /// the source root.
    pub fn output(&self, source: &str) -> Option<String> {
        let rest = within(source, &self.root_dir)?;
        let (stem, extension) = rest.rsplit_once('.')?;
        if stem.ends_with(".d") {
            return None;
        }
        let extension = emitted_extension(extension)?;
        Some(join(&self.out_dir, &format!("{stem}.{extension}")))
    }

    /// The files an emitted file may come from, most likely first: a script
    /// comes from a TypeScript or JavaScript source, any other file is copied
    /// from the same place under the source root.
    pub fn sources(&self, output: &str) -> Vec<String> {
        let Some(rest) = within(output, &self.out_dir) else {
            return Vec::new();
        };
        let path = join(&self.root_dir, rest);
        let file_name = path.rsplit('/').next().unwrap_or_default();
        match file_name.rsplit_once('.') {
            Some((_, extension)) if !source_extensions(extension).is_empty() => {
                let stem = &path[..path.len() - extension.len()];
                source_extensions(extension)
                    .iter()
                    .map(|source| format!("{stem}{source}"))
                    .collect()
            }
            _ => vec![path],
        }
    }
}

#[cfg(test)]
mod tests;
