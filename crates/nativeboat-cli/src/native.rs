//! Engine-free project compilation. The legacy package builder is never used.
use base64::Engine as _;
use nativeboat_bundler::resolve::{Resolved, Resolver, Target};
use nativeboat_compiler::{
    CompileOptions, CompileValue, DynamicModule, ModuleKind, ModuleSource, RequestKind,
    compile_sources_with_entries_and_options,
};
use nativeboat_project::settings::Settings;
use nativeboat_project::{Layout, Project};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

// Unit tests exercise generated behavior without paying for production
// optimization on every native fixture. Non-test builds retain release-level
// optimization for runtime libraries and generated applications.
fn native_opt_level() -> u8 {
    if cfg!(test) { 0 } else { 2 }
}

#[cfg(feature = "embedded-runtime")]
fn runtime_optimization_key(fingerprint: u64, optimization: u8) -> u64 {
    // Keep existing production libraries reusable, but separate fixture and
    // future optimization settings from the production default.
    if optimization == 2 {
        fingerprint
    } else {
        fnv(fnv(fingerprint, b"runtime-opt-level"), &[optimization])
    }
}

/// Options for the native toolchain, separate from legacy runtime packaging.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Unchanged Electron project with installed source dependencies.
    #[arg(default_value = ".")]
    pub project: PathBuf,
    /// Output directory, relative to the project unless absolute.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Rust compiler to build with instead of the toolchain bundled next to
    /// nativeboat (or, in a development checkout without one, `rustc`).
    /// Released compilers use only their own toolchain.
    #[arg(long, hide = cfg!(not(feature = "embedded-runtime")))]
    pub rustc: Option<PathBuf>,
    /// Directory for build copies of projects whose dependencies are not
    /// installed; defaults to `%LOCALAPPDATA%\nativeboat\projects`.
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
    /// Command that signs the executable before it is published, such as
    /// `signtool sign /a /fd SHA256 {file}`; `{file}` becomes the quoted
    /// executable path, which is appended when absent. It runs in the
    /// Windows shell from the project directory.
    #[arg(long)]
    pub sign: Option<String>,
    /// Directory for the Windows installer and the update metadata
    /// (`latest.yml`) electron-updater reads, relative to the project unless
    /// absolute. Builds of projects with an update feed write them to
    /// `<out>-setup` by default.
    #[arg(long)]
    pub installer: Option<PathBuf>,
    /// A NativeBoat settings file: JSON with the native build's own
    /// settings, which come before the project's, such as `publish`, the
    /// feed native builds update from. The project's files need no change.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

/// One separately instantiated JavaScript execution context in the native
/// application. Its generated graph is compiled into the executable while its
/// `Realm` and module cache remain owned by that context at run time.
struct CompiledContext {
    entry: PathBuf,
    /// The pages this renderer graph serves: `entry` and every later page
    /// that loads the same scripts, as a product page and its development
    /// copy do. Each keeps its own document and URL. Empty for preloads.
    pages: Vec<PathBuf>,
    /// Static script roots in this context; lazy dynamic imports are not roots.
    entry_count: usize,
    module: DynamicModule,
}

/// The source folders of plugin modules and the app-relative folders their
/// packages' builds emit them to, from each plugin entry's source and
/// emitted path: the folders above the parts of their paths they share.
fn emitted_folders(root: &Path, plugins: &[nativeboat_project::Entry]) -> Vec<(PathBuf, String)> {
    let mut folders: Vec<(PathBuf, String)> = Vec::new();
    for plugin in plugins {
        // JavaScript sources run from their own paths.
        let typescript = plugin.source.extension().is_some_and(|extension| {
            matches!(extension.to_str(), Some("ts" | "tsx" | "mts" | "cts"))
        });
        let Some(emitted) = plugin.runtime_paths.first().filter(|_| typescript) else {
            continue;
        };
        let mut source = plugin.source.with_extension("");
        let mut emitted: Vec<&str> = emitted.split('/').collect();
        if let Some(last) = emitted.last_mut() {
            *last = last.rsplit_once('.').map_or(*last, |(stem, _)| stem);
        }
        // Drop the shared tail: file name, then folders of the same name.
        while let (Some(name), Some(last)) = (source.file_name(), emitted.last())
            && name.to_string_lossy().eq_ignore_ascii_case(last)
            && emitted.len() > 1
        {
            emitted.pop();
            source.pop();
        }
        let folder = (source, emitted.join("/"));
        if folder.0.starts_with(root) && !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    folders
}

/// The realm registration of the package scripts a context may load while
/// it runs (`DynamicModule::scripts`).
fn script_registration(scripts: &[(String, usize)]) -> String {
    if scripts.is_empty() {
        return String::new();
    }
    let items = scripts
        .iter()
        .map(|(key, entry)| format!("({key:?}, {entry})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("realm.register_scripts(&[{items}]); ")
}

/// Successfully compiled application graphs plus every source blocker found
/// during discovery. Keeping successful contexts here lets later app-host
/// work consume them instead of discarding renderer/preload code on errors.
struct NativeCompilation {
    main: Option<DynamicModule>,
    /// The paths each forked module is started by, in the order of the main
    /// graph's roots after the main module: fork `k` is root `1 + k`.
    forks: Vec<Vec<String>>,
    preloads: Vec<CompiledContext>,
    renderers: Vec<CompiledContext>,
    /// Stylesheets imported by compiled renderer module graphs, keyed by the
    /// native module identifier that registers them in their renderer Realm.
    stylesheets: std::collections::BTreeMap<String, String>,
    errors: Vec<String>,
}

impl NativeCompilation {
    fn modules(&self) -> impl Iterator<Item = &DynamicModule> {
        self.main
            .iter()
            .chain(self.preloads.iter().map(|context| &context.module))
            .chain(self.renderers.iter().map(|context| &context.module))
    }
}

/// Options of the hidden `export-runtime` command, which release packaging
/// runs with a development build of nativeboat.
#[cfg(feature = "embedded-runtime")]
#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    /// The folder to write: the toolchain's `runtime-libs`.
    pub out: PathBuf,
}

/// Compiles the runtime libraries with the toolchain next to this program
/// and writes them into `out`, one folder per kind, without their sources.
/// Released compilers link these instead of compiling embedded sources.
#[cfg(feature = "embedded-runtime")]
pub fn export_runtime(args: &ExportArgs, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let cache = args.out.with_extension("build");
    let _ = std::fs::remove_dir_all(&cache);
    let result = export(&args.out, &cache);
    let _ = std::fs::remove_dir_all(&cache);
    match result {
        Ok(()) => {
            let _ = writeln!(
                stdout,
                "Exported the runtime libraries to {}",
                args.out.display()
            );
            0
        }
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            1
        }
    }
}

#[cfg(feature = "embedded-runtime")]
fn export(out: &Path, cache: &Path) -> Result<(), String> {
    let tool = select_toolchain(None, &std::env::current_exe().unwrap_or_default());
    if tool.sysroot.is_none() {
        return Err("export-runtime needs the toolchain folder next to nativeboat".to_owned());
    }
    let key = toolchain_key(&tool, cache)?;
    let (matcher, _matcher_copy) = matcher(&tool, cache, key)?;
    let (url, _url_copy) = url_parser(&tool, cache, key)?;
    let (encoding, _encoding_copy) = encoding_detector(&tool, cache, key, &matcher)?;
    let (renderer, _renderer_copy) = renderer_libraries(&tool, cache, key)?
        .ok_or_else(|| "the toolchain has no renderer libraries".to_owned())?;
    let libraries = (matcher.as_path(), url.as_path(), encoding.as_path());
    let (plain, _plain_copy) = runtime(&tool, cache, key, libraries, None)?;
    let (web, _web_copy) = runtime(&tool, cache, key, libraries, Some(&renderer))?;
    let dependencies = [web.as_path(), &matcher, &url, &encoding, &renderer];
    let (page, _page_copy) = page_library(&tool, cache, key, &dependencies, &renderer)?;
    let _ = std::fs::remove_dir_all(out);
    for (kind, directory) in [
        ("matcher", &matcher),
        ("url", &url),
        ("encoding", &encoding),
        ("runtime", &plain),
        ("runtime-web", &web),
        ("page", &page),
    ] {
        let target = out.join(kind);
        std::fs::create_dir_all(&target)
            .map_err(|error| format!("cannot create {}: {error}", target.display()))?;
        let entries = std::fs::read_dir(directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if path
                .extension()
                .is_some_and(|extension| extension == "rlib")
            {
                let file = target.join(path.file_name().expect("listed files have names"));
                std::fs::copy(&path, &file)
                    .map_err(|error| format!("cannot write {}: {error}", file.display()))?;
            }
        }
    }
    Ok(())
}

pub fn run(args: &Args, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let settings = match args.config.as_deref().map(Settings::read).transpose() {
        Ok(settings) => settings.unwrap_or_default(),
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            return 1;
        }
    };
    let load = |root: &Path| nativeboat_project::load_with(root, &settings);
    let mut project = match load(&args.project) {
        Ok(project) => project,
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            return 1;
        }
    };
    // The executable goes to the project's folder even when it is built
    // from a copy with installed dependencies.
    let out = output(&project, args);
    if let Some(install) = nativeboat_project::install::plan(&project.root, false) {
        let requested = nativeboat_bundler::resolve::realpath(
            &std::path::absolute(&args.project).unwrap_or(args.project.clone()),
        );
        let work = args
            .work_dir
            .clone()
            .unwrap_or_else(crate::prepare::default_work_dir);
        let copied = crate::prepare::prepare(
            &install,
            &requested,
            &work,
            std::slice::from_ref(&out),
            &crate::steps::shell(),
            stdout,
            stderr,
        )
        .and_then(|copy| load(&copy.of(&requested)));
        match copied {
            Ok(copied) => project = copied,
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                return 1;
            }
        }
    }
    // What the packaging script runs before it packages, such as building
    // helper programs or generating resources; compiling JavaScript is the
    // compiler's. The project is read again for the resources they made.
    let steps: Vec<_> = project
        .build_steps
        .iter()
        .filter(|step| !nativeboat_project::scripts::is_compiler(&step.command))
        .cloned()
        .collect();
    if !steps.is_empty() {
        let shell = crate::steps::shell();
        let mut tried = std::collections::BTreeSet::new();
        let root = project.root.clone();
        let ran = crate::steps::run_with(
            &project.root,
            &steps,
            &shell,
            &[],
            &mut |errors, stdout, stderr| {
                crate::prepare::install_missing(&root, errors, &shell, &mut tried, stdout, stderr)
            },
            stdout,
            stderr,
        )
        .and_then(|()| load(&project.root));
        match ran {
            Ok(reloaded) => project = reloaded,
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                return 1;
            }
        }
    }
    // Plugin packages with manifests of their own get the dependencies
    // their modules import.
    if crate::prepare::install_plugin_dependencies(&project, &crate::steps::shell(), stdout, stderr)
    {
        match load(&project.root) {
            Ok(reloaded) => project = reloaded,
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                return 1;
            }
        }
    }
    // What reading the project found to tell, such as settings this build
    // does not use.
    for warning in &project.warnings {
        let _ = writeln!(stderr, "warning: {warning}");
    }
    // Vue components compiled by the project's own compiler, when it has one.
    let components = match crate::vue_tool::prepare(&project, args) {
        Ok(components) => components.unwrap_or_default(),
        Err(errors) => {
            for error in errors {
                let _ = writeln!(stderr, "error: {error}");
            }
            return 1;
        }
    };
    // Tailwind CSS stylesheets built by the project's own compiler.
    let sheets = match crate::tailwind_tool::prepare(&project, args) {
        Ok(sheets) => sheets,
        Err(errors) => {
            for error in errors {
                let _ = writeln!(stderr, "error: {error}");
            }
            return 1;
        }
    };
    let compilation = compile_with(&project, web_platform(args), &components, &sheets);
    // Uses that compile but fail when the application runs them.
    let modules = compilation.main.iter().chain(
        compilation
            .preloads
            .iter()
            .chain(&compilation.renderers)
            .map(|context| &context.module),
    );
    for warning in modules.flat_map(DynamicModule::warnings) {
        let _ = writeln!(
            stderr,
            "warning: {}:{}:{}: {}",
            warning.path.display(),
            warning.line,
            warning.column,
            warning.message
        );
    }
    let result = if compilation.errors.is_empty() && compilation.main.is_some() {
        link(&project, &compilation, &out, args)
    } else {
        Err(compilation.errors)
    };
    match result {
        Ok(executable) => {
            let _ = writeln!(
                stdout,
                "Built native {} (no JavaScript engine)",
                executable.display()
            );
            if let Some(directory) =
                crate::installer::directory(&project, &out, args.installer.as_deref())
            {
                let _ = writeln!(
                    stdout,
                    "Built installer {}",
                    directory.join(&project.installer.artifact_name).display()
                );
            }
            0
        }
        Err(errors) => {
            for error in errors {
                let _ = writeln!(stderr, "error: {error}");
            }
            1
        }
    }
}

/// Whether this build's toolchain ships the renderer libraries, so renderer
/// pages get the Web API prelude over the runtime's document natives.
fn web_platform(args: &Args) -> bool {
    let tool = select_toolchain(
        args.rustc.as_deref(),
        &std::env::current_exe().unwrap_or_default(),
    );
    tool.sysroot
        .is_some_and(|sysroot| sysroot.join("renderer-libs").is_dir())
}

/// Gather blockers before creating output or invoking the native toolchain.
#[cfg(test)]
fn compile(project: &Project) -> NativeCompilation {
    compile_with(
        project,
        false,
        &crate::vue_tool::Components::new(),
        &crate::tailwind_tool::Sheets::new(),
    )
}

/// Builds a program NativeBoat runs as a build tool, such as the Vue SFC
/// compiler: its executable in `out`.
pub(crate) fn build_tool(
    project: &Project,
    out: &Path,
    args: &Args,
) -> Result<PathBuf, Vec<String>> {
    let compilation = compile_with(
        project,
        false,
        &crate::vue_tool::Components::new(),
        &crate::tailwind_tool::Sheets::new(),
    );
    if !compilation.errors.is_empty() {
        return Err(compilation.errors);
    }
    link(project, &compilation, out, args)
}

/// The CommonJS bundle a project's bundler builds a main or preload entry's
/// ES sources into, as its runtime path: electron-vite, vite-plugin-electron
/// and Forge's Vite plugin build CommonJS unless the package is an ES one.
fn commonjs_bundle(project: &Project, entry: &nativeboat_project::Entry) -> Option<String> {
    if project.layout == Layout::Plain {
        return None;
    }
    let module = std::fs::read_to_string(project.root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|package| package["type"] == "module");
    entry
        .runtime_paths
        .iter()
        .find(|path| path.ends_with(".cjs") || (!module && path.ends_with(".js")))
        .cloned()
}

/// Gather blockers; `web` puts the Web API prelude first in renderer graphs,
/// `components` are Vue components compiled by the project's compiler, and
/// `sheets` Tailwind CSS stylesheets built by the project's compiler.
fn compile_with(
    project: &Project,
    web: bool,
    components: &crate::vue_tool::Components,
    sheets: &crate::tailwind_tool::Sheets,
) -> NativeCompilation {
    let no_components = crate::vue_tool::Components::new();
    let mut stylesheets = std::collections::BTreeMap::new();
    let (mut main_options, mut errors) = compile_options(
        project,
        &project.main_env,
        &project.defines,
        project.layout == Layout::ElectronVite,
    );
    main_options.bundle = commonjs_bundle(project, &project.main);
    // Modules the program forks are roots of the main graph after the main
    // module: they share its compiled modules, and a child process started
    // as one of them evaluates that root instead of the main module.
    // Plugin packages' main modules follow them: the program loads them
    // by their paths, or forks them, while it runs.
    main_options.emitted_folders = emitted_folders(&project.root, &project.plugins);
    main_options.commonjs_folders = main_options
        .emitted_folders
        .iter()
        .map(|(folder, _)| folder.clone())
        .filter(|folder| {
            project.plugins.iter().any(|plugin| {
                plugin.source.starts_with(folder)
                    && nativeboat_project::plugins::emits_commonjs(&project.root, &plugin.source)
            })
        })
        .collect();
    let main_roots: Vec<PathBuf> = std::iter::once(project.main.source.clone())
        .chain(project.forks.iter().map(|fork| fork.source.clone()))
        .chain(project.plugins.iter().map(|plugin| plugin.source.clone()))
        .collect();
    let forks = project
        .forks
        .iter()
        .chain(&project.plugins)
        .map(|fork| {
            fork.runtime_paths
                .iter()
                .map(|path| path.replace('\\', "/").to_ascii_lowercase())
                .collect()
        })
        .collect();
    let main = graph_entries_with_stylesheets(
        project,
        &main_roots,
        &project.aliases,
        Target::MainImport,
        false,
        &main_options,
        &mut stylesheets,
        &no_components,
        sheets,
    );
    let main = match main {
        Ok(module) => Some(module),
        Err(found) => {
            errors.extend(found);
            None
        }
    };
    let mut preloads = Vec::new();
    for entry in &project.preloads {
        let (mut preload_options, option_errors) = compile_options(
            project,
            &project.preload_env,
            &project.preload_defines,
            project.layout == Layout::ElectronVite,
        );
        preload_options.bundle = commonjs_bundle(project, entry);
        errors.extend(option_errors);
        match graph_entries_with_stylesheets(
            project,
            std::slice::from_ref(&entry.source),
            &project.preload_aliases,
            Target::MainImport,
            true,
            &preload_options,
            &mut stylesheets,
            &no_components,
            sheets,
        ) {
            Ok(module) => preloads.push(CompiledContext {
                pages: Vec::new(),
                entry: entry.source.clone(),
                entry_count: 1,
                module,
            }),
            Err(found) => errors.extend(found),
        }
    }
    let (mut renderer_options, renderer_option_errors) = compile_options(
        project,
        &project.renderer_env,
        &renderer_defines(project),
        project.layout == Layout::ElectronVite,
    );
    // Pages may load package scripts with script elements while they run.
    renderer_options.package_scripts = true;
    errors.extend(renderer_option_errors);
    let mut renderers: Vec<CompiledContext> = Vec::new();
    let mut renderer_entries = Vec::new();
    for page in &project.pages {
        match renderer_scripts(project, page) {
            Ok((mut entries, script_errors)) => {
                errors.extend(script_errors);
                // With the Web API prelude, a page without scripts still gets
                // a renderer, which lays out and paints its document.
                if !entries.is_empty() || web {
                    if web {
                        entries.insert(0, crate::web_prelude::path(&project.root));
                    }
                    // Pages that load the same scripts share one compiled
                    // graph instead of compiling it again.
                    if let Some(shared) = renderer_entries
                        .iter()
                        .position(|compiled: &Vec<PathBuf>| *compiled == entries)
                    {
                        let pages = &mut renderers[shared].pages;
                        if !pages.contains(&page.source) {
                            pages.push(page.source.clone());
                        }
                        continue;
                    }
                    let entry_count = entries.len();
                    // Script entries from one page share this compiled graph,
                    // while each page gets its own runtime context/cache.
                    match graph_entries_with_stylesheets(
                        project,
                        &entries,
                        &project.renderer_aliases,
                        Target::RendererImport,
                        false,
                        &renderer_options,
                        &mut stylesheets,
                        components,
                        sheets,
                    ) {
                        // Package scripts the page may load while it runs are
                        // lazy entries of its graph.
                        Ok(module) => {
                            renderer_entries.push(entries);
                            renderers.push(CompiledContext {
                                entry: page.source.clone(),
                                pages: vec![page.source.clone()],
                                entry_count,
                                module,
                            });
                        }
                        Err(found) => errors.extend(found),
                    }
                }
            }
            Err(found) => errors.extend(found),
        }
    }
    NativeCompilation {
        main,
        forks,
        preloads,
        renderers,
        stylesheets,
        errors,
    }
}

/// Finds local JavaScript entry points in a renderer page. The host still
/// needs to compile HTML, styles and framework components around these graphs.
fn renderer_scripts(
    project: &Project,
    page: &nativeboat_project::Entry,
) -> Result<(Vec<PathBuf>, Vec<String>), Vec<String>> {
    let source = std::fs::read_to_string(&page.source).map_err(|error| {
        vec![format!(
            "{}:1:1: cannot read renderer HTML: {error}",
            page.source.display()
        )]
    })?;
    let resolver = Resolver {
        root: project.root.clone(),
        target: Target::RendererImport,
        aliases: project.renderer_aliases.clone(),
    };
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (index, element) in nativeboat_html::parse_document(&source)
        .elements()
        .into_iter()
        .filter(|element| element.name == "script")
        .enumerate()
    {
        let line = element.position.line.max(1);
        if element.attribute("type").is_some_and(|kind| {
            !kind.eq_ignore_ascii_case("module")
                && !kind.eq_ignore_ascii_case("text/javascript")
                && !kind.eq_ignore_ascii_case("application/javascript")
        }) {
            continue;
        }
        // Scripts without `type="module"` share their top level.
        let classic = !element
            .attribute("type")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("module"));
        let Some(url) = element.attribute("src") else {
            let text = element.text_content();
            if !text.trim().is_empty() {
                let path = crate::classic_script::inline(&page.source, index, &text);
                if classic {
                    crate::classic_script::mark_classic(&path);
                }
                entries.push(path);
            }
            continue;
        };
        if url.starts_with("//")
            || url
                .split_once(':')
                .is_some_and(|(scheme, _)| !scheme.contains(['/', '\\']))
        {
            errors.push(format!(
                "{}:{line}:1: remote renderer script '{url}' cannot be compiled",
                page.source.display()
            ));
            continue;
        }
        let url = url.split(['?', '#']).next().unwrap_or(url);
        let path = match url.strip_prefix('/') {
            Some(rooted) => project.renderer_root.join(rooted),
            None => page
                .source
                .parent()
                .unwrap_or(&project.renderer_root)
                .join(url),
        };
        if let Some(path) = resolver.resolve_path(&path) {
            if classic {
                crate::classic_script::mark_classic(&path);
            }
            entries.push(path);
        } else {
            errors.push(format!(
                "{}:{line}:1: cannot resolve renderer script '{url}'",
                page.source.display()
            ));
        }
    }
    // Handlers bind before the page's scripts run.
    if let Some(handlers) = crate::classic_script::handlers(&page.source, &source, &entries) {
        entries.insert(0, handlers);
    }
    Ok((entries, errors))
}

