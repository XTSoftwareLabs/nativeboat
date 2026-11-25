//! Compatibility analysis of Electron projects.
//!
//! The analyzer scans `package.json` and what the application ships, the
//! scripts of the main process, preload scripts and pages with the local
//! modules, Vue components and stylesheets they import, for frameworks,
//! Electron and Node.js APIs, web platform features and CSS, and reports how
//! well the native target supports each.

pub mod catalog;
pub mod js;
pub mod markup;

use catalog::{Feature, NATIVE_PACKAGES, Support, native_package};
use nativeboat_bundler::{Resolver, Target};
use nativeboat_project::Project;
use nativeboat_project::scan::scan_shipped;
use oxc::span::SourceType;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where a feature is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    /// File path relative to the project root.
    pub file: String,
    /// One-based line, or 0 when unknown.
    pub line: u32,
    /// One-based column, or 0 when unknown.
    pub column: u32,
    /// The source line.
    pub snippet: String,
}

/// A feature and where the project uses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureUse {
    /// The feature.
    pub feature: Feature,
    /// Its occurrences in discovery order.
    pub occurrences: Vec<Occurrence>,
}

/// The compatibility report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Weighted share of supported features, from 0 to 100.
    pub score: u32,
    /// Features in report order: supported, partial, unsupported.
    pub features: Vec<FeatureUse>,
    /// Number of source files scanned.
    pub files: usize,
}

/// Directories that never contain application sources.
/// Resolves a URL in a page: `/x` is relative to the renderer root.
fn page_url(root: &Path, page: &Path, url: &str) -> PathBuf {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    match url.strip_prefix('/') {
        Some(rooted) => root.join(rooted),
        None => page.parent().unwrap_or(root).join(url),
    }
}

/// Lists the files the application ships: the scripts of the main process,
/// preload scripts and pages with the local modules, Vue components and
/// stylesheets they import, and the pages with their stylesheets.
pub fn shipped_files(project: &Project) -> Vec<PathBuf> {
    let resolver = |target, aliases: &[_]| Resolver {
        root: project.root.clone(),
        target,
        aliases: aliases.to_vec(),
    };
    let renderer = resolver(Target::Renderer, &project.renderer_aliases);
    let preload = resolver(Target::Renderer, &project.preload_aliases);
    let mut files = scan_shipped(
        &project.main.source,
        &resolver(Target::Main, &project.aliases),
    )
    .files;
    for entry in &project.preloads {
        files.extend(scan_shipped(&entry.source, &preload).files);
    }
    let mut entries = Vec::new();
    for page in &project.pages {
        files.push(page.source.clone());
        let text = std::fs::read_to_string(&page.source).unwrap_or_default();
        for element in nativeboat_html::parse_document(&text).elements() {
            let (url, stylesheet) = match element.name.as_str() {
                "script" => (element.attribute("src"), false),
                "link" => (
                    element.attribute("href"),
                    element
                        .attribute("rel")
                        .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet")),
                ),
                _ => (None, false),
            };
            let Some(path) = url.filter(|url| !markup::is_remote(url)).and_then(|url| {
                renderer.resolve_path(&page_url(&project.renderer_root, &page.source, url))
            }) else {
                continue;
            };
            if stylesheet {
                files.push(path);
            } else if element.name == "script" {
                entries.push(path);
            }
        }
    }
    for entry in entries {
        files.extend(scan_shipped(&entry, &renderer).files);
    }
    let mut shipped = Vec::new();
    for file in files {
        let file = nativeboat_bundler::resolve::normalize(&file);
        if !shipped.contains(&file) {
            shipped.push(file);
        }
    }
    shipped.sort();
    shipped
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(index, _)| index + 1));
    starts
}

