//! TypeScript programs and where `tsc` emits them.
//!
//! A project compiled with `tsc` loads its output: `package.json` `main`
//! names `dist/main.js` while the source is `src/main.ts`. Each configuration
//! that emits JavaScript gives an [`Emit`] layout from `rootDir` and `outDir`.
//! Without `rootDir`, `tsc` uses the longest common directory of the
//! program's files, as listed by `files`, `include` and `exclude`.
//! Configurations are found at the project root (`tsconfig*.json`), in
//! `tsc -p` and `tsc -b` commands of `package.json` scripts and through
//! `references`; `extends` chains are followed.

use crate::config::read_jsonc;
use crate::glob::{has_wildcard, matches, path_segments};
use nativeboat_bundler::Emit;
use nativeboat_bundler::resolve::normalize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// How deep `extends` chains are followed.
const MAX_DEPTH: usize = 8;

/// Patterns relative to a directory.
#[derive(Debug, Clone, Default)]
struct Patterns {
    base: PathBuf,
    patterns: Vec<String>,
}

/// A configuration with its `extends` chain applied; paths are absolute.
#[derive(Debug, Clone, Default)]
struct Config {
    out_dir: Option<PathBuf>,
    root_dir: Option<PathBuf>,
    no_emit: bool,
    allow_js: bool,
    files: Option<Vec<PathBuf>>,
    include: Option<Patterns>,
    exclude: Option<Patterns>,
    references: Vec<PathBuf>,
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}

/// A referenced configuration: a file, or `tsconfig.json` in a directory.
fn config_file(path: PathBuf) -> PathBuf {
    if path.is_dir() {
        path.join("tsconfig.json")
    } else {
        path
    }
}

