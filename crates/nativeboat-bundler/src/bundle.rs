//! Module graph construction.

use crate::emit::Emit;
use crate::module::{DependencyKind, rewrite};
use crate::resolve::{Alias, Resolved, Resolver, Target};
use crate::transpile::{TranspileOptions, transpile};
use nativeboat_css::tokenizer::{Token, tokenize};
use nativeboat_css::{Rule, parse_stylesheet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// How one module's specifier was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// Another module of the bundle, by index.
    Module(usize),
    /// A built-in runtime module such as `fs` or `electron`.
    Builtin(String),
    /// An explicitly empty replacement from a package browser map.
    Empty,
    /// An optional `require` that could not be resolved; requiring it throws.
    Missing(String),
    /// A resolve-only file copied to this portable application path.
    File(String),
}

/// A module of the bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Path relative to the project root, with `/` separators.
    pub path: String,
    /// Where the compiled file lies when application sources are compiled
    /// file by file, as `tsc` does: the module's `__filename`.
    #[serde(default)]
    pub runtime_path: Option<String>,
    /// Code in the runtime module format.
    pub code: String,
    /// Specifiers used by the module and what they resolve to.
    pub dependencies: Vec<(String, Resolution)>,
}

/// A stylesheet imported by the bundle, in import order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    /// Path relative to the project root.
    pub path: String,
    /// CSS text with asset URLs rewritten to bundle URLs.
    pub source: String,
}

/// A file referenced by URL, such as an image or font.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Path relative to the project root.
    pub path: String,
    /// The URL the application sees, such as `/src/logo.png`.
    pub url: String,
}

/// The result of bundling.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Bundle {
    /// Entry modules, in order.
    pub entries: Vec<usize>,
    /// All modules.
    pub modules: Vec<Module>,
    /// Imported stylesheets.
    pub styles: Vec<Style>,
    /// Referenced assets.
    pub assets: Vec<Asset>,
    /// Non-fatal problems.
    pub warnings: Vec<String>,
}

/// An entry point: a file, optionally with inline source (e.g. a `<script>` element).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The file path; for inline scripts, a path that names the page.
    pub path: PathBuf,
    /// Inline source text replacing the file content.
    pub source: Option<String>,
}

/// Bundling options.
#[derive(Debug, Clone)]
pub struct BundleOptions {
    /// The project root.
    pub root: PathBuf,
    /// The process the bundle runs in.
    pub target: Target,
    /// Path aliases.
    pub aliases: Vec<Alias>,
    /// Transpilation options for application code.
    pub transpile: TranspileOptions,
    /// Output layouts of application sources compiled file by file.
    pub emits: Vec<Emit>,
    /// Portable application paths for prepared frontend output.
    pub mappings: Vec<crate::PathMapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Script,
    Json,
    Css { modules: bool },
    Raw,
    Asset,
    Native,
}

fn kind_of(path: &Path, query: &str) -> Kind {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    match query {
        "raw" => return Kind::Raw,
        "url" => return Kind::Asset,
        _ => {}
    }
    match extension {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" => Kind::Script,
        "json" => Kind::Json,
        "css" => Kind::Css {
            modules: name.ends_with(".module.css"),
        },
        "node" => Kind::Native,
        "txt" | "html" | "md" => Kind::Raw,
        _ => Kind::Asset,
    }
}

/// A JavaScript string literal.
fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

struct Builder<'o> {
    options: &'o BundleOptions,
    root: PathBuf,
    resolver: Resolver,
    bundle: Bundle,
    ids: HashMap<(PathBuf, String), usize>,
    queue: VecDeque<(usize, PathBuf, String, Option<String>)>,
    styled: Vec<PathBuf>,
    errors: Vec<String>,
}

/// Bundles the entries and everything they import.
pub fn bundle(entries: &[Entry], options: &BundleOptions) -> Result<Bundle, Vec<String>> {
    bundle_page(&[], entries, options)
}

