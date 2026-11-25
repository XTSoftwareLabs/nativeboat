//! electron-builder configuration: application metadata and the files
//! copied next to the application: the application's own files that `files`
//! selects, `extraResources` into `resources/` and `extraFiles` beside the
//! executable.

use crate::config::{read_jsonc, read_yaml};
use crate::glob::{has_wildcard, matches, segments};
use nativeboat_bundler::Emit;
use serde_json::Value;
use std::path::{Path, PathBuf};

mod installer;
mod updater;
pub use installer::{Installer, installer};
pub use updater::{app_update_yml, origin_remote, updater_config};

/// A file copied next to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraFile {
    /// The file to copy.
    pub source: PathBuf,
    /// Where it goes, relative to the executable's directory, with `/`
    /// separators.
    pub destination: String,
}

fn configuration(value: Value, path: &Path) -> Result<Value, String> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(format!(
            "invalid {}: build configuration must be an object",
            path.display()
        ))
    }
}

/// Static electron-builder configuration. The manifest's `build` field wins;
/// otherwise YAML, JSON and JSON5 files follow electron-builder's search order.
pub fn config(root: &Path, package: &Value) -> Result<Value, String> {
    if !package["build"].is_null() {
        return configuration(package["build"].clone(), &root.join("package.json"));
    }
    for name in [
        "electron-builder.yml",
        "electron-builder.yaml",
        "electron-builder.json",
        "electron-builder.json5",
        "electron-builder.js",
        "electron-builder.cjs",
        "electron-builder.mjs",
        "electron-builder.ts",
        "electron-builder.config.js",
        "electron-builder.config.cjs",
        "electron-builder.config.mjs",
        "electron-builder.config.ts",
    ] {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        let value = if name.ends_with(".yml") || name.ends_with(".yaml") {
            read_yaml(&path)?
        } else if [".js", ".cjs", ".mjs", ".ts"]
            .iter()
            .any(|extension| name.ends_with(extension))
        {
            // Settings that depend on running the module are unknown.
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            crate::config::evaluate::exported(&path, &text, root, "electron-builder configuration")?
                .json(false)
                .unwrap_or_default()
        } else {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            json5::from_str(&text)
                .map_err(|error| format!("invalid {}: {error}", path.display()))?
        };
        return configuration(value, &path);
    }
    Ok(Value::Null)
}

/// Production application metadata in a two-package layout overrides the
/// development manifest. Discovery and build scripts still use the root.
pub fn application_package(root: &Path, config: &Value, package: &Value) -> Value {
    let mut merged = package.clone();
    if let Some(Value::Object(application)) = config["directories"]["app"]
        .as_str()
        .and_then(|directory| read_jsonc(&root.join(directory).join("package.json")))
        && let Some(development) = merged.as_object_mut()
    {
        development.extend(application);
    }
    merged
}

/// Where the executable goes by default: electron-builder's output
/// directory, otherwise `dist`, or `release` when sources are compiled into
/// `dist`.
pub fn output_directory(config: &Value, emits: &[Emit]) -> String {
    if let Some(output) = config["directories"]["output"].as_str() {
        return output.to_owned();
    }
    let compiled = emits
        .iter()
        .any(|emit| emit.out_dir.split('/').next() == Some("dist"));
    if compiled { "release" } else { "dist" }.to_owned()
}

/// A set of files: a directory or file, where its files go and which of
/// them are copied.
struct FileSet {
    from: String,
    to: Option<String>,
    filters: Vec<String>,
}

fn file_sets(value: &Value) -> Vec<FileSet> {
    match value {
        Value::Array(items) => {
            // String patterns share the project-root matcher. Object mappings
            // each retain their own source, destination and ordered filters.
            let patterns: Vec<String> = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let mut sets = Vec::new();
            if !patterns.is_empty() {
                sets.push(FileSet {
                    from: ".".to_owned(),
                    to: None,
                    filters: patterns,
                });
            }
            sets.extend(
                items
                    .iter()
                    .filter(|item| !item.is_string())
                    .flat_map(file_sets),
            );
            sets
        }
        Value::Object(set) => {
            let filters = match &set.get("filter") {
                Some(Value::String(filter)) => vec![filter.clone()],
                Some(Value::Array(filters)) => filters
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                _ => vec!["**/*".to_owned()],
            };
            vec![FileSet {
                from: set
                    .get("from")
                    .and_then(Value::as_str)
                    .unwrap_or(".")
                    .to_owned(),
                to: set.get("to").and_then(Value::as_str).map(str::to_owned),
                filters,
            }]
        }
        _ => Vec::new(),
    }
}

fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if path.file_name().is_none_or(|name| name != "node_modules") {
                walk(&path, files);
            }
        } else {
            files.push(path);
        }
    }
}

/// A pattern's alternatives with its `{a,b}` groups expanded, as
/// minimatch expands them before matching.
fn braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let mut depth = 0;
    let mut parts = Vec::new();
    let mut start = open + 1;
    for (offset, character) in pattern[open..].char_indices() {
        let index = open + offset;
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    parts.push(&pattern[start..index]);
                    if parts.len() == 1 {
                        // A group without a comma is literal text.
                        return braces(&pattern[index + 1..])
                            .into_iter()
                            .map(|rest| format!("{}{rest}", &pattern[..=index]))
                            .collect();
                    }
                    let rest = braces(&pattern[index + 1..]);
                    return parts
                        .iter()
                        .flat_map(|part| braces(&format!("{}{part}", &pattern[..open])))
                        .flat_map(|head| rest.iter().map(move |tail| format!("{head}{tail}")))
                        .collect();
                }
            }
            ',' if depth == 1 => {
                parts.push(&pattern[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    vec![pattern.to_owned()]
}

/// Whether a filter's pattern matches a path, or the directory it names
/// holds the path.
fn pattern_matches(pattern: &str, path: &[Vec<char>]) -> bool {
    braces(pattern).iter().any(|pattern| {
        let directory = !pattern.contains(['.', '[']) && !has_wildcard(pattern);
        matches(&segments(pattern), path)
            || (directory && matches(&segments(&format!("{pattern}/**/*")), path))
    })
}

/// Whether a path relative to a file set's directory passes its filters:
/// Matching filters apply in order, allowing a later inclusion to restore a
/// file excluded by an earlier pattern. Leading `!` characters toggle negation.
fn selected(filters: &[String], relative: &str) -> bool {
    let path = segments(relative);
    // electron-builder supplies **/* for an empty or exclusion-only mapping.
    let mut selected = filters.iter().all(|filter| filter.starts_with('!'));
    for filter in filters {
        let pattern = filter.trim_start_matches('!');
        let excluded = (filter.len() - pattern.len()) % 2 == 1;
        if selected == excluded && pattern_matches(pattern, &path) {
            selected = !excluded;
        }
    }
    selected
}

/// Whether an exclusion removes a whole directory, so nothing under it is
/// copied unless a later filter includes the directory again.
fn pruned(filters: &[String], relative: &str) -> bool {
    let path = segments(relative);
    let mut pruned = false;
    for filter in filters {
        let pattern = filter.trim_start_matches('!');
        let excluded = (filter.len() - pattern.len()) % 2 == 1;
        if excluded {
            pruned |= braces(pattern)
                .iter()
                .any(|pattern| matches(&segments(pattern), &path));
        } else if pattern_matches(pattern, &path)
            || braces(pattern).iter().any(|pattern| {
                let pattern = segments(pattern);
                pattern.len() > path.len() && matches(&pattern[..path.len()], &path)
            })
        {
            pruned = false;
        }
    }
    pruned
}

/// Whether the build compiles a file into the executable, or packages
/// leave it out of applications: JavaScript, TypeScript and Vue sources,
/// source maps, the build's own configuration, lock files, and the files
/// electron-builder never packs.
fn compiled_or_ignored(relative: &str) -> bool {
    let name = relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .to_ascii_lowercase();
    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    let stem = name.split('.').next().unwrap_or_default();
    // TypeScript's and electron-builder's settings, which the build applied.
    if matches!(stem, "tsconfig" | "jsconfig" | "electron-builder")
        && matches!(extension, "json" | "json5" | "yml" | "yaml" | "toml")
    {
        return true;
    }
    relative
        .split('/')
        .any(|segment| segment.starts_with('.') || segment.eq_ignore_ascii_case("__pycache__"))
        || matches!(
            extension,
            "js" | "mjs"
                | "cjs"
                | "jsx"
                | "ts"
                | "mts"
                | "cts"
                | "tsx"
                | "vue"
                | "map"
                | "o"
                | "hprof"
                | "orig"
                | "pyc"
                | "pyo"
                | "rbc"
                | "swp"
                | "csproj"
                | "sln"
                | "suo"
                | "xproj"
                | "cc"
                | "iml"
        )
        || matches!(
            name.as_str(),
            "package-lock.json"
                | "npm-shrinkwrap.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "bun.lockb"
                | "bun.lock"
                | "thumbs.db"
                | "npm-debug.log"
        )
}

/// The application's own files that run beside the executable, as
/// electron-builder's `files` selects them for the package (every file
/// when the configuration has none): the pages' style sheets and images,
/// data files and the programs the application starts. Sources the build
/// compiles into the executable, installed packages, the build resources
/// folder unless a pattern names it, and the `skipped` folders, such as
/// the output, stay behind.
pub fn application_files(root: &Path, config: &Value, skipped: &[PathBuf]) -> Vec<ExtraFile> {
    let values: Vec<Value> = [&config["files"], &config["win"]["files"]]
        .into_iter()
        .flat_map(|value| match value {
            Value::Array(items) => items.clone(),
            Value::Null => Vec::new(),
            value => vec![value.clone()],
        })
        .collect();
    let mut sets = file_sets(&Value::Array(values));
    if sets.is_empty() {
        sets.push(FileSet {
            from: ".".to_owned(),
            to: None,
            filters: vec!["**/*".to_owned()],
        });
    }
    let resources = config["directories"]["buildResources"]
        .as_str()
        .unwrap_or("build");
    let resources = segments(resources);
    let skipped: Vec<PathBuf> = skipped
        .iter()
        .map(|path| nativeboat_bundler::resolve::normalize(&root.join(path)))
        .collect();
    let mut files = Vec::new();
    // The application's packages: its own and those nested in its folders,
    // whose installed dependencies ship with it.
    let mut packages = Vec::new();
    for set in sets {
        let from = nativeboat_bundler::resolve::normalize(&root.join(&set.from));
        if !from.is_dir() {
            continue;
        }
        // The build resources folder is packed only when a filter names it.
        let named_resources = set.filters.iter().any(|filter| {
            !filter.starts_with('!')
                && braces(filter)
                    .iter()
                    .any(|pattern| segments(pattern).starts_with(&resources))
        });
        let mut pending = vec![from.clone()];
        while let Some(directory) = pending.pop() {
            if directory.join("package.json").is_file() && !packages.contains(&directory) {
                packages.push(directory.clone());
            }
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&directory)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .collect();
            entries.sort();
            for path in entries {
                let relative = crate::relative(&from, &path);
                if path.is_dir() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let excluded = name.eq_ignore_ascii_case("node_modules")
                        || name.starts_with('.')
                        || skipped.contains(&nativeboat_bundler::resolve::normalize(&path))
                        || (!named_resources && segments(&relative) == resources)
                        || pruned(&set.filters, &relative);
                    if !excluded {
                        pending.push(path);
                    }
                } else if selected(&set.filters, &relative) && !compiled_or_ignored(&relative) {
                    files.push(ExtraFile {
                        destination: join(set.to.as_deref().unwrap_or(""), &relative),
                        source: path,
                    });
                }
            }
        }
    }
    files.extend(dependency_files(root, &packages));
    files.sort_by(|left, right| left.destination.cmp(&right.destination));
    files.dedup_by(|left, right| left.destination == right.destination);
    files
}