/// The file an `extends` entry names: a path, or a file of a package.
fn extended(directory: &Path, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if name.starts_with('.') || Path::new(name).is_absolute() {
        let path = directory.join(name);
        candidates.push(PathBuf::from(format!("{}.json", path.display())));
        candidates.push(path);
    } else {
        let mut current = Some(directory);
        while let Some(folder) = current {
            let path = folder.join("node_modules").join(name);
            candidates.push(PathBuf::from(format!("{}.json", path.display())));
            candidates.push(path.join("tsconfig.json"));
            candidates.push(path);
            current = folder.parent();
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// The `compilerOptions` a configuration gives with its `extends` chain
/// applied: each base's options, later bases over earlier ones, then its
/// own.
pub(crate) fn compiler_options(path: &Path) -> Option<Value> {
    merged_options(path, 0)
}

fn merged_options(path: &Path, depth: usize) -> Option<Value> {
    let json = read_jsonc(path)?;
    let directory = path.parent()?;
    let bases = match &json["extends"] {
        Value::String(name) => vec![name.clone()],
        other => strings(other).unwrap_or_default(),
    };
    let mut options = serde_json::Map::new();
    for name in bases.iter().filter(|_| depth < MAX_DEPTH) {
        if let Some(Value::Object(base)) =
            extended(directory, name).and_then(|file| merged_options(&file, depth + 1))
        {
            options.extend(base);
        }
    }
    if let Value::Object(own) = &json["compilerOptions"] {
        options.extend(own.clone());
    }
    Some(Value::Object(options))
}

fn load(path: &Path, depth: usize) -> Option<Config> {
    let json = read_jsonc(path)?;
    let directory = path.parent()?;
    let bases = match &json["extends"] {
        Value::String(name) => vec![name.clone()],
        other => strings(other).unwrap_or_default(),
    };
    let mut config = Config::default();
    for name in bases.iter().filter(|_| depth < MAX_DEPTH) {
        if let Some(base) = extended(directory, name).and_then(|file| load(&file, depth + 1)) {
            config = Config {
                references: Vec::new(),
                ..base
            };
        }
    }
    let options = &json["compilerOptions"];
    let path_option = |name: &str| {
        options[name]
            .as_str()
            .map(|dir| normalize(&directory.join(dir)))
    };
    config.out_dir = path_option("outDir").or(config.out_dir);
    config.root_dir = path_option("rootDir").or(config.root_dir);
    for flag in ["noEmit", "emitDeclarationOnly"] {
        if let Some(value) = options[flag].as_bool() {
            config.no_emit = value;
        }
    }
    config.allow_js = options["allowJs"].as_bool().unwrap_or(config.allow_js);
    if let Some(files) = strings(&json["files"]) {
        config.files = Some(
            files
                .iter()
                .map(|file| normalize(&directory.join(file)))
                .collect(),
        );
    }
    for (key, slot) in [
        ("include", &mut config.include),
        ("exclude", &mut config.exclude),
    ] {
        if let Some(patterns) = strings(&json[key]) {
            *slot = Some(Patterns {
                base: directory.to_path_buf(),
                patterns,
            });
        }
    }
    config.references = json["references"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reference| reference["path"].as_str())
        .map(|reference| config_file(normalize(&directory.join(reference))))
        .collect();
    Some(config)
}

/// The directory where a `tsconfig` pattern's wildcards start, and its
/// segments: a last segment without wildcards or extension names a
/// directory and everything inside it.
fn pattern_segments(base: &Path, pattern: &str) -> (PathBuf, Vec<Vec<char>>) {
    let path = normalize(&base.join(pattern));
    let start = path
        .components()
        .take_while(|part| !has_wildcard(&part.as_os_str().to_string_lossy()))
        .collect();
    let mut parts = path_segments(&path);
    if parts.last().is_some_and(|last| {
        !last
            .iter()
            .any(|character| matches!(character, '*' | '?' | '.'))
    }) {
        parts.push(vec!['*', '*']);
        parts.push(vec!['*']);
    }
    (start, parts)
}

/// Lists files below `directory`, skipping `node_modules` and hidden folders.
fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.is_dir() {
            if !name.starts_with('.') && name != "node_modules" {
                walk(&path, files);
            }
        } else {
            files.push(path);
        }
    }
}

/// The source files of a program, without declaration files.
fn program_files(config: &Config, directory: &Path) -> Vec<PathBuf> {
    let extensions: &[&str] = if config.allow_js {
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"]
    } else {
        &["ts", "tsx", "mts", "cts"]
    };
    let is_source = |path: &Path| {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        extensions
            .iter()
            .any(|extension| name.ends_with(&format!(".{extension}")))
            && !name.contains(".d.")
    };
    let mut files: Vec<PathBuf> = config.files.clone().unwrap_or_default();
    let include = match (&config.include, &config.files) {
        (Some(include), _) => Some(include.clone()),
        (None, Some(_)) => None,
        (None, None) => Some(Patterns {
            base: directory.to_path_buf(),
            patterns: vec!["**/*".to_owned()],
        }),
    };
    let exclude = config.exclude.clone().unwrap_or_else(|| Patterns {
        base: directory.to_path_buf(),
        patterns: ["node_modules", "bower_components", "jspm_packages"]
            .into_iter()
            .map(str::to_owned)
            .chain(
                config
                    .out_dir
                    .iter()
                    .map(|out| out.to_string_lossy().into_owned()),
            )
            .collect(),
    });
    let excluded: Vec<Vec<Vec<char>>> = exclude
        .patterns
        .iter()
        .map(|pattern| pattern_segments(&exclude.base, pattern).1)
        .collect();
    for (start, pattern) in include.iter().flat_map(|include| {
        include
            .patterns
            .iter()
            .map(|pattern| pattern_segments(&include.base, pattern))
    }) {
        let mut found = Vec::new();
        walk(&start, &mut found);
        for file in found {
            let parts = path_segments(&file);
            if matches(&pattern, &parts)
                && !excluded.iter().any(|exclusion| matches(exclusion, &parts))
                && !files.contains(&file)
            {
                files.push(file);
            }
        }
    }
    files.retain(|file| is_source(file));
    files
}

/// The longest common directory of the files.
fn common_directory(files: &[PathBuf]) -> Option<PathBuf> {
    let mut common = files.first()?.parent()?.to_path_buf();
    for file in files {
        while !file.starts_with(&common) {
            common.pop();
        }
    }
    Some(common)
}

/// `path` relative to `root`, with `..` when outside, and `/` separators.
pub fn relative_to(root: &Path, path: &Path) -> String {
    let root: Vec<Component> = root.components().collect();
    let path: Vec<Component> = path.components().collect();
    let shared = root
        .iter()
        .zip(&path)
        .take_while(|(left, right)| left.as_os_str().eq_ignore_ascii_case(right.as_os_str()))
        .count();
    let mut parts: Vec<String> = vec!["..".to_owned(); root.len() - shared];
    parts.extend(
        path[shared..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Configurations named by `tsc -p` and `tsc -b` in `package.json` scripts.
fn script_configs(root: &Path, package: &Value) -> Vec<PathBuf> {
    let mut configs = Vec::new();
    for script in package["scripts"]
        .as_object()
        .into_iter()
        .flat_map(|scripts| scripts.values())
        .filter_map(Value::as_str)
    {
        let words: Vec<&str> = script.split_whitespace().collect();
        for (index, word) in words.iter().enumerate() {
            if *word != "tsc" {
                continue;
            }
            let arguments: Vec<&str> = words[index + 1..]
                .iter()
                .copied()
                .take_while(|argument| !matches!(*argument, "&&" | "||" | ";" | "|"))
                .collect();
            // A type check (`--noEmit`, `--emitDeclarationOnly`) writes no
            // JavaScript, whatever its configuration says.
            let emits_nothing = arguments.iter().enumerate().any(|(at, argument)| {
                matches!(*argument, "--noEmit" | "--emitDeclarationOnly")
                    && arguments.get(at + 1) != Some(&"false")
            });
            if emits_nothing {
                continue;
            }
            let mut project = false;
            for argument in arguments {
                if matches!(argument, "-p" | "--project" | "-b" | "--build") {
                    project = true;
                } else if project && !argument.starts_with('-') {
                    let argument = argument.trim_matches(['"', '\'']);
                    configs.push(config_file(normalize(&root.join(argument))));
                } else {
                    project = false;
                }
            }
        }
    }
    configs
}

/// The output layouts of the project's TypeScript programs that emit
/// JavaScript, relative to the project root.
pub fn emits(root: &Path, package: &Value) -> Vec<Emit> {
    let mut pending = script_configs(root, package);
    let mut at_root: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            name.starts_with("tsconfig") && name.ends_with(".json") && path.is_file()
        })
        .collect();
    at_root.sort();
    pending.extend(at_root);
    let mut seen = Vec::new();
    let mut emits = Vec::new();
    while !pending.is_empty() {
        let path = pending.remove(0);
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        let Some(config) = load(&path, 0) else {
            continue;
        };
        // Each configuration is read once, so cycles of references end.
        pending.extend(config.references.iter().cloned());
        if config.no_emit {
            continue;
        }
        let directory = path.parent().unwrap_or(root);
        let Some(root_dir) = config
            .root_dir
            .clone()
            .or_else(|| common_directory(&program_files(&config, directory)))
        else {
            continue;
        };
        let out_dir = config.out_dir.clone().unwrap_or_else(|| root_dir.clone());
        let emit = Emit {
            root_dir: relative_to(root, &root_dir),
            out_dir: relative_to(root, &out_dir),
        };
        if !emits.contains(&emit) {
            emits.push(emit);
        }
    }
    emits
}

#[cfg(test)]
mod tests;