/// Bundles a page: its stylesheets (`<link>` and `<style>`) in document order,
/// then its scripts and everything they import.
pub fn bundle_page(
    styles: &[Entry],
    scripts: &[Entry],
    options: &BundleOptions,
) -> Result<Bundle, Vec<String>> {
    let mut builder = Builder {
        options,
        root: crate::resolve::realpath(&options.root),
        resolver: Resolver {
            root: options.root.clone(),
            target: options.target,
            aliases: options.aliases.clone(),
        },
        bundle: Bundle::default(),
        ids: HashMap::new(),
        queue: VecDeque::new(),
        styled: Vec::new(),
        errors: Vec::new(),
    };
    for style in styles {
        match style
            .source
            .clone()
            .map_or_else(|| std::fs::read_to_string(&style.path), Ok)
        {
            Ok(text) => {
                builder.add_style(&style.path, &text, false);
            }
            Err(error) => {
                let relative = builder.relative(&style.path);
                builder.errors.push(format!("{relative}: {error}"));
            }
        }
    }
    for entry in scripts {
        // Entry files resolve like imports: `./main.js` may be `./main.ts`.
        let path = crate::resolve::normalize(&entry.path);
        let path = match &entry.source {
            None => builder.resolver.resolve_path(&path).unwrap_or(path),
            Some(_) => path,
        };
        let id = builder.register(&path, "", entry.source.clone());
        builder.bundle.entries.push(id);
    }
    while let Some((id, path, query, source)) = builder.queue.pop_front() {
        builder.load(id, &path, &query, source);
    }
    if builder.errors.is_empty() {
        Ok(builder.bundle)
    } else {
        Err(builder.errors)
    }
}

