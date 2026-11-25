//! Vue single-file components compiled by the project's own
//! `@vue/compiler-sfc`, as `@vitejs/plugin-vue` compiles them for a
//! production build. NativeBoat compiles the compiler's browser ESM build
//! together with a small driver (`node/tools/vue-sfc.mjs`) into a native
//! tool once per compiler version, caches it, and runs it during builds.
//! Each component's result is cached by its content, so unchanged
//! components are not compiled again. Projects without the compiler use the
//! built-in template lowering of `vue_sfc`.

use crate::native::Args;
use nativeboat_project::Project;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The driver script; `COMPILER_SFC` names the compiler's bundle.
const DRIVER: crate::Text = nativeboat_conceal::text!("../node/tools/vue-sfc.mjs");

/// A compiled component: its module source and its stylesheets.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Compiled {
    pub(crate) code: String,
    pub(crate) styles: Vec<String>,
}

/// Compiled components by their source path.
pub(crate) type Components = BTreeMap<PathBuf, Compiled>;

/// The browser ESM build of the project's `@vue/compiler-sfc`: installed
/// directly, or beside the `vue` package it belongs to.
pub(crate) fn locate(root: &Path) -> Option<PathBuf> {
    const BUNDLE: &str = "dist/compiler-sfc.esm-browser.js";
    for directory in root.ancestors() {
        let modules = directory.join("node_modules");
        let direct = modules.join("@vue").join("compiler-sfc").join(BUNDLE);
        if direct.is_file() {
            return Some(direct);
        }
        if let Ok(vue) = std::fs::canonicalize(modules.join("vue"))
            && let Some(parent) = vue.parent()
        {
            let beside = parent.join("@vue").join("compiler-sfc").join(BUNDLE);
            if beside.is_file() {
                // Without Windows' verbatim prefix, so that it resolves as an
                // import specifier.
                let text = beside.to_string_lossy();
                return Some(PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)));
            }
        }
    }
    None
}

/// The `.vue` files of a project's own sources, outside `node_modules`.
pub(crate) fn components(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if name != "node_modules" && !name.to_string_lossy().starts_with('.') {
                    walk(&path, found);
                }
            } else if path.extension().is_some_and(|extension| extension == "vue") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort();
    found
}

/// The driver's source for a compiler bundle.
fn driver(bundle: &Path) -> String {
    let bundle = bundle.to_string_lossy().replace('\\', "/");
    DRIVER().replace("'COMPILER_SFC'", &format!("{bundle:?}"))
}

/// An I/O result, its failure described with the path involved.
fn io<T>(result: std::io::Result<T>, action: &str, path: &Path) -> Result<T, String> {
    result.map_err(|error| format!("cannot {action} {}: {error}", path.display()))
}

/// The cached native tool for a compiler bundle, built when missing.
fn tool(bundle: &Path, args: &Args, cache: &Path) -> Result<PathBuf, String> {
    let contents = io(std::fs::read(bundle), "read", bundle)?;
    let source = driver(bundle);
    let key = crate::native::fingerprint(&[contents.as_slice(), source.as_bytes()]);
    let directory = cache.join(format!("vue-sfc-{key:016x}"));
    let executable = directory.join("nativeboat-vue-sfc.exe");
    if executable.is_file() {
        return Ok(executable);
    }
    let moved = build(&directory, &source, args)
        .and_then(|built| io(std::fs::rename(&built, &executable), "cache", &built));
    // Only the executable is kept, and nothing of a failed build.
    let _ = std::fs::remove_dir_all(directory.join("project"));
    let _ = std::fs::remove_dir_all(directory.join("out"));
    if moved.is_err() {
        let _ = std::fs::remove_dir_all(&directory);
    }
    moved.map(|()| executable)
}

/// Builds the tool's sources in `directory` like any application: a manifest
/// and an entry module.
fn build(directory: &Path, source: &str, args: &Args) -> Result<PathBuf, String> {
    const MANIFEST: &str = r#"{"name":"nativeboat-vue-sfc","type":"module","main":"main.mjs","devDependencies":{"electron":"*"}}"#;
    let sources = directory.join("project");
    io(std::fs::create_dir_all(&sources), "create", &sources)?;
    let manifest = sources.join("package.json");
    io(std::fs::write(&manifest, MANIFEST), "write", &manifest)?;
    io(
        std::fs::write(sources.join("main.mjs"), source),
        "write",
        &sources,
    )?;
    let project = nativeboat_project::load(&sources)?;
    let out = directory.join("out");
    crate::native::build_tool(&project, &out, args).map_err(|errors| errors.join("; "))
}

/// Compiles every component of a project that ships `@vue/compiler-sfc`.
/// `None` when the project has no components or no compiler.
pub(crate) fn prepare(project: &Project, args: &Args) -> Result<Option<Components>, Vec<String>> {
    prepare_in(project, args, &crate::native::cache_directory())
}

