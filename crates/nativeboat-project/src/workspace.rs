//! Declared workspace members, without scanning dependency installations or
//! treating ordinary libraries as desktop applications.
use super::*;
use crate::glob::{matches, path_segments, segments};

fn patterns(root: &Path, package: &Value) -> Result<Vec<String>, String> {
    let yaml = root.join("pnpm-workspace.yaml");
    if yaml.exists() {
        let config = crate::config::read_yaml(&yaml)?;
        let packages = &config["packages"];
        if packages.is_null() {
            return Ok(Vec::new());
        }
        let values = packages
            .as_array()
            .ok_or("pnpm workspace packages must be an array of strings")?;
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "pnpm workspace packages must be an array of strings".to_owned())
            })
            .collect()
    } else {
        let workspaces = &package["workspaces"];
        let packages = if workspaces.is_object() {
            &workspaces["packages"]
        } else {
            workspaces
        };
        if packages.is_null() {
            return Ok(Vec::new());
        }
        let values = packages
            .as_array()
            .ok_or("workspace packages must be an array of strings")?;
        values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "workspace packages must be an array of strings".to_owned())
            })
            .collect()
    }
}

fn descend(pattern: &[Vec<char>], path: &[Vec<char>]) -> bool {
    (1..pattern.len()).any(|length| matches(&pattern[..length], path))
        || (pattern.last().is_some_and(|segment| *segment == ['*', '*']) && matches(pattern, path))
}

fn members(root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let include: Vec<_> = patterns
        .iter()
        .filter(|pattern| !pattern.starts_with('!'))
        .map(|pattern| segments(pattern))
        .collect();
    let exclude: Vec<_> = patterns
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('!'))
        .map(segments)
        .collect();
    let mut pending = vec![root.to_path_buf()];
    let mut members = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
            let name = entry.file_name();
            if name == "node_modules"
                || name.to_string_lossy().starts_with('.')
                || !entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            {
                continue;
            }
            let path = entry.path();
            let parts = path_segments(path.strip_prefix(root).unwrap());
            if include.iter().any(|pattern| matches(pattern, &parts))
                && !exclude.iter().any(|pattern| matches(pattern, &parts))
                && path.join("package.json").is_file()
            {
                members.push(path.clone());
            }
            if include.iter().any(|pattern| descend(pattern, &parts)) {
                pending.push(path);
            }
        }
    }
    members.sort();
    members
}

fn electron_app(root: &Path, package: &Value, shared: &BTreeMap<String, String>) -> bool {
    let own = dependencies(package);
    if own.contains_key("electron") || own.contains_key("electron-nightly") {
        return true;
    }
    if !shared.contains_key("electron") && !shared.contains_key("electron-nightly") {
        return false;
    }
    detect_layout(root, &own) != Layout::Plain
        || package["scripts"].as_object().is_some_and(|scripts| {
            scripts.values().filter_map(Value::as_str).any(|script| {
                script
                    .split_whitespace()
                    .any(|word| word == "electron" || word == "electron.cmd")
            })
        })
}

pub(super) fn application(
    root: &Path,
    package: &Value,
    settings: &crate::settings::Settings,
) -> Result<Option<Project>, String> {
    let patterns = patterns(root, package)?;
    let shared = dependencies(package);
    let mut applications = Vec::new();
    for member in members(root, &patterns) {
        let package = read_package(&member)?;
        if electron_app(&member, &package, &shared) {
            applications.push((member, package));
        }
    }
    match applications.len() {
        0 => Ok(None),
        1 => {
            let (root, package) = applications.pop().unwrap();
            load_package(&root, &package, shared, settings).map(Some)
        }
        _ => Err(format!(
            "multiple Electron applications in workspace: {}; analyze or build an application directory",
            applications
                .iter()
                .map(|(path, _)| relative(root, path))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests;
