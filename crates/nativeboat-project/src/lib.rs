//! Electron project discovery.
//!
//! Reads `package.json` and the build tool conventions of the project
//! (plain Electron, `tsc`, electron-vite, vite-plugin-electron and the
//! Electron Forge Vite plugin) to find the main process entry, preload
//! scripts and renderer pages, and maps each source to the path the main
//! process loads at run time.

pub mod builder;
pub mod config;
pub mod documents;
mod electron_vite;
mod glob;
pub mod install;
pub mod plugins;
pub mod scan;
pub mod scripts;
pub mod settings;
pub mod tsconfig;
pub mod vue;
mod workspace;

use config::{
    first_existing, load_env, tsconfig_aliases, tsconfig_decorators, tsconfig_modules, vite_aliases,
};
use nativeboat_bundler::{Alias, Emit, PathMapping, Resolver, Target};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How the project is built for Electron.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Sources are loaded by Electron directly, or after `tsc` compiles them
    /// file by file.
    Plain,
    /// `electron-vite`: `src/main`, `src/preload`, `src/renderer`, output in `out/`.
    ElectronVite,
    /// `vite-plugin-electron`: `electron/`, root `index.html`, output in `dist*/`.
    VitePluginElectron,
    /// Electron Forge with the Vite plugin, output in `.vite/`.
    ForgeVite,
}

/// A source file and the paths the application loads it from at run time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The source file.
    pub source: PathBuf,
    /// Paths relative to the project root, with `/` separators.
    pub runtime_paths: Vec<String>,
}

/// Application metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// The package name.
    pub name: String,
    /// The display name, used for the executable.
    pub product_name: String,
    /// The name Electron gives the running app (`app.name`): the package's
    /// `productName`, else its `name`; packaging options do not change it.
    pub app_name: String,
    /// The version.
    pub version: String,
    /// The description.
    pub description: String,
    /// The application id (`build.appId`).
    pub app_id: Option<String>,
    /// The application icon, if it exists.
    pub icon: Option<PathBuf>,
    /// The Electron locales the application keeps (electron-builder's
    /// `electronLanguages`); empty when it keeps them all.
    pub locales: Vec<String>,
}

/// A discovered Electron project.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The project root.
    pub root: PathBuf,
    /// Application metadata.
    pub metadata: Metadata,
    /// All dependencies and their version ranges.
    pub dependencies: BTreeMap<String, String>,
    /// The build layout.
    pub layout: Layout,
    /// The main process entry.
    pub main: Entry,
    /// Preload scripts.
    pub preloads: Vec<Entry>,
    /// Modules the program starts as child processes with `fork()`
    /// (Electron's `utilityProcess.fork`, Node's `child_process.fork`): each
    /// is compiled as an entry of its own, which the program runs when it
    /// starts as that child.
    pub forks: Vec<Entry>,
    /// The main modules of plugin packages (see [`plugins`]): entries the
    /// program loads when it requires or imports their paths.
    pub plugins: Vec<Entry>,
    /// Renderer pages.
    pub pages: Vec<Entry>,
    /// Base directory for root-relative renderer HTML resources.
    pub renderer_root: PathBuf,
    /// A configured renderer build that needs the project's plugin pipeline.
    pub renderer_build: Option<RendererBuild>,
    /// Portable paths for freshly prepared frontend files.
    pub mappings: Vec<PathMapping>,
    /// Module aliases for the main process.
    pub aliases: Vec<Alias>,
    /// Module aliases for preload scripts.
    pub preload_aliases: Vec<Alias>,
    /// Module aliases for renderer pages.
    pub renderer_aliases: Vec<Alias>,
    /// Where `tsc` writes compiled sources, for a plain layout.
    pub emits: Vec<Emit>,
    /// Whether TypeScript's `experimentalDecorators` are on.
    pub legacy_decorators: bool,
    /// Whether `emitDecoratorMetadata` is on as well.
    pub decorator_metadata: bool,
    /// How TypeScript compiles JSX and imports.
    pub typescript: TypeScript,
    /// `import.meta.env` for the main process.
    pub main_env: BTreeMap<String, Value>,
    /// `import.meta.env` for preload scripts.
    pub preload_env: BTreeMap<String, Value>,
    /// `import.meta.env` for renderer pages.
    pub renderer_env: BTreeMap<String, Value>,
    /// Compile-time defines for the main process.
    pub defines: Vec<(String, String)>,
    /// Compile-time defines for preload scripts.
    pub preload_defines: Vec<(String, String)>,
    /// Compile-time defines for renderer pages.
    pub renderer_defines: Vec<(String, String)>,
    /// Files copied next to the executable (electron-builder's
    /// `extraResources` and `extraFiles`).
    pub extra_files: Vec<builder::ExtraFile>,
    /// The application's own files copied beside the executable, as
    /// electron-builder's `files` selects them, without compiled sources.
    pub application_files: Vec<builder::ExtraFile>,
    /// The update feed written to resources/app-update.yml: the native
    /// build's own `nativeboat.publish`, or electron-builder's publish
    /// settings.
    pub updater_config: Option<Value>,
    /// The Windows installer native builds make for electron-builder's NSIS
    /// target, which updates install.
    pub installer: builder::Installer,
    /// Commands to run before compiling, from the project's scripts.
    pub build_steps: Vec<scripts::Step>,
    /// Where the executable goes unless told otherwise, relative to the root.
    pub output_directory: String,
    /// Non-fatal problems.
    pub warnings: Vec<String>,
}

