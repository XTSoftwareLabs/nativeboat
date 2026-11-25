//! Node-style module resolution.

use serde_json::Value;
use std::path::{Component, Path, PathBuf, Prefix};

/// Node 22.15 identifiers; this registry does not implement their exports.
const NODE_BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// Node 22.15 modules that cannot shadow an unprefixed user package.
const NODE_PREFIX_BUILTINS: &[&str] = &["sea", "sqlite", "test", "test/reporters"];

fn node_builtin(specifier: &str) -> bool {
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    NODE_BUILTINS.contains(&name)
        || (specifier.starts_with("node:") && NODE_PREFIX_BUILTINS.contains(&name))
}

/// Electron modules provided by the runtime.
const ELECTRON_MODULES: &[&str] = &[
    "electron",
    "electron/main",
    "electron/renderer",
    "electron/common",
];

/// Third-party package entry points supplied by native runtime adapters.
/// Subpaths and node-prefixed names do not implicitly select an adapter.
pub const NATIVE_ADAPTERS: &[&str] = &[
    "@vscode/os-proxy-resolver",
    "@vscode/windows-ca-certs",
    "bindings",
    "ced",
    "clip-filepaths",
    "keytar",
    "native-keymap",
    "node-pty",
];

/// Native addon binaries the runtime provides, by package and binary file:
/// a package's request for its binary by relative path resolves to the
/// runtime module of this name, whether or not the binary was built.
pub const NATIVE_BINDINGS: &[&str] = &[
    "@parcel/watcher/watcher.node",
    "@vscode/deviceid/windows.node",
    "@vscode/native-watchdog/watchdog.node",
    "@vscode/policy-watcher/vscode-policy-watcher.node",
    "@vscode/spdlog/spdlog.node",
    "@vscode/sqlite3/vscode-sqlite3.node",
    "@vscode/windows-mutex/CreateMutex.node",
    "@vscode/windows-process-tree/windows_process_tree.node",
    "@vscode/windows-registry/winregistry.node",
    "native-is-elevated/iselevated.node",
];

/// The package a module belongs to: the name after its path's last
/// `node_modules`, with its scope.
fn package_of(path: &Path) -> Option<String> {
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let at = components
        .iter()
        .rposition(|component| component == "node_modules")?;
    let first = components.get(at + 1)?;
    if first.starts_with('@') {
        Some(format!("{first}/{}", components.get(at + 2)?))
    } else {
        Some(first.clone())
    }
}

/// The runtime module a package's request for a `.node` binary names, by
/// relative path, with or without the extension, as Node finds it.
fn native_binding(specifier: &str, from: &Path) -> Option<String> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return None;
    }
    let file = specifier.rsplit('/').next()?;
    let file = if file.ends_with(".node") {
        file.to_owned()
    } else {
        format!("{file}.node")
    };
    let binding = format!("{}/{file}", package_of(from)?);
    NATIVE_BINDINGS
        .contains(&binding.as_str())
        .then_some(binding)
}

/// Electron's `original-fs` is `fs` without its ASAR support. Compiled
/// applications have no ASAR archives, so it is `fs` itself.
const ORIGINAL_FS: &str = "original-fs";

/// Returns the runtime module name if `specifier` names a built-in module.
pub fn builtin(specifier: &str) -> Option<String> {
    if specifier == ORIGINAL_FS {
        return Some("fs".to_owned());
    }
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    (node_builtin(specifier)
        || ELECTRON_MODULES.contains(&name)
        || NATIVE_ADAPTERS.contains(&specifier))
    .then(|| name.to_owned())
}

/// Which process the code runs in; selects `exports` conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The Electron main process: Node-style resolution.
    Main,
    /// Renderer and preload code: browser-style resolution.
    Renderer,
    /// Native main/preload ES imports; CommonJS conditions are excluded.
    MainImport,
    /// Native renderer ES imports; CommonJS conditions are excluded.
    RendererImport,
    /// Native main/preload static requires; ES import conditions are excluded.
    MainRequire,
    /// Native renderer static requires; ES import conditions are excluded.
    RendererRequire,
    /// Stylesheet `@import`s, which take packages' `style` export condition
    /// and field first, as Vite resolves them.
    Style,
}

