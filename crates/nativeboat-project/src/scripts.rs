//! The commands a project runs before it is packaged, from its
//! `package.json` scripts. Packagers such as electron-builder copy what
//! earlier steps produced, for example native sidecar programs, so
//! `nativeboat build` runs those steps too, except the ones that compile
//! JavaScript, which it does itself.

use serde_json::{Map, Value};
use std::path::Path;

/// A command of a `package.json` script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The script's name.
    pub script: String,
    /// The shell command.
    pub command: String,
    /// Whether the build goes on without what the step makes when it
    /// fails: steps nativeboat infers, not those the project's packaging
    /// runs.
    pub optional: bool,
}

/// Programs that compile or bundle the application's JavaScript and
/// TypeScript, which the compiler does itself.
const COMPILERS: &[&str] = &[
    "tsc",
    "vue-tsc",
    "vite",
    "electron-vite",
    "webpack",
    "webpack-cli",
    "esbuild",
    "rollup",
    "parcel",
    "babel",
    "swc",
    "tsup",
    "react-scripts",
    "vue-cli-service",
    "next",
    "ng",
];

/// Script names that run the application in development rather than build
/// it.
const NOT_BUILDS: &[&str] = &["dev", "debug", "watch", "test", "start", "lint", "serve"];

/// How deep scripts may run other scripts.
const DEPTH: usize = 16;

/// Programs that run the tasks their arguments name, as `gulp copy-icons
/// compile` runs two.
const TASK_RUNNERS: &[&str] = &["gulp", "grunt"];

/// Folders a project's own stylesheets are not in.
const NOT_SOURCES: &[&str] = &["node_modules"];