/// Expands finite one-variable dynamic-import templates into literal lazy
/// imports. Each generated branch remains a lazy graph edge, so modules are
/// compiled but evaluated only when the source expression selects them.
fn expand_template_dynamic_imports(path: &Path, source: &str) -> String {
    use oxc::ast::ast::{Expression, ImportExpression};
    use oxc::ast_visit::{Visit, walk};
    use oxc::span::SourceType;

    struct Patterns(Vec<(oxc::span::Span, String, String, String)>);

    impl<'a> Visit<'a> for Patterns {
        fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
            if let Expression::TemplateLiteral(template) = &import.source
                && template.expressions.len() == 1
                && template.quasis.len() == 2
                && let Expression::Identifier(identifier) = &template.expressions[0]
            {
                self.0.push((
                    import.span,
                    template.quasis[0].value.raw.to_string(),
                    identifier.name.to_string(),
                    template.quasis[1].value.raw.to_string(),
                ));
            }
            walk::walk_import_expression(self, import);
        }
    }

    let Ok(source_type) = SourceType::from_path(path) else {
        return source.to_owned();
    };
    let allocator = oxc::allocator::Allocator::default();
    let parsed = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return source.to_owned();
    }
    let mut patterns = Patterns(Vec::new());
    patterns.visit_program(&parsed.program);
    if patterns.0.is_empty() {
        return source.to_owned();
    }

    let mut replacements = Vec::new();
    let mut helpers = Vec::new();
    for (span, prefix, expression, suffix) in patterns.0 {
        let Some((directory, file_prefix)) = prefix.rsplit_once('/') else {
            continue;
        };
        let directory = path.parent().unwrap_or(Path::new(".")).join(directory);
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut matches = entries
            .flatten()
            .filter_map(|entry| {
                let file_name = entry.file_name().into_string().ok()?;
                let key = file_name.strip_prefix(file_prefix)?.strip_suffix(&suffix)?;
                if key.is_empty() || key.ends_with(".min") || !entry.path().is_file() {
                    return None;
                }
                Some((key.to_owned(), format!("{prefix}{key}{suffix}")))
            })
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup_by(|left, right| left.0 == right.0);
        if matches.is_empty() {
            continue;
        }

        let mut helper = "__nativeboat_dynamic_template_import".to_owned();
        while source.contains(&helper)
            || helpers
                .iter()
                .any(|(name, _): &(String, String)| name == &helper)
        {
            helper.push('_');
        }
        let cases = matches
            .iter()
            .map(|(key, specifier)| format!("case {key:?}: return import({specifier:?});"))
            .collect::<Vec<_>>()
            .join(" ");
        helpers.push((
            helper.clone(),
            format!(
                "const {helper} = ({expression}) => {{ switch ({expression}) {{ {cases} default: return Promise.reject(new Error('Unknown dynamic import key')); }} }};"
            ),
        ));
        let start = span.start as usize;
        let end = span.end as usize;
        if end > source.len() || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            continue;
        }
        let retained_newlines = source[start..end]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        replacements.push((
            start,
            end,
            format!("{helper}({expression}){}", "\n".repeat(retained_newlines)),
        ));
    }
    if replacements.is_empty() {
        return source.to_owned();
    }
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut expanded = source.to_owned();
    for (start, end, replacement) in replacements {
        expanded.replace_range(start..end, &replacement);
    }
    expanded.push('\n');
    for (_, helper) in helpers {
        expanded.push_str(&helper);
        expanded.push('\n');
    }
    expanded
}

fn compile_options(
    project: &Project,
    environment: &std::collections::BTreeMap<String, serde_json::Value>,
    defines: &[(String, String)],
    supports_import_meta_env: bool,
) -> (CompileOptions, Vec<String>) {
    let mut options = CompileOptions::default();
    let typescript = &project.typescript;
    // `react` asks for a factory; every other setting, and Vite's React
    // plugin, for the automatic runtime.
    options.jsx = if typescript.jsx.as_deref() == Some("react") {
        nativeboat_compiler::Jsx::Classic {
            factory: typescript
                .jsx_factory
                .clone()
                .unwrap_or_else(|| "React.createElement".to_owned()),
            fragment: typescript
                .jsx_fragment_factory
                .clone()
                .unwrap_or_else(|| "React.Fragment".to_owned()),
        }
    } else {
        nativeboat_compiler::Jsx::Automatic {
            import_source: typescript
                .jsx_import_source
                .clone()
                .unwrap_or_else(|| "react".to_owned()),
        }
    };
    options.verbatim_module_syntax = typescript.verbatim_module_syntax;
    options.legacy_decorators = project.legacy_decorators;
    options.assign_class_fields = typescript.assign_class_fields;
    options.application = Some(nativeboat_bundler::resolve::normalize(&project.root));
    // A module too large for one chunk's crate compiles in parts, which
    // chunks share out.
    options.split_module_bytes = Some(2 * CHUNK_BYTES);
    let mut errors = Vec::new();
    if supports_import_meta_env {
        let mut values = std::collections::BTreeMap::new();
        for (name, value) in environment {
            match compile_value(value.clone()) {
                Some(value) => {
                    values.insert(name.clone(), value);
                }
                None => errors.push(format!(
                    "{}:1:1: import.meta.env.{name} is not a static primitive value",
                    project.root.join("package.json").display()
                )),
            }
        }
        options.import_meta_env = Some(values);
    }
    for (name, source) in defines {
        if name == "global" && source.trim() == "globalThis" {
            options
                .defines
                .insert(name.clone(), CompileValue::GlobalThis);
            continue;
        }
        let value = serde_json::from_str(source).ok().and_then(compile_value);
        if is_define_name(name)
            && let Some(value) = value
        {
            options.defines.insert(name.clone(), value);
            continue;
        }
        errors.push(format!(
            "{}:1:1: native build-time define '{name}' is not a supported static literal",
            project.root.join("package.json").display()
        ));
    }
    (options, errors)
}

fn compile_value(value: serde_json::Value) -> Option<CompileValue> {
    match value {
        serde_json::Value::Null => Some(CompileValue::Null),
        serde_json::Value::Bool(value) => Some(CompileValue::Boolean(value)),
        serde_json::Value::Number(value) => value.as_f64().map(CompileValue::Number),
        serde_json::Value::String(value) => Some(CompileValue::String(value)),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => None,
    }
}

/// The renderer's build-time defines. A Vite production build also replaces
/// `process.env.NODE_ENV` in client code, so libraries take their production
/// paths rather than reading the variable at run time.
fn renderer_defines(project: &Project) -> Vec<(String, String)> {
    let mut defines = project.renderer_defines.clone();
    let vite = matches!(
        project.layout,
        Layout::ElectronVite | Layout::VitePluginElectron | Layout::ForgeVite
    );
    if vite
        && !defines
            .iter()
            .any(|(name, _)| name == "process.env.NODE_ENV")
    {
        defines.push((
            "process.env.NODE_ENV".to_owned(),
            "\"production\"".to_owned(),
        ));
    }
    defines
}

/// A define name: an identifier or a dotted path of identifiers.
fn is_define_name(name: &str) -> bool {
    name.split('.').all(is_identifier)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|character| {
            character == '_' || character == '$' || character.is_ascii_alphanumeric()
        })
}

#[cfg(test)]
fn graph(
    project: &Project,
    entry: &Path,
    aliases: &[nativeboat_bundler::resolve::Alias],
    target: Target,
    options: &CompileOptions,
) -> Result<DynamicModule, Vec<String>> {
    graph_entries(project, &[entry.to_path_buf()], aliases, target, options)
}

/// Compiles entries that execute in one realm into one shared module graph.
/// Renderer scripts on a page share globals and module-cache state, so
/// compiling each script independently would duplicate dependencies and lose
/// the page's execution ordering.
#[cfg(test)]
fn graph_entries(
    project: &Project,
    entries: &[PathBuf],
    aliases: &[nativeboat_bundler::resolve::Alias],
    target: Target,
    options: &CompileOptions,
) -> Result<DynamicModule, Vec<String>> {
    graph_entries_with_stylesheets(
        project,
        entries,
        aliases,
        target,
        false,
        options,
        &mut std::collections::BTreeMap::new(),
        &crate::vue_tool::Components::new(),
        &crate::tailwind_tool::Sheets::new(),
    )
}

/// With `script_entries`, entry files without ES module syntax compile as
/// CommonJS whatever their package's `type`, as Electron runs a sandboxed
/// preload script.
#[allow(clippy::too_many_arguments)]
fn graph_entries_with_stylesheets(
    project: &Project,
    entries: &[PathBuf],
    aliases: &[nativeboat_bundler::resolve::Alias],
    target: Target,
    script_entries: bool,
    options: &CompileOptions,
    stylesheets: &mut std::collections::BTreeMap<String, String>,
    components: &crate::vue_tool::Components,
    sheets: &crate::tailwind_tool::Sheets,
) -> Result<DynamicModule, Vec<String>> {
    if entries.is_empty() {
        return Err(vec!["an execution context must have an entry".into()]);
    }
    let resolver = Resolver {
        root: project.root.clone(),
        target,
        aliases: aliases.to_vec(),
    };
    let require_resolver = Resolver {
        target: match target {
            Target::RendererImport => Target::RendererRequire,
            _ => Target::MainRequire,
        },
        ..resolver.clone()
    };
    let mut resolution_errors = HashMap::new();
    let raw_modules = std::cell::RefCell::new(HashMap::<PathBuf, PathBuf>::new());
    let asset_modules = std::cell::RefCell::new(HashMap::<PathBuf, PathBuf>::new());
    let empty_modules = std::cell::RefCell::new(HashSet::<PathBuf>::new());
    let svg_modules = std::cell::RefCell::new(HashMap::<PathBuf, PathBuf>::new());
    let svg_errors = std::cell::RefCell::new(Vec::<String>::new());
    let stylesheet_errors = std::cell::RefCell::new(Vec::<String>::new());
    let vue_modules = std::cell::RefCell::new(HashMap::<PathBuf, PathBuf>::new());
    // Stylesheets of compiled Vue components, by their virtual paths.
    let vue_styles = std::cell::RefCell::new(HashMap::<PathBuf, String>::new());
    let vue_line_maps = std::cell::RefCell::new(HashMap::<PathBuf, Vec<u32>>::new());
    let vue_errors = std::cell::RefCell::new(Vec::<String>::new());
    let entries = entries
        .iter()
        .map(|entry| nativeboat_bundler::resolve::realpath(entry))
        .collect::<Vec<_>>();
    let entries = entries.iter().map(PathBuf::as_path).collect::<Vec<_>>();
    let compiled = compile_sources_with_entries_and_options(
        &entries,
        &mut |from, name, kind| {
            let mut unresolved = |message| {
                resolution_errors.insert(
                    (
                        from.to_path_buf(),
                        format!("cannot resolve module '{name}'"),
                    ),
                    message,
                );
                None
            };
            // A package script a page may load while it runs: the file in
            // the nearest installed package of that name, run as a classic
            // script.
            if kind == RequestKind::Script {
                let found = from
                    .ancestors()
                    .skip(1)
                    .map(|directory| directory.join("node_modules").join(name))
                    .find(|path| path.is_file())?;
                let found = nativeboat_bundler::resolve::realpath(&found);
                // ES modules among them, which programs load with computed
                // `import()`s, stay modules.
                let module =
                    std::fs::read_to_string(&found).is_ok_and(|text| module_source(&found, &text));
                if !module {
                    crate::classic_script::mark_classic(&found);
                }
                return Some(found);
            }
            // A script a worker may run: the file a relative path names next
            // to the module, or one a folder above it holds, up to its
            // package's folder; TypeScript sources stand for the JavaScript
            // they compile to.
            if kind == RequestKind::Worker {
                let found = crate::worker_scripts::locate(from, name)?;
                let module =
                    std::fs::read_to_string(&found).is_ok_and(|text| module_source(&found, &text));
                if !module {
                    crate::classic_script::mark_classic(&found);
                }
                return Some(found);
            }
            let resolver = if kind != RequestKind::Import {
                &require_resolver
            } else {
                &resolver
            };
            if let Some(path) = crate::packages::request(from, name) {
                return Some(path);
            }
            if let Some(index) = crate::vue_tool::style_index(name)
                && let Some(original) = vue_modules.borrow().get(from)
                && let Some(style) = components
                    .get(original)
                    .and_then(|component| component.styles.get(index))
            {
                let path =
                    PathBuf::from(format!("{}.nativeboat-style-{index}.css", from.display()));
                vue_styles.borrow_mut().insert(path.clone(), style.clone());
                return Some(path);
            }
            // Vite's `?raw` and CSS `?inline` queries export local asset text
            // as native JSON data. They never leave an asset loader or file
            // read in the application.
            if kind == RequestKind::Import
                && let Some((specifier, query)) = ["?raw", "?inline"]
                    .into_iter()
                    .find_map(|query| name.strip_suffix(query).map(|specifier| (specifier, query)))
            {
                return match resolver.resolve(specifier, from) {
                    Ok(Resolved::File(path)) => {
                        let key = PathBuf::from(format!("{}{query}", path.display()));
                        raw_modules.borrow_mut().insert(key.clone(), path);
                        Some(key)
                    }
                    Ok(Resolved::Builtin(_)) => unresolved(format!(
                        "Vite asset import '{name}' must resolve to a local file"
                    )),
                    Ok(Resolved::Empty) => unresolved(format!(
                        "Vite asset import '{name}' resolves to an empty browser module"
                    )),
                    Err(error) => unresolved(format!("cannot resolve module '{name}': {error}")),
                };
            }
            if kind == RequestKind::Resolve
                && let Some(identifier) = resolver.node_builtin_specifier(name)
            {
                return Some(PathBuf::from(identifier));
            }
            let (specifier, force_asset_url) = name
                .strip_suffix("?url")
                .map_or((name, false), |specifier| (specifier, true));
            match resolver.resolve(specifier, from) {
                Ok(Resolved::File(path)) if path.extension().is_some_and(|ext| ext == "vue") => {
                    let virtual_path = PathBuf::from(format!("{}.nativeboat.ts", path.display()));
                    vue_modules.borrow_mut().insert(virtual_path.clone(), path);
                    Some(virtual_path)
                }
                Ok(Resolved::File(path))
                    if !force_asset_url
                        && path
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg")) =>
                {
                    let virtual_path =
                        PathBuf::from(format!("{}.nativeboat-svg.ts", path.display()));
                    svg_modules.borrow_mut().insert(virtual_path.clone(), path);
                    Some(virtual_path)
                }
                Ok(Resolved::File(path))
                    if force_asset_url
                        || is_native_asset(path.extension().and_then(|ext| ext.to_str())) =>
                {
                    let virtual_path =
                        PathBuf::from(format!("{}.nativeboat-asset.json", path.display()));
                    asset_modules
                        .borrow_mut()
                        .insert(virtual_path.clone(), path);
                    Some(virtual_path)
                }
                Ok(Resolved::File(path)) => crate::packages::replacement(&path).or(Some(path)),
                Ok(Resolved::Empty) => {
                    let virtual_path = project
                        .root
                        .join(".nativeboat/generated/browser-empty-module.cjs");
                    empty_modules.borrow_mut().insert(virtual_path.clone());
                    Some(virtual_path)
                }
                // These must be compiled as platform operations, never runtime facades.
                Ok(Resolved::Builtin(name)) => crate::node_lib::builtin(&name).or_else(|| {
                    unresolved(format!(
                        "native platform module '{name}' is not compiled yet"
                    ))
                }),
                Err(error) => unresolved(error),
            }
        },
        &mut |path| {
            if let Some(original) = vue_modules.borrow().get(path).cloned()
                && let Some(component) = components.get(&original)
            {
                return Some(ModuleSource {
                    text: crate::vite_glob::expand(
                        path,
                        &project.root,
                        &crate::vue_tool::module_source(component),
                    ),
                    kind: ModuleKind::EsModule,
                });
            }
            if let Some(text) = vue_styles.borrow().get(path).cloned() {
                let identifier = format!("nativeboat:stylesheet:{}", path.display());
                let (text, errors) = crate::css_imports::inline(
                    path,
                    &text,
                    &project.root,
                    &project.renderer_aliases,
                );
                stylesheet_errors.borrow_mut().extend(errors);
                stylesheets.insert(identifier.clone(), text);
                return Some(ModuleSource {
                    text: identifier,
                    kind: ModuleKind::Native,
                });
            }
            if let Some(original) = vue_modules.borrow().get(path).cloned() {
                let source = std::fs::read_to_string(&original).ok()?;
                let component_name = original.file_stem().and_then(|name| name.to_str());
                let parsed = crate::vue_sfc::parse_with_component_name(&source, component_name);
                vue_line_maps
                    .borrow_mut()
                    .insert(path.to_path_buf(), parsed.line_map.clone());
                if let Some(line) = parsed.template_line {
                    vue_errors
                        .borrow_mut()
                        .push(crate::vue_sfc::template_diagnostic(&original, line));
                }
                return Some(ModuleSource {
                    text: crate::vite_glob::expand(path, &project.root, &parsed.module_source),
                    kind: ModuleKind::EsModule,
                });
            }
            if let Some(path) = raw_modules.borrow().get(path).cloned() {
                return raw_module_source(&path);
            }
            if empty_modules.borrow().contains(path) {
                return Some(ModuleSource {
                    text: "module.exports = {};\n".to_owned(),
                    kind: ModuleKind::CommonJs,
                });
            }
            if let Some(path) = asset_modules.borrow().get(path).cloned() {
                return asset_module_source(&path);
            }
            if let Some(original) = svg_modules.borrow().get(path).cloned() {
                let source = std::fs::read_to_string(&original).ok()?;
                return match crate::vue_sfc::svg_module_source(&source) {
                    Some(text) => Some(ModuleSource {
                        text,
                        kind: ModuleKind::EsModule,
                    }),
                    None => {
                        svg_errors.borrow_mut().push(format!(
                            "{}:1:1: SVG component imports must contain supported static SVG markup",
                            original.display()
                        ));
                        Some(ModuleSource {
                            text: "export default {};\n".to_owned(),
                            kind: ModuleKind::EsModule,
                        })
                    }
                };
            }
            if let Some(source) = crate::node_lib::source(path) {
                return Some(source);
            }
            if let Some(source) = crate::packages::source(path) {
                return Some(source);
            }
            if let Some(source) = crate::web_prelude::source(path) {
                return Some(source);
            }
            let text = match crate::classic_script::source(path) {
                Some(text) => text,
                None => std::fs::read_to_string(path).ok()?,
            };
            if path.extension().is_some_and(|extension| extension == "css") {
                // A Tailwind CSS sheet was built with its imports inlined.
                let (text, mut errors) = match sheets.get(path) {
                    Some(built) => (built.clone(), Vec::new()),
                    None => crate::css_imports::inline(
                        path,
                        &text,
                        &project.root,
                        &project.renderer_aliases,
                    ),
                };
                let (sheet, diagnostics) = nativeboat_css::parse_stylesheet(&text);
                errors.extend(diagnostics.into_iter().map(|diagnostic| {
                    format!(
                        "{}:{}:{}: {}",
                        path.display(),
                        diagnostic.position.line.max(1),
                        diagnostic.position.column.max(1),
                        diagnostic.message
                    )
                }));
                // Imports after other rules are ignored, as browsers ignore
                // them; those left at the start name other hosts.
                for rule in &sheet.rules {
                    if let nativeboat_css::Rule::Import { url, position, .. } = rule
                        && (url.contains("://") || url.starts_with("//"))
                    {
                        errors.push(format!(
                            "{}:{}:{}: stylesheets from other hosts cannot be imported",
                            path.display(),
                            position.line.max(1),
                            position.column.max(1)
                        ));
                    }
                }
                if !errors.is_empty() {
                    // The compiler needs a source record to finish discovering
                    // sibling imports; preserve CSS failures as a later error.
                    stylesheet_errors.borrow_mut().extend(errors);
                }
                let identifier = format!("nativeboat:stylesheet:{}", path.display());
                stylesheets.insert(identifier.clone(), text);
                return Some(ModuleSource {
                    text: identifier,
                    kind: ModuleKind::Native,
                });
            }
            let kind = if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                ModuleKind::Json
            } else if script_entries && entries.contains(&path) && !module_syntax(path, &text) {
                ModuleKind::CommonJs
            } else if module_source(path, &text) {
                ModuleKind::EsModule
            } else {
                ModuleKind::CommonJs
            };
            let text = if kind == ModuleKind::EsModule {
                let text = expand_template_dynamic_imports(path, &text);
                crate::vite_glob::expand(path, &project.root, &text)
            } else {
                text
            };
            Some(ModuleSource { text, kind })
        },
        options,
    );
    let mut errors = compiled
        .as_ref()
        .err()
        .map(|errors| {
            errors
                .iter()
                .cloned()
                .map(|mut error| {
                    let virtual_path = error.path.clone();
                    if let Some(message) =
                        resolution_errors.get(&(error.path.clone(), error.message.clone()))
                    {
                        error.message.clone_from(message);
                    }
                    if let Some(line) = vue_line_maps
                        .borrow()
                        .get(&virtual_path)
                        .and_then(|lines| lines.get(error.line.saturating_sub(1)))
                        .copied()
                        .filter(|line| *line > 0)
                    {
                        error.line = line as usize;
                    }
                    if let Some(original) = vue_modules.borrow().get(&error.path) {
                        error.path.clone_from(original);
                    }
                    error.to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    errors.extend(stylesheet_errors.into_inner());
    errors.extend(vue_errors.into_inner());
    errors.extend(svg_errors.into_inner());
    if errors.is_empty() {
        compiled.map_err(|_| unreachable!("compiler errors were collected above"))
    } else {
        Err(errors)
    }
}

fn raw_module_source(path: &Path) -> Option<ModuleSource> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some(ModuleSource {
        text: serde_json::to_string(&text).expect("serializing a Rust string to JSON cannot fail"),
        kind: ModuleKind::Json,
    })
}

fn is_native_asset(extension: Option<&str>) -> bool {
    extension.is_some_and(|extension| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "avif" | "bmp" | "gif" | "ico" | "jpeg" | "jpg" | "png" | "webp"
        )
    })
}

/// The media type of a file a page loads by extension: images and fonts.
fn asset_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "cur" | "ico" => "image/x-icon",
        "gif" => "image/gif",
        "jpeg" | "jpg" => "image/jpeg",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "otf" => "font/otf",
        "ttf" => "font/ttf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => return None,
    })
}

/// A style sheet with the local files its `url()`s name, relative to its
/// own path, as `data:` URLs, as bundlers inline assets: the application
/// carries them in its executable and loads them without a file system
/// layout to resolve against. Other URLs stay as they are.
pub(crate) fn inline_css_assets(path: &Path, text: &str) -> String {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let directory = path.parent().unwrap_or_else(|| Path::new(""));
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("url(") {
        let (before, after) = rest.split_at(start + 4);
        output.push_str(before);
        let Some(end) = after.find(')') else {
            rest = after;
            break;
        };
        let argument = after[..end].trim();
        let reference = argument.trim_matches(|character| character == '"' || character == '\'');
        let file = reference.split(['?', '#']).next().unwrap_or_default();
        let local = !file.is_empty()
            && !reference.starts_with('#')
            && !file.contains(':')
            && !file.starts_with("//");
        let inlined = local
            .then(|| directory.join(file))
            .filter(|file| {
                std::fs::metadata(file).is_ok_and(|meta| meta.is_file() && meta.len() <= LIMIT)
            })
            .and_then(|file| Some((asset_mime(&file)?, std::fs::read(&file).ok()?)))
            .map(|(mime, bytes)| {
                format!(
                    "\"data:{mime};base64,{}\"",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )
            });
        output.push_str(inlined.as_deref().unwrap_or(&after[..end]));
        rest = &after[end..];
    }
    output.push_str(rest);
    output
}

