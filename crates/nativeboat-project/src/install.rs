//! What installing a project's dependencies takes: whether the packages it
//! needs are installed, the directory a package manager installs them from,
//! and the package manager its `packageManager` field or lockfile names.

use crate::read_package;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A package manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    /// npm.
    Npm,
    /// pnpm.
    Pnpm,
    /// Yarn 1.
    Yarn,
    /// Yarn 2 and later.
    YarnBerry,
    /// Bun.
    Bun,
}

impl Manager {
    /// The program that runs it.
    pub fn program(self) -> &'static str {
        match self {
            Manager::Npm => "npm",
            Manager::Pnpm => "pnpm",
            Manager::Yarn | Manager::YarnBerry => "yarn",
            Manager::Bun => "bun",
        }
    }

    /// The npm package that provides the program.
    pub fn package(self) -> &'static str {
        match self {
            Manager::YarnBerry => "@yarnpkg/cli-dist",
            other => other.program(),
        }
    }
}

/// How to install the packages a project needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// The directory the package manager installs from: the application's
    /// own when it has a lockfile, or else the nearest declaring a workspace.
    pub root: PathBuf,
    /// The package manager.
    pub manager: Manager,
    /// The version the `packageManager` field pins.
    pub version: Option<String>,
    /// The needed packages that are not installed.
    pub missing: Vec<String>,
}

/// Lockfiles in the order their package managers are preferred.
const LOCKFILES: &[(&str, Manager)] = &[
    ("pnpm-lock.yaml", Manager::Pnpm),
    ("yarn.lock", Manager::Yarn),
    ("bun.lock", Manager::Bun),
    ("bun.lockb", Manager::Bun),
    ("package-lock.json", Manager::Npm),
    ("npm-shrinkwrap.json", Manager::Npm),
];

/// The files whose content decides what an installation contains.
pub const MANIFESTS: &[&str] = &[
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "package-lock.json",
    "npm-shrinkwrap.json",
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    "bunfig.toml",
];

/// What installing the packages the application at `root` needs takes, or
/// `None` when they are installed. It needs its runtime dependencies and,
/// with `tools`, the development dependencies of it and of the directory it
/// installs from, which its build scripts run. Optional packages and those
/// nativeboat provides itself are not needed.
pub fn plan(root: &Path, tools: bool) -> Option<Install> {
    let package = read_package(root).ok()?;
    let install = install_root(root);
    let top = read_package(&install).unwrap_or(Value::Null);
    let mut wanted = needed(&package, tools);
    if tools {
        wanted.extend(needed(&top, true));
    }
    let missing: Vec<String> = wanted
        .into_iter()
        .filter(|name| !installed(root, name))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let (manager, version) = manager(&install, &top);
    Some(Install {
        root: install,
        manager,
        version,
        missing,
    })
}

/// The packages a manifest needs.
fn needed(package: &Value, development: bool) -> BTreeSet<String> {
    let optional = package["optionalDependencies"].as_object();
    let sections: &[&str] = if development {
        &["dependencies", "devDependencies"]
    } else {
        &["dependencies"]
    };
    sections
        .iter()
        .filter_map(|section| package[*section].as_object())
        .flat_map(|entries| entries.keys())
        .filter(|name| !optional.is_some_and(|optional| optional.contains_key(*name)))
        .filter(|name| !provided(name))
        .cloned()
        .collect()
}

/// Whether nativeboat provides a package itself.
fn provided(name: &str) -> bool {
    name == "electron-nightly" || nativeboat_bundler::resolve::builtin(name).is_some()
}

/// Whether Node finds an installed package from `root`.
fn installed(root: &Path, name: &str) -> bool {
    root.ancestors().any(|directory| {
        directory
            .join("node_modules")
            .join(name)
            .join("package.json")
            .is_file()
    })
}

/// The directory a package manager installs `root`'s dependencies from:
/// `root` itself when it has a lockfile, or else the nearest directory
/// within the repository declaring a workspace. Lockfiles of directories
/// that declare no workspace do not cover the application.
fn install_root(root: &Path) -> PathBuf {
    if LOCKFILES
        .iter()
        .any(|(lockfile, _)| root.join(lockfile).is_file())
    {
        return root.to_path_buf();
    }
    for directory in root.ancestors() {
        if directory.join("pnpm-workspace.yaml").is_file()
            || read_package(directory).is_ok_and(|package| !package["workspaces"].is_null())
        {
            return directory.to_path_buf();
        }
        if directory.join(".git").exists() {
            break;
        }
    }
    root.to_path_buf()
}

/// The package manager the `packageManager` field pins, with its version,
/// or else the one whose lockfile or workspace file is present.
fn manager(root: &Path, package: &Value) -> (Manager, Option<String>) {
    let berry =
        |major: Option<u32>| major.map_or(root.join(".yarnrc.yml").is_file(), |major| major >= 2);
    if let Some((name, version)) = package["packageManager"]
        .as_str()
        .and_then(|field| field.split_once('@'))
    {
        let version = version.split('+').next().unwrap_or_default();
        let major = version
            .split('.')
            .next()
            .and_then(|major| major.parse().ok());
        let manager = match name {
            "npm" => Some(Manager::Npm),
            "pnpm" => Some(Manager::Pnpm),
            "yarn" if berry(major) => Some(Manager::YarnBerry),
            "yarn" => Some(Manager::Yarn),
            "bun" => Some(Manager::Bun),
            _ => None,
        };
        if let Some(manager) = manager {
            return (manager, Some(version.to_owned()));
        }
    }
    let manager = LOCKFILES
        .iter()
        .find(|(lockfile, _)| root.join(lockfile).is_file())
        .map(|(_, manager)| *manager)
        .or_else(|| {
            root.join("pnpm-workspace.yaml")
                .is_file()
                .then_some(Manager::Pnpm)
        })
        .unwrap_or(Manager::Npm);
    let manager = if manager == Manager::Yarn && berry(None) {
        Manager::YarnBerry
    } else {
        manager
    };
    (manager, None)
}

#[cfg(test)]
mod tests;