/// `path` relative to `root`, with `..` for files beside it such as
/// workspace packages, in forward slashes.
fn relative(root: &Path, path: &Path) -> String {
    let (root, path): (Vec<_>, Vec<_>) = (root.components().collect(), path.components().collect());
    let common = root.iter().zip(&path).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return path
            .iter()
            .collect::<PathBuf>()
            .to_string_lossy()
            .replace('\\', "/");
    }
    let mut parts = vec![String::from(".."); root.len() - common];
    parts.extend(
        path[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Features of a file by its kind: markup, stylesheet or script.
fn detect(path: &Path, source: &str, project: &Project) -> Vec<markup::Located> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".html") {
        return markup::detect_html(source);
    }
    if lower.ends_with(".css") {
        return markup::detect_css(source);
    }
    if lower.ends_with(".vue") {
        return component(source, project);
    }
    if let Some(package) = match path.extension().and_then(|extension| extension.to_str()) {
        Some("scss" | "sass") => Some("sass"),
        Some("less") => Some("less"),
        Some("styl" | "stylus") => Some("stylus"),
        _ => None,
    } {
        return vec![markup::Located {
            feature: catalog::framework(package).expect("known preprocessor"),
            line: 1,
            column: 1,
        }];
    }
    let starts = line_starts(source);
    // Scripts may hold JSX. TypeScript holds it only in `.tsx`, the variant
    // its path gives; elsewhere `<T>value` is a type assertion. (Turning JSX
    // off again does not restore oxc's standard variant.)
    let source_type = SourceType::from_path(path).unwrap_or_default();
    let source_type = if source_type.is_typescript() {
        source_type
    } else {
        source_type.with_jsx(true)
    };
    js::detect_with_metadata(
        source,
        source_type,
        project.legacy_decorators && project.decorator_metadata,
    )
    .into_iter()
    .map(|finding| {
        let offset = finding.offset as usize;
        let line = starts.partition_point(|start| *start <= offset);
        let column = source[starts[line - 1]..offset].chars().count() + 1;
        markup::Located {
            feature: finding.feature,
            line: line as u32,
            column: column as u32,
        }
    })
    .collect()
}

/// Features of a Vue component: its template as markup, its scripts and
/// styles in their languages, located in the component.
fn component(source: &str, project: &Project) -> Vec<markup::Located> {
    use nativeboat_project::vue::{BlockKind, blocks};
    let mut found = Vec::new();
    for block in blocks(source) {
        let lang = block.lang.as_deref();
        let kind = match (block.kind, lang) {
            (_, _) if block.src.is_some() => continue,
            (BlockKind::Template, None | Some("html")) => "template.html".to_owned(),
            (BlockKind::Script, Some(lang @ ("ts" | "tsx" | "jsx"))) => format!("script.{lang}"),
            (BlockKind::Script, _) => "script.js".to_owned(),
            (BlockKind::Style, None | Some("css")) => "style.css".to_owned(),
            (BlockKind::Style, Some(lang)) => format!("style.{lang}"),
            _ => continue,
        };
        let line = block.line(source) - 1;
        let column = source[..block.start]
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count() as u32;
        for mut item in detect(Path::new(&kind), block.content, project) {
            if block.scoped && vue_scoped_selector(&item.feature.id) {
                continue;
            }
            if item.line == 1 {
                item.column += column;
            }
            item.line += line;
            found.push(item);
        }
    }
    found
}

/// Selectors Vue's style compiler rewrites in a scoped style block, so the
/// page never sees them: `:deep()`, `:slotted()` and `:global()`, and their
/// `:v-` spellings.
fn vue_scoped_selector(id: &str) -> bool {
    id.strip_prefix("css:selector:")
        .map(|name| name.strip_prefix("v-").unwrap_or(name))
        .is_some_and(|name| matches!(name, "deep" | "slotted" | "global"))
}

fn snippet(source: &str, line: u32) -> String {
    let Some(index) = (line as usize).checked_sub(1) else {
        return String::new();
    };
    let text = source.lines().nth(index).unwrap_or_default().trim();
    let mut snippet: String = text.chars().take(100).collect();
    if text.chars().count() > 100 {
        snippet.push('…');
    }
    snippet
}