impl Target {
    fn conditions(self) -> &'static [&'static str] {
        match self {
            Target::Main => &[
                "node",
                "electron",
                "require",
                "import",
                "module",
                "production",
                "default",
            ],
            Target::Renderer => &[
                "browser",
                "electron",
                "import",
                "module",
                "require",
                "production",
                "default",
            ],
            Target::MainImport => &[
                "node",
                "electron",
                "import",
                "module",
                "production",
                "default",
            ],
            Target::RendererImport => &[
                "browser",
                "electron",
                "import",
                "module",
                "production",
                "default",
            ],
            Target::MainRequire => &["node", "electron", "require", "production", "default"],
            Target::RendererRequire => &["browser", "electron", "require", "production", "default"],
            Target::Style => &["style", "import", "module", "production", "default"],
        }
    }

    fn is_renderer(self) -> bool {
        matches!(
            self,
            Target::Renderer | Target::RendererImport | Target::RendererRequire
        )
    }
}

/// A path alias such as `@/*` → `src/*`, as in `tsconfig.json` `paths`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// Pattern with at most one `*`.
    pub pattern: String,
    /// Replacement text, with the same `*`.
    pub target: String,
    /// How the replacement is resolved.
    pub kind: AliasKind,
}

/// The meaning of an alias replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    /// A filesystem path, relative to the project root or absolute.
    Path,
    /// A module specifier, resolved from the importing file.
    Specifier,
}

/// The result of resolving a specifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A built-in module of the runtime.
    Builtin(String),
    /// A file on disk.
    File(PathBuf),
    /// A package's browser map explicitly replaces this module with nothing.
    Empty,
}

/// Resolves module specifiers relative to a project.
#[derive(Debug, Clone)]
pub struct Resolver {
    /// The project root.
    pub root: PathBuf,
    /// Selects package conditions.
    pub target: Target,
    /// Path aliases tried before `node_modules`.
    pub aliases: Vec<Alias>,
}

/// Extensions tried for extensionless imports, in order.
pub const EXTENSIONS: &[&str] = &[
    ".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs", ".mts", ".cts", ".json",
];

/// Joins and normalizes `.` and `..` components without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    None | Some(Component::ParentDir)
                ) {
                    normalized.push("..");
                } else {
                    normalized.pop();
                }
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// Keep canonical Windows disk/UNC paths in the same form as project paths.
/// Components preserve non-Unicode file names when removing the verbatim prefix.
fn plain_path(path: &Path) -> PathBuf {
    let mut components = path.components();
    let mut plain = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => PathBuf::from(r"\\").join(server).join(share),
            _ => return path.to_path_buf(),
        },
        _ => return path.to_path_buf(),
    };
    plain.extend(components);
    plain
}

/// Node's default module identity follows filesystem links. Synthetic entries
/// without a file retain their normalized path.
pub fn realpath(path: &Path) -> PathBuf {
    std::fs::canonicalize(path)
        .map(|path| plain_path(&path))
        .unwrap_or_else(|_| normalize(path))
}

pub(crate) fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Splits a bare specifier into the package name and the subpath (`.`-prefixed).
fn split_package(specifier: &str) -> (&str, String) {
    let mut parts = specifier.splitn(if specifier.starts_with('@') { 3 } else { 2 }, '/');
    let first = parts.next().unwrap_or_default();
    let name_end = if specifier.starts_with('@') {
        first.len() + parts.next().map_or(0, |scope_name| 1 + scope_name.len())
    } else {
        first.len()
    };
    let name = &specifier[..name_end];
    let rest = &specifier[name_end..];
    (name, format!(".{rest}"))
}

impl Resolver {
    /// The exact Node 22 builtin identifier after one project alias rewrite.
    /// Resolution-only requests return this name, never a synthetic file path.
    /// Electron and native-adapter packages do not become Node builtin names.
    pub fn node_builtin_specifier(&self, specifier: &str) -> Option<String> {
        let name = match self.alias(specifier) {
            Some((AliasKind::Specifier, replacement)) => replacement,
            Some((AliasKind::Path, _)) => return None,
            None => specifier.to_owned(),
        };
        node_builtin(&name).then_some(name)
    }

    /// The installed package directory from this importing file, including
    /// packages that provide only resources and have no JavaScript entry.
    pub fn package_directory(&self, name: &str, from: &Path) -> Option<PathBuf> {
        if builtin(name).is_some() || name.starts_with('.') || Path::new(name).is_absolute() {
            return None;
        }
        let from = realpath(from);
        let mut directory = from.parent();
        while let Some(current) = directory {
            let package = current.join("node_modules").join(name);
            if package.is_dir() {
                return Some(realpath(&package));
            }
            directory = current.parent();
        }
        None
    }
    /// Resolves `specifier` imported from the file `from`.
    pub fn resolve(&self, specifier: &str, from: &Path) -> Result<Resolved, String> {
        let found = match self.alias(specifier) {
            Some((AliasKind::Path, replacement)) => self
                .resolve_path(&self.root.join(replacement))
                .map(|path| Resolved::File(normalize(&path))),
            Some((AliasKind::Specifier, replacement)) => self.unaliased(&replacement, from),
            None => self.unaliased(specifier, from),
        };
        found.ok_or_else(|| format!("cannot resolve \"{specifier}\" from {}", from.display()))
    }