/// The files of the installed packages `packages` depend on when they run
/// (`dependencies` and `optionalDependencies`, transitively, found as Node
/// finds them) that run beside the executable, at their places in the
/// application folder: data, images, WebAssembly and the programs they
/// start. Their code is compiled into the executable; native addons, which
/// the runtime provides, other systems' libraries, build leftovers,
/// documentation and test folders stay behind.
fn dependency_files(root: &Path, packages: &[PathBuf]) -> Vec<ExtraFile> {
    let root = nativeboat_bundler::resolve::normalize(root);
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut pending: Vec<PathBuf> = packages.to_vec();
    let mut files = Vec::new();
    while let Some(package) = pending.pop() {
        let Some(manifest) = crate::config::read_jsonc(&package.join("package.json")) else {
            continue;
        };
        let names = ["dependencies", "optionalDependencies"]
            .into_iter()
            .filter_map(|key| manifest[key].as_object())
            .flat_map(|dependencies| dependencies.keys().cloned())
            .collect::<Vec<_>>();
        for name in names {
            let found = package
                .ancestors()
                .take_while(|folder| folder.starts_with(&root))
                .map(|folder| folder.join("node_modules").join(&name))
                .find(|candidate| candidate.join("package.json").is_file());
            let Some(found) = found.map(|path| nativeboat_bundler::resolve::normalize(&path))
            else {
                continue;
            };
            if seen.contains(&found) {
                continue;
            }
            seen.push(found.clone());
            collect_package_files(&root, &found, &mut files);
            pending.push(found);
        }
    }
    files
}