struct Collector {
    features: Vec<FeatureUse>,
}

impl Collector {
    fn add(&mut self, feature: Feature, occurrence: Occurrence) {
        match self
            .features
            .iter_mut()
            .find(|used| used.feature.id == feature.id)
        {
            Some(used) => {
                // Used anywhere in a less supported way, the feature is less
                // supported.
                if feature.support > used.feature.support {
                    used.feature = feature;
                }
                used.occurrences.push(occurrence);
            }
            None => self.features.push(FeatureUse {
                feature,
                occurrences: vec![occurrence],
            }),
        }
    }
}

fn is_native(root: &Path, package: &str) -> bool {
    if NATIVE_PACKAGES.contains(&package) {
        return true;
    }
    let directory = root.join("node_modules").join(package);
    if directory.join("binding.gyp").is_file() {
        return true;
    }
    std::fs::read_to_string(directory.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|manifest| manifest["gypfile"] == true)
}

/// Analyzes a project.
pub fn analyze(project: &Project) -> Report {
    let mut collector = Collector {
        features: Vec::new(),
    };
    let package_occurrence = || Occurrence {
        file: "package.json".to_owned(),
        line: 0,
        column: 0,
        snippet: String::new(),
    };
    for name in project.dependencies.keys() {
        if is_native(&project.root, name) {
            collector.add(native_package(name), package_occurrence());
        }
    }
    let files = shipped_files(project);
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let file = relative(&project.root, path);
        for item in detect(path, &source, project) {
            let feature = if item.feature.id == "language:decorators" && project.legacy_decorators {
                catalog::legacy_decorators()
            } else {
                item.feature
            };
            let occurrence = Occurrence {
                file: file.clone(),
                line: item.line,
                column: item.column,
                snippet: snippet(&source, item.line),
            };
            collector.add(feature, occurrence);
        }
    }
    let mut features = collector.features;
    features.sort_by_key(|used| used.feature.support);
    let score = if features.is_empty() {
        100
    } else {
        let total: f64 = features
            .iter()
            .map(|used| used.feature.support.weight())
            .sum();
        (total / features.len() as f64 * 100.0).round() as u32
    };
    Report {
        score,
        features,
        files: files.len(),
    }
}

impl Report {
    /// Whether any used feature is unsupported.
    pub fn has_unsupported(&self) -> bool {
        self.features
            .iter()
            .any(|used| used.feature.support == Support::Unsupported)
    }

    /// Renders the report as text.
    pub fn to_text(&self) -> String {
        let mut text = format!("Project compatibility: {}%\n", self.score);
        let width = self
            .features
            .iter()
            .map(|used| used.feature.name.chars().count() + 2)
            .max()
            .unwrap_or_default()
            .max(30);
        let mut previous = None;
        for used in &self.features {
            if previous != Some(used.feature.support) {
                text.push('\n');
                previous = Some(used.feature.support);
            }
            text.push_str(&format!(
                "{:<width$}{}\n",
                used.feature.name,
                used.feature.support.label()
            ));
        }
        for used in self
            .features
            .iter()
            .filter(|used| used.feature.support != Support::Supported)
        {
            text.push_str(&format!(
                "\n{} ({}): {}\n",
                used.feature.name,
                used.feature.support.label(),
                used.feature.note
            ));
            for occurrence in used.occurrences.iter().take(5) {
                if occurrence.line == 0 {
                    text.push_str(&format!("  {}\n", occurrence.file));
                } else {
                    text.push_str(&format!(
                        "  {}:{}\n      {}\n",
                        occurrence.file, occurrence.line, occurrence.snippet
                    ));
                }
            }
            if used.occurrences.len() > 5 {
                text.push_str(&format!("  … and {} more\n", used.occurrences.len() - 5));
            }
        }
        text
    }

    /// Renders the report as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
