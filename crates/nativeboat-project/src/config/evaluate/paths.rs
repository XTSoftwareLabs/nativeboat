//! Whitelisted configuration helpers, evaluated as data without loading Node.

use super::{Data, value};
use indexmap::IndexMap;
use oxc::ast::ast::Argument;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Function {
    DefineConfig,
    Stringify,
    Join,
    Resolve,
    Dirname,
    FileUrlToPath,
    Url,
    Require,
}

fn text(text: impl Into<String>) -> Data {
    Data::Literal(Value::String(text.into()))
}

pub(crate) fn module(name: &str) -> Option<Data> {
    let functions: &[(&str, Function)] = match name.strip_prefix("node:").unwrap_or(name) {
        "path" => &[
            ("join", Function::Join),
            ("resolve", Function::Resolve),
            ("dirname", Function::Dirname),
        ],
        "url" => &[
            ("fileURLToPath", Function::FileUrlToPath),
            ("URL", Function::Url),
        ],
        "electron-vite" | "vite" => &[("defineConfig", Function::DefineConfig)],
        _ => return None,
    };
    Some(Data::Object(
        functions
            .iter()
            .map(|(name, function)| (name.to_string(), Data::Function(*function)))
            .collect(),
    ))
}

pub(crate) fn bindings(path: &Path) -> BTreeMap<String, Data> {
    let directory = path.parent().unwrap_or(Path::new("."));
    let meta = url::Url::from_file_path(path)
        .ok()
        .map(|url| Data::Object(IndexMap::from([("url".to_owned(), text(url.to_string()))])))
        .unwrap_or_default();
    BTreeMap::from([
        ("__dirname".to_owned(), text(directory.to_string_lossy())),
        ("__filename".to_owned(), text(path.to_string_lossy())),
        ("import.meta".to_owned(), meta),
        ("\0cwd".to_owned(), text(directory.to_string_lossy())),
        ("URL".to_owned(), Data::Function(Function::Url)),
        ("require".to_owned(), Data::Function(Function::Require)),
        (
            "defineConfig".to_owned(),
            Data::Function(Function::DefineConfig),
        ),
        (
            "JSON".to_owned(),
            Data::Object(IndexMap::from([(
                "stringify".to_owned(),
                Data::Function(Function::Stringify),
            )])),
        ),
    ])
}

/// Node path normalization retains an absolute root when traversing above it.
fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match result.components().next_back() {
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                _ => result.push(".."),
            },
            other => result.push(other),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

pub(super) fn call(
    function: Function,
    arguments: &[Argument<'_>],
    bindings: &BTreeMap<String, Data>,
) -> Data {
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| {
            argument
                .as_expression()
                .map_or(Data::Unknown, |expression| value(expression, bindings))
        })
        .collect();
    match (function, arguments.as_slice()) {
        (Function::Stringify, [data]) => data.stringify().map(text).unwrap_or_default(),
        (Function::Require, [Data::Literal(Value::String(name))]) => {
            module(name).unwrap_or_default()
        }
        (Function::Dirname, [Data::Literal(Value::String(path))]) => {
            let path = Path::new(path);
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    if path.has_root() {
                        path
                    } else {
                        Path::new(".")
                    }
                });
            text(parent.to_string_lossy())
        }
        (Function::Join | Function::Resolve, _) => {
            let Some(parts) = arguments
                .iter()
                .map(|part| match part {
                    Data::Literal(Value::String(part)) => Some(part.as_str()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
            else {
                return Data::Unknown;
            };
            let path = if function == Function::Join {
                PathBuf::from(
                    parts
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join("/"),
                )
            } else {
                let Some(Data::Literal(Value::String(root))) = bindings.get("\0cwd") else {
                    return Data::Unknown;
                };
                let mut path = PathBuf::from(root);
                for part in parts {
                    path.push(part);
                }
                path
            };
            text(normalize(&path).to_string_lossy().replace('\\', "/"))
        }
        (Function::FileUrlToPath, [data]) => {
            let Some(Value::String(input)) = data.json(true) else {
                return Data::Unknown;
            };
            let Ok(url) = url::Url::parse(&input) else {
                return Data::Unknown;
            };
            // Node rejects encoded separators; decoding them would change the path.
            let lower = url.path().to_ascii_lowercase();
            if lower.contains("%2f") || (cfg!(windows) && lower.contains("%5c")) {
                return Data::Unknown;
            }
            url.to_file_path()
                .ok()
                .and_then(|path| path.to_str().map(str::to_owned))
                .map(text)
                .unwrap_or_default()
        }
        (Function::Url, [input] | [input, _]) => {
            let Some(Value::String(input)) = input.json(true) else {
                return Data::Unknown;
            };
            let base = if let Some(base) = arguments.get(1) {
                let Some(Value::String(base)) = base.json(true) else {
                    return Data::Unknown;
                };
                let Ok(base) = url::Url::parse(&base) else {
                    return Data::Unknown;
                };
                Some(base)
            } else {
                None
            };
            url::Url::options()
                .base_url(base.as_ref())
                .parse(&input)
                .map(Data::Url)
                .unwrap_or_default()
        }
        _ => Data::Unknown,
    }
}