/// Splits a command line into the commands `&&` and `;` chain, outside
/// quotes.
fn commands(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        let separator = match (quote, character) {
            (Some(open), _) => {
                if character == open {
                    quote = None;
                }
                false
            }
            (None, '"' | '\'') => {
                quote = Some(character);
                false
            }
            (None, ';') => true,
            (None, '&') if characters.peek() == Some(&'&') => {
                characters.next();
                true
            }
            _ => false,
        };
        if separator {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(character);
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

/// The words of a command, without their quotes.
fn words(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|word| word.trim_matches(['"', '\'']).to_owned())
        .collect()
}

/// A program's name without directory and extension, in lowercase:
/// `tsc` for `./node_modules/.bin/tsc.cmd`.
fn name(program: &str) -> String {
    let file = Path::new(program)
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let stem = match file.rsplit_once('.') {
        Some((stem, "cmd" | "exe" | "js" | "mjs" | "cjs" | "ps1")) => stem.to_owned(),
        _ => file,
    };
    stem.to_lowercase()
}

/// The words from the program a command runs on, past environment
/// assignments and launchers such as `cross-env` and `npx`.
fn launched(words: &[String]) -> &[String] {
    let mut rest = words;
    loop {
        let Some(first) = rest.first() else {
            return rest;
        };
        let skip = match name(first).as_str() {
            _ if first.contains('=') && !first.starts_with('-') => 1,
            "cross-env" | "cross-env-shell" | "npx" | "pnpx" | "dotenv" => 1,
            "npm" | "pnpm" | "yarn" | "bun"
                if matches!(rest.get(1).map(String::as_str), Some("exec" | "x" | "dlx")) =>
            {
                2
            }
            _ => return rest,
        };
        rest = &rest[skip..];
        while rest.first().is_some_and(|word| word.starts_with('-')) {
            rest = &rest[1..];
        }
    }
}

/// Whether a command compiles the application's code.
/// Whether a command compiles or bundles JavaScript, which the compiler
/// does itself.
pub fn is_compiler(command: &str) -> bool {
    let words = words(command);
    launched(&words)
        .first()
        .is_some_and(|program| COMPILERS.contains(&name(program).as_str()))
}

/// Identifies the configured build tool through npm-style launch wrappers.
pub fn uses_tool(command: &str, tool: &str) -> bool {
    launched(&words(command))
        .first()
        .is_some_and(|program| name(program) == tool)
}

fn compiler_step(compiler: &str) -> Step {
    Step {
        script: "nativeboat:renderer".to_owned(),
        command: format!("{compiler} build"),
        optional: false,
    }
}

/// Rebuilding addons for Electron's ABI cannot produce a nativeboat module.
/// Ordinary native sidecars and Node build tools still run normally.
fn is_electron_rebuild(command: &str) -> bool {
    let words = words(command);
    let Some((program, arguments)) = launched(&words).split_first() else {
        return false;
    };
    if name(program) == "electron-rebuild"
        || program == "@electron/rebuild"
        || program.starts_with("@electron/rebuild@")
    {
        return true;
    }
    name(program) == "electron-builder"
        && matches!(
            arguments
                .iter()
                .find(|word| !word.starts_with('-'))
                .map(String::as_str),
            Some("install-app-deps" | "node-gyp-rebuild")
        )
}

/// Whether a command packages the application.
fn is_packager(words: &[String]) -> bool {
    let words = launched(words);
    let Some((program, arguments)) = words.split_first() else {
        return false;
    };
    let action = arguments
        .iter()
        .find(|word| !word.starts_with('-'))
        .map(String::as_str);
    match name(program).as_str() {
        "electron-builder" => !matches!(
            action,
            Some("install-app-deps" | "node-gyp-rebuild" | "create-self-signed-cert" | "start")
        ),
        "electron-forge" => matches!(action, Some("package" | "make" | "publish")),
        "electron-packager" => true,
        _ => false,
    }
}

/// The script a command runs through a package manager, with the
/// arguments passed on to it.
fn invoked(words: &[String], scripts: &Map<String, Value>) -> Option<(String, Vec<String>)> {
    let (manager, rest) = words.split_first()?;
    if !matches!(name(manager).as_str(), "npm" | "pnpm" | "yarn" | "bun") {
        return None;
    }
    let flags = |from: usize| {
        from + rest[from..]
            .iter()
            .take_while(|word| word.starts_with('-'))
            .count()
    };
    let mut index = flags(0);
    if matches!(
        rest.get(index).map(String::as_str),
        Some("run" | "run-script")
    ) {
        index = flags(index + 1);
    }
    let script = rest
        .get(index)
        .filter(|script| scripts.contains_key(*script))?;
    let mut arguments = rest[index + 1..].to_vec();
    if arguments.first().is_some_and(|word| word == "--") {
        arguments.remove(0);
    }
    Some((script.clone(), arguments))
}

/// Appends the commands a script runs to `steps`, with its `pre` and
/// `post` scripts, up to one that packages the application, which it
/// returns.
fn expand(
    scripts: &Map<String, Value>,
    script: &str,
    arguments: &[String],
    depth: usize,
    steps: &mut Vec<Step>,
) -> Option<String> {
    let line = scripts.get(script).and_then(Value::as_str)?;
    if depth > DEPTH {
        return None;
    }
    let hooks = !script.starts_with("pre") && !script.starts_with("post");
    if hooks && let Some(packager) = expand(scripts, &format!("pre{script}"), &[], depth + 1, steps)
    {
        return Some(packager);
    }
    let mut line = line.to_owned();
    for argument in arguments {
        line.push(' ');
        line.push_str(argument);
    }
    for command in commands(&line) {
        let words = words(&command);
        if is_packager(&words) {
            return Some(command);
        }
        match invoked(&words, scripts) {
            Some((inner, passed)) => {
                if let Some(packager) = expand(scripts, &inner, &passed, depth + 1, steps) {
                    return Some(packager);
                }
            }
            None => steps.push(Step {
                script: script.to_owned(),
                command,
                optional: false,
            }),
        }
    }
    if hooks {
        return expand(scripts, &format!("post{script}"), &[], depth + 1, steps);
    }
    None
}

/// How well a packaging script suits a Windows build: 0 when it targets
/// Windows, 1 when it names no platform, 2 when it targets another.
fn rank(script: &str, packager: &str) -> u8 {
    let script = script.to_lowercase();
    let parts: Vec<&str> = script
        .split(|character: char| !character.is_ascii_alphanumeric())
        .collect();
    let words = words(&packager.to_lowercase());
    // A platform named by a part of the script's name, a flag, or a
    // `--platform` value.
    let targets = |prefixes: &[&str], flags: &[&str], platforms: &[&str]| {
        parts
            .iter()
            .any(|part| prefixes.iter().any(|prefix| part.starts_with(prefix)))
            || words.iter().any(|word| {
                flags.contains(&word.as_str())
                    || platforms
                        .iter()
                        .any(|platform| word == platform || word.ends_with(&format!("={platform}")))
            })
    };
    if targets(&["win"], &["--win", "-w", "--windows"], &["win32"]) {
        0
    } else if targets(
        &["mac", "darwin", "osx", "linux"],
        &["--mac", "-m", "--macos", "--linux", "-l"],
        &["darwin", "mas", "linux"],
    ) {
        2
    } else {
        1
    }
}

/// The names resources are searched for by: their file names without
/// extension, of three letters or more.
fn terms(missing: &[String]) -> Vec<String> {
    missing
        .iter()
        .filter_map(|path| Path::new(path).file_stem())
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .filter(|term| term.len() > 2)
        .collect()
}

/// Scripts that name a missing resource, by its file or directory name, in
/// their commands or in the local files they run; development and test
/// scripts are left out.
fn producers(root: &Path, scripts: &Map<String, Value>, missing: &[String]) -> Vec<Step> {
    let terms = terms(missing);
    let mut steps = Vec::new();
    for (script, line) in scripts {
        let Some(line) = line.as_str() else {
            continue;
        };
        let lower = script.to_lowercase();
        if NOT_BUILDS.iter().any(|word| lower.contains(word)) {
            continue;
        }
        let mut text = line.to_lowercase();
        for word in words(line) {
            if let Ok(content) = std::fs::read_to_string(root.join(&word)) {
                text.push_str(&content.to_lowercase());
            }
        }
        if terms.iter().any(|term| text.contains(term.as_str())) {
            expand(scripts, script, &[], 0, &mut steps);
        }
    }
    steps
}

/// The local files the project's stylesheets name in `url()`s that do not
/// exist, relative to the project: files an earlier build step makes, such
/// as an icon font copied out of a package. Packages' folders and hidden
/// folders are not searched.
pub fn missing_stylesheet_assets(root: &Path) -> Vec<String> {
    let mut missing = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for path in entries {
            let file = path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
            if path.is_dir() {
                if !file.starts_with('.') && !NOT_SOURCES.contains(&file.as_str()) {
                    folders.push(path);
                }
            } else if file.to_ascii_lowercase().ends_with(".css")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                let directory = path.parent().unwrap_or(root);
                for reference in local_urls(&text) {
                    let asset = directory.join(&reference);
                    if !asset.exists() {
                        let relative: std::path::PathBuf = asset
                            .strip_prefix(root)
                            .unwrap_or(&asset)
                            .components()
                            .collect();
                        missing.push(relative.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    missing
}

/// The local files a stylesheet's `url()`s name, outside comments: not
/// `data:` or other URLs with a scheme, fragments or root-relative paths,
/// and without their query or fragment.
fn local_urls(text: &str) -> Vec<String> {
    let mut code = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        code.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    code.push_str(rest);
    let mut urls = Vec::new();
    let mut rest = code.as_str();
    while let Some(start) = rest.find("url(") {
        rest = &rest[start + 4..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let reference = rest[..end]
            .trim()
            .trim_matches(|character| character == '"' || character == '\'');
        let file = reference.split(['?', '#']).next().unwrap_or_default();
        if !file.is_empty() && !file.contains(':') && !file.starts_with('/') {
            urls.push(file.to_owned());
        }
        rest = &rest[end..];
    }
    urls
}

/// A task runner's command with only the tasks whose names contain one of
/// `terms`, and its options; `None` when it is no task runner or runs none
/// of them.
fn matching_tasks(command: &str, terms: &[String]) -> Option<String> {
    let words = words(command);
    let runner = words
        .iter()
        .position(|word| TASK_RUNNERS.contains(&name(word).as_str()))?;
    let mut kept: Vec<&str> = words[..=runner].iter().map(String::as_str).collect();
    let mut matched = false;
    for word in &words[runner + 1..] {
        let task = !word.starts_with('-');
        let named = task && {
            let lower = word.to_lowercase();
            terms.iter().any(|term| lower.contains(term.as_str()))
        };
        if !task || named {
            kept.push(word);
        }
        matched |= named;
    }
    matched.then(|| kept.join(" "))
}

/// The tasks build scripts run through a task runner that are named after
/// a missing stylesheet asset, each run alone: VS Code's `copy-codicons`
/// copies the icon font its stylesheet names, without the extension
/// compiles the same script runs. Only task names are searched, so files
/// that test stylesheets name do not select whole builds. They are
/// optional: without the asset, a page still runs.
fn asset_tasks(scripts: &Map<String, Value>, assets: &[String]) -> Vec<Step> {
    let terms = terms(assets);
    let mut steps = Vec::new();
    if terms.is_empty() {
        return steps;
    }
    for script in scripts.keys() {
        let lower = script.to_lowercase();
        if NOT_BUILDS.iter().any(|word| lower.contains(word)) {
            continue;
        }
        let mut expanded = Vec::new();
        expand(scripts, script, &[], 0, &mut expanded);
        for step in expanded {
            if let Some(command) = matching_tasks(&step.command, &terms)
                && !steps.iter().any(|known: &Step| known.command == command)
            {
                steps.push(Step {
                    script: step.script,
                    command,
                    optional: true,
                });
            }
        }
    }
    steps
}

/// The commands to run before compiling: those the packaging script runs
/// before it packages the application, preferring a script that builds for
/// Windows, or without one, the scripts that name missing resources.
/// Commands that compile JavaScript are left out.
pub fn build_steps(root: &Path, package: &Value, missing: &[String]) -> Vec<Step> {
    build_steps_with_compiler(root, package, missing, None)
}

/// Preserves a configured plugin compiler in the application's own sequence.
pub fn build_steps_with_compiler(
    root: &Path,
    package: &Value,
    missing: &[String],
    compiler: Option<&str>,
) -> Vec<Step> {
    let Some(scripts) = package["scripts"].as_object() else {
        return compiler.map(compiler_step).into_iter().collect();
    };
    let mut best: Option<(u8, Vec<Step>)> = None;
    for script in scripts.keys() {
        let mut steps = Vec::new();
        if let Some(packager) = expand(scripts, script, &[], 0, &mut steps) {
            let rank = rank(script, &packager);
            if best.as_ref().is_none_or(|(best, _)| rank < *best) {
                best = Some((rank, steps));
            }
        }
    }
    let steps = match best {
        Some((_, steps)) => steps,
        None if compiler.is_some() && scripts.contains_key("build") => {
            let mut steps = Vec::new();
            expand(scripts, "build", &[], 0, &mut steps);
            steps
        }
        None => {
            let mut steps = producers(root, scripts, missing);
            steps.extend(asset_tasks(scripts, &missing_stylesheet_assets(root)));
            steps
        }
    };
    let mut unique: Vec<Step> = Vec::new();
    for step in steps {
        if (!is_compiler(&step.command)
            || compiler.is_some_and(|tool| uses_tool(&step.command, tool)))
            && !is_electron_rebuild(&step.command)
            && !unique.contains(&step)
        {
            unique.push(step);
        }
    }
    if let Some(compiler) = compiler
        && !unique.iter().any(|step| uses_tool(&step.command, compiler))
    {
        unique.push(compiler_step(compiler));
    }
    unique
}

#[cfg(test)]
mod tests;