fn asset_module_source(path: &Path) -> Option<ModuleSource> {
    let bytes = std::fs::read(path).ok()?;
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "jpeg" | "jpg" => "image/jpeg",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => return None,
    };
    let url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    Some(ModuleSource {
        text: serde_json::to_string(&url).expect("serializing an asset URL cannot fail"),
        kind: ModuleKind::Json,
    })
}

/// Whether a JavaScript or TypeScript file has ES module syntax: imports,
/// exports, `import.meta` or top-level `await`. Other files are modules.
fn module_syntax(path: &Path, text: &str) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "js" | "jsx" | "ts" | "tsx" | "cjs" | "cts"
    ) {
        return true;
    }
    let allocator = oxc::allocator::Allocator::default();
    let Ok(source_type) = nativeboat_bundler::transpile::source_type(path, true) else {
        return true;
    };
    let parsed = oxc::parser::Parser::new(&allocator, text, source_type).parse();
    parsed.program.source_type.is_module()
}

/// Do not silently change CommonJS/sloppy script semantics into module semantics.
fn module_source(path: &Path, text: &str) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "cjs" | "cts" => false,
        "js" | "jsx" | "ts" | "tsx" => {
            let allocator = oxc::allocator::Allocator::default();
            let source_type = nativeboat_bundler::transpile::source_type(path, true)
                .expect("matched source extensions are supported");
            let parsed = oxc::parser::Parser::new(&allocator, text, source_type).parse();
            if parsed.program.source_type.is_module() {
                return true;
            }
            // The closest package scope owns .js format, including linked
            // dependencies outside the application's own root.
            for directory in path.ancestors().skip(1) {
                let manifest = directory.join("package.json");
                if manifest.is_file() {
                    return std::fs::read_to_string(manifest)
                        .ok()
                        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                        .is_some_and(|package| package["type"] == "module");
                }
            }
            false
        }
        _ => true,
    }
}

// Embed the dependency-free native value library so development builds
// compile it with the selected rustc. Released compilers leave the sources
// out and link the libraries their toolchain ships precompiled.
#[cfg(feature = "embedded-runtime")]
macro_rules! native_sources {
    ($($name:literal),+ $(,)?) => {
        const SOURCES: &[(&str, &str)] = &[$(($name, include_str!(concat!("../../nativeboat-native/src/", $name)))),+];
    };
}
#[cfg(feature = "embedded-runtime")]
native_sources!(
    "ced.rs",
    "abort_builtin.rs",
    "lib.rs",
    "array_builtin.rs",
    "bigint.rs",
    "bigint_builtin.rs",
    "binding.rs",
    "blob_binding.rs",
    "buffer_binding.rs",
    "builtins.rs",
    "class.rs",
    "child_process_builtin.rs",
    "collection_builtin.rs",
    "com.rs",
    "console_builtin.rs",
    "context_bridge.rs",
    "coroutine.rs",
    "crypto_builtin.rs",
    "crypto_cipher.rs",
    "crypto_platform.rs",
    "date_builtin.rs",
    "dom_binding.rs",
    "app_host.rs",
    "app_host/frame.rs",
    "app_host/menu_draw.rs",
    "app_host/menu_frame.rs",
    "app_host/notify.rs",
    "app_host/taskbar.rs",
    "electron_builtin.rs",
    "electron_clipboard.rs",
    "electron_crash_reporter.rs",
    "electron_dialog.rs",
    "electron_ipc.rs",
    "electron_window.rs",
    "electron_app.rs",
    "electron_login_item.rs",
    "electron_menu.rs",
    "electron_native_image.rs",
    "electron_native_image/resize.rs",
    "electron_native_image/windows.rs",
    "electron_power.rs",
    "electron_safe_storage.rs",
    "electron_system.rs",
    "electron_notify.rs",
    "electron_protocol.rs",
    "electron_screen.rs",
    "electron_shell.rs",
    "electron_single_instance.rs",
    "encoding.rs",
    "fork.rs",
    "native_addons.rs",
    "native_addons/network.rs",
    "native_addons/parcel_watcher.rs",
    "native_addons/process_tree.rs",
    "native_addons/pty.rs",
    "native_addons/spdlog.rs",
    "native_addons/sqlite3.rs",
    "native_addons/sqlite3/system.rs",
    "sqlite.rs",
    "sqlite/engine.rs",
    "sqlite/format.rs",
    "sqlite/sql.rs",
    "spelling.rs",
    "string_decoder.rs",
    "structured_clone_builtin.rs",
    "pipe_binding.rs",
    "realm_ports.rs",
    "tcp_binding.rs",
    "text_encoding_builtin.rs",
    "web_crypto_builtin.rs",
    "errors.rs",
    "event_target_builtin.rs",
    "event_loop.rs",
    "work_queue.rs",
    "float.rs",
    "fs_win.rs",
    "fs_binding.rs",
    "fs_binding/stat_watcher.rs",
    "fs_event_binding.rs",
    "function.rs",
    "global_number.rs",
    "global_uri.rs",
    "http_binding.rs",
    "icu_converter.rs",
    "inspect.rs",
    "keyboard.rs",
    "keyboard/data.rs",
    "iter.rs",
    "json_builtin.rs",
    "keytar.rs",
    "clip_filepaths.rs",
    "libm.rs",
    "location.rs",
    "libm/acos.rs",
    "libm/acosh.rs",
    "libm/asin.rs",
    "libm/asinh.rs",
    "libm/atan.rs",
    "libm/atan2.rs",
    "libm/atanh.rs",
    "libm/cos.rs",
    "libm/cosh.rs",
    "libm/exp.rs",
    "libm/expm1.rs",
    "libm/expo2.rs",
    "libm/k_cos.rs",
    "libm/k_expo2.rs",
    "libm/k_sin.rs",
    "libm/k_tan.rs",
    "libm/log.rs",
    "libm/log10.rs",
    "libm/log1p.rs",
    "libm/log2.rs",
    "libm/pow.rs",
    "libm/rem_pio2.rs",
    "libm/rem_pio2_large.rs",
    "libm/sin.rs",
    "libm/sinh.rs",
    "libm/tan.rs",
    "libm/tanh.rs",
    "math.rs",
    "math_sum.rs",
    "modules.rs",
    "node_error.rs",
    "node_error_table.rs",
    "number.rs",
    "number_builtin.rs",
    "object_builtin.rs",
    "radix.rs",
    "object.rs",
    "ops.rs",
    "performance_builtin.rs",
    "platform.rs",
    "process_builtin.rs",
    "process_usage.rs",
    "process_events.rs",
    "proxy_builtin.rs",
    "os_binding.rs",
    "uv_error.rs",
    "url_binding.rs",
    "wasm.rs",
    "wasm/compile.rs",
    "wasm/decode.rs",
    "wasm/exec.rs",
    "wasm_binding.rs",
    "weak_builtin.rs",
    "intl_builtin.rs",
    "woff.rs",
    "zlib_builtin.rs",
    "environment_builtin.rs",
    "promise_builtin.rs",
    "realm.rs",
    "reflect_builtin.rs",
    "regexp_builtin.rs",
    "string.rs",
    "string_builtin.rs",
    "symbol.rs",
    "symbol_builtin.rs",
    "system_proxy.rs",
    "table.rs",
    "trace.rs",
    "typed_array_builtin.rs",
    "unicode.rs",
    "unicode_tables.rs",
    "value.rs",
    "web.rs",
    "web/dom_ops.rs",
    "web/history.rs",
    "web/media.rs",
    "web/page.rs",
    "worker.rs",
    "frame.rs"
);

// The generated application toolchain cannot run Cargo or fetch crates. Keep
// the tiny no-std DEFLATE backend's sources in the compiler package and build
// the two rlibs with the selected rustc alongside the native runtime.
#[cfg(feature = "embedded-runtime")]
const DEFLATE_SOURCES: &[(&str, &str)] = &[
    (
        "adler2/src/algo.rs",
        include_str!("../vendor/adler2-2.0.1/src/algo.rs"),
    ),
    (
        "adler2/src/lib.rs",
        include_str!("../vendor/adler2-2.0.1/src/lib.rs"),
    ),
    (
        "miniz_oxide/src/lib.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/lib.rs"),
    ),
    (
        "miniz_oxide/src/shared.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/shared.rs"),
    ),
    (
        "miniz_oxide/src/deflate/buffer.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/buffer.rs"),
    ),
    (
        "miniz_oxide/src/deflate/core.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/core.rs"),
    ),
    (
        "miniz_oxide/src/deflate/mod.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/mod.rs"),
    ),
    (
        "miniz_oxide/src/deflate/stored.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/stored.rs"),
    ),
    (
        "miniz_oxide/src/deflate/stream.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/stream.rs"),
    ),
    (
        "miniz_oxide/src/deflate/zlib.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/deflate/zlib.rs"),
    ),
    (
        "miniz_oxide/src/inflate/core.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/inflate/core.rs"),
    ),
    (
        "miniz_oxide/src/inflate/mod.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/inflate/mod.rs"),
    ),
    (
        "miniz_oxide/src/inflate/output_buffer.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/inflate/output_buffer.rs"),
    ),
    (
        "miniz_oxide/src/inflate/stream.rs",
        include_str!("../vendor/miniz_oxide-0.9.1/src/inflate/stream.rs"),
    ),
];

/// Process startup for executables: entry point, allocator, panic handling
/// and memory primitives, in place of a C runtime.
#[cfg(feature = "embedded-runtime")]
const START: &str = include_str!("../../nativeboat-start/src/lib.rs");
/// The installer program, which installers link with their payload after it.
#[cfg(feature = "embedded-runtime")]
const SETUP: &str = include_str!("../setup/setup.rs");
/// The third-party notices the start crate embeds in every executable.
const NOTICES: &str = include_str!("../../nativeboat-start/src/NOTICES.txt");

/// The notices of all third-party code an executable contains: the start
/// crate's and those of the matcher, URL and encoding libraries it links.
fn app_notices() -> String {
    format!(
        "{NOTICES}\n{}\n{}\n{}",
        nativeboat_regexp_build::NOTICES,
        nativeboat_url_build::NOTICES,
        nativeboat_encoding_build::NOTICES
    )
}

/// The file beside each executable that holds the notices it embeds, for
/// distributing with the application.
const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.txt";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A successfully created, uniquely owned staging directory. Failed builds
/// retain any previously published executable and remove only this directory.
struct Work(PathBuf);

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// With `NATIVEBOAT_MODULE_MAP` naming a file, appends which source file
/// each compiled module function (`m<index>`) of a realm graph comes from,
/// with the size of its Rust source, its parts and the chunk compiling its
/// last part, so profiles and costly chunks of a build can be read.
fn write_module_map(
    path: Option<&std::ffi::OsStr>,
    graph: &str,
    units: &ModuleUnits<'_>,
    unit_chunk: &[Option<usize>],
) -> Result<(), String> {
    use std::io::Write as _;
    let Some(path) = path else {
        return Ok(());
    };
    let mut text = String::new();
    for (module, &last) in units.last.iter().enumerate() {
        let source = units.sources[last];
        let parts = last + 1 - units.first[module];
        let bytes = units.sources[units.first[module]..=last]
            .iter()
            .map(|part| part.len())
            .sum::<usize>();
        let file = source
            .split_once("/// `")
            .and_then(|(_, rest)| rest.split_once('`'))
            .map_or("?", |(file, _)| file);
        let function = source
            .split_once("fn m")
            .and_then(|(_, rest)| rest.split_once('('))
            .map_or("?", |(index, _)| index);
        let chunk = unit_chunk[last].map_or_else(|| "-".to_owned(), |chunk| chunk.to_string());
        let parts = if parts > 1 {
            format!(" parts {parts}")
        } else {
            String::new()
        };
        text.push_str(&format!(
            "{graph} m{function} {file} {bytes} chunk {chunk}{parts}\n"
        ));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| {
            format!(
                "cannot write module map {}: {error}",
                Path::new(path).display()
            )
        })
}

/// The link argument that writes a map of the executable's symbols to the
/// path `NATIVEBOAT_LINK_MAP` names, for profiling and diagnosing it.
fn link_map(path: Option<OsString>) -> Vec<OsString> {
    path.map(|path| {
        let mut argument = OsString::from("-Clink-arg=/MAP:");
        argument.push(path);
        argument
    })
    .into_iter()
    .collect()
}

/// Removes the staging directories in `directory` that builds which were
/// killed left behind: those named `<prefix>...-<process id>-<count>` whose
/// process no longer runs.
fn remove_abandoned_staging(directory: &Path, prefix: &str) {
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let mut parts = name.rsplitn(3, '-');
        let (Some(count), Some(process), Some(_)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let staged = name.starts_with(prefix)
            && count.bytes().all(|byte| byte.is_ascii_digit())
            && entry.path().is_dir();
        if let (true, Ok(process)) = (staged, process.parse::<u32>())
            && !process_running(process)
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Whether a process with this id runs; unknown processes are taken as
/// running, which keeps their directories.
fn process_running(process: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: the handle is checked and closed; the exit code is written to
    // a local.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process);
        if handle.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0;
        let running = GetExitCodeProcess(handle, &mut code) == 0 || code == STILL_ACTIVE;
        CloseHandle(handle);
        running
    }
}

/// All libraries read by a build must outlive its final link. Shared leases
/// allow concurrent builds; cache cleanup takes the exclusive lease instead.
struct CacheUse {
    cache: PathBuf,
    lock: std::fs::File,
}

fn cache_lock_file(cache: &Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(cache)?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache.join(".active-builds.lock"))
}

impl CacheUse {
    fn new(cache: &Path) -> Result<Self, String> {
        // Builds that were killed never ran their cleanup: run it now when
        // no other build is using the cache.
        if let Ok(exclusive) = cache_lock_file(cache)
            && exclusive.try_lock().is_ok()
        {
            prune_cache(cache);
        }
        let lock = cache_lock_file(cache)
            .map_err(|error| format!("cannot open native cache lease: {error}"))?;
        lock.lock_shared()
            .map_err(|error| format!("cannot acquire native cache lease: {error}"))?;
        remove_abandoned_staging(cache, ".");
        Ok(Self {
            cache: cache.to_path_buf(),
            lock,
        })
    }
}

impl Drop for CacheUse {
    fn drop(&mut self) {
        let _ = self.lock.unlock();
        let Ok(exclusive) = cache_lock_file(&self.cache) else {
            return;
        };
        if exclusive.try_lock().is_err() {
            return;
        }
        prune_cache(&self.cache);
    }
}

/// Removes superseded cache entries; the caller holds the exclusive lock.
fn prune_cache(cache: &Path) {
    prune_generated_aliases(cache);
    let kinds: std::collections::BTreeSet<_> = std::fs::read_dir(cache)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (!name.starts_with('.')).then(|| {
                name.split_once('-')
                    .map_or(name.clone(), |(kind, _)| kind.to_owned())
            })
        })
        .collect();
    for kind in kinds {
        prune_unlocked(cache, &kind);
    }
}

pub(crate) fn create_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

fn write_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().expect("staged files are in a directory");
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, content)
}

fn invoke(tool: &Path, args: &[OsString], directory: &Path) -> std::io::Result<Output> {
    let tool = if tool.components().count() > 1 {
        std::path::absolute(tool)?
    } else {
        tool.to_path_buf()
    };
    // Nothing from an installed Microsoft toolchain may reach the link.
    Command::new(tool)
        .args(args)
        .current_dir(directory)
        .env_remove("LIB")
        .env_remove("LIBPATH")
        .output()
}

/// The compiler and, for a bundled toolchain, its sysroot.
pub(crate) struct Toolchain {
    rustc: PathBuf,
    sysroot: Option<PathBuf>,
}

/// `--rustc` when given; otherwise the toolchain shipped in `toolchain`
/// next to the nativeboat program, so users install nothing else; otherwise
/// `rustc` on the search path, for development checkouts without one.
fn select_toolchain(explicit: Option<&Path>, program: &Path) -> Toolchain {
    if let Some(rustc) = explicit {
        return Toolchain {
            rustc: rustc.to_path_buf(),
            sysroot: None,
        };
    }
    let bundled = program.with_file_name("toolchain");
    let rustc = bundled.join("bin").join("rustc.exe");
    if rustc.is_file() {
        Toolchain {
            rustc,
            sysroot: Some(bundled),
        }
    } else {
        Toolchain {
            rustc: PathBuf::from("rustc"),
            sysroot: None,
        }
    }
}

pub(crate) fn toolchain(
    tool: &Toolchain,
    args: &[OsString],
    directory: &Path,
) -> Result<(), String> {
    toolchain_output(tool, args, directory).map(drop)
}

/// Runs the toolchain, returning its standard output when it succeeds.
fn toolchain_output(
    tool: &Toolchain,
    args: &[OsString],
    directory: &Path,
) -> Result<Vec<u8>, String> {
    let mut arguments = Vec::new();
    if let Some(sysroot) = &tool.sysroot {
        arguments.push(OsString::from("--sysroot"));
        arguments.push(sysroot.clone().into_os_string());
    }
    arguments.extend_from_slice(args);
    let tool_path = &tool.rustc;
    let argument_file = argument_file(&arguments, directory);
    let output = match &argument_file {
        Some(file) => {
            let mut at = OsString::from("@");
            at.push(file.as_os_str());
            invoke(tool_path, &[at], directory)
        }
        None => invoke(tool_path, &arguments, directory),
    };
    if let Some(file) = &argument_file {
        let _ = std::fs::remove_file(file);
    }
    let output = output.map_err(|error| format!("cannot run {}: {error}", tool_path.display()))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "native toolchain failed ({}), command {:?}:\n{}{}",
            output.status,
            std::iter::once(tool_path.as_os_str())
                .chain(arguments.iter().map(OsString::as_os_str))
                .collect::<Vec<_>>(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Command lines longer than this pass their arguments in a file. Windows
/// limits a command line to 32767 characters, and the rustup proxy that
/// `rustc` on the search path often is adds the real compiler's path when
/// it relaunches it; a late chunk of a large program names hundreds of
/// dependency folders.
const ARGUMENT_FILE_THRESHOLD: usize = 16 * 1024;

/// Writes long `arguments` to a rustc argument file in `directory`, one per
/// line, returning its path; `None` when the command line is short, or an
/// argument cannot be written that way (a newline, or text that is not
/// Unicode).
fn argument_file(arguments: &[OsString], directory: &Path) -> Option<PathBuf> {
    let length: usize = arguments.iter().map(|argument| argument.len() + 3).sum();
    if length <= ARGUMENT_FILE_THRESHOLD {
        return None;
    }
    let mut text = String::with_capacity(length);
    for argument in arguments {
        let argument = argument.to_str()?;
        if argument.contains(['\n', '\r']) {
            return None;
        }
        text.push_str(argument);
        text.push('\n');
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    // Absolute, as rustc resolves the file against its working directory.
    let file = std::path::absolute(directory.join(format!(
        "nativeboat-rustc-{}-{}.args",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )))
    .ok()?;
    std::fs::write(&file, text).ok()?;
    Some(file)
}

/// Where built libraries are cached for every project: `cache` beside the
/// default work directory. `--work-dir` holds only project build copies.
pub(crate) fn cache_directory() -> PathBuf {
    crate::prepare::default_work_dir().with_file_name("cache")
}

/// A content fingerprint of several byte strings.
pub(crate) fn fingerprint(parts: &[&[u8]]) -> u64 {
    parts.iter().fold(0xcbf2_9ce4_8422_2325, |hash, part| {
        fnv(fnv(hash, &(part.len() as u64).to_le_bytes()), part)
    })
}

fn fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn artifact_fingerprint(path: &Path) -> Result<u64, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "cannot read generated dependency {}: {error}",
            path.display()
        )
    })?;
    Ok(fnv(0xcbf2_9ce4_8422_2325, &bytes))
}

/// A cached folder of libraries by the contents of its rlibs: an entry
/// pruned and rebuilt at the same path may compile to other bytes, and what
/// was compiled against the old copy no longer links with the new one.
#[cfg(any(feature = "embedded-runtime", test))]
fn libraries_fingerprint(directory: &Path) -> Result<u64, String> {
    let mut libraries: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "rlib")
        })
        .collect();
    libraries.sort();
    let mut fingerprint = fnv(
        0xcbf2_9ce4_8422_2325,
        directory.as_os_str().as_encoded_bytes(),
    );
    for library in libraries {
        fingerprint = fnv(fingerprint, &artifact_fingerprint(&library)?.to_le_bytes());
    }
    Ok(fingerprint)
}

/// The argument that names `staging` as `name` in a library's paths, so the
/// same sources compile to the same bytes in any staging folder: the keys of
/// the libraries that depend on it follow its bytes.
fn remap_staging(staging: &Path, name: &str) -> OsString {
    let mut remap = OsString::from("--remap-path-prefix=");
    remap.push(staging);
    remap.push("=");
    remap.push(name);
    remap
}

fn generated_toolchain_key(toolchain_key: u64, dependency_fingerprint: u64) -> u64 {
    fnv(toolchain_key, &dependency_fingerprint.to_le_bytes())
}

/// The toolchain's identity for cache keys: its version details, sysroot
/// and the code generation every cached library shares. Creates the cache
/// directory.
fn toolchain_key(tool: &Toolchain, cache: &Path) -> Result<u64, String> {
    std::fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create cache {}: {error}", cache.display()))?;
    let identity = toolchain_output(tool, &["-vV".into()], cache)?;
    let mut key = fnv(0xcbf2_9ce4_8422_2325, &identity);
    // Libraries built before rlibs left out their LLVM bitcode are rebuilt.
    key = fnv(key, b"-Cembed-bitcode=no");
    if let Some(sysroot) = &tool.sysroot {
        key = fnv(key, sysroot.as_os_str().as_encoded_bytes());
    }
    Ok(key)
}