/// The TypeScript configuration's settings for how modules compile, as
/// written in `compilerOptions`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeScript {
    /// `jsx`, such as `react-jsx` or `react`.
    pub jsx: Option<String>,
    /// `jsxImportSource`, the package of the automatic runtime.
    pub jsx_import_source: Option<String>,
    /// `jsxFactory`, such as `h`, for the classic runtime.
    pub jsx_factory: Option<String>,
    /// `jsxFragmentFactory`, for the classic runtime.
    pub jsx_fragment_factory: Option<String>,
    /// `verbatimModuleSyntax`: imports stay as written.
    pub verbatim_module_syntax: bool,
    /// `useDefineForClassFields: false`, given or defaulted from `target`:
    /// classes assign their fields, after their parameter properties.
    pub assign_class_fields: bool,
}

/// The portable directory written by the configured renderer build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererBuild {
    /// Directory inside resources/app, with `/` separators.
    pub runtime_root: String,
}

impl Project {
    /// Whether the project depends on a package.
    pub fn depends_on(&self, name: &str) -> bool {
        self.dependencies.contains_key(name)
    }

    /// Keeps prepared files' runtime URLs independent of the build directory.
    pub fn runtime_path(&self, path: &Path) -> String {
        self.mappings
            .iter()
            .find_map(|mapping| mapping.map(path))
            .unwrap_or_else(|| relative(&self.root, path))
    }

