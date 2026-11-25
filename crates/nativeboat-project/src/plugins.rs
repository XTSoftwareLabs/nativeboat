//! Plugin packages: folders the application ships whose code it loads by
//! path while it runs, such as an editor's built-in extensions. The `plugins`
//! setting names them with folder globs (`"extensions/*"`, with `!` before a
//! glob that leaves folders out); the `main` module of each package in such
//! a folder, and of packages nested in it, is compiled as an entry the
//! program loads when it requires or imports the module's path.
//!
//! A `main` that names an emitted file the package's own build would make
//! (`./out/main`) is compiled from the TypeScript source its `tsconfig.json`
//! emits it from (`outDir` mirrors `rootDir`).

use crate::Entry;
use crate::config::read_jsonc;
use std::path::{Path, PathBuf};

/// How deep below a plugin folder nested packages are found.
const NESTED_DEPTH: usize = 3;

/// The TypeScript sources a JavaScript file may be compiled from.
pub const SOURCES: &[(&str, &[&str])] =
    &[("js", &["ts", "tsx"]), ("mjs", &["mts"]), ("cjs", &["cts"])];

/// The folder globs a `plugins` setting gives: a list of strings.
pub fn patterns(value: &serde_json::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|pattern| pattern.as_str().map(str::to_owned))
        .collect()
}

/// The entries of the plugin packages `patterns` name under `root`, and a
/// warning for each package whose `main` module has no file or source.
pub fn entries(root: &Path, patterns: &[String]) -> (Vec<Entry>, Vec<String>) {
    let (exclude, include): (Vec<&String>, Vec<&String>) = patterns
        .iter()
        .partition(|pattern| pattern.starts_with('!'));
    let include: Vec<_> = include.iter().map(|p| crate::glob::segments(p)).collect();
    let exclude: Vec<_> = exclude
        .iter()
        .map(|p| crate::glob::segments(&p[1..]))
        .collect();
    let depth = include.iter().map(Vec::len).max().unwrap_or(0);
    let mut folders = Vec::new();
    collect_folders(root, depth, &mut folders);
    folders.retain(|folder| {
        let segments = crate::glob::path_segments(folder.strip_prefix(root).unwrap_or(folder));
        include
            .iter()
            .any(|pattern| crate::glob::matches(pattern, &segments))
            && !exclude
                .iter()
                .any(|pattern| crate::glob::matches(pattern, &segments))
    });
    folders.sort();
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for folder in folders {
        let mut packages = Vec::new();
        collect_packages(&folder, NESTED_DEPTH, &mut packages);
        packages.sort();
        for package in packages {
            let Some(manifest) = read_jsonc(&package.join("package.json")) else {
                continue;
            };
            let Some(main) = manifest["main"].as_str() else {
                continue;
            };
            let emitted =
                emitted_file(&nativeboat_bundler::resolve::normalize(&package.join(main)));
            match source_of_emitted(&package, &emitted) {
                Some(source) if !entries.iter().any(|entry: &Entry| entry.source == source) => {
                    let path = crate::relative(root, &emitted);
                    let mut runtime_paths = vec![path.clone()];
                    if let Some(stem) = path.strip_suffix(".js") {
                        runtime_paths.push(stem.to_owned());
                    }
                    entries.push(Entry {
                        source,
                        runtime_paths,
                    });
                }
                Some(_) => {}
                None => warnings.push(format!(
                    "the plugin package {} names \"{main}\" as its main module, which has no file and no TypeScript source its tsconfig.json emits it from",
                    crate::relative(root, &package)
                )),
            }
        }
    }
    (entries, warnings)
}

/// Folders below `folder` down to `depth` levels, skipping dependencies
/// and hidden folders.
fn collect_folders(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    for child in subfolders(folder) {
        found.push(child.clone());
        collect_folders(&child, depth - 1, found);
    }
}

/// The folders holding a `package.json` at `folder` and below it.
fn collect_packages(folder: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if folder.join("package.json").is_file() {
        found.push(folder.to_path_buf());
    }
    if depth == 0 {
        return;
    }
    for child in subfolders(folder) {
        collect_packages(&child, depth - 1, found);
    }
}

