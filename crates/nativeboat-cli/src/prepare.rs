//! Prepares projects whose dependencies are not installed. Their sources are
//! mirrored into a build directory of their own, where the package manager
//! the project names installs the dependencies, so that the project's folder
//! is left unchanged. Later builds update the mirror and install again only
//! when the package manifests change.

use crate::steps;
use nativeboat_project::install::{Install, MANIFESTS, Manager};
use nativeboat_project::scripts::Step;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The directory build copies are kept in unless `--work-dir` names another.
pub fn default_work_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("nativeboat")
        .join("projects")
}

/// A mirror of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Copy {
    /// The directory mirrored.
    pub(crate) source: PathBuf,
    /// Where it is mirrored.
    pub(crate) target: PathBuf,
}

impl Copy {
    /// Where the mirror holds `path`, or `path` when it lies outside the
    /// mirrored directory.
    pub(crate) fn of(&self, path: &Path) -> PathBuf {
        match path.strip_prefix(&self.source) {
            Ok(relative) if relative.as_os_str().is_empty() => self.target.clone(),
            Ok(relative) => self.target.join(relative),
            Err(_) => path.to_path_buf(),
        }
    }
}

/// Mirrors the project at `project` into `work` and installs the
/// dependencies `install` describes there. The mirror covers the repository
/// the project belongs to, or else the project and its install root, except
/// what the repository ignores, installed packages and `excluded`.
pub(crate) fn prepare(
    install: &Install,
    project: &Path,
    work: &Path,
    excluded: &[PathBuf],
    shell: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<Copy, String> {
    let scope = if install.root.starts_with(project) {
        project
    } else {
        &install.root
    };
    let repository = scope
        .ancestors()
        .find(|directory| directory.join(".git").exists());
    let source = repository.unwrap_or(scope).to_path_buf();
    let work = std::path::absolute(work).unwrap_or(work.to_path_buf());
    std::fs::create_dir_all(&work).map_err(|error| format!("{}: {error}", work.display()))?;
    let work = nativeboat_bundler::resolve::realpath(&work);
    let name = mirror_name(&source);
    let copy = Copy {
        target: work.join(&name),
        source,
    };
    let mut excluded = excluded.to_vec();
    excluded.push(work.clone());
    let files = files(&copy.source, repository.is_some(), &excluded);
    sync(&copy, &files)?;
    let root = copy.of(&install.root);
    let (command, environment) = command(install, &finder(steps::search_path(&root)))?;
    let stamp = format!("{command}\n{:016x}\n", manifests(&copy.target, &files));
    let stamp_path = work.join(format!("{name}.installed"));
    if root.join("node_modules").is_dir()
        && std::fs::read_to_string(&stamp_path).is_ok_and(|previous| previous == stamp)
    {
        let _ = writeln!(stdout, "Building from {}", copy.target.display());
        return Ok(copy);
    }
    let _ = std::fs::remove_file(&stamp_path);
    let _ = writeln!(
        stdout,
        "Installing dependencies ({} missing, such as {}) to build from {}; {} is left unchanged",
        install.missing.len(),
        install.missing[0],
        copy.target.display(),
        copy.source.display()
    );
    let step = Step {
        script: "nativeboat:install".to_owned(),
        command,
        optional: false,
    };
    steps::run_with(
        &root,
        &[step],
        shell,
        &environment,
        &mut |_, _, _| false,
        stdout,
        stderr,
    )?;
    std::fs::write(&stamp_path, stamp)
        .map_err(|error| format!("{}: {error}", stamp_path.display()))?;
    Ok(copy)
}

/// After a build step failed for packages Node could not find, installs
/// the dependencies of the folders in the project that declare them and
/// install on their own, with a lockfile of their own, as the project's
/// install installs its own: a project's build tools may have their own
/// manifest, as VS Code's `build` folder does, which its install script
/// installs and a native build does not run. Each folder is tried once.
/// Returns whether anything was installed, so the step runs again.
pub(crate) fn install_missing(
    root: &Path,
    errors: &str,
    shell: &Path,
    tried: &mut BTreeSet<PathBuf>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> bool {
    let mut installed = false;
    for (package, importer) in steps::missing_imports(errors) {
        let Some(folder) = importer
            .ancestors()
            .skip(1)
            .take_while(|folder| folder.starts_with(root))
            .find(|folder| folder.join("package.json").is_file())
        else {
            continue;
        };
        let Some(install) = nativeboat_project::install::plan(folder, true) else {
            continue;
        };
        if install.root != folder
            || !install.missing.contains(&package)
            || !tried.insert(folder.to_path_buf())
        {
            continue;
        }
        let Ok((command, environment)) = command(&install, &finder(steps::search_path(folder)))
        else {
            continue;
        };
        let _ = writeln!(
            stdout,
            "Installing the dependencies of {} for {package}, which a build step needs",
            folder.display()
        );
        let step = Step {
            script: "nativeboat:install".to_owned(),
            command,
            optional: false,
        };
        installed |= steps::run_with(
            folder,
            &[step],
            shell,
            &environment,
            &mut |_, _, _| false,
            stdout,
            stderr,
        )
        .is_ok();
    }
    installed
}

/// Installs the runtime dependencies of plugin packages with a manifest and
/// lockfile of their own (see `nativeboat_project::plugins`), and of the
/// packages around them below the project's folder, where workspaces keep
/// what their plugins share (VS Code's `extensions/node_modules`), as a
/// project's own install script installs them when the native build's
/// install does not run scripts. Installed folders cost nothing. Returns
/// whether anything was installed, so the project is read again.
pub(crate) fn install_plugin_dependencies(
    project: &nativeboat_project::Project,
    shell: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> bool {
    let folders: BTreeSet<PathBuf> = project
        .plugins
        .iter()
        .flat_map(|entry| {
            entry
                .source
                .ancestors()
                .skip(1)
                .take_while(|folder| folder.starts_with(&project.root) && *folder != project.root)
                .filter(|folder| folder.join("package.json").is_file())
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut installed = false;
    for folder in folders {
        let Some(install) = nativeboat_project::install::plan(&folder, false) else {
            continue;
        };
        if install.root != folder || install.missing.is_empty() {
            continue;
        }
        let Ok((command, environment)) = command(&install, &finder(steps::search_path(&folder)))
        else {
            continue;
        };
        let _ = writeln!(
            stdout,
            "Installing the dependencies of the plugin package {}",
            folder.display()
        );
        let step = Step {
            script: "nativeboat:install".to_owned(),
            command,
            optional: false,
        };
        installed |= steps::run_with(
            &folder,
            &[step],
            shell,
            &environment,
            &mut |_, _, _| false,
            stdout,
            stderr,
        )
        .is_ok();
    }
    installed
}

/// The name of the mirror of `source`: its folder name, made unique by a
/// hash of its path.
fn mirror_name(source: &Path) -> String {
    let name: String = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    let hash = fnv(
        FNV_OFFSET,
        source.to_string_lossy().to_lowercase().as_bytes(),
    );
    format!("{name}-{:08x}", hash & 0xffff_ffff)
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Extends a 64-bit FNV-1a hash.
fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A hash of the path and content of the package manifests among `files`.
fn manifests(target: &Path, files: &BTreeSet<PathBuf>) -> u64 {
    files
        .iter()
        .filter(|relative| {
            relative
                .file_name()
                .is_some_and(|name| MANIFESTS.iter().any(|manifest| name == *manifest))
        })
        .fold(FNV_OFFSET, |hash, relative| {
            let hash = fnv(hash, relative.to_string_lossy().as_bytes());
            fnv(
                fnv(hash, b"\0"),
                &std::fs::read(target.join(relative)).unwrap_or_default(),
            )
        })
}

/// Whether a path names a directory that is never mirrored.
fn skipped(name: &std::ffi::OsStr) -> bool {
    name == "node_modules" || name == ".git"
}

/// The files to mirror, relative to `source`: those the repository tracks
/// or does not ignore, with ignored `.env` files, or else all files.
fn files(source: &Path, repository: bool, excluded: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    match repository.then(|| listed(source)).flatten() {
        Some(entries) => {
            for relative in entries {
                let path = source.join(&relative);
                if path.is_dir() {
                    walk(source, &path, excluded, &mut files);
                } else if path.is_file() {
                    files.insert(relative);
                }
            }
        }
        None => walk(source, source, excluded, &mut files),
    }
    files.retain(|relative| {
        !relative
            .components()
            .any(|component| skipped(component.as_os_str()))
            && !excluded
                .iter()
                .any(|excluded| source.join(relative).starts_with(excluded))
    });
    files
}

/// What git lists in the repository at `source`, or `None` when it cannot.
fn listed(source: &Path) -> Option<Vec<PathBuf>> {
    let list = |options: &[&str]| -> Option<Vec<String>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(source)
            .args(["ls-files", "-z"])
            .args(options)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        Some(
            String::from_utf8_lossy(&output.stdout)
                .split('\0')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    };
    let mut entries = list(&["--cached", "--others", "--exclude-standard"])?;
    // Ignored environment files configure builds on the developer's machine.
    entries.extend(
        list(&["--others", "--ignored", "--exclude-standard", "--directory"])?
            .into_iter()
            .filter(|entry| {
                !entry.ends_with('/')
                    && entry
                        .rsplit('/')
                        .next()
                        .unwrap_or(entry)
                        .starts_with(".env")
            }),
    );
    Some(
        entries
            .iter()
            .map(|entry| entry.split('/').collect())
            .collect(),
    )
}

/// Adds the readable files below `directory` to `files`, relative to
/// `source`.
fn walk(source: &Path, directory: &Path, excluded: &[PathBuf], files: &mut BTreeSet<PathBuf>) {
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        if skipped(&entry.file_name()) || excluded.iter().any(|excluded| path.starts_with(excluded))
        {
            continue;
        }
        if path.is_dir() {
            walk(source, &path, excluded, files);
        } else if let Ok(relative) = path.strip_prefix(source) {
            files.insert(relative.to_path_buf());
        }
    }
}

/// Removes what the mirror holds beyond `files`, except installed packages,
/// and copies the files that changed into it.
fn sync(copy: &Copy, files: &BTreeSet<PathBuf>) -> Result<(), String> {
    prune(&copy.target, &copy.target, files);
    for relative in files {
        let from = copy.source.join(relative);
        let to = copy.target.join(relative);
        let current = match (from.metadata(), to.metadata()) {
            (Ok(from), Ok(to)) => {
                to.is_file() && from.len() == to.len() && from.modified().ok() == to.modified().ok()
            }
            _ => false,
        };
        if !current {
            to.parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::copy(&from, &to))
                .map_err(|error| format!("{}: {error}", to.display()))?;
        }
    }
    Ok(())
}

/// Removes the files below `directory` that are not among `files`, and the
/// directories that become empty, keeping installed packages. Returns
/// whether `directory` is empty.
fn prune(target: &Path, directory: &Path, files: &BTreeSet<PathBuf>) -> bool {
    let mut empty = true;
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let path = entry.path();
        let removed = if entry.file_name() == "node_modules" {
            false
        } else if path.is_dir() {
            prune(target, &path, files) && std::fs::remove_dir(&path).is_ok()
        } else {
            path.strip_prefix(target)
                .is_ok_and(|relative| !files.contains(relative))
                && std::fs::remove_file(&path).is_ok()
        };
        empty &= removed;
    }
    empty
}

/// Finds programs the way the Windows shell does on `path`.
fn finder(path: OsString) -> impl Fn(&str) -> Option<PathBuf> {
    move |program| {
        std::env::split_paths(&path).find_map(|directory| {
            ["com", "exe", "bat", "cmd"]
                .iter()
                .map(|extension| directory.join(format!("{program}.{extension}")))
                .find(|candidate| candidate.is_file())
        })
    }
}

/// Environment variables a command runs with.
type Environment = Vec<(&'static str, &'static str)>;

/// The command that installs the dependencies and the environment it runs
/// in. The project's package manager runs if it is installed, otherwise
/// Corepack runs the version the project pins, and otherwise npm fetches
/// it. Install scripts do not run: they prepare development environments,
/// such as downloading Electron, rebuilding native modules for it and
/// patching packages, which native applications do not use.
fn command(
    install: &Install,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Result<(String, Environment), String> {
    let manager = install.manager;
    let program = manager.program();
    let quote = |path: PathBuf| format!("\"{}\"", path.display());
    // A global Yarn 1 does not run the Yarn version a project pins.
    let pinned_berry = manager == Manager::YarnBerry && install.version.is_some();
    let corepack = install.version.is_some()
        && matches!(manager, Manager::Pnpm | Manager::Yarn | Manager::YarnBerry);
    let launcher = if let Some(found) = find(program).filter(|_| !pinned_berry) {
        quote(found)
    } else if let Some(corepack) = find("corepack").filter(|_| corepack) {
        format!("{} {program}", quote(corepack))
    } else if let Some(npm) = find("npm").filter(|_| manager != Manager::Npm) {
        format!(
            "{} exec --yes --package={}@{} -- {program}",
            quote(npm),
            manager.package(),
            install.version.as_deref().unwrap_or("latest")
        )
    } else {
        return Err(format!(
            "installing the project's dependencies needs {program}, which was not found; install Node.js from https://nodejs.org and try again"
        ));
    };
    let arguments = match manager {
        Manager::Npm => "install --ignore-scripts --include=dev --no-audit --no-fund",
        Manager::Pnpm => "install --ignore-scripts --no-frozen-lockfile",
        Manager::Yarn => "install --ignore-scripts --non-interactive",
        Manager::YarnBerry => "install --mode=skip-build --no-immutable",
        Manager::Bun => "install --ignore-scripts",
    };
    let mut environment = vec![
        ("CI", "true"),
        ("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0"),
        // Building needs the development dependencies.
        ("NODE_ENV", "development"),
    ];
    if manager == Manager::YarnBerry {
        // Native bundling resolves installed packages, not Plug'n'Play maps.
        environment.extend([
            ("YARN_NODE_LINKER", "node-modules"),
            ("YARN_ENABLE_SCRIPTS", "0"),
        ]);
    }
    Ok((format!("{launcher} {arguments}"), environment))
}

#[cfg(test)]
mod tests;