fn prepare_in(
    project: &Project,
    args: &Args,
    cache: &Path,
) -> Result<Option<Components>, Vec<String>> {
    let files = components(&project.root);
    let (Some(page), Some(bundle)) = (project.pages.first(), locate(&project.root)) else {
        return Ok(None);
    };
    if files.is_empty() {
        return Ok(None);
    }
    let tool =
        tool(&bundle, args, cache).map_err(|error| vec![format!("the Vue SFC tool: {error}")])?;
    // Component ids hash paths from the renderer root, Vite's `root`.
    let root = page.source.parent().unwrap_or(&project.root);
    compile(&tool, root, &files, cache).map(Some)
}

/// Each component's cached result, or the tool's result for those changed.
fn compile(
    tool: &Path,
    root: &Path,
    files: &[PathBuf],
    cache: &Path,
) -> Result<Components, Vec<String>> {
    // The tool's path names its compiler and driver.
    let tool_key = tool.to_string_lossy();
    let results = cache.join("vue-components");
    let mut compiled = Components::new();
    let mut pending = Vec::new();
    for file in files {
        let source = io(std::fs::read(file), "read", file).map_err(|error| vec![error])?;
        let key = crate::native::fingerprint(&[
            tool_key.as_bytes(),
            root.as_os_str().as_encoded_bytes(),
            file.as_os_str().as_encoded_bytes(),
            &source,
        ]);
        let entry = results.join(format!("{key:016x}.json"));
        match std::fs::read_to_string(&entry)
            .ok()
            .and_then(|text| parse(&text))
        {
            Some(component) => {
                compiled.insert(file.clone(), component);
            }
            None => pending.push((file.clone(), entry)),
        }
    }
    if pending.is_empty() {
        return Ok(compiled);
    }
    let names: Vec<String> = pending
        .iter()
        .map(|(file, _)| file.to_string_lossy().replace('\\', "/"))
        .collect();
    let results = run(tool, root, &names, &results).map_err(|error| vec![error])?;
    let mut errors = Vec::new();
    for ((file, entry), name) in pending.iter().zip(&names) {
        let result = &results[name.as_str()];
        if let Some(messages) = result["errors"].as_array() {
            errors.extend(messages.iter().map(|message| {
                format!(
                    "{}:1:1: {}",
                    file.display(),
                    message.as_str().unwrap_or_default()
                )
            }));
        } else if let Some(component) = parse(&result.to_string()) {
            let _ = std::fs::write(entry, result.to_string());
            compiled.insert(file.clone(), component);
        } else {
            errors.push(format!(
                "{}:1:1: the Vue SFC tool returned no module",
                file.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(compiled)
    } else {
        Err(errors)
    }
}

/// Runs the tool on some components: its results by component.
fn run(
    tool: &Path,
    root: &Path,
    names: &[String],
    results: &Path,
) -> Result<serde_json::Value, String> {
    io(std::fs::create_dir_all(results), "create", results)?;
    let request = results.join(format!("request-{}.json", std::process::id()));
    let output = results.join(format!("output-{}.json", std::process::id()));
    let body =
        serde_json::json!({ "root": root.to_string_lossy().replace('\\', "/"), "files": names });
    io(
        std::fs::write(&request, body.to_string()),
        "write",
        &request,
    )?;
    let status = std::process::Command::new(tool)
        .arg(&request)
        .arg(&output)
        .status();
    let _ = std::fs::remove_file(&request);
    let status = io(status, "run", tool)?;
    let text = std::fs::read_to_string(&output);
    let _ = std::fs::remove_file(&output);
    if !status.success() {
        return Err(format!("the Vue SFC tool failed ({status})"));
    }
    let text = io(text, "read the output of", tool)?;
    serde_json::from_str(&text).map_err(|error| format!("invalid Vue SFC tool output: {error}"))
}

/// A component result: `{ "code": ..., "styles": [...] }`.
fn parse(text: &str) -> Option<Compiled> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(Compiled {
        code: value["code"].as_str()?.to_owned(),
        styles: value["styles"]
            .as_array()?
            .iter()
            .map(|style| style.as_str().map(str::to_owned))
            .collect::<Option<_>>()?,
    })
}

/// The module source NativeBoat compiles for a component: the tool's module,
/// then its stylesheets' imports, which the plugin also places after the
/// script so that imported components' styles come first.
pub(crate) fn module_source(component: &Compiled) -> String {
    let mut source = component.code.clone();
    for index in 0..component.styles.len() {
        source.push_str(&format!("\nimport {:?};", style_request(index)));
    }
    source.push('\n');
    source
}

const STYLE_REQUEST: &str = "nativeboat:vue-style:";

fn style_request(index: usize) -> String {
    format!("{STYLE_REQUEST}{index}")
}

/// A component module's stylesheet request: its index.
pub(crate) fn style_index(request: &str) -> Option<usize> {
    request.strip_prefix(STYLE_REQUEST)?.parse().ok()
}

#[cfg(test)]
mod tests;
