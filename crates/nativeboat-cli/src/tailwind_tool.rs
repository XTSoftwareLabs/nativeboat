//! Tailwind CSS stylesheets built by the project's own `tailwindcss`
//! compiler, as `@tailwindcss/vite` builds them. NativeBoat compiles the
//! compiler's ESM build, the plugin and configuration modules the
//! stylesheets name, and a small driver (`node/tools/tailwind.mjs`) into a
//! native tool, caches it, and runs it during builds on each stylesheet that
//! uses Tailwind's directives, with its imports inlined. The class
//! candidates come from the renderer's sources, as Tailwind's scanner finds
//! them; candidates Tailwind does not know are ignored.

use crate::native::Args;
use nativeboat_bundler::resolve::{Resolved, Resolver, Target};
use nativeboat_project::Project;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The driver script.
const DRIVER: crate::Text = nativeboat_conceal::text!("../node/tools/tailwind.mjs");

/// Built stylesheets by their source path.
pub(crate) type Sheets = BTreeMap<PathBuf, String>;

/// Tailwind's directives; a stylesheet without them is plain CSS.
const DIRECTIVES: &[&str] = &[
    "@tailwind",
    "@theme",
    "@apply",
    "@utility",
    "@variant",
    "@custom-variant",
    "@plugin",
    "@config",
    "@source",
    "@reference",
];

/// Whether a stylesheet, its imports inlined, needs Tailwind's compiler.
pub(crate) fn needed(css: &str) -> bool {
    DIRECTIVES.iter().any(|directive| css.contains(directive))
}

/// An I/O result, its failure described with the path involved.
fn io<T>(result: std::io::Result<T>, action: &str, path: &Path) -> Result<T, String> {
    result.map_err(|error| format!("cannot {action} {}: {error}", path.display()))
}

/// The stylesheets of a project's own sources, outside `node_modules` and
/// hidden folders.
fn stylesheets(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(root, &mut |path| {
        if path.extension().is_some_and(|extension| extension == "css") {
            found.push(path.to_path_buf());
        }
    });
    found.sort();
    found
}

/// Visits the files under a folder, skipping `node_modules`, hidden folders
/// and build outputs, as Tailwind's scanner skips ignored files.
fn walk(directory: &Path, visit: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "node_modules" && !name.starts_with('.') {
                walk(&path, visit);
            }
        } else {
            visit(&path);
        }
    }
}

/// Builds every stylesheet of the project that uses Tailwind's directives.
pub(crate) fn prepare(project: &Project, args: &Args) -> Result<Sheets, Vec<String>> {
    prepare_in(project, args, &crate::native::cache_directory())
}

fn prepare_in(project: &Project, args: &Args, cache: &Path) -> Result<Sheets, Vec<String>> {
    let mut sheets = Sheets::new();
    let mut errors = Vec::new();
    for path in stylesheets(&project.root) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !needed(&text) && !text.contains("tailwindcss") {
            continue;
        }
        let (css, mut import_errors) =
            crate::css_imports::inline(&path, &text, &project.root, &project.renderer_aliases);
        if !needed(&css) {
            continue;
        }
        errors.append(&mut import_errors);
        match build(project, &path, &css, args, cache) {
            Ok(built) => {
                sheets.insert(path, built);
            }
            Err(error) => errors.push(format!("{}:1:1: Tailwind CSS: {error}", path.display())),
        }
    }
    if errors.is_empty() {
        Ok(sheets)
    } else {
        Err(errors)
    }
}

/// A module specifier resolved for the tool to import, from a stylesheet.
fn resolve(specifier: &str, from: &Path, root: &Path) -> Option<PathBuf> {
    let resolver = Resolver {
        root: root.to_path_buf(),
        target: Target::MainImport,
        aliases: Vec::new(),
    };
    match resolver.resolve(specifier, from) {
        Ok(Resolved::File(path)) => Some(nativeboat_bundler::resolve::realpath(&path)),
        _ => None,
    }
}

