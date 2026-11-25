//! Resolve-only dependency resources used by computed package specifiers.
use super::{Builder, Resolution};
use crate::resolve::{Resolved, read_json, realpath};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

fn compatible(value: Option<&Value>, expected: &str) -> bool {
    let Some(value) = value else {
        return true;
    };
    let values: Vec<&str> = match value {
        Value::String(text) => vec![text],
        Value::Array(values) => values.iter().filter_map(Value::as_str).collect(),
        _ => return false,
    };
    if values.iter().any(|value| {
        value
            .strip_prefix('!')
            .is_some_and(|value| value == expected || value == "any")
    }) {
        return false;
    }
    !values.iter().any(|value| !value.starts_with('!'))
        || values.contains(&expected)
        || values.contains(&"any")
}

fn entries(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    Ok(std::fs::read_dir(directory)?
        .flatten()
        .map(|entry| entry.path())
        .collect())
}

fn walk(
    directory: &Path,
    seen: &mut HashSet<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    if !seen.insert(realpath(directory)) {
        return Ok(());
    }
    let mut children = entries(directory)?;
    children.sort();
    for path in children {
        if path
            .file_name()
            .is_some_and(|name| name == "node_modules" || name == ".git")
        {
            continue;
        }
        if path.is_dir() {
            walk(&path, seen, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn files(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    walk(directory, &mut HashSet::new(), &mut files)?;
    Ok(files)
}

impl Builder<'_> {
    pub(super) fn resolve_declared_files(&mut self, id: usize, from: &Path) {
        let package = from.parent().and_then(|directory| {
            directory
                .ancestors()
                .find_map(|directory| read_json(&directory.join("package.json")))
        });
        let Some(package) = package else {
            return;
        };
        let mut names = BTreeSet::new();
        for field in ["dependencies", "optionalDependencies"] {
            if let Some(dependencies) = package.get(field).and_then(Value::as_object) {
                names.extend(dependencies.keys());
            }
        }
        for name in names {
            let Some(directory) = self.resolver.package_directory(name, from) else {
                continue;
            };
            let manifest = read_json(&directory.join("package.json")).unwrap_or(Value::Null);
            if !compatible(manifest.get("os"), "win32") || !compatible(manifest.get("cpu"), "x64") {
                continue;
            }
            let paths = match files(&directory) {
                Ok(paths) => paths,
                Err(error) => {
                    self.bundle.warnings.push(format!(
                        "{}: cannot enumerate dependency {name}: {error}",
                        self.relative(from)
                    ));
                    continue;
                }
            };
            let mut specifiers = vec![name.clone()];
            for path in paths {
                if let Ok(relative) = path.strip_prefix(&directory) {
                    specifiers.push(format!(
                        "{name}/{}",
                        relative.to_string_lossy().replace('\\', "/")
                    ));
                }
            }
            for specifier in specifiers {
                if self.bundle.modules[id]
                    .dependencies
                    .iter()
                    .any(|(existing, _)| existing == &specifier)
                {
                    continue;
                }
                if let Ok(Resolved::File(file)) = self.resolver.resolve(&specifier, from) {
                    let url = self.asset_url(&file);
                    self.bundle.modules[id].dependencies.push((
                        specifier,
                        Resolution::File(url.trim_start_matches('/').to_owned()),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