    fn unaliased(&self, specifier: &str, from: &Path) -> Option<Resolved> {
        if let Some(name) = builtin(specifier) {
            return Some(Resolved::Builtin(name));
        }
        if let Some(name) = native_binding(specifier, from) {
            return Some(Resolved::Builtin(name));
        }
        let from = realpath(from);
        let directory = from.parent().unwrap_or(&self.root);
        let found = if specifier.starts_with("./")
            || specifier.starts_with("../")
            || specifier == "."
            || specifier == ".."
        {
            return self.resolve_relative(&directory.join(specifier), &from);
        } else if let Some(rooted) = specifier.strip_prefix('/') {
            self.resolve_path(&self.root.join(rooted))
        } else if Path::new(specifier).is_absolute() {
            self.resolve_path(Path::new(specifier))
        } else {
            return self.resolve_package(specifier, directory);
        };
        found.map(|path| Resolved::File(normalize(&path)))
    }

    fn alias(&self, specifier: &str) -> Option<(AliasKind, String)> {
        for alias in &self.aliases {
            let replacement = match alias.pattern.split_once('*') {
                Some((prefix, suffix)) => {
                    let middle = specifier
                        .strip_prefix(prefix)
                        .and_then(|rest| rest.strip_suffix(suffix));
                    match middle {
                        Some(middle) => alias.target.replacen('*', middle, 1),
                        None => continue,
                    }
                }
                None if specifier == alias.pattern => alias.target.clone(),
                None => continue,
            };
            return Some((alias.kind, replacement));
        }
        None
    }