    /// Uses only output produced by the current build, retaining configured
    /// page URLs and leaving the main and preload source graphs unchanged.
    pub fn apply_renderer_output(&mut self, output: &Path) -> Result<(), String> {
        let Some(build) = &self.renderer_build else {
            return Ok(());
        };
        let directory = nativeboat_bundler::resolve::realpath(&output.join("renderer"));
        let pages = self
            .pages
            .iter()
            .map(|entry| {
                let relative = entry.source.strip_prefix(&self.renderer_root).map_or_else(
                    |_| PathBuf::from(file_name(&entry.source)),
                    Path::to_path_buf,
                );
                let source = directory.join(relative);
                if !source.is_file() {
                    return Err(format!(
                        "the configured renderer build did not produce {}",
                        source.display()
                    ));
                }
                Ok(Entry {
                    source,
                    runtime_paths: entry.runtime_paths.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.mappings = vec![PathMapping {
            source_root: directory.clone(),
            runtime_root: build.runtime_root.clone(),
        }];
        self.renderer_root = directory;
        self.pages = pages;
        Ok(())
    }
}

fn relative(root: &Path, path: &Path) -> String {
    let root = nativeboat_bundler::resolve::normalize(root);
    let path = nativeboat_bundler::resolve::normalize(path);
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn string(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn metadata(root: &Path, package: &Value, build: &Value) -> Metadata {
    let name = string(&package["name"]).unwrap_or_else(|| "app".to_owned());
    let app_name = string(&package["productName"]).unwrap_or_else(|| name.clone());
    let product_name = string(&package["productName"])
        .or_else(|| string(&build["productName"]))
        .unwrap_or_else(|| name.clone());
    let icon = [&build["win"]["icon"], &build["icon"]]
        .into_iter()
        .filter_map(string)
        .map(|icon| root.join(icon))
        .find(|icon| icon.is_file());
    Metadata {
        name,
        product_name,
        app_name,
        version: string(&package["version"]).unwrap_or_else(|| "0.0.0".to_owned()),
        description: string(&package["description"]).unwrap_or_default(),
        app_id: string(&build["appId"]),
        icon,
        locales: [
            &build["win"]["electronLanguages"],
            &build["electronLanguages"],
        ]
        .into_iter()
        .find(|locales| !locales.is_null())
        .map(|locales| match locales {
            Value::Array(items) => items.iter().filter_map(string).collect(),
            other => string(other).into_iter().collect(),
        })
        .unwrap_or_default(),
    }
}

fn detect_layout(root: &Path, dependencies: &BTreeMap<String, String>) -> Layout {
    let electron_vite_config = first_existing(root, electron_vite::CONFIGS);
    if electron_vite_config.is_some() || dependencies.contains_key("electron-vite") {
        Layout::ElectronVite
    } else if dependencies.contains_key("@electron-forge/plugin-vite") {
        Layout::ForgeVite
    } else if dependencies.contains_key("vite-plugin-electron") {
        Layout::VitePluginElectron
    } else {
        Layout::Plain
    }
}

/// Lists `*.html` files directly inside a directory, sorted.
fn html_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_html(path))
        .collect();
    files.sort();
    files
}

fn is_html(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn env_for(all: &BTreeMap<String, String>, prefixes: &[&str]) -> BTreeMap<String, Value> {
    let mut env: BTreeMap<String, Value> = all
        .iter()
        .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect();
    env.insert("MODE".to_owned(), Value::String("production".to_owned()));
    env.insert("PROD".to_owned(), Value::Bool(true));
    env.insert("DEV".to_owned(), Value::Bool(false));
    env.insert("SSR".to_owned(), Value::Bool(false));
    env.insert("BASE_URL".to_owned(), Value::String("/".to_owned()));
    env
}

/// Output directories that build tools commonly compile sources into.
const OUTPUT_DIRECTORIES: [&str; 6] = ["dist", "build", "out", "lib", "app", "output"];

/// The source of a file the application loads: the file itself, or the
/// source a compiled file comes from.
fn source_of(root: &Path, emits: &[Emit], resolver: &Resolver, path: &Path) -> Option<PathBuf> {
    // Sources win over what a build put in an output directory, which may
    // be stale or partial.
    let output = relative(root, path);
    emits
        .iter()
        .flat_map(|emit| emit.sources(&output))
        .find_map(|source| resolver.resolve_path(&root.join(source)))
        .or_else(|| resolver.resolve_path(path))
}

/// The paths, relative to the root, a source is loaded by at run time: its
/// own path and where `tsc` emits it.
fn runtime_paths(
    root: &Path,
    emits: &[Emit],
    source: &Path,
    mut known: Vec<String>,
) -> Vec<String> {
    let own = relative(root, source);
    known.extend(emits.iter().filter_map(|emit| emit.output(&own)));
    // A compiled module's `import.meta.url` names its source, so a program
    // that names a module's emitted JavaScript relative to the compiled
    // modules' root, as VS Code's `FileAccess.asFileUri('bootstrap-fork.js')`
    // does, names the source's path with the emitted extension.
    if let Some(emitted) = emitted_in_place(&own) {
        known.push(emitted);
    }
    known.push(own);
    let mut paths = Vec::new();
    for path in known {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// A TypeScript source's path with the extension `tsc` gives its output in
/// place: `.ts` to `.js`, `.mts` to `.mjs`, `.cts` to `.cjs` and `.tsx` to
/// `.jsx`; `None` for other files and declaration files.
fn emitted_in_place(path: &str) -> Option<String> {
    if path.ends_with(".d.ts") || path.ends_with(".d.mts") || path.ends_with(".d.cts") {
        return None;
    }
    let (stem, extension) = path.rsplit_once('.')?;
    let emitted = match extension {
        "ts" => "js",
        "mts" => "mjs",
        "cts" => "cjs",
        "tsx" => "jsx",
        _ => return None,
    };
    Some(format!("{stem}.{emitted}"))
}

/// Finds the file a path in main-process code names. The path may be
/// relative to the source file, to where `tsc` emits that file, to an output
/// directory's root (resource paths such as VS Code's
/// `FileAccess.asBrowserUri('vs/…/workbench.html')`, resolved against the
/// compiled modules' root) or to the application root (`app.getAppPath()`).
/// Returns the source and the paths the application loads it by.
fn locate(
    root: &Path,
    emits: &[Emit],
    resolver: &Resolver,
    reference: &scan::Reference,
) -> Option<(PathBuf, Vec<String>)> {
    let directory = relative(root, &reference.directory);
    let mut bases = vec![reference.directory.clone()];
    bases.extend(
        emits
            .iter()
            .filter_map(|emit| emit.output(&format!("{directory}/file.js")))
            .filter_map(|output| root.join(output).parent().map(Path::to_path_buf)),
    );
    bases.push(root.to_path_buf());
    bases.extend(
        emits
            .iter()
            .filter(|emit| !emit.out_dir.is_empty())
            .map(|emit| root.join(&emit.out_dir)),
    );
    let mut found: Option<PathBuf> = None;
    let mut paths = Vec::new();
    for base in bases {
        let candidate = nativeboat_bundler::resolve::normalize(&base.join(&reference.literal));
        if let Some(source) = source_of(root, emits, resolver, &candidate)
            && found.get_or_insert_with(|| source.clone()) == &source
        {
            paths.push(relative(root, &candidate));
        }
    }
    let source = found?;
    let paths = runtime_paths(root, emits, &source, paths);
    Some((source, paths))
}

/// Adds a located entry, merging the runtime paths of one source.
fn add_entry(entries: &mut Vec<Entry>, (source, paths): (PathBuf, Vec<String>)) {
    match entries.iter_mut().find(|entry| entry.source == source) {
        Some(entry) => {
            for path in paths {
                if !entry.runtime_paths.contains(&path) {
                    entry.runtime_paths.push(path);
                }
            }
        }
        None => entries.push(Entry {
            source,
            runtime_paths: paths,
        }),
    }
}

fn read_package(root: &Path) -> Result<Value, String> {
    let manifest = root.join("package.json");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("invalid {}: {error}", manifest.display()))
}

fn dependencies(package: &Value) -> BTreeMap<String, String> {
    let mut dependencies = BTreeMap::new();
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(entries) = package[section].as_object() {
            for (name, version) in entries {
                dependencies.insert(
                    name.clone(),
                    version.as_str().unwrap_or_default().to_owned(),
                );
            }
        }
    }
    dependencies
}

/// The Electron version a project fetches itself and builds its native
/// modules against, as `.npmrc` tells node-gyp and prebuild:
/// `runtime=electron` with `target=<version>`.
fn npmrc_electron(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join(".npmrc")).ok()?;
    let mut runtime = None;
    let mut target = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with(['#', ';']) {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).trim();
        match key.trim() {
            "runtime" => runtime = Some(value.to_owned()),
            "target" => target = Some(value.to_owned()),
            _ => {}
        }
    }
    target.filter(|target| runtime.as_deref() == Some("electron") && !target.is_empty())
}

/// Discovers the Electron application, including a unique app inside a
/// declared pnpm, npm or Yarn workspace.
pub fn load(root: &Path) -> Result<Project, String> {
    load_with(root, &settings::Settings::default())
}

/// Loads a project with NativeBoat's own settings for the build, which
/// come before what the project's files say.
pub fn load_with(root: &Path, settings: &settings::Settings) -> Result<Project, String> {
    // Physical absolute roots agree with Node module identity, including
    // Windows short names and linked workspace directories.
    let root = &std::path::absolute(root).unwrap_or(root.to_path_buf());
    let root = &nativeboat_bundler::resolve::realpath(root);
    let package = read_package(root)?;
    match load_package(root, &package, BTreeMap::new(), settings) {
        Ok(project) => Ok(project),
        Err(error) => {
            // An explicitly configured root application owns its diagnostics.
            // Workspace discovery only replaces a tooling root's fallback.
            if package["main"].is_string() || first_existing(root, electron_vite::CONFIGS).is_some()
            {
                return Err(error);
            }
            workspace::application(root, &package, settings)?.ok_or(error)
        }
    }
}

fn load_package(
    root: &Path,
    package: &Value,
    mut dependencies: BTreeMap<String, String>,
    settings: &settings::Settings,
) -> Result<Project, String> {
    let own_dependencies = self::dependencies(package);
    let layout = detect_layout(root, &own_dependencies);
    dependencies.extend(own_dependencies);
    if !dependencies.contains_key("electron")
        && !dependencies.contains_key("electron-nightly")
        && let Some(version) = npmrc_electron(root)
    {
        dependencies.insert("electron".to_owned(), version);
    }
    if !dependencies.contains_key("electron") && !dependencies.contains_key("electron-nightly") {
        return Err(format!(
            "{} is not an Electron project: package.json does not depend on electron",
            root.display()
        ));
    }
    let main_field = string(&package["main"]).unwrap_or_else(|| "index.js".to_owned());
    let main_field = main_field
        .strip_prefix("./")
        .unwrap_or(&main_field)
        .to_owned();

    let mut aliases = tsconfig_aliases(root);
    for config in [
        "vite.config.ts",
        "vite.config.mts",
        "vite.config.js",
        "vite.config.mjs",
        "vite.renderer.config.ts",
        "vite.renderer.config.mjs",
    ]
    .into_iter()
    {
        let path = root.join(config);
        if path.is_file() {
            for alias in vite_aliases(&path, root) {
                if !aliases
                    .iter()
                    .any(|existing| existing.pattern == alias.pattern)
                {
                    aliases.push(alias);
                }
            }
        }
    }
    let mut resolver = Resolver {
        root: root.to_path_buf(),
        target: Target::Main,
        aliases: aliases.clone(),
    };
    let vite = if layout == Layout::ElectronVite {
        Some(electron_vite::discover(root, &resolver, &main_field)?)
    } else {
        None
    };
    let mut preload_aliases = aliases.clone();
    let mut renderer_aliases = aliases.clone();
    if let Some(vite) = &vite {
        for (destination, configured) in [&mut aliases, &mut preload_aliases, &mut renderer_aliases]
            .into_iter()
            .zip(&vite.aliases)
        {
            let inherited = std::mem::take(destination);
            destination.extend(configured.iter().cloned());
            destination.extend(inherited);
        }
        resolver.aliases = aliases.clone();
    }
    let renderer_root = vite
        .as_ref()
        .map_or_else(|| root.to_path_buf(), |vite| vite.renderer_root.clone());
    let renderer_build = vite
        .as_ref()
        .filter(|vite| vite.renderer_transforms)
        .map(|vite| RendererBuild {
            runtime_root: relative(root, &vite.renderer_output),
        });

    let conventional_main: &[&str] = match layout {
        Layout::ElectronVite => &[],
        Layout::VitePluginElectron => &[
            "electron/main.ts",
            "electron/main.js",
            "electron/main/index.ts",
            "electron/main/index.js",
        ],
        Layout::ForgeVite => &["src/main.ts", "src/main.js", "src/index.ts", "src/index.js"],
        Layout::Plain => &[],
    };
    let mut emits = if layout == Layout::Plain {
        tsconfig::emits(root, package)
    } else {
        Vec::new()
    };
    let main_path = root.join(&main_field);
    let mut main_source = vite
        .as_ref()
        .and_then(|vite| vite.main.as_ref())
        .map(|entry| entry.source.clone())
        .or_else(|| first_existing(root, conventional_main))
        .or_else(|| source_of(root, &emits, &resolver, &main_path));
    // Without a TypeScript configuration that says so, sources may still be
    // compiled into an output directory, from `src` or from the root.
    let output_directory = main_field.split('/').next().unwrap_or_default();
    if main_source.is_none() && OUTPUT_DIRECTORIES.contains(&output_directory) {
        for root_dir in ["src", ""] {
            let emit = Emit {
                root_dir: root_dir.to_owned(),
                out_dir: output_directory.to_owned(),
            };
            main_source = source_of(root, std::slice::from_ref(&emit), &resolver, &main_path);
            if main_source.is_some() {
                emits.push(emit);
                break;
            }
        }
    }
    let main_source = main_source
        .ok_or_else(|| format!("cannot find the main process entry \"{main_field}\""))?;
    let main = Entry {
        runtime_paths: if layout == Layout::Plain {
            runtime_paths(root, &emits, &main_source, Vec::new())
        } else {
            vec![main_field.clone()]
        },
        source: main_source,
    };
    let scan = scan::scan_main(&main.source, &resolver);
    let mut warnings = Vec::new();
    let mut defines = Vec::new();
    let mut preload_defines = Vec::new();
    let mut renderer_defines = Vec::new();

    let mut preloads = Vec::new();
    let mut pages = Vec::new();
    match layout {
        Layout::ElectronVite => {
            let vite = vite.unwrap();
            [defines, preload_defines, renderer_defines] = vite.defines;
            preloads = vite.preloads;
            pages = vite.pages;
        }
        Layout::VitePluginElectron => {
            if let Some(source) = first_existing(
                root,
                &[
                    "electron/preload.ts",
                    "electron/preload.js",
                    "electron/preload.mts",
                    "electron/preload/index.ts",
                    "electron/preload/index.js",
                ],
            ) {
                preloads.push(Entry {
                    source,
                    runtime_paths: vec![
                        "dist-electron/preload.js".to_owned(),
                        "dist-electron/preload.mjs".to_owned(),
                        "dist-electron/preload/index.js".to_owned(),
                        "dist-electron/preload/index.mjs".to_owned(),
                    ],
                });
            }
            for source in html_files(root) {
                let runtime = format!("dist/{}", file_name(&source));
                pages.push(Entry {
                    source,
                    runtime_paths: vec![runtime],
                });
            }
        }
        Layout::ForgeVite => {
            if let Some(source) = first_existing(root, &["src/preload.ts", "src/preload.js"]) {
                preloads.push(Entry {
                    source,
                    runtime_paths: vec![".vite/build/preload.js".to_owned()],
                });
            }
            for global in &scan.forge_globals {
                if let Some(window) = global.strip_suffix("_VITE_NAME") {
                    defines.push((
                        global.clone(),
                        format!("\"{}\"", window.to_ascii_lowercase()),
                    ));
                } else {
                    defines.push((global.clone(), "undefined".to_owned()));
                }
            }
            for source in html_files(root) {
                let runtime = format!(".vite/renderer/main_window/{}", file_name(&source));
                pages.push(Entry {
                    source,
                    runtime_paths: vec![runtime],
                });
            }
        }
        Layout::Plain => {
            for reference in &scan.preloads {
                // Main-process modules are not preload scripts.
                if let Some(found) = locate(root, &emits, &resolver, reference)
                    && !scan.files.contains(&found.0)
                {
                    add_entry(&mut preloads, found);
                }
            }
            for reference in &scan.pages {
                if let Some(found) =
                    locate(root, &emits, &resolver, reference).filter(|(source, _)| is_html(source))
                {
                    add_entry(&mut pages, found);
                }
            }
            if pages.is_empty()
                && let Some(source) = first_existing(root, &["index.html", "src/index.html"])
            {
                pages.push(Entry {
                    runtime_paths: vec![relative(root, &source)],
                    source,
                });
            }
        }
    }
    let mut forks = Vec::new();
    for reference in &scan.forks {
        if let Some(found) = locate(root, &emits, &resolver, reference)
            && !is_html(&found.0)
        {
            add_entry(&mut forks, found);
        }
    }
    // Modules forked by paths the program builds while it runs, such as a
    // plugin's language server, are named in the `forks` setting.
    let fork_paths = settings.forks.clone().or_else(|| {
        package["nativeboat"]
            .get("forks")
            .and_then(plugins::patterns)
    });
    for path in fork_paths.iter().flatten() {
        match plugins::module_entry(root, path) {
            Some(entry) => add_entry(&mut forks, (entry.source, entry.runtime_paths)),
            None => warnings.push(format!(
                "the forked module \"{path}\" has no file and no TypeScript source its tsconfig.json emits it from"
            )),
        }
    }
    // Documents the pages show in frames are compiled as pages are.
    let document_patterns = settings.documents.clone().or_else(|| {
        package["nativeboat"]
            .get("documents")
            .and_then(plugins::patterns)
    });
    documents::add_frames(root, &mut pages, document_patterns.as_deref());
    // Applications without windows, such as tray applications and
    // programs, have no pages to find.
    if pages.is_empty() && scan.opens_windows {
        warnings.push("no renderer HTML page was found".to_owned());
    }
    let env = load_env(root);
    let compiler_options = config::compiler_options(root, &main.source);
    let (legacy_decorators, decorator_metadata) = tsconfig_decorators(&compiler_options);
    let typescript = tsconfig_modules(&compiler_options);
    let build = builder::config(root, package)?;
    let application_package = builder::application_package(root, &build, package);
    let mut missing = Vec::new();
    let extra_files = builder::extra_files(root, &build, &mut warnings, &mut missing);
    let origin = builder::origin_remote(root);
    let updater_config = builder::updater_config(
        &build,
        &application_package,
        settings.publish.as_ref(),
        origin.as_deref(),
    )?;
    let plugin_patterns = settings.plugins.clone().or_else(|| {
        application_package["nativeboat"]
            .get("plugins")
            .and_then(plugins::patterns)
    });
    let (plugins, plugin_warnings) = match &plugin_patterns {
        Some(patterns) => plugins::entries(root, patterns),
        None => (Vec::new(), Vec::new()),
    };
    warnings.extend(plugin_warnings);
    let native_feed =
        settings.publish.is_some() || application_package["nativeboat"].get("publish").is_some();
    if updater_config.is_some() && !native_feed {
        warnings.push(
            "auto-updates use electron-builder's publish feed, where Electron builds are published; \
             give native builds their own feed as \"publish\" in a settings file named with --config, \
             or as \"nativeboat\": {\"publish\": ...} in package.json, or null to turn updates off"
                .to_owned(),
        );
    }
    for key in &settings.unknown {
        warnings.push(format!(
            "the settings file gives \"{key}\", which this NativeBoat does not use"
        ));
    }
    let metadata = metadata(root, &application_package, &build);
    let installer = builder::installer(
        &build,
        &application_package,
        &metadata,
        updater_config.as_ref(),
        &mut warnings,
    );
    let build_steps = scripts::build_steps_with_compiler(
        root,
        package,
        &missing,
        renderer_build.as_ref().map(|_| "electron-vite"),
    );
    let output_directory = builder::output_directory(&build, &emits);
    let application_files =
        builder::application_files(root, &build, &[std::path::PathBuf::from(&output_directory)]);
    Ok(Project {
        root: root.to_path_buf(),
        metadata,
        dependencies,
        layout,
        main,
        preloads,
        forks,
        plugins,
        pages,
        renderer_root,
        renderer_build,
        mappings: Vec::new(),
        aliases,
        preload_aliases,
        renderer_aliases,
        emits,
        legacy_decorators,
        decorator_metadata,
        typescript,
        main_env: env_for(&env, &["VITE_", "MAIN_VITE_"]),
        preload_env: env_for(&env, &["VITE_", "PRELOAD_VITE_"]),
        renderer_env: env_for(&env, &["VITE_", "RENDERER_VITE_"]),
        defines,
        preload_defines,
        renderer_defines,
        extra_files,
        application_files,
        updater_config,
        installer,
        build_steps,
        output_directory,
        warnings,
    })
}

#[cfg(test)]
mod tests;
