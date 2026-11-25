//! Static electron-vite entry discovery. Reading a config never executes it.

use crate::config::evaluate::Data;
use crate::{Entry, config::first_existing, file_name, html_files, relative};
use nativeboat_bundler::{Alias, AliasKind, Resolver, resolve::normalize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub(crate) const CONFIGS: &[&str] = &[
    "electron.vite.config.js",
    "electron.vite.config.ts",
    "electron.vite.config.mjs",
    "electron.vite.config.cjs",
    "electron.vite.config.mts",
    "electron.vite.config.cts",
];

struct Config {
    data: Value,
    defines: [Vec<(String, String)>; 3],
    aliases: [Vec<Alias>; 3],
}

fn aliases(data: &Data, scope: &str) -> Result<Vec<Alias>, String> {
    let Some(resolve) = data
        .property(scope)
        .and_then(|section| section.property("resolve"))
    else {
        return Ok(Vec::new());
    };
    let resolve = resolve
        .fields()
        .ok_or_else(|| format!("cannot statically resolve electron-vite {scope}.resolve"))?;
    let Some(data) = resolve.get("alias") else {
        return Ok(Vec::new());
    };
    let setting = format!("electron-vite {scope}.resolve.alias");
    let pairs: Vec<(Data, Data)> = match data {
        Data::Object(fields) => fields
            .iter()
            .map(|(key, value)| (Data::Literal(Value::String(key.clone())), value.clone()))
            .collect(),
        Data::Array(items) => items
            .iter()
            .map(|item| {
                if item.property("customResolver").is_some() {
                    return Err(format!("unsupported custom resolver in {setting}"));
                }
                Ok((
                    item.property("find").unwrap_or_default(),
                    item.property("replacement").unwrap_or_default(),
                ))
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(format!("cannot statically resolve {setting}")),
    };
    let mut aliases = Vec::new();
    for (find, replacement) in pairs {
        let (Some(Value::String(find)), Some(Value::String(target))) =
            (find.json(true), replacement.json(true))
        else {
            return Err(format!("cannot statically resolve {setting} entry"));
        };
        // String rules have exact/prefix semantics, not wildcard semantics.
        if find.is_empty() || find.contains('*') || target.contains('*') {
            return Err(format!("unsupported string pattern in {setting}: {find:?}"));
        }
        let kind = if Path::new(&target).is_absolute() {
            AliasKind::Path
        } else {
            AliasKind::Specifier
        };
        aliases.push(Alias {
            pattern: find.clone(),
            target: target.clone(),
            kind,
        });
        aliases.push(Alias {
            pattern: format!("{find}/*"),
            target: format!("{target}/*"),
            kind,
        });
    }
    Ok(aliases)
}

fn definitions(data: &Data, scope: &str) -> Result<Vec<(String, String)>, String> {
    let Some(defines) = data
        .property(scope)
        .and_then(|section| section.property("define"))
    else {
        return Ok(Vec::new());
    };
    let fields = defines
        .fields()
        .ok_or_else(|| format!("cannot statically resolve electron-vite {scope}.define"))?;
    fields
        .into_iter()
        .map(|(name, value)| {
            let code = match &value {
                Data::Literal(Value::String(code)) => code.clone(),
                value => value.stringify().ok_or_else(|| {
                    format!("cannot statically resolve electron-vite {scope}.define.{name}")
                })?,
            };
            Ok((name, code))
        })
        .collect()
}

fn config(root: &Path) -> Result<Config, String> {
    let Some(path) = first_existing(root, CONFIGS) else {
        return Ok(Config {
            data: Value::Object(Map::new()),
            defines: Default::default(),
            aliases: Default::default(),
        });
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let result = crate::config::evaluate::exported(&path, &source, root, "electron-vite config")?;
    let data = result.json(false).unwrap();
    if !data.is_object() {
        return Err(format!(
            "cannot statically resolve exported electron-vite config {}",
            path.display()
        ));
    }
    Ok(Config {
        data,
        defines: [
            definitions(&result, "main")?,
            definitions(&result, "preload")?,
            definitions(&result, "renderer")?,
        ],
        aliases: [
            aliases(&result, "main")?,
            aliases(&result, "preload")?,
            aliases(&result, "renderer")?,
        ],
    })
}

fn text(value: Option<&Value>, fallback: &str, setting: &str) -> Result<String, String> {
    match value {
        None => Ok(fallback.to_owned()),
        Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(format!("cannot statically resolve electron-vite {setting}")),
    }
}

fn inputs(input: &Value) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    let mut add = |name: Option<&str>, input: &Value| -> Result<(), String> {
        let path = text(Some(input), "", "build input")?;
        let name = name.map(str::to_owned).unwrap_or_else(|| {
            Path::new(&path)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        entries.push((name, path));
        Ok(())
    };
    match input {
        Value::Array(items) => {
            for item in items {
                add(None, item)?;
            }
        }
        Value::Object(items) => {
            for (name, item) in items {
                add(Some(name), item)?;
            }
        }
        _ => add(None, input)?,
    }
    Ok(entries)
}

fn rollup(section: &Value) -> &Value {
    section["build"]
        .get("rolldownOptions")
        .unwrap_or(&section["build"]["rollupOptions"])
}

fn configured_input(section: &Value) -> Option<&Value> {
    rollup(section)
        .get("input")
        .or_else(|| section["build"]["lib"].get("entry"))
}

fn scripts(
    root: &Path,
    resolver: &Resolver,
    section: &Value,
    scope: &str,
) -> Result<Vec<Entry>, String> {
    let input = match configured_input(section) {
        Some(input) => inputs(input)?,
        None => {
            let names = ["index", scope];
            let mut source = None;
            for name in names {
                for extension in ["js", "ts", "mjs", "cjs", "mts", "cts"] {
                    let path = root.join(format!("src/{scope}/{name}.{extension}"));
                    if path.is_file() {
                        source = Some(path);
                        break;
                    }
                }
                if source.is_some() {
                    break;
                }
            }
            source
                .map(|source| {
                    (
                        source.file_stem().unwrap().to_string_lossy().into_owned(),
                        relative(root, &source),
                    )
                })
                .into_iter()
                .collect()
        }
    };
    let directory = text(
        section["build"].get("outDir"),
        &format!("out/{scope}"),
        "build.outDir",
    )?;
    let output = rollup(section).get("output").unwrap_or(&Value::Null);
    let output = output
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(output);
    let pattern = output.get("entryFileNames");
    let mut entries = Vec::new();
    for (name, input) in input {
        let source = resolver
            .resolve_path(&root.join(&input))
            .ok_or_else(|| format!("cannot find electron-vite {scope} input \"{input}\""))?;
        let names = if pattern.is_some() {
            vec![text(pattern, "", "output.entryFileNames")?.replace("[name]", &name)]
        } else if scope == "preload" {
            vec![format!("{name}.js"), format!("{name}.mjs")]
        } else {
            vec![format!("{name}.js")]
        };
        let mut runtime_paths = Vec::new();
        for name in names {
            if name.contains('[') {
                return Err(format!(
                    "cannot statically resolve electron-vite output name \"{name}\""
                ));
            }
            runtime_paths.push(relative(root, &root.join(&directory).join(name)));
        }
        entries.push(Entry {
            source,
            runtime_paths,
        });
    }
    Ok(entries)
}

#[derive(Debug)]
pub(crate) struct Entries {
    pub main: Option<Entry>,
    pub preloads: Vec<Entry>,
    pub pages: Vec<Entry>,
    pub renderer_root: PathBuf,
    pub renderer_output: PathBuf,
    pub renderer_transforms: bool,
    pub defines: [Vec<(String, String)>; 3],
    pub aliases: [Vec<Alias>; 3],
}

pub(crate) fn discover(
    root: &Path,
    resolver: &Resolver,
    main_field: &str,
) -> Result<Entries, String> {
    let Config {
        data: config,
        defines,
        aliases,
    } = config(root)?;
    for scope in ["main", "preload", "renderer"] {
        let section = &config[scope];
        for (setting, item) in [
            (scope.to_owned(), config.get(scope)),
            (format!("{scope}.build"), section.get("build")),
            (
                format!("{scope}.build.rollupOptions"),
                section["build"].get("rollupOptions"),
            ),
            (
                format!("{scope}.build.rolldownOptions"),
                section["build"].get("rolldownOptions"),
            ),
        ] {
            if item.is_some_and(|value| !value.is_object()) {
                return Err(format!("cannot statically resolve electron-vite {setting}"));
            }
        }
    }
    let mut mains = scripts(root, resolver, &config["main"], "main")?;
    let main = if mains.len() <= 1 {
        mains.pop()
    } else {
        let index = mains
            .iter()
            .position(|entry| entry.runtime_paths.iter().any(|path| path == main_field))
            .ok_or_else(|| {
                format!("package main \"{main_field}\" does not select an electron-vite main input")
            })?;
        Some(mains.remove(index))
    };
    let preloads = scripts(root, resolver, &config["preload"], "preload")?;
    let renderer = &config["renderer"];
    let renderer_transforms = renderer
        .get("plugins")
        .is_some_and(|plugins| !matches!(plugins, Value::Array(items) if items.is_empty()))
        || renderer.get("css").is_some()
        || renderer.get("assetsInclude").is_some();
    let renderer_root =
        normalize(&root.join(text(renderer.get("root"), "src/renderer", "renderer.root")?));
    let directory = root.join(text(
        renderer["build"].get("outDir"),
        "out/renderer",
        "renderer.build.outDir",
    )?);
    let sources = match configured_input(renderer) {
        Some(input) => inputs(input)?
            .into_iter()
            .map(|(_, input)| {
                let source = normalize(&root.join(&input));
                if source.is_file() && crate::is_html(&source) {
                    Ok(source)
                } else {
                    Err(format!(
                        "cannot find electron-vite renderer HTML input \"{input}\""
                    ))
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => html_files(&renderer_root),
    };
    let pages = sources
        .into_iter()
        .map(|source| {
            let output = source
                .strip_prefix(&renderer_root)
                .map_or_else(|_| PathBuf::from(file_name(&source)), Path::to_path_buf);
            Entry {
                runtime_paths: vec![relative(root, &directory.join(output))],
                source,
            }
        })
        .collect();
    Ok(Entries {
        main,
        preloads,
        pages,
        renderer_root,
        renderer_output: directory,
        renderer_transforms,
        defines,
        aliases,
    })
}

#[cfg(test)]
mod tests;