    /// Resolves a file or directory path, trying extensions and index files.
    pub fn resolve_path(&self, path: &Path) -> Option<PathBuf> {
        if path.is_file() {
            return Some(realpath(path));
        }
        // TypeScript sources import each other by the names of the compiled
        // files: `./ipc.js` is `./ipc.ts` before compilation.
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy());
        for source in crate::emit::source_extensions(extension.as_deref().unwrap_or_default()) {
            let candidate = path.with_extension(source);
            if candidate.is_file() {
                return Some(realpath(&candidate));
            }
        }
        let text = path.as_os_str().to_string_lossy();
        for extension in EXTENSIONS {
            let candidate = PathBuf::from(format!("{text}{extension}"));
            if candidate.is_file() {
                return Some(realpath(&candidate));
            }
        }
        if path.is_dir() {
            let manifest = path.join("package.json");
            if let Some(main) = read_json(&manifest)
                .as_ref()
                .and_then(|package| self.main_field(package))
                && let Some(found) = self.resolve_path(&path.join(main))
            {
                return Some(found);
            }
            return self.resolve_path(&path.join("index"));
        }
        None
    }

    fn main_field(&self, package: &Value) -> Option<String> {
        if self.target == Target::Style
            && let Some(style) = package.get("style").and_then(Value::as_str)
        {
            return Some(style.to_owned());
        }
        if self.target.is_renderer()
            && let Some(browser) = package.get("browser").and_then(Value::as_str)
        {
            return Some(browser.to_owned());
        }
        if matches!(self.target, Target::Renderer | Target::RendererImport)
            && let Some(module) = package.get("module").and_then(Value::as_str)
        {
            return Some(module.to_owned());
        }
        package
            .get("main")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn resolve_package(&self, specifier: &str, from: &Path) -> Option<Resolved> {
        let (name, subpath) = split_package(specifier);
        let mut directory = Some(from);
        while let Some(current) = directory {
            let package_directory = current.join("node_modules").join(name);
            if package_directory.is_dir() {
                return self.resolve_in_package(&package_directory, &subpath);
            }
            directory = current.parent();
        }
        None
    }

    fn resolve_in_package(&self, directory: &Path, subpath: &str) -> Option<Resolved> {
        let package = read_json(&directory.join("package.json"));
        if let Some(exports) = package.as_ref().and_then(|package| package.get("exports")) {
            let target = self.exports_target(exports, subpath)?;
            return self
                .resolve_path(&directory.join(target))
                .map(Resolved::File);
        }
        if self.target.is_renderer()
            && let Some(Value::Object(map)) =
                package.as_ref().and_then(|package| package.get("browser"))
        {
            let key = if subpath == "." { "." } else { subpath };
            for candidate in [key.to_owned(), format!("{key}.js")] {
                match map.get(&candidate) {
                    Some(Value::String(replacement)) => {
                        return self
                            .resolve_path(&directory.join(replacement))
                            .map(Resolved::File);
                    }
                    Some(Value::Bool(false)) => return Some(Resolved::Empty),
                    _ => {}
                }
            }
        }
        let resolved = self.resolve_path(&directory.join(subpath))?;
        // The map also replaces the file the package's entry names, such as
        // its `main`, as bundlers resolve it.
        if self.target.is_renderer()
            && let Some(Value::Object(map)) =
                package.as_ref().and_then(|package| package.get("browser"))
            && let Some(replaced) = self.browser_replacement(&realpath(directory), map, &resolved)
        {
            return replaced;
        }
        Some(Resolved::File(resolved))
    }

    /// A file's replacement in its package's `browser` map, which names files
    /// by their path from the package root, with or without an extension:
    /// `None` when the map does not name the file.
    fn browser_replacement(
        &self,
        root: &Path,
        map: &serde_json::Map<String, Value>,
        resolved: &Path,
    ) -> Option<Option<Resolved>> {
        let relative = resolved.strip_prefix(root).ok()?;
        let mut key = String::from("./");
        key.push_str(&relative.to_string_lossy().replace('\\', "/"));
        let extensionless = Path::new(&key)
            .with_extension("")
            .to_string_lossy()
            .into_owned();
        for candidate in [key, extensionless] {
            match map.get(&candidate) {
                Some(Value::String(replacement)) => {
                    return Some(
                        self.resolve_path(&root.join(replacement))
                            .map(Resolved::File),
                    );
                }
                Some(Value::Bool(false)) => return Some(Some(Resolved::Empty)),
                _ => {}
            }
        }
        None
    }

    /// Applies a package's `browser` file map to relative imports as well as
    /// package subpaths. Packages such as Axios import their Node adapters
    /// relatively, so consulting the map only for bare package specifiers
    /// accidentally pulls `http`, `https`, and `net` into renderer graphs.
    fn resolve_relative(&self, path: &Path, from: &Path) -> Option<Resolved> {
        let resolved = self.resolve_path(path)?;
        if !self.target.is_renderer() {
            return Some(Resolved::File(resolved));
        }
        let from = realpath(from);
        let mut ancestor = Some(if from.is_dir() {
            from
        } else {
            from.parent()?.to_path_buf()
        });
        while let Some(current) = ancestor {
            let manifest = current.join("package.json");
            if let Some(package) = read_json(&manifest) {
                let root = manifest.parent()?;
                if resolved.starts_with(root)
                    && let Some(Value::Object(map)) = package.get("browser")
                    && let Some(replaced) = self.browser_replacement(root, map, &resolved)
                {
                    return replaced;
                }
                return Some(Resolved::File(resolved));
            }
            ancestor = current.parent().map(Path::to_path_buf);
        }
        Some(Resolved::File(resolved))
    }

    /// Resolves `subpath` through a package's `exports` field.
    fn exports_target(&self, exports: &Value, subpath: &str) -> Option<String> {
        let map = match exports {
            Value::Object(map) if map.keys().any(|key| key.starts_with('.')) => map,
            _ => {
                return (subpath == ".")
                    .then(|| self.condition_target(exports, None))
                    .flatten();
            }
        };
        if let Some(entry) = map.get(subpath) {
            return self.condition_target(entry, None);
        }
        let mut best: Option<(&str, &Value, String)> = None;
        for (key, value) in map {
            let Some((prefix, suffix)) = key.split_once('*') else {
                continue;
            };
            let Some(middle) = subpath
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
            else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(current, _, _)| prefix.len() > current.len())
            {
                best = Some((prefix, value, middle.to_owned()));
            }
        }
        let (_, value, middle) = best?;
        self.condition_target(value, Some(&middle))
    }

    fn condition_target(&self, value: &Value, wildcard: Option<&str>) -> Option<String> {
        match value {
            Value::String(target) => Some(match wildcard {
                Some(middle) => target.replace('*', middle),
                None => target.clone(),
            }),
            Value::Array(options) => options
                .iter()
                .find_map(|option| self.condition_target(option, wildcard)),
            Value::Object(conditions) => conditions
                .iter()
                .filter(|(key, _)| self.target.conditions().contains(&key.as_str()))
                .find_map(|(_, target)| self.condition_target(target, wildcard)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