impl Builder<'_> {
    fn resource_path(&self, path: &Path) -> String {
        let relative = self.relative(path);
        if !Path::new(&relative).is_absolute() {
            return relative;
        }
        match relative.split_once("node_modules/") {
            Some((prefix, rest)) => {
                format!("node_modules/.nativeboat/{}/{rest}", scope_suffix(prefix))
            }
            None => {
                let (prefix, name) = relative.rsplit_once('/').unwrap_or(("", &relative));
                format!(".nativeboat/files/{}/{name}", scope_suffix(prefix))
            }
        }
    }

    fn resolve_file(&mut self, path: &Path) -> Resolution {
        let url = self.asset_url(path);
        Resolution::File(url.trim_start_matches('/').to_owned())
    }

    fn relative(&self, path: &Path) -> String {
        if let Some(mapped) = self
            .options
            .mappings
            .iter()
            .find_map(|mapping| mapping.map(path))
        {
            return mapped;
        }
        let relative = path
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.options.root))
            .unwrap_or(path);
        relative.to_string_lossy().replace('\\', "/")
    }

    fn register(&mut self, path: &Path, query: &str, source: Option<String>) -> usize {
        let path = crate::resolve::realpath(path);
        let path = path.as_path();
        let key = (path.to_path_buf(), query.to_owned());
        if let Some(id) = self.ids.get(&key) {
            return *id;
        }
        let id = self.bundle.modules.len();
        let relative = self.relative(path);
        // Compiled sources run from their output files; packages stay where
        // they are.
        let packaged = relative.split('/').any(|part| part == "node_modules");
        let runtime_path = if packaged {
            // Keep external dependency filenames portable, even when pnpm
            // installs them outside the selected workspace application.
            Some(self.resource_path(path))
        } else {
            (!self.options.emits.is_empty()).then(|| {
                self.options
                    .emits
                    .iter()
                    .find_map(|emit| emit.output(&relative))
                    .unwrap_or(relative.clone())
            })
        };
        self.bundle.modules.push(Module {
            path: relative,
            runtime_path,
            code: String::new(),
            dependencies: Vec::new(),
        });
        self.ids.insert(key, id);
        self.queue
            .push_back((id, path.to_path_buf(), query.to_owned(), source));
        id
    }

    fn asset_url(&mut self, path: &Path) -> String {
        let relative = self.relative(path);
        let url = format!("/{}", self.resource_path(path));
        if !self
            .bundle
            .assets
            .iter()
            .any(|asset| asset.path == relative)
        {
            self.bundle.assets.push(Asset {
                path: relative,
                url: url.clone(),
            });
        }
        url
    }

    fn load(&mut self, id: usize, path: &Path, query: &str, source: Option<String>) {
        let kind = kind_of(path, query);
        if kind == Kind::Asset {
            let url = self.asset_url(path);
            self.bundle.modules[id].code = format!("module.exports = {};", js_string(&url));
            return;
        }
        if kind == Kind::Native {
            self.errors.push(format!(
                "{}: native Node.js addons are not supported",
                self.relative(path)
            ));
            return;
        }
        let text = match source.map_or_else(|| std::fs::read_to_string(path), Ok) {
            Ok(text) => text,
            Err(error) => {
                self.errors
                    .push(format!("{}: {error}", self.relative(path)));
                return;
            }
        };
        let code = match kind {
            Kind::Json => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(_) => format!("module.exports = {};", text.trim()),
                Err(error) => {
                    self.errors
                        .push(format!("{}: invalid JSON: {error}", self.relative(path)));
                    return;
                }
            },
            Kind::Raw => format!("module.exports = {};", js_string(&text)),
            Kind::Css { modules } => {
                let exports = self.add_style(path, &text, modules);
                format!("module.exports = {exports};")
            }
            _ => match self.script(id, path, &text) {
                Some(code) => code,
                None => return,
            },
        };
        self.bundle.modules[id].code = code;
    }

    fn script(&mut self, id: usize, path: &Path, text: &str) -> Option<String> {
        let in_package = path
            .components()
            .any(|component| component.as_os_str() == "node_modules");
        let options = TranspileOptions {
            jsx_in_js: self.options.transpile.jsx_in_js && !in_package,
            ..self.options.transpile.clone()
        };
        let rewritten = transpile(text, path, &options).and_then(|code| rewrite(&code));
        let module = match rewritten {
            Ok(module) => module,
            Err(errors) => {
                let relative = self.relative(path);
                self.errors.extend(
                    errors
                        .into_iter()
                        .map(|error| format!("{relative}: {error}")),
                );
                return None;
            }
        };
        let relative = self.relative(path);
        self.bundle.warnings.extend(
            module
                .warnings
                .iter()
                .map(|warning| format!("{relative}: {warning}")),
        );
        for dependency in &module.dependencies {
            let (specifier, query) = match dependency.specifier.split_once('?') {
                Some((specifier, query)) => (specifier, query),
                None => (dependency.specifier.as_str(), ""),
            };
            let resolution = match self.resolver.resolve(specifier, path) {
                Ok(Resolved::Builtin(name)) => Resolution::Builtin(name),
                Ok(Resolved::Empty) => Resolution::Empty,
                Ok(Resolved::File(file))
                    if dependency.kind == DependencyKind::Resolve
                        && !matches!(kind_of(&file, query), Kind::Script | Kind::Json) =>
                {
                    self.resolve_file(&file)
                }
                Ok(Resolved::File(file)) => Resolution::Module(self.register(&file, query, None)),
                Err(error)
                    if matches!(
                        dependency.kind,
                        DependencyKind::Require | DependencyKind::Resolve
                    ) =>
                {
                    self.bundle.warnings.push(format!("{relative}: {error}"));
                    Resolution::Missing(dependency.specifier.clone())
                }
                Err(error) => {
                    self.errors.push(error);
                    continue;
                }
            };
            self.bundle.modules[id]
                .dependencies
                .push((dependency.specifier.clone(), resolution));
        }
        if module.dynamic_resolutions {
            self.resolve_declared_files(id, path);
        }
        Some(module.code)
    }

    /// Adds a stylesheet and its imports; returns the JavaScript exports object.
    fn add_style(&mut self, path: &Path, text: &str, modules: bool) -> String {
        let path = crate::resolve::realpath(path);
        let path = path.as_path();
        if self.styled.iter().any(|styled| styled == path) {
            return "{}".to_owned();
        }
        self.styled.push(path.to_path_buf());
        let (sheet, _) = parse_stylesheet(text);
        for rule in &sheet.rules {
            if let Rule::Import { url, .. } = rule {
                match self.resolver.resolve(url, path) {
                    Ok(Resolved::File(file)) => match std::fs::read_to_string(&file) {
                        Ok(imported) => {
                            self.add_style(&file, &imported, false);
                        }
                        Err(error) => self
                            .errors
                            .push(format!("{}: {error}", self.relative(&file))),
                    },
                    Ok(Resolved::Builtin(name)) => {
                        self.errors
                            .push(format!("cannot import module \"{name}\" from CSS"));
                    }
                    Ok(Resolved::Empty) => {}
                    Err(error) => self.errors.push(error),
                }
            }
        }
        let (source, classes) = self.rewrite_css(path, text, modules);
        let relative = self.relative(path);
        self.bundle.styles.push(Style {
            path: relative,
            source,
        });
        let entries: Vec<String> = classes
            .iter()
            .map(|(name, scoped)| format!("{}: {}", js_string(name), js_string(scoped)))
            .collect();
        format!("{{{}}}", entries.join(", "))
    }

    /// Rewrites `url()` references to asset URLs and, for CSS modules, scopes class names.
    ///
    /// Classes inside `:global(...)` keep their names and the wrapper is removed.
    fn rewrite_css(
        &mut self,
        path: &Path,
        text: &str,
        modules: bool,
    ) -> (String, BTreeMap<String, String>) {
        let tokens = tokenize(text);
        let suffix = scope_suffix(&self.relative(path));
        let mut output = String::with_capacity(text.len());
        let mut classes = BTreeMap::new();
        let mut open: Vec<bool> = Vec::new();
        let mut in_import = false;
        for (index, spanned) in tokens.iter().enumerate() {
            match &spanned.token {
                Token::AtKeyword(name) => in_import = name.eq_ignore_ascii_case("import"),
                Token::Semicolon => in_import = false,
                _ => {}
            }
            let end = tokens.get(index + 1).map_or(text.len(), |next| next.offset);
            let original = &text[spanned.offset..end];
            let previous = index.checked_sub(1).map(|previous| &tokens[previous].token);
            let next = tokens.get(index + 1).map(|next| &next.token);
            let global = Token::Function("global".to_owned());
            let replacement = match &spanned.token {
                Token::Colon if modules && next == Some(&global) => Some(String::new()),
                Token::Function(name) => {
                    let is_global = modules && name == "global";
                    open.push(is_global);
                    is_global.then(String::new)
                }
                Token::LeftParen => {
                    open.push(false);
                    None
                }
                Token::RightParen => (open.pop() == Some(true)).then(String::new),
                Token::Url(url) if !in_import => self
                    .css_url(path, url)
                    .map(|url| format!("url({})", js_string(&url))),
                Token::String(url)
                    if !in_import
                        && matches!(previous, Some(Token::Function(name)) if name.eq_ignore_ascii_case("url")) =>
                {
                    self.css_url(path, url).map(|url| js_string(&url))
                }
                Token::Ident(class)
                    if modules && previous == Some(&Token::Delim('.')) && !open.contains(&true) =>
                {
                    let scoped = format!("{class}_{suffix}");
                    classes.insert(class.clone(), scoped.clone());
                    Some(scoped)
                }
                _ => None,
            };
            output.push_str(replacement.as_deref().unwrap_or(original));
        }
        (output, classes)
    }

    fn css_url(&mut self, path: &Path, url: &str) -> Option<String> {
        if url.is_empty() || url.starts_with("data:") || url.starts_with('#') || url.contains("://")
        {
            return None;
        }
        let clean = url.split(['?', '#']).next().unwrap_or(url);
        match self.resolver.resolve(clean, path) {
            Ok(Resolved::File(file)) => Some(self.asset_url(&file)),
            _ => {
                self.bundle.warnings.push(format!(
                    "{}: cannot resolve url({url})",
                    self.relative(path)
                ));
                None
            }
        }
    }
}

/// A short stable hash used to scope CSS module class names.
fn scope_suffix(path: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for byte in path.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("{hash:08x}").chars().take(6).collect()
}

mod resolution;
#[cfg(test)]
mod tests;