/// The files of one installed package that run beside the executable, its
/// own nested packages left to their dependents.
fn collect_package_files(root: &Path, package: &Path, files: &mut Vec<ExtraFile>) {
    let mut pending = vec![package.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    "node_modules"
                        | "test"
                        | "tests"
                        | "__tests__"
                        | "example"
                        | "examples"
                        | "powered-test"
                ) && !name.starts_with('.')
                {
                    pending.push(path);
                }
                continue;
            }
            let relative = crate::relative(root, &path);
            let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
            let left_behind = matches!(
                extension,
                "node"
                    | "obj"
                    | "pdb"
                    | "lib"
                    | "exp"
                    | "ilk"
                    | "a"
                    | "h"
                    | "hh"
                    | "hpp"
                    | "c"
                    | "cc"
                    | "cpp"
                    | "cxx"
                    | "gyp"
                    | "gypi"
                    | "so"
                    | "dylib"
                    | "md"
                    | "markdown"
                    | "tsbuildinfo"
            );
            if !left_behind && !compiled_or_ignored(&relative) {
                files.push(ExtraFile {
                    destination: relative,
                    source: path,
                });
            }
        }
    }
}

fn join(directory: &str, path: &str) -> String {
    let directory = directory.trim_matches('/');
    if directory.is_empty() || directory == "." {
        path.to_owned()
    } else {
        format!("{directory}/{path}")
    }
}

/// The file names a set's filters spell out, without wildcards.
fn literal_names(filters: &[String]) -> Vec<String> {
    filters
        .iter()
        .filter(|filter| !has_wildcard(filter) && !filter.contains(['{', '[', '!']))
        .map(|filter| filter.rsplit('/').next().unwrap_or(filter).to_owned())
        .collect()
}

/// The files `extraResources` and `extraFiles` copy, including those of the
/// `win` section. Missing sources are reported as warnings and listed in
/// `missing`: paths that do not exist and file names a filter spells out.
pub fn extra_files(
    root: &Path,
    config: &Value,
    warnings: &mut Vec<String>,
    missing: &mut Vec<String>,
) -> Vec<ExtraFile> {
    let mut files = Vec::new();
    for (key, prefix) in [("extraResources", "resources"), ("extraFiles", "")] {
        let values: Vec<Value> = [&config[key], &config["win"][key]]
            .into_iter()
            .flat_map(|value| match value {
                Value::Array(items) => items.clone(),
                value => vec![value.clone()],
            })
            .collect();
        for set in file_sets(&Value::Array(values)) {
            let from = root.join(&set.from);
            let destination = join(prefix, set.to.as_deref().unwrap_or(""));
            if from.is_file() {
                let name = from
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                files.push(ExtraFile {
                    destination: set
                        .to
                        .map_or_else(|| join(prefix, &name), |to| join(prefix, &to)),
                    source: from,
                });
                continue;
            }
            if !from.is_dir() {
                warnings.push(format!(
                    "{key} \"{}\" does not exist and is not copied; build it before compiling",
                    set.from
                ));
                missing.push(set.from.clone());
                missing.extend(literal_names(&set.filters));
                continue;
            }
            let mut found = Vec::new();
            walk(&from, &mut found);
            let before = files.len();
            for source in found {
                let relative = crate::relative(&from, &source);
                if selected(&set.filters, &relative) {
                    files.push(ExtraFile {
                        destination: join(&destination, &relative),
                        source,
                    });
                }
            }
            if files.len() == before {
                warnings.push(format!(
                    "{key} \"{}\" has no files to copy; build them before compiling",
                    set.from
                ));
                missing.extend(literal_names(&set.filters));
            }
        }
    }
    files
}

#[cfg(test)]
mod tests;