/// The `@plugin` and `@config` modules a stylesheet names, as written.
fn modules(css: &str) -> Vec<String> {
    let mut names = Vec::new();
    for directive in ["@plugin", "@config"] {
        let mut rest = css;
        while let Some(start) = rest.find(directive) {
            rest = &rest[start + directive.len()..];
            let text = rest.trim_start();
            let Some(quote) = text.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                continue;
            };
            if let Some(end) = text[1..].find(quote) {
                names.push(text[1..1 + end].to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// The driver's source for a compiler and its modules.
fn driver(compiler: &Path, modules: &[(String, PathBuf)]) -> String {
    let slash = |path: &Path| path.to_string_lossy().replace('\\', "/");
    let mut imports = String::new();
    let mut entries = Vec::new();
    for (index, (name, path)) in modules.iter().enumerate() {
        imports.push_str(&format!("import * as m{index} from {:?};\n", slash(path)));
        entries.push(format!("[{name:?}, m{index}]"));
    }
    let source = DRIVER()
        .replace("'TAILWINDCSS'", &format!("{:?}", slash(compiler)))
        .replace(
            "new Map(MODULES)",
            &format!("new Map([{}])", entries.join(", ")),
        );
    format!("{imports}{source}")
}

/// The cached native tool for a compiler and its modules, built when missing.
fn tool(
    compiler: &Path,
    modules: &[(String, PathBuf)],
    args: &Args,
    cache: &Path,
) -> Result<PathBuf, String> {
    let source = driver(compiler, modules);
    let mut parts = vec![
        io(std::fs::read(compiler), "read", compiler)?,
        source.clone().into_bytes(),
    ];
    for (_, path) in modules {
        parts.push(io(std::fs::read(path), "read", path)?);
    }
    let parts: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let key = crate::native::fingerprint(&parts);
    let directory = cache.join(format!("tailwind-{key:016x}"));
    let executable = directory.join("nativeboat-tailwind.exe");
    if executable.is_file() {
        return Ok(executable);
    }
    const MANIFEST: &str = r#"{"name":"nativeboat-tailwind","type":"module","main":"main.mjs","devDependencies":{"electron":"*"}}"#;
    let sources = directory.join("project");
    io(std::fs::create_dir_all(&sources), "create", &sources)?;
    io(
        std::fs::write(sources.join("package.json"), MANIFEST),
        "write",
        &sources,
    )?;
    io(
        std::fs::write(sources.join("main.mjs"), &source),
        "write",
        &sources,
    )?;
    let built = nativeboat_project::load(&sources).and_then(|project| {
        crate::native::build_tool(&project, &directory.join("out"), args)
            .map_err(|errors| errors.join("; "))
    });
    let moved = built.and_then(|built| io(std::fs::rename(&built, &executable), "cache", &built));
    let _ = std::fs::remove_dir_all(&sources);
    let _ = std::fs::remove_dir_all(directory.join("out"));
    if moved.is_err() {
        let _ = std::fs::remove_dir_all(&directory);
    }
    moved.map(|()| executable)
}

/// Builds one stylesheet whose imports are inlined.
fn build(
    project: &Project,
    path: &Path,
    css: &str,
    args: &Args,
    cache: &Path,
) -> Result<String, String> {
    let compiler = resolve("tailwindcss", path, &project.root).ok_or(
        "the stylesheet uses Tailwind's directives, but the tailwindcss package is not installed",
    )?;
    let mut modules = Vec::new();
    for name in self::modules(css) {
        let found = resolve(&name, path, &project.root)
            .ok_or_else(|| format!("cannot resolve the plugin or configuration \"{name}\""))?;
        modules.push((name, found));
    }
    let tool = tool(&compiler, &modules, args, cache)?;
    let base = path.parent().unwrap_or(&project.root);
    let candidates = candidates(&sources(project, path, css));
    let results = cache.join("tailwind-sheets");
    io(std::fs::create_dir_all(&results), "create", &results)?;
    let joined = candidates.join("\n");
    let key = crate::native::fingerprint(&[
        tool.to_string_lossy().as_bytes(),
        css.as_bytes(),
        joined.as_bytes(),
    ]);
    let cached = results.join(format!("{key:016x}.css"));
    if let Ok(css) = std::fs::read_to_string(&cached) {
        return Ok(css);
    }
    let request = results.join(format!("request-{}.json", std::process::id()));
    let output = results.join(format!("output-{}.json", std::process::id()));
    let body = serde_json::json!({
        "css": css,
        "base": base.to_string_lossy().replace('\\', "/"),
        "candidates": candidates,
    });
    io(
        std::fs::write(&request, body.to_string()),
        "write",
        &request,
    )?;
    let status = std::process::Command::new(&tool)
        .arg(&request)
        .arg(&output)
        .status();
    let _ = std::fs::remove_file(&request);
    let status = io(status, "run", &tool)?;
    let text = std::fs::read_to_string(&output);
    let _ = std::fs::remove_file(&output);
    if !status.success() {
        return Err(format!("the Tailwind CSS tool failed ({status})"));
    }
    let value: serde_json::Value = serde_json::from_str(&io(text, "read the output of", &tool)?)
        .map_err(|error| format!("invalid Tailwind CSS tool output: {error}"))?;
    if let Some(errors) = value["errors"].as_array() {
        return Err(errors
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join("; "));
    }
    let built = value["css"]
        .as_str()
        .ok_or("the Tailwind CSS tool returned no stylesheet")?
        .to_owned();
    let _ = std::fs::write(&cached, &built);
    Ok(built)
}

/// The folders whose files Tailwind scans for candidates: the renderer's
/// root, as Vite's, and those `@source` names, relative to the stylesheet.
fn sources(project: &Project, path: &Path, css: &str) -> Vec<PathBuf> {
    let mut found = vec![project.renderer_root.clone()];
    let directory = path.parent().unwrap_or(&project.root);
    let mut rest = css;
    while let Some(start) = rest.find("@source") {
        rest = &rest[start + "@source".len()..];
        let text = rest.trim_start();
        let Some(quote) = text.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        if let Some(end) = text[1..].find(quote) {
            let source = directory.join(&text[1..1 + end]);
            if source.exists() {
                found.push(source);
            }
        }
    }
    found
}

/// Files whose contents are not text a class can appear in.
fn binary(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "avif"
                    | "ico"
                    | "bmp"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "eot"
                    | "mp4"
                    | "webm"
                    | "mp3"
                    | "wav"
                    | "zip"
                    | "gz"
                    | "exe"
                    | "dll"
                    | "node"
                    | "pdf"
                    | "lock"
                    | "map"
            )
        })
}

/// The class candidates in the files under some folders or files: runs of
/// characters between whitespace, quotes and markup delimiters, with
/// brackets and parentheses kept whole, as arbitrary values have them.
pub(crate) fn candidates(roots: &[PathBuf]) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut visit = |path: &Path| {
        if binary(path) {
            return;
        }
        if let Ok(text) = std::fs::read_to_string(path) {
            extract(&text, &mut found);
        }
    };
    for root in roots {
        if root.is_file() {
            visit(root);
        } else {
            walk(root, &mut visit);
        }
    }
    found.into_iter().collect()
}

fn extract(text: &str, found: &mut BTreeSet<String>) {
    let mut token = String::new();
    let mut depth = 0usize;
    let flush = |token: &mut String, found: &mut BTreeSet<String>| {
        let candidate = token.trim_end_matches(['.', ',', ':', ';']);
        if !candidate.is_empty() && candidate.len() <= 256 {
            found.insert(candidate.to_owned());
        }
        token.clear();
    };
    for character in text.chars() {
        match character {
            '[' | '(' => {
                depth += 1;
                token.push(character);
            }
            ']' | ')' if depth > 0 => {
                depth -= 1;
                token.push(character);
            }
            _ if depth > 0 && !character.is_whitespace() && !matches!(character, '"' | '`') => {
                token.push(character);
            }
            _ if character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | '`' | '<' | '>' | '{' | '}' | '=' | ';' | ',' | '(' | ')'
                ) =>
            {
                depth = 0;
                flush(&mut token, found);
            }
            _ => token.push(character),
        }
    }
    flush(&mut token, found);
}

#[cfg(test)]
mod tests;