/// Libraries built once into the cache directory `name` and reused while
/// their key is unchanged. A complete build is published by renaming its
/// staging directory. When another build published first, this build uses
/// that copy, so every crate that links the library links the same one;
/// when nothing could be published, it uses its own copy, which the
/// returned guard removes.
fn cached(
    cache: &Path,
    name: &str,
    library: &str,
    build: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(PathBuf, Option<Work>), String> {
    let directory = cache.join(name);
    if directory.join(library).is_file() {
        mark_used(&directory);
        return Ok((directory, None));
    }
    let staging = cache.join(format!(
        ".{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let guard = Work(staging.clone());
    if let Err(error) = build(&staging) {
        // NATIVEBOAT_KEEP_FAILED keeps a failed build's sources to inspect;
        // the next build removes them as abandoned staging.
        if std::env::var_os("NATIVEBOAT_KEEP_FAILED").is_some() {
            std::mem::forget(guard);
            return Err(format!("{error}\nkept {}", staging.display()));
        }
        return Err(error);
    }
    if staging.join(library).is_file() && std::fs::rename(&staging, &directory).is_ok() {
        std::mem::forget(guard);
        mark_used(&directory);
        prune(cache, name);
        return Ok((directory, None));
    }
    if directory.join(library).is_file() {
        mark_used(&directory);
        return Ok((directory, None));
    }
    Ok((staging, Some(guard)))
}

/// How many builds of each kind the cache keeps.
const CACHE_KEEP: usize = 3;

/// The file whose time records when a build last used a cache entry.
const LAST_USE: &str = ".last-use";

/// Libraries a build used this recently stay: a long build that another
/// build overtakes still links them, and replacing one would change every
/// crate compiled against it.
const CACHE_IN_USE: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

/// Records that a build uses a cache entry now.
fn mark_used(directory: &Path) {
    let _ = std::fs::write(directory.join(LAST_USE), b"");
}

/// When a build last used a cache entry, if one recorded it.
fn last_use(directory: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(directory.join(LAST_USE))
        .ok()?
        .modified()
        .ok()
}

/// Removes all but the most recently built entries of `name`'s kind, so
/// builds for changed sources or toolchains do not accumulate.
fn prune(cache: &Path, name: &str) {
    let Ok(exclusive) = cache_lock_file(cache) else {
        return;
    };
    if exclusive.try_lock().is_err() {
        return;
    }
    prune_unlocked(cache, name);
}

fn prune_unlocked(cache: &Path, name: &str) {
    let kind = name.split_once('-').map_or(name, |(kind, _)| kind);
    let prefix = format!("{kind}-");
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(cache)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| {
            let path = entry.path();
            if std::fs::read_to_string(path.join("owner-directory.txt"))
                .is_ok_and(|owner| !Path::new(&owner).exists())
            {
                let _ = std::fs::remove_dir_all(&path);
                return None;
            }
            let built = entry.metadata().ok()?.modified().ok()?;
            Some((last_use(&path).map_or(built, |used| used.max(built)), path))
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    // Generated application crates are large and rebuilt per source change:
    // only the latest build of each is worth keeping.
    let generated = kind.starts_with("generated_");
    let keep = if generated { 1 } else { CACHE_KEEP };
    let now = std::time::SystemTime::now();
    for (_, path) in entries.into_iter().skip(keep) {
        let in_use = !generated
            && last_use(&path)
                .is_some_and(|used| now.duration_since(used).is_ok_and(|age| age < CACHE_IN_USE));
        if !in_use {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// Writes embedded sources below a directory.
fn write_sources(directory: &Path, sources: &[(&str, &str)]) -> Result<(), String> {
    for (file, source) in sources {
        let path = directory.join(file);
        write_file(&path, source.as_bytes())
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(())
}

/// The folder of a packaged toolchain that holds the runtime libraries
/// precompiled, one folder per library kind.
pub(crate) const PREBUILT: &str = "runtime-libs";

/// The toolchain's precompiled libraries of a kind, holding `library`, for
/// released compilers. Development builds compile the sources they embed,
/// which may be newer than a toolchain's libraries.
fn prebuilt(tool: &Toolchain, kind: &str, library: &str) -> Option<PathBuf> {
    if cfg!(feature = "embedded-runtime") || native_opt_level() != 2 {
        return None;
    }
    shipped(tool, kind, library)
}

/// The folder of a kind of library in the toolchain, if it holds `library`.
fn shipped(tool: &Toolchain, kind: &str, library: &str) -> Option<PathBuf> {
    let directory = tool.sysroot.as_ref()?.join(PREBUILT).join(kind);
    directory.join(library).is_file().then_some(directory)
}

/// The error for libraries a toolchain neither ships nor can build.
#[cfg(not(feature = "embedded-runtime"))]
fn missing(tool: &Toolchain, kind: &str) -> String {
    match &tool.sysroot {
        Some(sysroot) => format!(
            "{} lacks the {kind} libraries; reinstall NativeBoat",
            sysroot.join(PREBUILT).display()
        ),
        None => format!(
            "the {kind} libraries come with the toolchain folder next to nativeboat; \
             build without --rustc"
        ),
    }
}

/// Released compilers carry no runtime sources: their libraries come only
/// from the toolchain.
#[cfg(not(feature = "embedded-runtime"))]
mod without_sources {
    use super::{Toolchain, Work, missing};
    use std::path::{Path, PathBuf};

    type Built = Result<(PathBuf, Option<Work>), String>;

    pub(super) fn matcher_from_source(tool: &Toolchain, _: &Path, _: u64) -> Built {
        Err(missing(tool, "matcher"))
    }

    pub(super) fn url_parser_from_source(tool: &Toolchain, _: &Path, _: u64) -> Built {
        Err(missing(tool, "URL"))
    }

    pub(super) fn encoding_detector_from_source(
        tool: &Toolchain,
        _: &Path,
        _: u64,
        _: &Path,
    ) -> Built {
        Err(missing(tool, "encoding"))
    }

    pub(super) fn runtime_from_source(
        tool: &Toolchain,
        _: &Path,
        _: u64,
        _: (&Path, &Path, &Path),
        renderer: Option<&Path>,
    ) -> Built {
        Err(missing(
            tool,
            if renderer.is_some() {
                "runtime-web"
            } else {
                "runtime"
            },
        ))
    }

    pub(super) fn page_library_from_source(
        tool: &Toolchain,
        _: &Path,
        _: u64,
        _: &[&Path],
        _: &Path,
    ) -> Built {
        Err(missing(tool, "page"))
    }
}
#[cfg(not(feature = "embedded-runtime"))]
use without_sources::*;

/// The packaged regular expression matcher and its dependencies.
fn matcher(tool: &Toolchain, cache: &Path, key: u64) -> Result<(PathBuf, Option<Work>), String> {
    match prebuilt(tool, "matcher", "libnativeboat_regexp.rlib") {
        Some(directory) => Ok((directory, None)),
        None => matcher_from_source(tool, cache, key),
    }
}

#[cfg(feature = "embedded-runtime")]
fn matcher_from_source(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
) -> Result<(PathBuf, Option<Work>), String> {
    let fingerprint = nativeboat_regexp_build::fingerprint();
    let name = format!("matcher-{fingerprint:016x}{key:016x}");
    cached(cache, &name, "libnativeboat_regexp.rlib", |staging| {
        nativeboat_regexp_build::build(staging, &mut |args, directory| {
            toolchain(tool, args, directory)
        })
        .map(drop)
    })
}

/// The pinned no_std URL parser and Unicode dependencies.
fn url_parser(tool: &Toolchain, cache: &Path, key: u64) -> Result<(PathBuf, Option<Work>), String> {
    match prebuilt(tool, "url", "liburl.rlib") {
        Some(directory) => Ok((directory, None)),
        None => url_parser_from_source(tool, cache, key),
    }
}

#[cfg(feature = "embedded-runtime")]
fn url_parser_from_source(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
) -> Result<(PathBuf, Option<Work>), String> {
    let fingerprint = nativeboat_url_build::fingerprint();
    let name = format!("url-{fingerprint:016x}{key:016x}");
    cached(cache, &name, "liburl.rlib", |staging| {
        nativeboat_url_build::build(staging, &mut |args, directory| {
            toolchain(tool, args, directory)
        })
        .map(drop)
    })
}

/// The pinned no_std character-encoding detector.
fn encoding_detector(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    matcher: &Path,
) -> Result<(PathBuf, Option<Work>), String> {
    match prebuilt(tool, "encoding", "libchardetng.rlib") {
        Some(directory) => Ok((directory, None)),
        None => encoding_detector_from_source(tool, cache, key, matcher),
    }
}

#[cfg(feature = "embedded-runtime")]
fn encoding_detector_from_source(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    matcher: &Path,
) -> Result<(PathBuf, Option<Work>), String> {
    let fingerprint = fnv(
        nativeboat_encoding_build::fingerprint(),
        &libraries_fingerprint(matcher)?.to_le_bytes(),
    );
    let name = format!("encoding-{fingerprint:016x}{key:016x}-shared");
    cached(cache, &name, "libchardetng.rlib", |staging| {
        nativeboat_encoding_build::build_with_shared_memchr(
            staging,
            Some(&matcher.join("libmemchr.rlib")),
            &mut |args, directory| toolchain(tool, args, directory),
        )
        .map(drop)
    })
}

/// The rlib of a packaged renderer crate, named with its Cargo hash.
fn renderer_rlib(renderer: &Path, crate_name: &str) -> Result<PathBuf, String> {
    let prefix = format!("lib{crate_name}-");
    std::fs::read_dir(renderer)
        .map_err(|error| format!("cannot read {}: {error}", renderer.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
                && path
                    .extension()
                    .is_some_and(|extension| extension == "rlib")
        })
        .ok_or_else(|| format!("{} has no {crate_name} rlib", renderer.display()))
}

/// The renderer crates compiled for the packaged toolchain target. They are
/// built when NativeBoat is packaged, so project builds still invoke rustc
/// directly and never need Cargo.
fn renderer_libraries(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
) -> Result<Option<(PathBuf, Option<Work>)>, String> {
    let Some(sysroot) = &tool.sysroot else {
        return Ok(None);
    };
    let source = sysroot.join("renderer-libs");
    if !source.is_dir() {
        return Ok(None);
    }
    let mut libraries: Vec<_> = std::fs::read_dir(&source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "rlib" || extension == "dll")
        })
        .collect();
    libraries.sort();
    if libraries.iter().all(|path| {
        !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("libnativeboat_layout-"))
    }) {
        return Err(format!(
            "{} does not contain nativeboat-layout",
            source.display()
        ));
    }
    let mut fingerprint = key;
    for path in &libraries {
        let name = path
            .file_name()
            .ok_or_else(|| format!("invalid renderer library path {}", path.display()))?
            .to_string_lossy();
        let contents = std::fs::read(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        fingerprint = fnv(fnv(fingerprint, name.as_bytes()), &contents);
    }
    let name = format!("renderer-{fingerprint:016x}");
    cached(cache, &name, "renderer-libs.txt", |staging| {
        std::fs::create_dir_all(staging)
            .map_err(|error| format!("cannot create {}: {error}", staging.display()))?;
        let mut manifest = String::new();
        for path in &libraries {
            let file = path.file_name().expect("validated renderer library name");
            std::fs::copy(path, staging.join(file))
                .map_err(|error| format!("cannot cache {}: {error}", path.display()))?;
            manifest.push_str(&file.to_string_lossy());
            manifest.push('\n');
        }
        write_file(&staging.join("renderer-libs.txt"), manifest.as_bytes())
            .map_err(|error| format!("cannot write renderer cache manifest: {error}"))
    })
    .map(Some)
}

/// The start crate and native value library, linked with the matcher, URL
/// parser and encoding detector. Executables link only these libraries,
/// `core`, `alloc` and operating
/// system DLLs: no Rust `std`, C runtime, MSVC or Windows SDK library.
///
/// With the packaged renderer libraries, the runtime also has its `dom`
/// feature: the document natives of renderer pages' Web APIs.
fn runtime(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    libraries: (&Path, &Path, &Path),
    renderer: Option<&Path>,
) -> Result<(PathBuf, Option<Work>), String> {
    let kind = if renderer.is_some() {
        "runtime-web"
    } else {
        "runtime"
    };
    match prebuilt(tool, kind, "libnativeboat_native.rlib") {
        Some(directory) => Ok((directory, None)),
        None => runtime_from_source(tool, cache, key, libraries, renderer),
    }
}

#[cfg(feature = "embedded-runtime")]
fn runtime_from_source(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    (matcher, url, encoding): (&Path, &Path, &Path),
    renderer: Option<&Path>,
) -> Result<(PathBuf, Option<Work>), String> {
    let notices = app_notices();
    let sources: Vec<(&str, &str)> = SOURCES
        .iter()
        .copied()
        .chain(DEFLATE_SOURCES.iter().copied())
        .chain([
            ("start/lib.rs", START),
            ("start/NOTICES.txt", notices.as_str()),
            ("setup/lib.rs", SETUP),
        ])
        .collect();
    let mut fingerprint = key;
    for dependency in [matcher, url, encoding] {
        fingerprint = fnv(
            fingerprint,
            &libraries_fingerprint(dependency)?.to_le_bytes(),
        );
    }
    for (file, source) in &sources {
        fingerprint = fnv(fnv(fingerprint, file.as_bytes()), source.as_bytes());
    }
    // The document crates it links, by content: other renderer crates, such
    // as layout and painting, change without changing the runtime.
    if let Some(renderer) = renderer {
        for name in ["nativeboat_dom", "nativeboat_html"] {
            let library = renderer_rlib(renderer, name)?;
            if let Some(file) = library.file_name() {
                fingerprint = fnv(fingerprint, file.as_encoded_bytes());
            }
            fingerprint = fnv(fingerprint, &artifact_fingerprint(&library)?.to_le_bytes());
        }
    }
    let fingerprint = runtime_optimization_key(fingerprint, native_opt_level());
    let name = format!("runtime-{fingerprint:016x}");
    cached(cache, &name, "libnativeboat_native.rlib", |staging| {
        write_sources(staging, &sources)?;
        // Paths in the libraries name no staging folder, so the same sources
        // compile to the same bytes, which generated crates' keys follow.
        let remap = remap_staging(staging, "nativeboat-runtime");
        for mut arguments in [
            vec![
                "adler2/src/lib.rs".into(),
                "--edition=2021".into(),
                "--crate-type=rlib".into(),
                "--crate-name=adler2".into(),
                "-Cpanic=abort".into(),
                "-Copt-level=2".into(),
                "--cap-lints=allow".into(),
                "-o".into(),
                "libadler2.rlib".into(),
            ],
            vec![
                "miniz_oxide/src/lib.rs".into(),
                "--edition=2021".into(),
                "--crate-type=rlib".into(),
                "--crate-name=miniz_oxide".into(),
                "--cfg".into(),
                "feature=\"with-alloc\"".into(),
                "-Cpanic=abort".into(),
                "-Copt-level=2".into(),
                "--cap-lints=allow".into(),
                "-Ldependency=.".into(),
                "--extern=adler2=libadler2.rlib".into(),
                "-o".into(),
                "libminiz_oxide.rlib".into(),
            ],
        ] {
            arguments.push(remap.clone());
            toolchain(tool, &arguments, staging)?;
        }
        let mut extern_matcher = OsString::from("--extern=nativeboat_regexp=");
        extern_matcher.push(matcher.join("libnativeboat_regexp.rlib"));
        let mut extern_deflate = OsString::from("--extern=miniz_oxide=");
        extern_deflate.push(staging.join("libminiz_oxide.rlib"));
        let mut extern_chardetng = OsString::from("--extern=chardetng=");
        extern_chardetng.push(encoding.join("libchardetng.rlib"));
        let mut matcher_dependencies = OsString::from("-Ldependency=");
        matcher_dependencies.push(matcher);
        let mut url_dependencies = OsString::from("-Ldependency=");
        url_dependencies.push(url);
        for (source, name, links_matcher) in [
            ("start/lib.rs", "nativeboat_start", false),
            ("lib.rs", "nativeboat_native", true),
        ] {
            let mut arguments: Vec<OsString> = vec![
                source.into(),
                "--edition=2024".into(),
                "--crate-type=rlib".into(),
                format!("--crate-name={name}").into(),
                "-Cpanic=abort".into(),
                format!("-Copt-level={}", native_opt_level()).into(),
                "-Dwarnings".into(),
                "-o".into(),
                format!("lib{name}.rlib").into(),
                remap.clone(),
            ];
            if links_matcher {
                arguments.push(extern_matcher.clone());
                arguments.push(extern_deflate.clone());
                arguments.push(extern_chardetng.clone());
                arguments.push("-Ldependency=.".into());
                arguments.push(matcher_dependencies.clone());
                arguments.push(url_dependencies.clone());
                let mut encoding_dependencies = OsString::from("-Ldependency=");
                encoding_dependencies.push(encoding);
                arguments.push(encoding_dependencies);
                for name in ["url", "idna"] {
                    let mut argument = OsString::from(format!("--extern={name}="));
                    argument.push(url.join(format!("lib{name}.rlib")));
                    arguments.push(argument);
                }
                if let Some(renderer) = renderer {
                    arguments.push("--cfg".into());
                    arguments.push("feature=\"dom\"".into());
                    for name in ["nativeboat_dom", "nativeboat_html"] {
                        let mut argument = OsString::from(format!("--extern={name}="));
                        argument.push(renderer_rlib(renderer, name)?);
                        arguments.push(argument);
                    }
                    let mut dependencies = OsString::from("-Ldependency=");
                    dependencies.push(renderer);
                    arguments.push(dependencies);
                }
            }
            toolchain(tool, &arguments, staging)?;
        }
        toolchain(
            tool,
            &[
                "setup/lib.rs".into(),
                "--edition=2024".into(),
                "--crate-type=rlib".into(),
                "--crate-name=nativeboat_setup".into(),
                "-Cpanic=abort".into(),
                "-Copt-level=s".into(),
                "-Dwarnings".into(),
                "--extern=nativeboat_start=libnativeboat_start.rlib".into(),
                extern_deflate,
                "-Ldependency=.".into(),
                "-o".into(),
                "libnativeboat_setup.rlib".into(),
                remap,
            ],
            staging,
        )?;
        Ok(())
    })
}

/// The Node.js library modules, compiled from their sources once per runtime
/// build and generated library, and linked with the runtime.
fn node_library(
    tool: &Toolchain,
    cache: &Path,
    runtime: &Path,
    (matcher, url, encoding): (&Path, &Path, &Path),
    renderer: Option<&Path>,
    selected: &std::collections::BTreeSet<String>,
) -> Result<(PathBuf, Option<Work>), String> {
    let source = crate::node_lib::library(selected);
    // This large precompiled compatibility graph is the most memory-intensive
    // generated crate. Keep its single cold build within ordinary developer
    // memory while leaving the final application at release optimization.
    const NODE_OPT_LEVEL: u8 = 1;
    // The runtime it links counts by content, below, not by its folder.
    let fingerprint = fnv(0xcbf2_9ce4_8422_2325, &[NODE_OPT_LEVEL]);
    let fingerprint = fnv(fingerprint, source.as_bytes());
    let runtime_fingerprint = artifact_fingerprint(&runtime.join("libnativeboat_native.rlib"))?;
    let fingerprint = generated_toolchain_key(fingerprint, runtime_fingerprint);
    let name = format!("node-{fingerprint:016x}");
    cached(cache, &name, "libnativeboat_node.rlib", |staging| {
        write_sources(staging, &[("lib.rs", source.as_str())])?;
        let mut native = OsString::from("--extern=nativeboat_native=");
        native.push(runtime.join("libnativeboat_native.rlib"));
        let mut runtime_dependencies = OsString::from("-L");
        runtime_dependencies.push(runtime);
        let mut matcher_dependencies = OsString::from("-Ldependency=");
        matcher_dependencies.push(matcher);
        let mut url_dependencies = OsString::from("-Ldependency=");
        url_dependencies.push(url);
        let mut encoding_dependencies = OsString::from("-Ldependency=");
        encoding_dependencies.push(encoding);
        let mut arguments: Vec<OsString> = vec![
            "lib.rs".into(),
            "--edition=2024".into(),
            "--crate-type=rlib".into(),
            "--crate-name=nativeboat_node".into(),
            "-Cpanic=abort".into(),
            format!("-Copt-level={NODE_OPT_LEVEL}").into(),
            "-Ccodegen-units=1".into(),
            "-Cembed-bitcode=no".into(),
            "-Dwarnings".into(),
            native,
            runtime_dependencies,
            matcher_dependencies,
            url_dependencies,
            encoding_dependencies,
            "-o".into(),
            "libnativeboat_node.rlib".into(),
            remap_staging(staging, "nativeboat-node"),
        ];
        if let Some(renderer) = renderer {
            let mut dependencies = OsString::from("-Ldependency=");
            dependencies.push(renderer);
            arguments.push(dependencies);
        }
        toolchain(tool, &arguments, staging)?;
        // Only the library is linked; its generated source is not kept.
        let _ = std::fs::remove_file(staging.join("lib.rs"));
        Ok(())
    })
}

/// The page backend is separate from the value runtime, so geometry and
/// painting changes do not rebuild application and dependency module chunks.
fn page_library(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    dependencies: &[&Path],
    renderer: &Path,
) -> Result<(PathBuf, Option<Work>), String> {
    match prebuilt(tool, "page", "libnativeboat_page.rlib") {
        Some(directory) => Ok((directory, None)),
        None => page_library_from_source(tool, cache, key, dependencies, renderer),
    }
}

#[cfg(feature = "embedded-runtime")]
fn page_library_from_source(
    tool: &Toolchain,
    cache: &Path,
    key: u64,
    dependencies: &[&Path],
    renderer: &Path,
) -> Result<(PathBuf, Option<Work>), String> {
    const SOURCE: &str = include_str!("../../nativeboat-page/src/lib.rs");
    const WINDOW: &str = include_str!("../../nativeboat-page/src/window.rs");
    const ASSETS: &str = include_str!("../../nativeboat-page/src/assets.rs");
    let fingerprint = fnv(
        fnv(
            generated_toolchain_key(
                key,
                artifact_fingerprint(&dependencies[0].join("libnativeboat_native.rlib"))?,
            ),
            renderer.as_os_str().as_encoded_bytes(),
        ),
        &[SOURCE.as_bytes(), WINDOW.as_bytes(), ASSETS.as_bytes()].concat(),
    );
    cached(
        cache,
        &format!("page-{fingerprint:016x}"),
        "libnativeboat_page.rlib",
        |staging| {
            write_file(&staging.join("lib.rs"), SOURCE.as_bytes())
                .and_then(|()| write_file(&staging.join("window.rs"), WINDOW.as_bytes()))
                .and_then(|()| write_file(&staging.join("assets.rs"), ASSETS.as_bytes()))
                .map_err(|error| format!("cannot write native page source: {error}"))?;
            let mut arguments: Vec<OsString> = vec![
                "lib.rs".into(),
                "--edition=2024".into(),
                "--crate-name=nativeboat_page".into(),
                "--crate-type=rlib".into(),
                "-Cpanic=abort".into(),
                "-Copt-level=2".into(),
                "-Ccodegen-units=1".into(),
                "-Cembed-bitcode=no".into(),
                "-Dwarnings".into(),
                "-o".into(),
                "libnativeboat_page.rlib".into(),
                remap_staging(staging, "nativeboat-page"),
            ];
            let mut runtime = OsString::from("--extern=nativeboat_native=");
            runtime.push(dependencies[0].join("libnativeboat_native.rlib"));
            arguments.push(runtime);
            for name in [
                "nativeboat_dom",
                "nativeboat_css",
                "nativeboat_layout",
                "nativeboat_ir",
                "nativeboat_win32",
            ] {
                let mut argument = OsString::from(format!("--extern={name}="));
                argument.push(renderer_rlib(renderer, name)?);
                arguments.push(argument);
            }
            for dependency in dependencies {
                let mut argument = OsString::from("-Ldependency=");
                argument.push(dependency);
                arguments.push(argument);
            }
            toolchain(tool, &arguments, staging)
        },
    )
}

/// The output directory: `--out` relative to the project, or the project's
/// configured output directory.
fn output(project: &Project, args: &Args) -> PathBuf {
    project.root.join(
        args.out
            .as_deref()
            .unwrap_or(Path::new(&project.output_directory)),
    )
}

/// The version of the `electron` package the project resolves, which the
/// application reports as `process.versions.electron`.
fn electron_version(root: &Path) -> Option<String> {
    root.ancestors().find_map(|directory| {
        let manifest = directory.join("node_modules/electron/package.json");
        let text = std::fs::read_to_string(manifest).ok()?;
        let package: serde_json::Value = serde_json::from_str(&text).ok()?;
        package["version"].as_str().map(str::to_owned)
    })
}

/// The customer's signing command with the executable path filled in.
fn signing_command(command: &str, executable: &Path) -> String {
    let file = format!("\"{}\"", executable.display());
    if command.contains("{file}") {
        command.replace("{file}", &file)
    } else {
        format!("{command} {file}")
    }
}

/// Signs the staged executable with the customer's own tool, so a failed
/// signature never publishes an unsigned executable.
pub(crate) fn sign(command: &str, executable: &Path, directory: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let command = signing_command(command, executable);
    let output = Command::new(crate::steps::shell())
        .raw_arg(format!("/d /s /c \"{command}\""))
        .current_dir(directory)
        .output()
        .map_err(|error| format!("cannot run the signing command: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "signing failed ({}): {command}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Retry transient Windows sharing violations while security software scans a
/// freshly linked executable before it can be moved into the output folder.
fn publish_with_retry(
    source: &Path,
    destination: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    for attempt in 0..6 {
        match rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(25 * (1 << attempt)));
            }
            Err(error) => return Err(error),
        }
    }
    rename(source, destination)
}

fn rename_executable(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

fn link(
    project: &Project,
    compilation: &NativeCompilation,
    directory: &Path,
    args: &Args,
) -> Result<PathBuf, Vec<String>> {
    link_inner(project, compilation, directory, args).map_err(|error| vec![error])
}

fn link_inner(
    project: &Project,
    compilation: &NativeCompilation,
    directory: &Path,
    args: &Args,
) -> Result<PathBuf, String> {
    if compilation.main.is_none() {
        return Err("main module did not compile".into());
    }
    let name = &project.metadata.product_name;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
    {
        return Err("productName must be a single valid executable filename".into());
    }
    std::fs::create_dir_all(directory).map_err(|error| {
        format!(
            "cannot create native output {}: {error}",
            directory.display()
        )
    })?;
    remove_abandoned_staging(directory, ".nativeboat-build-");
    // A build that was killed can leave its staging directory behind under
    // a process id Windows has since reused; take the next free name.
    let mut attempts = 0;
    let path = loop {
        let path = directory.join(format!(
            ".nativeboat-build-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match create_directory(&path) {
            Ok(()) => break path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempts < 64 => {
                attempts += 1;
            }
            Err(error) => {
                return Err(format!(
                    "cannot create native staging directory {}: {error}",
                    path.display()
                ));
            }
        }
    };
    let work = Work(path);
    let rustc = select_toolchain(
        args.rustc.as_deref(),
        &std::env::current_exe().unwrap_or_default(),
    );
    // The matcher and runtime libraries are compiled once per toolchain and
    // reused by later builds; only the application is compiled each time.
    let cache = cache_directory();
    let _cache_use = CacheUse::new(&cache)?;
    let key = toolchain_key(&rustc, &cache)?;
    let (matcher, _matcher_copy) = matcher(&rustc, &cache, key)?;
    let (url, _url_copy) = url_parser(&rustc, &cache, key)?;
    let (encoding, _encoding_copy) = encoding_detector(&rustc, &cache, key, &matcher)?;
    let (renderer, _renderer_copy) = if compilation.renderers.is_empty() {
        (None, None)
    } else {
        match renderer_libraries(&rustc, &cache, key)? {
            Some((directory, copy)) => (Some(directory), copy),
            None => (None, None),
        }
    };
    let (runtime, _runtime_copy) = runtime(
        &rustc,
        &cache,
        key,
        (&matcher, &url, &encoding),
        renderer.as_deref(),
    )?;
    // The Node.js library modules a program uses are compiled once per
    // selection and linked only when used.
    let node = match crate::node_lib::selected(compilation.modules()) {
        Some(selected) => Some(node_library(
            &rustc,
            &cache,
            &runtime,
            (&matcher, &url, &encoding),
            renderer.as_deref(),
            &selected,
        )?),
        None => None,
    };
    // A runtime with document natives finds the renderer crates it links.
    let mut dependencies = vec![runtime.as_path(), &matcher, &url, &encoding];
    dependencies.extend(renderer.as_deref());
    let page = renderer
        .as_deref()
        .map(|renderer| page_library(&rustc, &cache, key, &dependencies, renderer))
        .transpose()?;
    let context_libraries = compile_context_libraries(
        &rustc,
        key,
        &work.0,
        &dependencies,
        compilation,
        &project.root,
    )?;
    let library = |name: &str| {
        let mut argument = OsString::from(format!("--extern={name}="));
        argument.push(runtime.join(format!("lib{name}.rlib")));
        argument
    };
    let mut runtime_dependencies = OsString::from("-L");
    runtime_dependencies.push(&runtime);
    let mut matcher_dependencies = OsString::from("-Ldependency=");
    matcher_dependencies.push(&matcher);
    let mut url_dependencies = OsString::from("-Ldependency=");
    url_dependencies.push(&url);
    let mut encoding_dependencies = OsString::from("-Ldependency=");
    encoding_dependencies.push(&encoding);
    let source = application_source(project, compilation, node.is_some(), renderer.is_some())?;
    write_file(&work.0.join("app.rs"), source.as_bytes())
        .map_err(|error| format!("cannot write generated native application: {error}"))?;
    // The manifest, version information and icon, written by nativeboat.
    let resources = work.0.join("app.res");
    let data = crate::resources::resources(&project.metadata)?;
    write_file(&resources, &data)
        .map_err(|error| format!("cannot write {}: {error}", resources.display()))?;
    let mut arguments: Vec<OsString> = vec![
        "app.rs".into(),
        "--edition=2024".into(),
        "--crate-name=nativeboat_app".into(),
        library("nativeboat_native"),
        library("nativeboat_start"),
        runtime_dependencies,
        matcher_dependencies,
        url_dependencies,
        encoding_dependencies,
    ];
    if let Some((node, _)) = &node {
        let mut argument = OsString::from("--extern=nativeboat_node=");
        argument.push(node.join("libnativeboat_node.rlib"));
        arguments.push(argument);
    }
    if let Some((page, _)) = &page {
        let mut argument = OsString::from("--extern=nativeboat_page=");
        argument.push(page.join("libnativeboat_page.rlib"));
        arguments.push(argument);
    }
    for library in context_libraries {
        let mut argument = OsString::from(format!("--extern={}", library.crate_name));
        argument.push("=");
        argument.push(library.path);
        arguments.push(argument);
        arguments.extend(library.dependency_dirs.iter().map(|path| {
            let mut dependency = OsString::from("-Ldependency=");
            dependency.push(path);
            dependency
        }));
    }
    let mut generated_dependencies = OsString::from("-Ldependency=");
    generated_dependencies.push(&work.0);
    arguments.push(generated_dependencies);
    if let Some(renderer) = &renderer {
        for crate_name in [
            "nativeboat_win32",
            "nativeboat_layout",
            "nativeboat_ir",
            "nativeboat_dom",
            "nativeboat_css",
            "nativeboat_html",
        ] {
            let library = renderer_rlib(renderer, crate_name)?;
            let mut argument = OsString::from(format!("--extern={crate_name}="));
            argument.push(library);
            arguments.push(argument);
        }
        let mut dependencies = OsString::from("-Ldependency=");
        dependencies.push(renderer);
        arguments.push(dependencies);
    }
    // MarkText lowers many dependency sources into one Rust crate. Use the
    // memory-bounded optimization profile for this whole-program translation;
    // the native runtime and renderer rlibs retain their own release settings.
    const APP_OPT_LEVEL: u8 = 0;
    toolchain(
        &rustc,
        &[
            arguments,
            vec![
                "-Cpanic=abort".into(),
                format!("-Copt-level={APP_OPT_LEVEL}").into(),
                "-Ccodegen-units=1".into(),
                "-Dwarnings".into(),
                "-Clinker=rust-lld".into(),
                "-Clinker-flavor=lld-link".into(),
                "-Clink-arg=/NODEFAULTLIB".into(),
                "-Clink-arg=/ENTRY:nativeboat_start".into(),
                // Electron applications are windowed programs: no console
                // window opens with them. Redirected standard handles still
                // reach the program.
                "-Clink-arg=/SUBSYSTEM:WINDOWS".into(),
                "-Clink-arg=/STACK:268435456".into(),
                // rustc turns identical code folding off for unoptimized
                // crates such as this one; generated code repeats many
                // identical functions, which folding merges.
                "-Clink-arg=/OPT:ICF".into(),
                {
                    let mut argument = OsString::from("-Clink-arg=");
                    argument.push(&resources);
                    argument
                },
                "-o".into(),
                "app.exe".into(),
            ],
            link_map(std::env::var_os("NATIVEBOAT_LINK_MAP")),
        ]
        .concat(),
        &work.0,
    )?;
    if let Some(command) = &args.sign {
        sign(command, &work.0.join("app.exe"), &project.root)?;
    }
    // The application's files first, so extraResources and extraFiles
    // replace any of them, as in an electron-builder package. Files inside
    // the output folder are the build's own.
    let output = nativeboat_bundler::resolve::normalize(directory);
    let application_files: Vec<_> = project
        .application_files
        .iter()
        .filter(|file| !nativeboat_bundler::resolve::normalize(&file.source).starts_with(&output))
        .cloned()
        .collect();
    copy_extra_files(&application_files, directory)?;
    copy_extra_files(&project.extra_files, directory)?;
    write_plugin_files(project, compilation, directory)?;
    // electron-updater reads its feed from the resources folder.
    if let Some(feed) = &project.updater_config {
        let path = directory.join("resources").join("app-update.yml");
        std::fs::create_dir_all(directory.join("resources"))
            .and_then(|()| std::fs::write(&path, nativeboat_project::builder::app_update_yml(feed)))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    let executable = directory.join(format!("{name}.exe"));
    publish_with_retry(&work.0.join("app.exe"), &executable, rename_executable)
        .map_err(|error| format!("cannot publish {}: {error}", executable.display()))?;
    let notices = directory.join(NOTICES_FILE);
    std::fs::write(&notices, app_notices())
        .map_err(|error| format!("cannot write {}: {error}", notices.display()))?;
    if let Some(destination) =
        crate::installer::directory(project, directory, args.installer.as_deref())
    {
        crate::installer::write(&crate::installer::Inputs {
            rustc: &rustc,
            runtime: &runtime,
            work: &work.0,
            resources: &resources,
            project,
            application: directory,
            executable: &format!("{name}.exe"),
            destination: &destination,
            sign: args.sign.as_deref(),
        })?;
    }
    Ok(executable)
}

struct ContextLibrary {
    crate_name: String,
    path: PathBuf,
    dependency_dirs: Vec<PathBuf>,
}

struct GeneratedCacheInputs<'a> {
    toolchain_key: u64,
    dependencies: &'a [&'a Path],
}

struct GeneratedRlib<'a> {
    externs: &'a [OsString],
    crate_name: &'a str,
    source_name: &'a str,
    source: &'a str,
    fingerprint: u64,
    /// Whether the crate holds application code, which is optimized.
    /// Linking crates (module tables and dispatchers) run once per module
    /// and compile fastest unoptimized: optimizing their thousands of small
    /// functions takes rustc minutes and saves nothing at run time.
    application: bool,
}

/// Compiles each application realm graph as its own rlib. MarkText's combined
/// main, preload and renderer graph exceeded rustc's memory even at opt-level
/// zero; separate crates cap peak codegen memory and keep the small app host
/// crate independent of the generated module AST size.
fn compile_context_libraries(
    rustc: &Toolchain,
    toolchain_key: u64,
    staging: &Path,
    dependencies: &[&Path],
    compilation: &NativeCompilation,
    root: &Path,
) -> Result<Vec<ContextLibrary>, String> {
    let runtime_fingerprint =
        artifact_fingerprint(&dependencies[0].join("libnativeboat_native.rlib"))?;
    let generated_toolchain_key = generated_toolchain_key(toolchain_key, runtime_fingerprint);
    let mut libraries = Vec::new();
    let mut contexts = Vec::new();
    if let Some(main) = &compilation.main {
        libraries.push(compile_split_context_library(
            rustc,
            staging,
            GeneratedCacheInputs {
                toolchain_key: generated_toolchain_key,
                dependencies,
            },
            "main",
            (0, "0"),
            // Start-up evaluates the main module only: forked modules run in
            // their own processes, and lazily imported ones when imported.
            (main, 1),
            &std::collections::BTreeMap::new(),
        )?);
    }
    for (kind, entries) in [("preload", &compilation.preloads)] {
        for (index, context) in entries.iter().enumerate() {
            let crate_name = format!("nativeboat_{kind}_{index}");
            contexts.push((
                crate_name,
                context_library_source(kind, index, context, &compilation.stylesheets),
            ));
        }
    }
    for (crate_name, source) in contexts {
        let output = staging.join(format!("lib{crate_name}.rlib"));
        let source_name = format!("{crate_name}.rs");
        write_file(&staging.join(&source_name), source.as_bytes())
            .map_err(|error| format!("cannot write generated {crate_name}: {error}"))?;
        let mut native = OsString::from("--extern=nativeboat_native=");
        native.push(dependencies[0].join("libnativeboat_native.rlib"));
        let dependency_paths = dependencies.iter().map(|path| {
            let mut argument = OsString::from("-Ldependency=");
            argument.push(path);
            argument
        });
        let mut arguments = vec![
            source_name.into(),
            "--edition=2024".into(),
            format!("--crate-name={crate_name}").into(),
            "--crate-type=rlib".into(),
            "-Cpanic=abort".into(),
            format!("-Copt-level={}", generated_opt_level()).into(),
            "-Ccodegen-units=1".into(),
            "-Cembed-bitcode=no".into(),
            "-Dwarnings".into(),
            native,
        ];
        arguments.extend(dependency_paths);
        arguments.extend(["-o".into(), output.as_os_str().to_owned()]);
        toolchain(rustc, &arguments, staging)?;
        libraries.push(ContextLibrary {
            crate_name,
            path: output,
            dependency_dirs: Vec::new(),
        });
    }
    for (index, context) in compilation.renderers.iter().enumerate() {
        libraries.push(compile_split_context_library(
            rustc,
            staging,
            GeneratedCacheInputs {
                toolchain_key: generated_toolchain_key,
                dependencies,
            },
            "renderer",
            (index, &graph_name(root, &context.entry)),
            (&context.module, context.entry_count),
            &compilation.stylesheets,
        )?);
    }
    Ok(libraries)
}

/// Compiles a large context as module chunks plus a small graph/link crate.
/// Chunk boundaries retain the compiler's module ids and shared cell table;
/// only independent Rust functions move between rlibs.
/// The folder of links to a graph's generated crates, made empty: crates
/// that depend on others find them all there. One search path for a graph
/// keeps rustc's library search path, which it adds to `PATH` when it loads a
/// procedural macro, within Windows' 32 767-character limit, which one path
/// per cached crate passed for graphs of a few hundred chunks.
fn linked_dependencies(staging: &Path, kind: &str, graph: &str) -> Result<PathBuf, String> {
    let folder = staging.join(format!("linked-{kind}-{graph}"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    Ok(folder)
}

/// Links a generated crate into the folder, replacing one of its name; it is
/// copied where it cannot be linked.
fn link_dependency(folder: &Path, rlib: &Path) -> Result<(), String> {
    let Some(name) = rlib.file_name() else {
        return Ok(());
    };
    let target = folder.join(name);
    let _ = std::fs::remove_file(&target);
    std::fs::hard_link(rlib, &target)
        .or_else(|_| std::fs::copy(rlib, &target).map(drop))
        .map_err(|error| {
            format!(
                "cannot link {} into {}: {error}",
                rlib.display(),
                folder.display()
            )
        })
}

/// The name of a renderer graph's chunk and forwarder crates: a hash of its
/// first page's path in the project. The chunks keep their cached builds when
/// other pages are added or removed, or when pages that load the same scripts
/// come to share a graph, which an index into the list of graphs would change.
fn graph_name(root: &Path, entry: &Path) -> String {
    let relative = entry.strip_prefix(root).unwrap_or(entry);
    let path = relative
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    format!("p{:012x}", fingerprint(&[path.as_bytes()]) >> 16)
}

fn compile_split_context_library(
    rustc: &Toolchain,
    staging: &Path,
    cache_inputs: GeneratedCacheInputs<'_>,
    kind: &str,
    (index, graph): (usize, &str),
    module_graph: (&DynamicModule, usize),
    stylesheets: &std::collections::BTreeMap<String, String>,
) -> Result<ContextLibrary, String> {
    let (module, entry_count) = module_graph;
    let units = ModuleUnits::of(module.module_rust_parts());
    let (chunks, unit_chunk) = module_chunk_map(&units, CHUNK_BYTES);
    write_module_map(
        std::env::var_os("NATIVEBOAT_MODULE_MAP").as_deref(),
        &format!("{kind}_{index}"),
        &units,
        &unit_chunk,
    )?;
    let chunk_names = (0..chunks.len())
        .map(|chunk| format!("nativeboat_{kind}_{graph}_chunk_{chunk}"))
        .collect::<Vec<_>>();
    let chunk_dependencies = chunk_dependencies(&units, &unit_chunk, chunks.len());
    let cache = cache_directory();
    let linked = linked_dependencies(staging, kind, graph)?;
    // Sources and cache keys first, in dependency order: a chunk's key
    // includes its dependencies' keys.
    let order = chunk_compile_order(&chunk_dependencies);
    let mut sources = vec![String::new(); chunks.len()];
    let mut keys = vec![0u64; chunks.len()];
    for &chunk_index in &order {
        let mut source = dynamic_graph_prelude();
        for unit in &chunks[chunk_index] {
            source.push_str(&qualify_chunk_references(
                &units,
                *unit,
                chunk_index,
                &unit_chunk,
                &chunk_names,
            ));
            source.push('\n');
        }
        let dependency_keys = chunk_dependencies[chunk_index]
            .iter()
            .map(|dependency| keys[*dependency])
            .collect::<Vec<_>>();
        keys[chunk_index] = generated_rlib_fingerprint(
            cache_inputs.toolchain_key,
            &chunk_names[chunk_index],
            &source,
            &dependency_keys,
        );
        sources[chunk_index] = source;
    }
    // Independent chunks compile concurrently.
    let outputs = run_dependency_jobs(
        &order,
        &chunk_dependencies,
        generated_compile_jobs(),
        // NATIVEBOAT_KEEP_GOING reports every chunk that fails to compile,
        // not just the first, while developing the compiler.
        std::env::var_os("NATIVEBOAT_KEEP_GOING").is_some(),
        |chunk_index, done: &[Option<(PathBuf, u64)>]| {
            let crate_name = &chunk_names[chunk_index];
            let source_name = format!("{crate_name}.rs");
            // Every finished chunk includes this one's transitive dependencies,
            // which the folder of linked crates holds.
            let mut dependency_paths = cache_inputs
                .dependencies
                .iter()
                .map(|path| path.to_path_buf())
                .collect::<Vec<_>>();
            dependency_paths.push(linked.clone());
            let chunk_externs = chunk_dependencies[chunk_index]
                .iter()
                .map(|other| {
                    let name = &chunk_names[*other];
                    let path = done[*other]
                        .as_ref()
                        .expect("chunk dependencies are compiled before their dependents");
                    let mut argument = OsString::from(format!("--extern={name}="));
                    argument.push(&path.0);
                    argument
                })
                .collect::<Vec<_>>();
            let dependency_refs = dependency_paths
                .iter()
                .map(PathBuf::as_path)
                .collect::<Vec<_>>();
            let dependency_artifacts = chunk_dependencies[chunk_index]
                .iter()
                .map(|dependency| done[*dependency].as_ref().unwrap().1)
                .collect::<Vec<_>>();
            let fingerprint =
                generated_dependency_fingerprint(keys[chunk_index], &dependency_artifacts);
            let (output, cache_hit) = compile_generated_rlib_cached(
                rustc,
                &cache,
                staging,
                &dependency_refs,
                GeneratedRlib {
                    externs: &chunk_externs,
                    crate_name,
                    source_name: &source_name,
                    source: &sources[chunk_index],
                    fingerprint,
                    application: true,
                },
            )?;
            eprintln!(
                "nativeboat: {kind} chunk {}/{} {} ({})",
                chunk_index + 1,
                chunks.len(),
                crate_name,
                if cache_hit { "cache hit" } else { "compiled" }
            );
            let identity = artifact_fingerprint(&output)?;
            link_dependency(&linked, &output)?;
            Ok((output, identity))
        },
    )?;
    let (chunk_paths, chunk_keys): (Vec<_>, Vec<_>) = outputs
        .into_iter()
        .map(|(path, identity)| (Some(path), Some(identity)))
        .unzip();

    // Keep the final project crate's direct dependency fanout bounded. A
    // MarkText renderer can contain dozens of module chunks; importing every
    // chunk into one crate makes rustc load all generated metadata at once and
    // can exhaust its memory before the executable link begins.
    const MAX_FORWARDER_CHILDREN: usize = 8;
    let mut generated_dependency_paths = cache_inputs
        .dependencies
        .iter()
        .map(|path| path.to_path_buf())
        .collect::<Vec<_>>();
    generated_dependency_paths.push(linked.clone());
    let generated_dependency_refs = generated_dependency_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let mut forwarders = chunk_names
        .iter()
        .zip(&chunk_paths)
        .zip(&chunks)
        .enumerate()
        .map(|(index, ((crate_name, path), unit_ids))| {
            // A chunk evaluates the modules whose last parts it compiles.
            let module_ids = unit_ids
                .iter()
                .filter(|unit| units.last[units.modules[**unit]] == **unit)
                .map(|unit| units.modules[*unit])
                .collect::<Vec<_>>();
            ForwarderLibrary {
                crate_name: crate_name.clone(),
                path: path
                    .as_ref()
                    .expect("all chunks are compiled before dispatchers")
                    .clone(),
                commonjs_modules: module_ids
                    .iter()
                    .copied()
                    .filter(|module| {
                        units.sources[units.last[*module]]
                            .contains(&format!("fn require_c{module}("))
                    })
                    .collect(),
                modules: module_ids,
                is_dispatcher: false,
                fingerprint: chunk_keys[index].expect("all renderer chunks have a cache key"),
                dependency_dirs: vec![linked.clone()],
            }
        })
        .collect::<Vec<_>>();
    let mut level = 0;
    while forwarders.len() > 1 || level == 0 {
        let mut next = Vec::new();
        for (group, children) in forwarders.chunks(MAX_FORWARDER_CHILDREN).enumerate() {
            next.push(compile_module_dispatcher(
                rustc,
                &cache,
                staging,
                cache_inputs.toolchain_key,
                &generated_dependency_refs,
                ForwarderIdentity {
                    kind,
                    graph,
                    level,
                    group,
                },
                children,
            )?);
        }
        forwarders = next;
        level += 1;
    }
    let dispatcher = forwarders
        .pop()
        .expect("a renderer graph with modules has a module dispatcher");
    let mut dispatcher_extern = OsString::from(format!("--extern={}=", dispatcher.crate_name));
    dispatcher_extern.push(&dispatcher.path);

    let crate_name = format!("nativeboat_{kind}_{index}");
    let mut source = dynamic_graph_prelude();
    source.push_str(&linked_entry_source(
        module.entry_rust_source(),
        &dispatcher.crate_name,
        &dispatcher.modules,
        &dispatcher.commonjs_modules,
        entry_count,
        module.scripts(),
        stylesheets,
    ));
    let source_name = format!("{crate_name}.rs");
    let mut renderer_dependency_paths = generated_dependency_paths;
    renderer_dependency_paths.extend(dispatcher.dependency_dirs.iter().cloned());
    let mut seen_dependency_paths = std::collections::BTreeSet::new();
    renderer_dependency_paths.retain(|path| seen_dependency_paths.insert(path.clone()));
    let renderer_dependency_refs = renderer_dependency_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let fingerprint = generated_rlib_fingerprint(
        cache_inputs.toolchain_key,
        &crate_name,
        &source,
        &[dispatcher.fingerprint],
    );
    let externs = [dispatcher_extern];
    let (output, _) = compile_generated_rlib_cached(
        rustc,
        &cache,
        staging,
        &renderer_dependency_refs,
        GeneratedRlib {
            externs: &externs,
            crate_name: &crate_name,
            source_name: &source_name,
            source: &source,
            fingerprint,
            application: false,
        },
    )?;
    Ok(ContextLibrary {
        crate_name,
        path: output,
        dependency_dirs: renderer_dependency_paths,
    })
}

struct ForwarderLibrary {
    crate_name: String,
    path: PathBuf,
    modules: Vec<usize>,
    commonjs_modules: Vec<usize>,
    is_dispatcher: bool,
    fingerprint: u64,
    dependency_dirs: Vec<PathBuf>,
}

struct ForwarderIdentity<'a> {
    kind: &'a str,
    graph: &'a str,
    level: usize,
    group: usize,
}

fn compile_module_dispatcher(
    rustc: &Toolchain,
    cache: &Path,
    staging: &Path,
    toolchain_key: u64,
    dependencies: &[&Path],
    identity: ForwarderIdentity<'_>,
    children: &[ForwarderLibrary],
) -> Result<ForwarderLibrary, String> {
    let crate_name = format!(
        "nativeboat_{}_{}_forwarder_{}_{}",
        identity.kind, identity.graph, identity.level, identity.group
    );
    let modules = children
        .iter()
        .flat_map(|child| child.modules.iter().copied())
        .collect::<Vec<_>>();
    let commonjs_modules = children
        .iter()
        .flat_map(|child| child.commonjs_modules.iter().copied())
        .collect::<Vec<_>>();
    let source = module_dispatcher_source(&crate_name, children);
    let source_name = format!("{crate_name}.rs");
    let externs = children
        .iter()
        .map(|child| {
            let mut argument = OsString::from(format!("--extern={}=", child.crate_name));
            argument.push(&child.path);
            argument
        })
        .collect::<Vec<_>>();
    let dependency_keys = children
        .iter()
        .map(|child| child.fingerprint)
        .collect::<Vec<_>>();
    let mut dependency_paths = dependencies
        .iter()
        .map(|path| path.to_path_buf())
        .collect::<Vec<_>>();
    dependency_paths.extend(
        children
            .iter()
            .flat_map(|child| child.dependency_dirs.iter().cloned()),
    );
    let mut seen_dependency_paths = std::collections::BTreeSet::new();
    dependency_paths.retain(|path| seen_dependency_paths.insert(path.clone()));
    let dependency_refs = dependency_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let fingerprint =
        generated_rlib_fingerprint(toolchain_key, &crate_name, &source, &dependency_keys);
    let (path, _) = compile_generated_rlib_cached(
        rustc,
        cache,
        staging,
        &dependency_refs,
        GeneratedRlib {
            externs: &externs,
            crate_name: &crate_name,
            source_name: &source_name,
            source: &source,
            fingerprint,
            application: false,
        },
    )?;
    // The forwarder joins its children in the folder of linked crates.
    let mut dependency_dirs = children
        .iter()
        .flat_map(|child| child.dependency_dirs.iter().cloned())
        .collect::<Vec<_>>();
    if let Some(linked) = dependency_dirs.first().cloned() {
        link_dependency(&linked, &path)?;
    } else if let Some(parent) = path.parent() {
        dependency_dirs.push(parent.to_path_buf());
    }
    let mut seen_dependency_dirs = std::collections::BTreeSet::new();
    dependency_dirs.retain(|path| seen_dependency_dirs.insert(path.clone()));
    let fingerprint = artifact_fingerprint(&path)?;
    Ok(ForwarderLibrary {
        crate_name,
        path,
        modules,
        commonjs_modules,
        is_dispatcher: true,
        fingerprint,
        dependency_dirs,
    })
}

fn module_dispatcher_source(crate_name: &str, children: &[ForwarderLibrary]) -> String {
    use std::fmt::Write as _;

    let mut source = String::from(
        "#![no_std]\nextern crate alloc;\nextern crate nativeboat_native;\n\
         use ::alloc::vec::Vec;\n\
         use ::nativeboat_native::{binding, ErrorKind, Object, Realm, Thrown, Value};\n",
    );
    for child in children {
        let _ = writeln!(source, "extern crate {};", child.crate_name);
    }
    let _ = writeln!(
        source,
        "pub fn evaluate_es(module: usize, realm: &mut Realm, cells: &::alloc::rc::Rc<Vec<binding::Binding>>, cache: &::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>) -> Result<(), Thrown> {{\nlet _ = (&cells, &cache);\nmatch module {{"
    );
    for child in children {
        for module in &child.modules {
            let call = if child.is_dispatcher {
                format!(
                    "{}::evaluate_es({module}, realm, cells, cache)",
                    child.crate_name
                )
            } else {
                format!("{}::m{module}(realm, cells, cache)", child.crate_name)
            };
            let _ = writeln!(source, "{module} => {call},");
        }
    }
    let _ = writeln!(
        source,
        "_ => Err(realm.throw(ErrorKind::Range, {crate_name:?})),\n}}\n}}"
    );
    let _ = writeln!(
        source,
        "pub fn evaluate_commonjs(module: usize, realm: &mut Realm, cells: &::alloc::rc::Rc<Vec<binding::Binding>>, cache: &::alloc::rc::Rc<Vec<::core::cell::RefCell<Option<Object>>>>) -> Result<Value, Thrown> {{\nlet _ = (&cells, &cache);\nmatch module {{"
    );
    for child in children {
        for module in &child.commonjs_modules {
            let call = if child.is_dispatcher {
                format!(
                    "{}::evaluate_commonjs({module}, realm, cells, cache)",
                    child.crate_name
                )
            } else {
                format!(
                    "{}::require_c{module}(realm, cells, cache)",
                    child.crate_name
                )
            };
            let _ = writeln!(source, "{module} => {call},");
        }
    }
    let _ = writeln!(
        source,
        "_ => Err(realm.throw(ErrorKind::Range, {crate_name:?})),\n}}\n}}"
    );
    source
}

/// Chunk crates import what the compiler's own output does, so names its
/// emitter uses resolve in every crate.
fn dynamic_graph_prelude() -> String {
    format!(
        "#![no_std]\nextern crate alloc;\nextern crate nativeboat_native;\n{}",
        nativeboat_compiler::GENERATED_IMPORTS
    )
}

/// The largest Rust a chunk crate gets, unless one module part is larger.
const CHUNK_BYTES: usize = 2 * 1024 * 1024;

/// The parts of a graph's modules, as the units chunks are made of.
struct ModuleUnits<'a> {
    /// Each unit's Rust: the modules' parts in order.
    sources: Vec<&'a str>,
    /// Each unit's module.
    modules: Vec<usize>,
    /// Each module's first unit; its part `n` is unit `first + n`.
    first: Vec<usize>,
    /// Each module's last unit, which defines its functions.
    last: Vec<usize>,
}

impl<'a> ModuleUnits<'a> {
    fn of(parts: &'a [Vec<String>]) -> ModuleUnits<'a> {
        let mut units = ModuleUnits {
            sources: Vec::new(),
            modules: Vec::new(),
            first: Vec::with_capacity(parts.len()),
            last: Vec::with_capacity(parts.len()),
        };
        for (module, module_parts) in parts.iter().enumerate() {
            units.first.push(units.sources.len());
            for part in module_parts {
                units.sources.push(part);
                units.modules.push(module);
            }
            units.last.push(units.sources.len() - 1);
        }
        units
    }

    /// The unit a reference names: `m<module>(` or `require_c<module>(` the
    /// module's last part, `P<module>_<part>_` that part.
    fn named(&self, reference: &str) -> Option<usize> {
        let (prefix, rest) = ["require_c", "m", "P"]
            .into_iter()
            .find_map(|prefix| reference.strip_prefix(prefix).map(|rest| (prefix, rest)))?;
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let module = rest[..digits].parse::<usize>().ok()?;
        let rest = &rest[digits..];
        if prefix != "P" {
            return (rest.starts_with('(') && module < self.last.len()).then(|| self.last[module]);
        }
        let rest = rest.strip_prefix('_')?;
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let part = rest[..digits].parse::<usize>().ok()?;
        let unit = self.first.get(module)? + part;
        (rest[digits..].starts_with('_') && unit <= self.last[module]).then_some(unit)
    }
}

fn module_chunk_map(
    units: &ModuleUnits<'_>,
    limit: usize,
) -> (Vec<Vec<usize>>, Vec<Option<usize>>) {
    let edges = (0..units.sources.len())
        .map(|unit| unit_targets(units, unit))
        .collect::<Vec<_>>();
    let components = strongly_connected_components(&edges);
    let mut chunks: Vec<Vec<usize>> = Vec::new();
    let mut bytes = 0usize;
    for component in components {
        let component_bytes = component
            .iter()
            .map(|index| units.sources[*index].len())
            .sum::<usize>();
        if component_bytes == 0 {
            continue;
        }
        if chunks.is_empty()
            || (!chunks.last().unwrap().is_empty() && bytes + component_bytes > limit)
        {
            chunks.push(Vec::new());
            bytes = 0;
        }
        chunks.last_mut().unwrap().extend(component.iter().copied());
        bytes += component_bytes;
    }
    let mut unit_chunk = vec![None; units.sources.len()];
    for (chunk, members) in chunks.iter().enumerate() {
        for unit in members {
            unit_chunk[*unit] = Some(chunk);
        }
    }
    (chunks, unit_chunk)
}

/// The other units a unit's Rust names.
fn unit_targets(units: &ModuleUnits<'_>, unit: usize) -> Vec<usize> {
    let source = units.sources[unit];
    let mut targets = std::collections::BTreeSet::new();
    for prefix in ["require_c", "m", "P"] {
        let mut remaining = source;
        while let Some(position) = remaining.find(prefix) {
            if let Some(target) = units.named(&remaining[position..])
                && target != unit
            {
                targets.insert(target);
            }
            remaining = &remaining[position + prefix.len()..];
        }
    }
    targets.into_iter().collect()
}

fn strongly_connected_components(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct Search<'a> {
        edges: &'a [Vec<usize>],
        next: usize,
        indexes: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        active: Vec<bool>,
        components: Vec<Vec<usize>>,
    }
    fn visit(node: usize, search: &mut Search<'_>) {
        let index = search.next;
        search.next += 1;
        search.indexes[node] = Some(index);
        search.low[node] = index;
        search.stack.push(node);
        search.active[node] = true;
        for target in &search.edges[node] {
            if search.indexes[*target].is_none() {
                visit(*target, search);
                search.low[node] = search.low[node].min(search.low[*target]);
            } else if search.active[*target] {
                search.low[node] = search.low[node].min(search.indexes[*target].unwrap());
            }
        }
        if search.low[node] == index {
            let mut component = Vec::new();
            loop {
                let member = search.stack.pop().unwrap();
                search.active[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            component.sort_unstable();
            search.components.push(component);
        }
    }
    let mut search = Search {
        edges,
        next: 0,
        indexes: vec![None; edges.len()],
        low: vec![0; edges.len()],
        stack: Vec::new(),
        active: vec![false; edges.len()],
        components: Vec::new(),
    };
    for node in 0..edges.len() {
        if search.indexes[node].is_none() {
            visit(node, &mut search);
        }
    }
    search.components
}

fn chunk_dependencies(
    units: &ModuleUnits<'_>,
    unit_chunk: &[Option<usize>],
    chunk_count: usize,
) -> Vec<std::collections::BTreeSet<usize>> {
    let mut chunks = vec![std::collections::BTreeSet::new(); chunk_count];
    for unit in 0..units.sources.len() {
        let Some(chunk) = unit_chunk[unit] else {
            continue;
        };
        for target in unit_targets(units, unit) {
            if let Some(target_chunk) = unit_chunk[target]
                && target_chunk != chunk
            {
                chunks[chunk].insert(target_chunk);
            }
        }
    }
    chunks
}

/// Qualifies a unit's references into units of other chunks (`m<id>(` and
/// `require_c<id>(` calls, `P<id>_<part>_` items of a module's parts) with
/// the owning chunk's crate, in one pass. Rust string literals and line
/// comments are copied unchanged.
fn qualify_chunk_references(
    units: &ModuleUnits<'_>,
    unit: usize,
    chunk: usize,
    unit_chunk: &[Option<usize>],
    chunk_names: &[String],
) -> String {
    let code = units.sources[unit];
    let bytes = code.as_bytes();
    let mut output = String::with_capacity(code.len() + code.len() / 16);
    let mut copied = 0;
    let mut index = 0;
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b':';
    while index < bytes.len() {
        match bytes[index] {
            // A quote character literal is not a string.
            b'\''
                if bytes.get(index + 1) == Some(&b'"') && bytes.get(index + 2) == Some(&b'\'') =>
            {
                index += 3;
            }
            b'"' => {
                // A string literal; raw strings are skipped below.
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            b'r' if (index == 0 || !identifier(bytes[index - 1]))
                && matches!(bytes.get(index + 1), Some(b'#' | b'"')) =>
            {
                let hashes = bytes[index + 1..]
                    .iter()
                    .take_while(|byte| **byte == b'#')
                    .count();
                if bytes.get(index + 1 + hashes) != Some(&b'"') {
                    index += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let body = index + 2 + hashes;
                index = code[body..]
                    .find(&close)
                    .map_or(bytes.len(), |end| body + end + close.len());
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = code[index..]
                    .find('\n')
                    .map_or(bytes.len(), |end| index + end);
            }
            b'm' | b'r' | b'P' if index == 0 || !identifier(bytes[index - 1]) => {
                if let Some(target) = units.named(&code[index..])
                    && target != unit
                    && let Some(target_chunk) = unit_chunk[target]
                    && target_chunk != chunk
                {
                    output.push_str(&code[copied..index]);
                    output.push_str(&chunk_names[target_chunk]);
                    output.push_str("::");
                    copied = index;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    output.push_str(&code[copied..]);
    let module = units.modules[unit];
    if units.last[module] != unit {
        return output;
    }
    // A chunk's own entry points are called from other chunks and dispatchers.
    output
        .replacen(&format!("fn m{module}("), &format!("pub fn m{module}("), 1)
        .replacen(
            &format!("fn require_c{module}("),
            &format!("pub fn require_c{module}("),
            1,
        )
}

/// How many generated crates compile at once: `NATIVEBOAT_JOBS`, or as many
/// as available memory allows at about 1.5 GiB per rustc, up to a third of
/// the processors and at most four.
fn generated_compile_jobs() -> usize {
    std::env::var("NATIVEBOAT_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse::<usize>().ok())
        .filter(|jobs| *jobs > 0)
        .unwrap_or_else(|| {
            let processors = std::thread::available_parallelism().map_or(1, usize::from);
            let memory = (available_memory() / (1536 * 1024 * 1024)) as usize;
            (processors / 3).min(memory).clamp(1, 4)
        })
}

/// Available physical memory in bytes; unknown memory allows one job.
fn available_memory() -> u64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        // SAFETY: the structure is plain integers, for which zero is valid.
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: the structure is writable and its length is set.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return 0;
    }
    status.ullAvailPhys
}

/// Runs `work` for every job once its dependencies have finished, up to
/// `jobs` at a time, preferring `order`. `work` sees every finished result.
/// The first error stops new jobs and is returned once running jobs end.
/// With `keep_going`, jobs that do not depend on a failed one still run, and
/// every error is returned, to find all of a build's failures in one run.
fn run_dependency_jobs<T: Clone + Send>(
    order: &[usize],
    dependencies: &[std::collections::BTreeSet<usize>],
    jobs: usize,
    keep_going: bool,
    work: impl Fn(usize, &[Option<T>]) -> Result<T, String> + Sync,
) -> Result<Vec<T>, String> {
    struct State<T> {
        done: Vec<Option<T>>,
        started: Vec<bool>,
        running: usize,
        errors: Vec<String>,
    }
    let count = dependencies.len();
    let state = std::sync::Mutex::new(State {
        done: vec![None; count],
        started: vec![false; count],
        running: 0,
        errors: Vec::new(),
    });
    let changed = std::sync::Condvar::new();
    let worker = || {
        let mut guard = state.lock().expect("job state lock");
        loop {
            if !keep_going && !guard.errors.is_empty() {
                return;
            }
            let ready = order.iter().copied().find(|job| {
                !guard.started[*job]
                    && dependencies[*job]
                        .iter()
                        .all(|dependency| guard.done[*dependency].is_some())
            });
            let Some(job) = ready else {
                // Jobs left unstarted depend on a failed one.
                if guard.running == 0 {
                    return;
                }
                guard = changed.wait(guard).expect("job state lock");
                continue;
            };
            guard.started[job] = true;
            guard.running += 1;
            let done = guard.done.clone();
            drop(guard);
            let result = work(job, &done);
            guard = state.lock().expect("job state lock");
            guard.running -= 1;
            match result {
                Ok(value) => guard.done[job] = Some(value),
                Err(error) => guard.errors.push(error),
            }
            changed.notify_all();
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..jobs.min(count).max(1) {
            scope.spawn(worker);
        }
        worker();
    });
    let state = state.into_inner().expect("job state lock");
    if !state.errors.is_empty() {
        let mut message = state.errors.join("\n\n");
        let skipped = state.started.iter().filter(|started| !**started).count();
        if keep_going {
            message.push_str(&format!(
                "\n\n{} jobs failed; {skipped} jobs depending on them did not run",
                state.errors.len()
            ));
        }
        return Err(message);
    }
    Ok(state
        .done
        .into_iter()
        .map(|value| value.expect("every job finished"))
        .collect())
}

fn chunk_compile_order(dependencies: &[std::collections::BTreeSet<usize>]) -> Vec<usize> {
    fn visit(
        chunk: usize,
        dependencies: &[std::collections::BTreeSet<usize>],
        seen: &mut [bool],
        order: &mut Vec<usize>,
    ) {
        if seen[chunk] {
            return;
        }
        seen[chunk] = true;
        for dependency in &dependencies[chunk] {
            visit(*dependency, dependencies, seen, order);
        }
        order.push(chunk);
    }
    let mut seen = vec![false; dependencies.len()];
    let mut order = Vec::with_capacity(dependencies.len());
    for chunk in 0..dependencies.len() {
        visit(chunk, dependencies, &mut seen, &mut order);
    }
    order
}

fn linked_entry_source(
    entry: &str,
    dispatcher: &str,
    modules: &[usize],
    commonjs_modules: &[usize],
    entry_count: usize,
    scripts: &[(String, usize)],
    stylesheets: &std::collections::BTreeMap<String, String>,
) -> String {
    use std::fmt::Write as _;
    let mut entry = entry.to_owned();
    for module in commonjs_modules {
        entry = entry.replace(
            &format!("require_c{module}("),
            &format!("{dispatcher}::evaluate_commonjs({module}, "),
        );
    }
    for module in modules {
        entry = entry.replace(
            &format!("m{module}("),
            &format!("{dispatcher}::evaluate_es({module}, "),
        );
    }
    let mut source = format!("extern crate {dispatcher};\n");
    source.push_str(&entry);
    source.push_str("\n#[allow(dead_code)] pub fn initialize(realm: &mut Realm) -> Result<Vec<Object>, Thrown> { ");
    for (identifier, stylesheet) in stylesheets {
        let _ = write!(
            source,
            "realm.register_native_stylesheet({identifier:?}, {stylesheet:?}); "
        );
    }
    source.push_str(&script_registration(scripts));
    let entries = (0..entry_count)
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(source, "module_entries(realm, &[{entries}]) }}");
    // One root of the graph by itself, as a forked child process runs it.
    source.push_str(
        "#[allow(dead_code)] pub fn initialize_entry(realm: &mut Realm, entry: usize) -> Result<Vec<Object>, Thrown> { module_entries(realm, &[entry]) }\n",
    );
    source
}

fn generated_rlib_arguments(
    dependencies: &[&Path],
    externs: &[OsString],
    crate_name: &str,
    source_name: &str,
    output: &Path,
    opt_level: &str,
) -> Vec<OsString> {
    let mut native = OsString::from("--extern=nativeboat_native=");
    native.push(dependencies[0].join("libnativeboat_native.rlib"));
    let mut arguments = vec![
        source_name.into(),
        "--edition=2024".into(),
        format!("--crate-name={crate_name}").into(),
        "--crate-type=rlib".into(),
        "-Cpanic=abort".into(),
        format!("-Copt-level={opt_level}").into(),
        "-Ccodegen-units=1".into(),
        "-Cembed-bitcode=no".into(),
        "-Dwarnings".into(),
        native,
    ];
    arguments.extend(externs.iter().cloned());
    arguments.extend(dependencies.iter().map(|path| {
        let mut argument = OsString::from("-Ldependency=");
        argument.push(path);
        argument
    }));
    arguments.extend(["-o".into(), output.as_os_str().to_owned()]);
    arguments
}

fn compile_generated_rlib(
    rustc: &Toolchain,
    staging: &Path,
    dependencies: &[&Path],
    generated: &GeneratedRlib<'_>,
    output: &Path,
) -> Result<(), String> {
    let opt_level = if generated.application {
        generated_opt_level()
    } else {
        "0".to_owned()
    };
    let mut arguments = generated_rlib_arguments(
        dependencies,
        generated.externs,
        generated.crate_name,
        generated.source_name,
        output,
        &opt_level,
    );
    let mut staging_dependencies = OsString::from("-Ldependency=");
    staging_dependencies.push(staging);
    arguments.push(staging_dependencies);
    arguments.push(remap_staging(staging, "nativeboat-generated"));
    toolchain(rustc, &arguments, staging)?;
    let source_name = generated.source_name;
    std::fs::remove_file(staging.join(source_name))
        .map_err(|error| format!("cannot remove compiled source {source_name}: {error}"))
}

/// Compiles a generated crate once for a source/toolchain/dependency identity.
/// Renderer chunks are expensive to compile, so retaining their rlibs avoids
/// recompiling unchanged chunks on every application build.
fn compile_generated_rlib_cached(
    rustc: &Toolchain,
    cache: &Path,
    fallback_staging: &Path,
    dependencies: &[&Path],
    generated: GeneratedRlib<'_>,
) -> Result<(PathBuf, bool), String> {
    let owner = generated_owner(fallback_staging);
    let scope = generated_cache_scope(fallback_staging);
    let cache_name = format!(
        "generated_{scope:016x}_{}-{:016x}",
        generated.crate_name, generated.fingerprint
    );
    let library = format!("lib{}.rlib", generated.crate_name);
    let cache_hit = cache.join(&cache_name).join(&library).is_file();
    let (directory, fallback) = cached(cache, &cache_name, &library, |build| {
        write_file(
            &build.join("owner-directory.txt"),
            owner.to_string_lossy().as_bytes(),
        )
        .map_err(|error| format!("cannot write generated cache owner: {error}"))?;
        write_file(
            &build.join(generated.source_name),
            generated.source.as_bytes(),
        )
        .map_err(|error| format!("cannot write generated {}: {error}", generated.crate_name))?;
        compile_generated_rlib(
            rustc,
            build,
            dependencies,
            &generated,
            &build.join(&library),
        )
    })?;
    let output = directory.join(&library);
    if let Some(fallback) = fallback {
        let local = fallback_staging.join(&library);
        std::fs::copy(&output, &local).map_err(|error| {
            format!(
                "cannot copy generated {} into the build: {error}",
                generated.crate_name
            )
        })?;
        drop(fallback);
        Ok((local, cache_hit))
    } else {
        Ok((output, cache_hit))
    }
}

/// The directory owning a build's generated crates: its staging directory's
/// parent, spelled canonically so that `C:/out` and `C:\out` share entries.
fn generated_owner(staging: &Path) -> PathBuf {
    let owner = staging.parent().unwrap_or(staging);
    std::fs::canonicalize(owner).unwrap_or_else(|_| owner.to_path_buf())
}

fn generated_cache_scope(staging: &Path) -> u64 {
    fnv(
        0xcbf2_9ce4_8422_2325,
        generated_owner(staging).as_os_str().as_encoded_bytes(),
    )
}

/// Removes generated crates that are superseded but that pruning by kind
/// keeps: entries named before they were scoped by owner, once a scoped
/// entry of the same crate exists, and entries that one owner left under
/// several spellings of its directory, of which only the newest of each
/// crate stays.
fn prune_generated_aliases(cache: &Path) {
    let mut newest: HashMap<(String, PathBuf), (std::time::SystemTime, PathBuf)> = HashMap::new();
    let mut unscoped = Vec::new();
    let mut scoped_crates = HashSet::new();
    for entry in std::fs::read_dir(cache).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("generated_") else {
            continue;
        };
        let crate_of = |rest: &str| {
            rest.split_once('-')
                .map_or(rest, |(crate_name, _)| crate_name)
                .to_owned()
        };
        let scoped = rest.get(16..17) == Some("_")
            && rest[..16].bytes().all(|byte| byte.is_ascii_hexdigit());
        if !scoped {
            unscoped.push((crate_of(rest), entry.path()));
            continue;
        }
        let crate_name = crate_of(&rest[17..]);
        scoped_crates.insert(crate_name.clone());
        let Some(owner) = std::fs::read_to_string(entry.path().join("owner-directory.txt"))
            .ok()
            .and_then(|owner| std::fs::canonicalize(owner.trim()).ok())
        else {
            continue;
        };
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        let path = entry.path();
        match newest.entry((crate_name, owner)) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert((modified, path));
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let older = if modified > slot.get().0 {
                    slot.insert((modified, path)).1
                } else {
                    path
                };
                let _ = std::fs::remove_dir_all(older);
            }
        }
    }
    for (crate_name, path) in unscoped {
        if scoped_crates.contains(&crate_name) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn generated_dependency_fingerprint(mut fingerprint: u64, artifacts: &[u64]) -> u64 {
    for artifact in artifacts {
        fingerprint = fnv(fingerprint, &artifact.to_le_bytes());
    }
    fingerprint
}

/// The optimization level of the crates compiled from application code:
/// `NATIVEBOAT_GENERATED_OPT_LEVEL`, else `s`. Optimizing for size halves
/// generated code against level 0 and runs as fast as level 2.
fn generated_opt_level() -> String {
    std::env::var("NATIVEBOAT_GENERATED_OPT_LEVEL").unwrap_or_else(|_| "s".into())
}

/// The cache key of a generated crate. `toolchain_key` holds the runtime's
/// content, the only library generated crates link besides each other.
fn generated_rlib_fingerprint(
    toolchain_key: u64,
    crate_name: &str,
    source: &str,
    generated_dependency_keys: &[u64],
) -> u64 {
    let mut fingerprint = fnv(0xcbf2_9ce4_8422_2325, &toolchain_key.to_le_bytes());
    fingerprint = fnv(fingerprint, crate_name.as_bytes());
    fingerprint = fnv(
        fingerprint,
        b"edition=2024;panic=abort;codegen-units=1;cache-v1",
    );
    fingerprint = fnv(fingerprint, generated_opt_level().as_bytes());
    fingerprint = fnv(fingerprint, source.as_bytes());
    for dependency in generated_dependency_keys {
        fingerprint = fnv(fingerprint, &dependency.to_le_bytes());
    }
    fingerprint
}

fn context_library_source(
    kind: &str,
    index: usize,
    context: &CompiledContext,
    stylesheets: &std::collections::BTreeMap<String, String>,
) -> String {
    use std::fmt::Write as _;
    let entries = (0..context.entry_count)
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let mut source =
        String::from("#![no_std]\nextern crate alloc;\nextern crate nativeboat_native;\n");
    source.push_str(context.module.rust_source());
    source.push_str("\n#[allow(dead_code)] pub fn initialize(realm: &mut Realm) -> Result<Vec<Object>, Thrown> { ");
    for (identifier, stylesheet) in stylesheets {
        let _ = write!(
            source,
            "realm.register_native_stylesheet({identifier:?}, {stylesheet:?}); "
        );
    }
    let _ = write!(
        source,
        "module_entries(realm, &[{entries}]) }}\n// {kind} context {index}: {:?}\n",
        context.entry
    );
    source
}

/// Copies configured Electron `extraFiles` and `extraResources` beside the
/// generated executable. The project loader has already expanded directory
/// globs, so each entry must be a regular file with a relative destination.
/// Plugin modules are compiled into the executable. A file at each plugin's
/// emitted main path lets the program find and resolve it as it would the
/// file its package's build makes, and the other files of a TypeScript
/// plugin's source folder (scripts, data) go to the folder it emits to.
fn write_plugin_files(
    project: &Project,
    compilation: &NativeCompilation,
    output: &Path,
) -> Result<(), String> {
    // Plugin mains, and the modules (such as an installed package's) that
    // import what the program serves, have files at their paths, which
    // programs resolve the real paths of.
    let host_importers = compilation
        .main
        .iter()
        .flat_map(|module| module.host_importers().iter());
    let mains = project
        .plugins
        .iter()
        .filter_map(|plugin| plugin.runtime_paths.first());
    for path in mains.chain(host_importers) {
        let file = output.join(path);
        if !file.exists() {
            std::fs::create_dir_all(file.parent().unwrap_or(output))
                .and_then(|()| {
                    std::fs::write(&file, "// Compiled into the application's executable.\n")
                })
                .map_err(|error| format!("cannot write {}: {error}", file.display()))?;
        }
    }
    fn copy_folder(source: &Path, target: &Path) -> Result<(), String> {
        let Ok(listing) = std::fs::read_dir(source) else {
            return Ok(());
        };
        for entry in listing.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if path.is_dir() {
                copy_folder(&path, &target.join(entry.file_name()))?;
                continue;
            }
            // Each module gets a file at its emitted path, which programs
            // resolve real paths of (an editor finds a module's extension
            // by its path); declarations emit nothing.
            let emitted = [("tsx", "js"), ("ts", "js"), ("mts", "mjs"), ("cts", "cjs")]
                .iter()
                .find(|(source, _)| name.ends_with(&format!(".{source}")));
            let destination = match emitted {
                Some(_) if name.ends_with(".d.ts") || name.ends_with(".d.mts") => continue,
                Some((_, extension)) => target.join(
                    path.with_extension(extension)
                        .file_name()
                        .unwrap_or_default(),
                ),
                None => target.join(entry.file_name()),
            };
            if destination.exists() {
                continue;
            }
            std::fs::create_dir_all(target)
                .and_then(|()| match emitted {
                    Some(_) => std::fs::write(
                        &destination,
                        "// Compiled into the application's executable.\n",
                    ),
                    None => std::fs::copy(&path, &destination).map(drop),
                })
                .map_err(|error| format!("cannot write {}: {error}", destination.display()))?;
        }
        Ok(())
    }
    for (source, emitted) in emitted_folders(&project.root, &project.plugins) {
        copy_folder(&source, &output.join(emitted))?;
    }
    Ok(())
}

fn copy_extra_files(
    files: &[nativeboat_project::builder::ExtraFile],
    output: &Path,
) -> Result<(), String> {
    for file in files {
        let relative = Path::new(&file.destination);
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "invalid native resource destination '{}'",
                file.destination
            ));
        }
        let metadata = std::fs::metadata(&file.source).map_err(|error| {
            format!(
                "cannot read native resource {}: {error}",
                file.source.display()
            )
        })?;
        if !metadata.is_file() {
            return Err(format!(
                "native resource {} is not a regular file",
                file.source.display()
            ));
        }
        let destination = output.join(relative);
        let parent = destination
            .parent()
            .ok_or_else(|| format!("invalid native resource destination '{}'", file.destination))?;
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create native resource directory {}: {error}",
                parent.display()
            )
        })?;
        std::fs::copy(&file.source, &destination).map_err(|error| {
            format!(
                "cannot copy native resource {} to {}: {error}",
                file.source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn application_source(
    project: &Project,
    compilation: &NativeCompilation,
    has_node_library: bool,
    has_renderer_layout: bool,
) -> Result<String, String> {
    let main = compilation
        .main
        .as_ref()
        .ok_or_else(|| "main module did not compile".to_owned())?;
    let run_host = main.native_dependencies().contains("electron");
    let has_native_keymap = main.native_dependencies().contains("native-keymap");
    let (preload_declarations, preload_opener) =
        preload_host_source(project, compilation, has_node_library);
    let (renderer_declarations, renderer_opener, renderer_after) =
        renderer_host_source(project, compilation, has_node_library, has_renderer_layout);
    // Windows get their preloads and pages when they load them: at startup,
    // and in the host loop for windows opened later.
    let window_host = format!(
        "{preload_declarations}{renderer_declarations}macro_rules! _nativeboat_open_windows {{ () => {{ {preload_opener}{renderer_opener} }} }}\n_nativeboat_open_windows!();\n{renderer_after}"
    );
    let mut context_crates =
        String::from("extern crate nativeboat_main_0 as nativeboat_main_graph;\n");
    for index in 0..compilation.preloads.len() {
        use std::fmt::Write as _;
        let _ = writeln!(
            context_crates,
            "extern crate nativeboat_preload_{index} as preload_{index};"
        );
    }
    for index in 0..compilation.renderers.len() {
        use std::fmt::Write as _;
        let _ = writeln!(
            context_crates,
            "extern crate nativeboat_renderer_{index} as renderer_{index};"
        );
    }
    if has_renderer_layout && !compilation.renderers.is_empty() {
        context_crates.push_str("extern crate nativeboat_page;\nextern crate nativeboat_win32;\n");
    }
    let mut stylesheet_registration = String::new();
    for (identifier, stylesheet) in &compilation.stylesheets {
        use std::fmt::Write as _;
        let _ = write!(
            stylesheet_registration,
            "realm.register_native_stylesheet({identifier:?}, {stylesheet:?}); "
        );
    }
    // Forked modules: the paths each is started by, which `fork()` matches,
    // and the branch a child process started as one takes instead of the
    // main module.
    {
        use std::fmt::Write as _;
        let tables = compilation
            .forks
            .iter()
            .map(|paths| {
                let paths = paths
                    .iter()
                    .map(|path| format!("{path:?}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("&[{paths}]")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            stylesheet_registration,
            "::nativeboat_native::fork::register(&[{tables}]); "
        );
        // Forked modules and plugins load in this process too, by their
        // paths, when the program requires or imports them.
        let scripts = compilation
            .forks
            .iter()
            .enumerate()
            .flat_map(|(fork, paths)| {
                paths
                    .iter()
                    .map(move |path| format!("({path:?}, {})", 1 + fork))
            })
            .collect::<Vec<_>>();
        if !scripts.is_empty() && compilation.main.is_some() {
            let _ = write!(
                stylesheet_registration,
                "realm.register_scripts(&[{}]); ",
                scripts.join(", ")
            );
        }
        if !compilation.forks.is_empty() && compilation.main.is_some() {
            stylesheet_registration.push_str(
                "if let Some(__fork) = ::nativeboat_native::fork::requested_entry() { return ::nativeboat_native::fork::run(&mut realm, 1 + __fork, ::nativeboat_main_graph::initialize_entry); } ",
            );
        }
    }
    Ok(format!(
        "#![no_std]\n#![no_main]\nextern crate alloc;\nextern crate nativeboat_start;\n{}\n\
         #[allow(unused_imports)] use ::nativeboat_native::{{Realm, Value}};\n#[unsafe(no_mangle)]\npub fn nativeboat_main() -> i32 {{ let mut realm = Realm::new(); realm.set_global(\"__nativeboat__\", Value::Boolean(true)); {}{}{}{}{}#[allow(unused_mut)] let mut status = {}; {}{}{} }}\n",
        context_crates,
        format_args!(
            "{}{}",
            electron_version(&project.root)
                .map(|version| format!("realm.electron_version = Some({version:?}); "))
                .unwrap_or_default(),
            if project.metadata.locales.is_empty() {
                String::new()
            } else {
                format!(
                    "::nativeboat_native::platform::keep_locales(&{:?}); ",
                    project.metadata.locales
                )
            }
        ),
        format_args!(
            "realm.electron_app_version = Some({:?}); ",
            project.metadata.version
        ),
        format_args!(
            "realm.electron_app_name = Some({:?}); {}",
            project.metadata.app_name,
            if compilation.renderers.is_empty() {
                ""
            } else {
                "realm.electron_hosts_pages = true; "
            }
        ),
        if has_node_library {
            "realm.node_library = Some(::nativeboat_node::require); "
        } else {
            ""
        },
        stylesheet_registration,
        if run_host {
            "realm.start_host(::nativeboat_main_graph::initialize).unwrap_or(0)"
        } else {
            "realm.run_main(::nativeboat_main_graph::initialize)"
        },
        window_host,
        "",
        if run_host {
            format!(
                "if status == 0 {{ ::nativeboat_native::app_host::set_menu_command_handler((&mut realm as *mut Realm).cast(), Some(::nativeboat_native::electron_menu::dispatch_menu_command)); {}::nativeboat_native::app_host::set_window_event_handler((&mut realm as *mut Realm).cast(), Some(::nativeboat_native::electron_window::window_event)); ::nativeboat_native::app_host::set_notify_handler((&mut realm as *mut Realm).cast(), Some(::nativeboat_native::electron_notify::notify_event)); {}let host = ::nativeboat_native::app_host::run_host_loop(|| {{ let mut signals = ::alloc::vec::Vec::new(); let mut wait = realm.step_host()?; _nativeboat_open_windows!(); if status != 0 {{ return Err(status); }} {} {} // Another realm may have queued jobs here after this one's step.
 // Realms call each other: one may have scheduled work in another after that one's step.
 let wake = [realm.next_wake()].into_iter().chain(_nativeboat_preload_realms.iter().map(|(_, preload)| preload.next_wake())){}.flatten().min(); wait.delay = match (wait.delay, wake) {{ (Some(delay), Some(wake)) => Some(delay.min(wake)), (delay, wake) => delay.or(wake) }};
 let busy = realm.has_queued_jobs() || realm.has_pending_electron_windows() || _nativeboat_preload_realms.iter().any(|(window, preload)| preload.has_queued_jobs() || realm.has_electron_messages(*window)){}; signals.extend(wait.signal); // Electron runs until it quits; with no window and nothing pending nothing could make it quit.
 if !busy && ::nativeboat_native::app_host::window_count() == 0 && !realm.keeps_alive() {{ ::nativeboat_native::app_host::request_quit(0); return Ok((Some(0), signals)); }} Ok((if busy {{ Some(0) }} else {{ wait.delay }}, signals)) }}); let finished = realm.finish_host(); match host {{ Ok(0) => finished, Ok(code) => code, Err(_) => 1 }} }} else {{ status }}",
                format_args!(
                    "{}{}",
                    if has_native_keymap {
                        "::nativeboat_native::app_host::set_keyboard_layout_change_handler((&mut realm as *mut Realm).cast(), Some(::nativeboat_native::keyboard::layout_changed)); "
                    } else {
                        ""
                    },
                    // Pages' keys reach `before-input-event` first.
                    if has_renderer_layout && !compilation.renderers.is_empty() {
                        "::nativeboat_native::app_host::set_key_filter((&mut realm as *mut Realm).cast(), Some(::nativeboat_page::electron_before_input)); "
                    } else {
                        ""
                    }
                ),
                if has_renderer_layout && !compilation.renderers.is_empty() {
                    "let mut _nativeboat_next_frame = 0.0_f64; let mut _nativeboat_frame_failures = ::alloc::collections::BTreeSet::new(); "
                } else {
                    ""
                },
                "for (window, preload) in _nativeboat_preload_realms.iter_mut() { realm.deliver_electron_messages(*window, preload); wait = wait.merge(preload.step_host()?, &mut signals); }",
                if compilation.renderers.is_empty() {
                    String::new()
                } else if has_renderer_layout {
                    format!("{}{}", renderer_frame_step_source(), PAGE_LOAD_SOURCE)
                } else {
                    format!("for renderer in _nativeboat_renderer_realms.iter_mut() {{ wait = wait.merge(renderer.1.step_host()?, &mut signals); if let Some((_, preload)) = _nativeboat_preload_realms.iter_mut().find(|(window, _)| *window == renderer.0) {{ realm.deliver_electron_messages(renderer.0, preload); wait = wait.merge(preload.step_host()?, &mut signals); }} realm.deliver_electron_messages_to_page(renderer.0, &mut renderer.1); wait = wait.merge(renderer.1.step_host()?, &mut signals); }} for (_, preload) in _nativeboat_preload_realms.iter_mut() {{ preload.run_deferred_ticks(); }}{PAGE_LOAD_SOURCE}")
                },
                if compilation.renderers.is_empty() {
                    ""
                } else {
                    ".chain(_nativeboat_renderer_realms.iter().map(|renderer| renderer.1.next_wake()))"
                },
                if compilation.renderers.is_empty() {
                    ""
                } else {
                    " || _nativeboat_renderer_realms.iter().any(|renderer| renderer.1.has_queued_jobs() || realm.has_electron_messages(renderer.0)) || !_nativeboat_loading.is_empty()"
                }
            )
        } else {
            "status".to_owned()
        },
    ))
}

/// Reports pages loaded once their first tasks have run: when no jobs are
/// queued and no timer is due within a few milliseconds, as the timers
/// scripts set without a delay while starting are, or a tenth of a second
/// after they started. As in Chromium, messages the main process sends on
/// `did-finish-load` then find the listeners pages set up while starting.
const PAGE_LOAD_SOURCE: &str = " let now = ::nativeboat_native::platform::now(); _nativeboat_loading.retain(|(window, started)| match _nativeboat_renderer_realms.iter().find(|renderer| renderer.0 == *window) { Some(renderer) if now - *started < 100.0 && renderer.1.has_imminent_work(4) => true, Some(_) => { realm.electron_page_loaded(*window); false } None => false });";

/// Pending frame callbacks keep the GUI loop awake until the next frame;
/// callback-created microtasks drain before measuring the next wait.
fn renderer_frame_step_source() -> &'static str {
    r#"
let now = ::nativeboat_native::platform::now();
let frame_due = now >= _nativeboat_next_frame;
if frame_due { _nativeboat_next_frame = now + 16.0; }
for renderer in _nativeboat_renderer_realms.iter_mut() {
    let page_window = _nativeboat_page_windows.iter().find(|(window, _)| *window == renderer.0).map(|(_, page_window)| page_window.clone());
    if let Some(page_window) = &page_window { page_window.enter(); }
    let outcome = (|| -> Result<(), i32> {
    let page = &mut renderer.1;
    // Tasks already due run before messages that arrived after them, as in
    // Chromium's task queue: a timer a key press set runs before the
    // message of a menu shortcut that followed it.
    wait = wait.merge(page.step_host()?, &mut signals);
    // Messages those tasks made the main process send reach the preload's
    // listeners first, as earlier ones did: a context-isolated page
    // listens through its preload.
    if let Some((_, preload)) = _nativeboat_preload_realms.iter_mut().find(|(window, _)| *window == renderer.0) {
        realm.deliver_electron_messages(renderer.0, preload);
        wait = wait.merge(preload.step_host()?, &mut signals);
    }
    realm.deliver_electron_messages_to_page(renderer.0, page);
    wait = wait.merge(page.step_host()?, &mut signals);
    if let Some((_, view, _)) = _nativeboat_native_pages.iter().find(|(window, _, _)| *window == renderer.0) {
        let viewport = ::nativeboat_native::app_host::client_size(::nativeboat_native::app_host::Window::from_raw(renderer.0)).unwrap_or((800.0, 600.0));
        view.set_viewport(viewport.0, viewport.1);
        view.extend_stylesheets(page.take_native_stylesheets());
    }
    if _nativeboat_frame_failures.contains(&renderer.0) { return Ok(()); }
    let pending = match page.web_call("hasFrames", &[]) {
        Ok(value) => ::nativeboat_native::ops::to_boolean(&value),
        Err(thrown) => {
            _nativeboat_frame_failures.insert(renderer.0);
            page.queue_host_error(thrown);
            wait = wait.merge(page.step_host()?, &mut signals);
            false
        }
    };
    if pending && frame_due {
        if let Err(thrown) = page.web_call("frame", &[]) { page.queue_host_error(thrown); }
        wait = wait.merge(page.step_host()?, &mut signals);
        if let Some((_, view, _)) = _nativeboat_native_pages.iter().find(|(window, _, _)| *window == renderer.0) {
            view.extend_stylesheets(page.take_native_stylesheets());
        }
    }
    let pending = if pending && frame_due {
        match page.web_call("hasFrames", &[]) {
            Ok(value) => ::nativeboat_native::ops::to_boolean(&value),
            Err(thrown) => {
                _nativeboat_frame_failures.insert(renderer.0);
                page.queue_host_error(thrown);
                wait = wait.merge(page.step_host()?, &mut signals);
                false
            }
        }
    } else { pending };
    if pending {
        let delay = ((_nativeboat_next_frame - ::nativeboat_native::platform::now()).max(0.0) as u32).saturating_add(1);
        wait.delay = Some(wait.delay.map_or(delay, |other| other.min(delay)));
    }
    Ok(())
    })();
    if let Some(page_window) = &page_window { page_window.leave(); }
    outcome?;
}
// Pages render at most once a frame; a change between frames wakes the
// host for the next one.
for (_, page_window) in &_nativeboat_page_windows {
    if frame_due {
        page_window.render();
    } else if page_window.needs_render() {
        let delay = ((_nativeboat_next_frame - ::nativeboat_native::platform::now()).max(0.0) as u32).saturating_add(1);
        wait.delay = Some(wait.delay.map_or(delay, |other| other.min(delay)));
    } else if let Some(at) = page_window.wake_at() {
        let delay = ((at - ::nativeboat_native::platform::now()).max(0.0) as u32).saturating_add(1);
        wait.delay = Some(wait.delay.map_or(delay, |other| other.min(delay)));
    }
}
// Ticks pages' code queued in their preloads' Node environments run once
// the pages have rendered, as on Electron's next Node turn.
for (_, preload) in _nativeboat_preload_realms.iter_mut() {
    if frame_due {
        preload.run_deferred_ticks();
    } else if preload.has_deferred_ticks() {
        let delay = ((_nativeboat_next_frame - ::nativeboat_native::platform::now()).max(0.0) as u32).saturating_add(1);
        wait.delay = Some(wait.delay.map_or(delay, |other| other.min(delay)));
    }
}
"#
}

/// Runs each configured preload in a distinct Realm after the main module has
/// yielded. The main Realm remains live for synchronous IPC dispatch, and the
/// preload Realms stay alive for the native window loop. Renderer bridge values
/// are retained on those Realms until document lifecycle wiring consumes them.
fn preload_host_source(
    project: &Project,
    compilation: &NativeCompilation,
    has_node_library: bool,
) -> (String, String) {
    use std::fmt::Write as _;

    let declarations = "let mut _nativeboat_preload_realms: ::alloc::vec::Vec<(usize, ::alloc::boxed::Box<Realm>)> = ::alloc::vec::Vec::new();\n".to_owned();
    if compilation.preloads.is_empty() {
        return (declarations, String::new());
    }
    let mut source = String::from(
        "if status == 0 {\n    for (window, requested) in realm.take_electron_window_preloads() {\n        let requested = requested.replace('\\\\', \"/\").to_ascii_lowercase();\n        let mut matched = false;\n",
    );
    for (index, context) in compilation.preloads.iter().enumerate() {
        let entry = project
            .preloads
            .iter()
            .find(|entry| entry.source == context.entry);
        let mut candidates = Vec::new();
        for extension in ["js", "mjs", "cjs", "ts", "mts", "cts", "jsx", "tsx"] {
            let candidate = context.entry.with_extension(extension);
            if let Ok(relative) = candidate.strip_prefix(&project.root) {
                candidates.push(
                    relative
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
        }
        if let Some(entry) = entry {
            for path in &entry.runtime_paths {
                candidates.push(path.replace('\\', "/").to_ascii_lowercase());
            }
        }
        candidates.sort();
        candidates.dedup();
        let predicate = candidates
            .iter()
            .map(|candidate| format!("requested.ends_with({candidate:?})"))
            .collect::<Vec<_>>()
            .join(" || ");
        let _ = writeln!(
            source,
            "        if !matched && ({predicate}) {{\n            matched = true;\n            let mut preload_realm = ::alloc::boxed::Box::new(Realm::new());\n            preload_realm.electron_version = realm.electron_version;\n            preload_realm.electron_app_name = realm.electron_app_name;\n            preload_realm.electron_app_version = realm.electron_app_version;\n            preload_realm.set_electron_window(window);\n            preload_realm.set_page_arguments(realm.electron_window_arguments(window));\n            {}            preload_realm.install_preload_window();\n            unsafe {{ preload_realm.connect_electron_ipc(&mut realm as *mut Realm); }}\n            let preload_status = preload_realm.start_host(|realm| preload_{index}::initialize(realm).map(drop)).unwrap_or(0);\n            if preload_status != 0 {{ status = preload_status; break; }}\n            _nativeboat_preload_realms.push((window, preload_realm));\n        }}",
            if has_node_library {
                "preload_realm.node_library = Some(::nativeboat_node::require);\n            "
            } else {
                ""
            },
        );
    }
    source.push_str("        if !matched { ::nativeboat_native::platform::write_stderr(&::alloc::format!(\"nativeboat: a window requested the preload '{requested}', which this build did not compile\\n\")); status = 1; break; }\n    }\n}\n");
    (declarations, source)
}

/// Starts a compiled page graph in its own engine-free Realm when a window
/// requests a matching `loadFile`/`loadURL` target. Preload bridge globals are
/// copied into that page before its entry modules execute.
fn renderer_host_source(
    project: &Project,
    compilation: &NativeCompilation,
    has_node_library: bool,
    has_renderer_layout: bool,
) -> (String, String, String) {
    use std::fmt::Write as _;

    if compilation.renderers.is_empty() {
        return (String::new(), String::new(), String::new());
    }
    let mut source = String::from(
        "let mut _nativeboat_renderer_realms: ::alloc::vec::Vec<(usize, ::alloc::boxed::Box<Realm>, ::nativeboat_native::dom_binding::NativeDocument, ::alloc::vec::Vec<::alloc::string::String>)> = ::alloc::vec::Vec::new();\n// Windows whose pages still run their first tasks, with when they started.\nlet mut _nativeboat_loading: ::alloc::vec::Vec<(usize, f64)> = ::alloc::vec::Vec::new();\n",
    );
    if has_renderer_layout {
        source.push_str(
        "fn _nativeboat_measurer(window: usize, electron: Option<&str>) -> ::alloc::rc::Rc<::nativeboat_win32::GdiMeasurer> {\n    let _ = ::nativeboat_win32::init();\n    if electron.is_some() { ::nativeboat_win32::follow_chromium(::nativeboat_native::web::chrome_major(electron)); }\n    ::nativeboat_win32::follow_locale(&::nativeboat_native::platform::user_locale());\n    let measurer = ::nativeboat_win32::GdiMeasurer::new();\n    measurer.set_scale(::nativeboat_native::app_host::scale(::nativeboat_native::app_host::Window::from_raw(window)) as f32);\n    ::alloc::rc::Rc::new(measurer)\n}\n",
        );
        source.push_str(
            r#"unsafe extern "system" fn _nativeboat_paint(window: usize, dc: *mut ::core::ffi::c_void, context: *mut ::core::ffi::c_void) {
    if context.is_null() { return; }
    let pages = unsafe { &*context.cast::<::alloc::vec::Vec<_NativeBoatPage>>() };
    if let Some((_, page, _)) = pages.iter().find(|(attached, _, _)| *attached == window) {
        let host = ::nativeboat_native::app_host::Window::from_raw(window);
        let viewport = ::nativeboat_native::app_host::client_size(host).unwrap_or((800.0, 600.0));
        let scale = ::nativeboat_native::app_host::scale(host) as f32;
        page.set_viewport(viewport.0, viewport.1);
        if let Some(scene) = page.scene() {
            let size = ((viewport.0 * scale + 0.5) as i32, (viewport.1 * scale + 0.5) as i32);
            unsafe { ::nativeboat_win32::paint_tree(dc, &scene.tree, scale, size, ::nativeboat_css::Color::rgb(255, 255, 255)) };
        }
    }
}
unsafe extern "system" fn _nativeboat_drag(context: *mut ::core::ffi::c_void, window: usize, x: f32, y: f32) -> bool {
    if context.is_null() { return false; }
    let pages = unsafe { &*context.cast::<::alloc::vec::Vec<_NativeBoatPage>>() };
    pages.iter().find(|(attached, _, _)| *attached == window).is_some_and(|(_, page, _)| page.drags(x, y))
}
unsafe fn _nativeboat_print(context: *mut ::core::ffi::c_void, window: usize, request: &::nativeboat_native::app_host::PrintRequest) -> Result<::nativeboat_native::app_host::PrintOutcome, ::alloc::string::String> {
    if context.is_null() { return Err("printing is not available".into()); }
    let pages = unsafe { &*context.cast::<::alloc::vec::Vec<_NativeBoatPage>>() };
    match pages.iter().find(|(attached, _, _)| *attached == window) {
        Some((_, page, _)) => ::nativeboat_page::print_page(page, window, request),
        None => Err("the window shows no page".into()),
    }
}
"#,
        );
        source.push_str(
            "type _NativeBoatPage = (usize, ::alloc::rc::Rc<::nativeboat_page::DocumentPage>, ::core::cell::Cell<Option<(u64, u64)>>);\nlet mut _nativeboat_native_pages: ::alloc::vec::Vec<_NativeBoatPage> = ::alloc::vec::Vec::new();\nlet mut _nativeboat_page_windows: ::alloc::vec::Vec<(usize, ::alloc::rc::Rc<::nativeboat_page::PageWindow>)> = ::alloc::vec::Vec::new();\nlet _nativeboat_page_handler = ::nativeboat_page::PageWindows::default();\n",
        );
    }
    if has_renderer_layout {
        source.push_str(&frame_documents_source(project, compilation));
    }
    let declarations = source;
    let mut source = String::new();
    source.push_str(
        "if status == 0 {\n    for (window, requested) in realm.take_electron_window_loads() {\n        let requested_url = requested;\n        let requested = requested_url.split(['?', '#']).next().unwrap_or_default().replace('\\\\', \"/\").to_ascii_lowercase();\n        let mut matched = false;\n",
    );
    for (index, context) in compilation.renderers.iter().enumerate() {
        // Every page the graph serves matches its own URLs and builds its
        // own document around the shared scripts.
        for page_source in &context.pages {
            let document = project
                .pages
                .iter()
                .find(|page| page.source == *page_source)
                .map(renderer_document_spec)
                .unwrap_or_else(|| "&[]".to_owned());
            let mut candidates = Vec::new();
            if let Some(page) = project
                .pages
                .iter()
                .find(|page| page.source == *page_source)
            {
                for path in &page.runtime_paths {
                    candidates.push(path.replace('\\', "/").to_ascii_lowercase());
                }
                if let Ok(relative) = page.source.strip_prefix(&project.root) {
                    candidates.push(
                        relative
                            .to_string_lossy()
                            .replace('\\', "/")
                            .to_ascii_lowercase(),
                    );
                }
                if let Some(name) = page.source.file_name().and_then(|name| name.to_str()) {
                    candidates.push(name.to_ascii_lowercase());
                }
            }
            candidates.sort();
            candidates.dedup();
            let predicate = candidates
                .iter()
                .map(|candidate| format!("requested.ends_with({candidate:?})"))
                .collect::<Vec<_>>()
                .join(" || ");
            let page_build = if has_renderer_layout {
                "page_view.extend_stylesheets(native_stylesheets.clone());\n            _nativeboat_native_pages.push((window, page_view.clone(), ::core::cell::Cell::new(None)));\n            let page_window = ::alloc::rc::Rc::new(unsafe { ::nativeboat_page::PageWindow::adopt(page_view, &mut *renderer_realm as *mut Realm, window as *mut ::core::ffi::c_void) }.expect(\"the page's window takes its input\"));\n            _nativeboat_page_handler.add(page_window.clone());\n            _nativeboat_page_windows.push((window, page_window));\n            "
            } else {
                ""
            };
            // With the renderer libraries, the runtime has document natives: the
            // page's document is parsed from its HTML for the Web API prelude,
            // which the renderer graph runs first, and `ready` fires the
            // document's load events after the page's scripts.
            let (web_install, web_ready) = if has_renderer_layout {
                let html = project
                    .pages
                    .iter()
                    .find(|page| page.source == *page_source)
                    .and_then(|page| std::fs::read_to_string(&page.source).ok())
                    .unwrap_or_default();
                (
                    format!(
                        "let renderer_global = Value::Object(renderer_realm.global_object());\n            let location = ::nativeboat_native::ops::get(&mut renderer_realm, &renderer_global, &::nativeboat_native::Key::name(\"location\")).expect(\"renderer location is installed\");\n            let href = ::nativeboat_native::ops::get(&mut renderer_realm, &location, &::nativeboat_native::Key::name(\"href\")).expect(\"renderer location has href\");\n            let page_url = ::nativeboat_native::ops::to_string(&mut renderer_realm, &href).expect(\"renderer location href is a string\").to_rust_lossy();\n            let page_view = ::alloc::rc::Rc::new(::nativeboat_page::DocumentPage::new(::alloc::rc::Rc::new(::nativeboat_native::web::BlankPage {{ url: page_url, viewport: ::nativeboat_native::app_host::client_size(::nativeboat_native::app_host::Window::from_raw(window)).unwrap_or((800.0, 600.0)) }}), _nativeboat_measurer(window, renderer_realm.electron_version)));\n            if let Some(storage) = ::nativeboat_native::electron_builtin::page_storage_file(&mut realm) {{ page_view.set_storage_file(storage); }}\n            let web_document = renderer_realm.install_web({html:?}, page_view.clone());\n            page_view.set_document(web_document);\n            renderer_realm.set_worker_scope(|realm| renderer_{index}::module_entries(realm, &[0]).map(drop));\n            renderer_realm.set_frame_documents(&_NATIVEBOAT_FRAME_DOCUMENTS, _nativeboat_frame_page);\n            "
                    ),
                    "if let Err(thrown) = renderer_realm.web_call(\"ready\", &[]) { renderer_realm.queue_host_error(thrown); }\n            ",
                )
            } else {
                (String::new(), "")
            };
            let _ = writeln!(
                source,
                "        if !matched && ({predicate}) {{\n            matched = true;\n            let mut renderer_realm = ::alloc::boxed::Box::new(Realm::new());\n            renderer_realm.electron_version = realm.electron_version;\n            renderer_realm.electron_app_name = realm.electron_app_name;\n            renderer_realm.electron_app_version = realm.electron_app_version;\n            renderer_realm.set_electron_window(window);\n            renderer_realm.set_page_arguments(realm.electron_window_arguments(window));\n            renderer_realm.set_global(\"window\", Value::Object(renderer_realm.global_object()));\n            unsafe {{ renderer_realm.connect_electron_ipc(&mut realm as *mut Realm); }}\n            let mut page_preload: *mut Realm = ::core::ptr::null_mut();\n            if let Some((_, preload_realm)) = _nativeboat_preload_realms.iter().find(|(attached, _)| *attached == window) {{\n                page_preload = &**preload_realm as *const Realm as *mut Realm;\n                unsafe {{ ::nativeboat_native::context_bridge::expose(page_preload, &mut *renderer_realm as *mut Realm); }}\n            }}\n            unsafe {{ renderer_realm.run_page_scripts(page_preload); }}\n            let native_document = ::nativeboat_native::dom_binding::install_for_window(&mut *renderer_realm, {document}, window);\n            renderer_realm.set_renderer_location(&requested_url);\n            {}{web_install}            let renderer_status = renderer_realm.start_host(|realm| renderer_{index}::initialize(realm).map(drop)).unwrap_or(0);\n            if renderer_status != 0 {{ status = renderer_status; break; }}\n            {web_ready}_nativeboat_loading.push((window, ::nativeboat_native::platform::now()));\n            let native_stylesheets = renderer_realm.take_native_stylesheets();\n            {}            _nativeboat_renderer_realms.push((window, renderer_realm, native_document, native_stylesheets));\n        }}",
                if has_node_library {
                    "renderer_realm.node_library = Some(::nativeboat_node::require);\n            "
                } else {
                    ""
                },
                page_build,
            );
        }
    }
    source.push_str("        if !matched { ::nativeboat_native::platform::write_stderr(&::alloc::format!(\"nativeboat: a window loaded '{requested_url}', which this build did not compile\\n\")); status = 1; break; }\n    }\n}\n");
    // The pages vector stays in place, so its handlers serve later windows.
    let after = if has_renderer_layout {
        "::nativeboat_native::app_host::set_paint_handler(&_nativeboat_native_pages as *const _ as *mut ::core::ffi::c_void, Some(_nativeboat_paint));\n::nativeboat_native::app_host::set_drag_handler(&_nativeboat_native_pages as *const _ as *mut ::core::ffi::c_void, Some(_nativeboat_drag));\n::nativeboat_native::app_host::set_print_handler(&_nativeboat_native_pages as *const _ as *mut ::core::ffi::c_void, Some(_nativeboat_print));\nfor (window, _, _) in &_nativeboat_native_pages { ::nativeboat_native::app_host::invalidate_window(::nativeboat_native::app_host::Window::from_raw(*window)); }\n".to_owned()
    } else {
        String::new()
    };
    (declarations, source, after)
}

/// The documents frames may show, every page the build compiled, found by
/// the same paths as a window's page; and the function that makes a
/// frame's page in its realm.
fn frame_documents_source(project: &Project, compilation: &NativeCompilation) -> String {
    use std::fmt::Write as _;
    let mut documents = Vec::new();
    for (index, context) in compilation.renderers.iter().enumerate() {
        for page_source in &context.pages {
            let Some(page) = project
                .pages
                .iter()
                .find(|page| page.source == *page_source)
            else {
                continue;
            };
            let Ok(html) = std::fs::read_to_string(&page.source) else {
                continue;
            };
            let mut paths: Vec<String> = page
                .runtime_paths
                .iter()
                .map(|path| path.replace('\\', "/").to_ascii_lowercase())
                .collect();
            if let Ok(relative) = page.source.strip_prefix(&project.root) {
                paths.push(
                    relative
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_ascii_lowercase(),
                );
            }
            if let Some(name) = page.source.file_name().and_then(|name| name.to_str()) {
                paths.push(format!("/{}", name.to_ascii_lowercase()));
            }
            paths.sort();
            paths.dedup();
            documents.push(format!(
                "::nativeboat_native::frame::FrameDocument {{ paths: &{paths:?}, html: {html:?}, scope: |realm| renderer_{index}::module_entries(realm, &[0]).map(drop), initialize: |realm| renderer_{index}::initialize(realm).map(drop) }}"
            ));
        }
    }
    let mut source = String::new();
    let _ = writeln!(
        source,
        "static _NATIVEBOAT_FRAME_DOCUMENTS: [::nativeboat_native::frame::FrameDocument; {}] = [{}];",
        documents.len(),
        documents.join(", ")
    );
    source.push_str("fn _nativeboat_frame_page(realm: &mut Realm, url: &str, html: &str, viewport: (f32, f32), window: usize) {\n    let page_view = ::alloc::rc::Rc::new(::nativeboat_page::DocumentPage::new(::alloc::rc::Rc::new(::nativeboat_native::web::BlankPage { url: url.into(), viewport }), _nativeboat_measurer(window, realm.electron_version)));\n    let web_document = realm.install_web(html, page_view.clone());\n    page_view.set_document(web_document);\n}\n");
    source
}

/// Lowers a page's static body to Rust node specs so generated applications
/// hydrate the native document without parsing HTML at startup.
fn renderer_document_spec(page: &nativeboat_project::Entry) -> String {
    let Ok(source) = std::fs::read_to_string(&page.source) else {
        return "&[]".to_owned();
    };
    let document = nativeboat_html::parse_document(&source);
    let children = document
        .body()
        .children
        .iter()
        .map(|node| renderer_node_spec(node, "http://www.w3.org/1999/xhtml"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("&[{children}]")
}

fn renderer_node_spec(node: &nativeboat_html::Node, namespace: &str) -> String {
    use std::fmt::Write as _;
    match node {
        nativeboat_html::Node::Text(text) => {
            format!(
                "::nativeboat_native::dom_binding::NodeSpec::Text({:?}.into())",
                text.text
            )
        }
        nativeboat_html::Node::Comment(text) => {
            format!("::nativeboat_native::dom_binding::NodeSpec::Comment({text:?}.into())")
        }
        nativeboat_html::Node::Element(element) => {
            let namespace = match element.name.as_str() {
                "svg" => "http://www.w3.org/2000/svg",
                "math" => "http://www.w3.org/1998/Math/MathML",
                _ => namespace,
            };
            // A `<foreignObject>` is an SVG element whose children are HTML.
            let child_namespace =
                if element.name == "foreignObject" && namespace == "http://www.w3.org/2000/svg" {
                    "http://www.w3.org/1999/xhtml"
                } else {
                    namespace
                };
            let mut attributes = String::new();
            for (index, attribute) in element.attributes.iter().enumerate() {
                if index > 0 {
                    attributes.push_str(", ");
                }
                let _ = write!(
                    attributes,
                    "({:?}.into(), {:?}.into())",
                    attribute.name, attribute.value
                );
            }
            let children = element
                .children
                .iter()
                .map(|node| renderer_node_spec(node, child_namespace))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "::nativeboat_native::dom_binding::NodeSpec::Element {{ name: {:?}.into(), namespace: {:?}.into(), attributes: ::alloc::vec![{attributes}], children: ::alloc::vec![{children}] }}",
                element.name, namespace
            )
        }
    }
}

/// Places preload and page graphs in distinct Rust modules so they can all be
/// linked into one executable without sharing generated module caches. Each
/// window receives a separately instantiated preload Realm; renderer
/// attachment is handled by the page lifecycle.
#[cfg(test)]
fn context_rust_source(compilation: &NativeCompilation) -> String {
    use std::fmt::Write as _;

    let mut source = String::new();
    for (kind, contexts) in [
        ("preload", &compilation.preloads),
        ("renderer", &compilation.renderers),
    ] {
        for (index, context) in contexts.iter().enumerate() {
            let entries = (0..context.entry_count)
                .map(|entry| entry.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let mut style_registration = String::new();
            if kind == "renderer" || kind == "preload" {
                for (identifier, stylesheet) in &compilation.stylesheets {
                    let _ = writeln!(
                        style_registration,
                        "realm.register_native_stylesheet({identifier:?}, {stylesheet:?});"
                    );
                }
            }
            let _ = writeln!(
                source,
                "// {kind} execution context for {:?}.\n#[allow(dead_code)] mod {kind}_{index} {{\n{}\n#[allow(dead_code)] pub fn initialize(realm: &mut Realm) -> Result<Vec<Object>, Thrown> {{ {style_registration}{}module_entries(realm, &[{entries}]) }}\n}}",
                context.entry,
                context.module.rust_source(),
                script_registration(context.module.scripts()),
            );
        }
    }
    source
}

#[cfg(test)]
mod tests;