fn subfolders(folder: &Path) -> Vec<PathBuf> {
    let Ok(listing) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = listing
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name != "node_modules" && !name.starts_with('.')
        })
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    folders
}

/// The file a module path names as Node resolves it: the path itself when
/// it has a JavaScript extension, else the path with `.js`, or the folder's
/// `index.js` when only that exists.
fn emitted_file(path: &Path) -> PathBuf {
    let known = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SOURCES.iter().any(|(emitted, _)| *emitted == extension));
    if known {
        return path.to_path_buf();
    }
    let index = path.join("index.js");
    if path.is_dir() && (index.is_file() || source_of_emitted(path, &index).is_some()) {
        return index;
    }
    let mut file = path.as_os_str().to_owned();
    file.push(".js");
    PathBuf::from(file)
}

/// The file an emitted JavaScript path is, or the TypeScript source a
/// `tsconfig.json` between it and `package` emits it from.
pub fn source_of_emitted(package: &Path, emitted: &Path) -> Option<PathBuf> {
    if let Some(found) = existing(emitted) {
        return Some(found);
    }
    let mut folder = emitted.parent();
    while let Some(current) = folder {
        let config = current.join("tsconfig.json");
        if config.is_file()
            && let Some((out_dir, root_dir)) = directories(&config)
            && let Ok(relative) = emitted.strip_prefix(&out_dir)
            && let Some(found) = existing(&root_dir.join(relative))
        {
            return Some(found);
        }
        if current == package || !current.starts_with(package) {
            break;
        }
        folder = current.parent();
    }
    None
}

/// Whether the TypeScript `source` compiles to CommonJS, as the nearest
/// `tsconfig.json` above it in `package` sets `module`: `commonjs`, or
/// `node16`/`nodenext` in a package that is not `"type": "module"`.
pub fn emits_commonjs(package: &Path, source: &Path) -> bool {
    let mut folder = source.parent();
    while let Some(current) = folder {
        let config = current.join("tsconfig.json");
        if config.is_file() {
            let Some(options) = crate::tsconfig::compiler_options(&config) else {
                return false;
            };
            return match options["module"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("commonjs") => true,
                Some("node16" | "node18" | "node20" | "nodenext") => !current
                    .ancestors()
                    .take_while(|ancestor| ancestor.starts_with(package))
                    .find_map(|ancestor| read_jsonc(&ancestor.join("package.json")))
                    .is_some_and(|manifest| manifest["type"] == "module"),
                _ => false,
            };
        }
        if current == package || !current.starts_with(package) {
            break;
        }
        folder = current.parent();
    }
    false
}

/// A configuration's `outDir` and `rootDir`, resolved from its folder; the
/// root defaults to the literal start of its first `include` pattern, as
/// TypeScript infers it from the files included.
fn directories(config: &Path) -> Option<(PathBuf, PathBuf)> {
    let folder = config.parent()?;
    let options = crate::tsconfig::compiler_options(config)?;
    let out_dir = folder.join(options["outDir"].as_str()?);
    let root_dir = match options["rootDir"].as_str() {
        Some(root) => folder.join(root),
        None => {
            let json = read_jsonc(config)?;
            let first = json["include"].as_array()?.first()?.as_str()?;
            let literal: Vec<&str> = first
                .split('/')
                .take_while(|part| !crate::glob::has_wildcard(part))
                .collect();
            folder.join(literal.join("/"))
        }
    };
    Some((
        nativeboat_bundler::resolve::normalize(&out_dir),
        nativeboat_bundler::resolve::normalize(&root_dir),
    ))
}

/// The file at `path`, or the source of the same name it is compiled from.
pub fn existing(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let extension = path.extension()?.to_str()?;
    let (_, sources) = SOURCES.iter().find(|(emitted, _)| *emitted == extension)?;
    sources
        .iter()
        .map(|source| path.with_extension(source))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests;
