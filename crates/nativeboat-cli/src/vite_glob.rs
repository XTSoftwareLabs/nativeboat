//! Vite's `import.meta.glob`: a call with literal patterns becomes an object
//! of the files it matches, as Vite's build transforms it. Each value is a
//! lazy `() => import(path)` loader, or with `eager: true` the module itself,
//! imported statically. The `import` option selects one export and `query`
//! is appended to each import path. Keys are the paths as the patterns name
//! them: relative to the importing module, or from the project root for
//! patterns starting with `/`.

use std::path::Path;

/// One `import.meta.glob(...)` call: its span and literal arguments.
struct Call {
    start: usize,
    end: usize,
    patterns: Vec<String>,
    eager: bool,
    import: Option<String>,
    query: Option<String>,
}

/// The module source with its `import.meta.glob` calls expanded; the source
/// unchanged when it has none, or none with literal arguments.
pub(crate) fn expand(path: &Path, root: &Path, source: &str) -> String {
    if !source.contains("import.meta.glob") {
        return source.to_owned();
    }
    let calls = calls(path, source);
    if calls.is_empty() {
        return source.to_owned();
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    // Each call's eager imports, kept in source order.
    let mut imports = vec![String::new(); calls.len()];
    let mut expanded = source.to_owned();
    for (index, call) in calls.iter().enumerate().rev() {
        let files = matches(directory, root, &call.patterns);
        let mut entries = Vec::new();
        for (position, (key, specifier)) in files.iter().enumerate() {
            let specifier = match &call.query {
                Some(query) => format!("{specifier}?{query}"),
                None => specifier.clone(),
            };
            let value = if call.eager {
                let local = format!("__nativeboat_glob_{index}_{position}");
                imports[index].push_str(&match &call.import {
                    Some(name) => format!("import {{ {name} as {local} }} from {specifier:?};\n"),
                    None => format!("import * as {local} from {specifier:?};\n"),
                });
                local
            } else {
                match &call.import {
                    Some(name) => {
                        format!("() => import({specifier:?}).then((module) => module[{name:?}])")
                    }
                    None => format!("() => import({specifier:?})"),
                }
            };
            entries.push(format!("{key:?}: {value}"));
        }
        // Lines are kept inside the expression, so later source positions are
        // unchanged and it still ends where the call ended.
        let newlines = expanded[call.start..call.end].matches('\n').count();
        let replacement = format!(
            "Object.assign(Object.create(null), {{{}{}}})",
            entries.join(", "),
            "\n".repeat(newlines)
        );
        expanded.replace_range(call.start..call.end, &replacement);
    }
    // Hoisted like any import, so prepending keeps evaluation order.
    imports.concat() + &expanded
}

/// The literal `import.meta.glob` calls of a module.
fn calls(path: &Path, source: &str) -> Vec<Call> {
    use oxc::ast::ast::{Argument, Expression, ObjectPropertyKind, PropertyKey};
    use oxc::ast_visit::{Visit, walk};
    use oxc::span::SourceType;

    struct Calls(Vec<Call>);

    fn string(expression: &Expression<'_>) -> Option<String> {
        match expression {
            Expression::StringLiteral(literal) => Some(literal.value.to_string()),
            Expression::TemplateLiteral(template) if template.expressions.is_empty() => template
                .quasis
                .first()
                .map(|quasi| quasi.value.raw.to_string()),
            _ => None,
        }
    }

    impl<'a> Visit<'a> for Calls {
        fn visit_call_expression(&mut self, call: &oxc::ast::ast::CallExpression<'a>) {
            if let Expression::StaticMemberExpression(member) = call.callee.without_parentheses()
                && member.property.name == "glob"
                && matches!(
                    member.object.without_parentheses(),
                    Expression::ImportMeta(_)
                )
                && let Some(first) = call.arguments.first().and_then(Argument::as_expression)
            {
                let patterns = match first {
                    Expression::ArrayExpression(array) => array
                        .elements
                        .iter()
                        .map(|element| element.as_expression().and_then(string))
                        .collect::<Option<Vec<_>>>(),
                    expression => string(expression).map(|pattern| vec![pattern]),
                };
                let mut glob = patterns.map(|patterns| Call {
                    start: call.span.start as usize,
                    end: call.span.end as usize,
                    patterns,
                    eager: false,
                    import: None,
                    query: None,
                });
                if let (Some(found), Some(Argument::ObjectExpression(options))) =
                    (glob.as_mut(), call.arguments.get(1))
                {
                    for property in &options.properties {
                        let ObjectPropertyKind::ObjectProperty(property) = property else {
                            glob = None;
                            break;
                        };
                        let name = match &property.key {
                            PropertyKey::StaticIdentifier(name) => name.name.to_string(),
                            PropertyKey::StringLiteral(name) => name.value.to_string(),
                            _ => String::new(),
                        };
                        match (name.as_str(), &property.value) {
                            ("eager", Expression::BooleanLiteral(value)) => {
                                found.eager = value.value
                            }
                            ("import", value) if string(value).is_some() => {
                                found.import = string(value)
                            }
                            ("query", value) if string(value).is_some() => {
                                found.query = string(value)
                                    .map(|query| query.trim_start_matches('?').to_owned());
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(glob) = glob
                    && !glob.patterns.is_empty()
                {
                    self.0.push(glob);
                }
            }
            walk::walk_call_expression(self, call);
        }
    }

    let Ok(source_type) = SourceType::from_path(path) else {
        return Vec::new();
    };
    let allocator = oxc::allocator::Allocator::default();
    let parsed = oxc::parser::Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Vec::new();
    }
    let mut calls = Calls(Vec::new());
    calls.visit_program(&parsed.program);
    calls.0.sort_by_key(|call| call.start);
    calls.0
}

/// The files the patterns match, as `(key, import path)` pairs in path order.
/// Keys and paths are lexical: symbolic links such as pnpm's package links
/// keep the names the patterns use.
fn matches(directory: &Path, root: &Path, patterns: &[String]) -> Vec<(String, String)> {
    let (negative, positive): (Vec<&String>, Vec<&String>) = patterns
        .iter()
        .partition(|pattern| pattern.starts_with('!'));
    let excluded: Vec<Vec<String>> = negative
        .iter()
        .flat_map(|pattern| braces(&pattern[1..]))
        .map(|pattern| segments(&pattern))
        .collect();
    let mut found = Vec::new();
    for pattern in positive.iter().flat_map(|pattern| braces(pattern)) {
        let absolute = pattern.starts_with('/');
        let base = if absolute { root } else { directory };
        let parts = segments(&pattern);
        let fixed: Vec<String> = parts
            .iter()
            .take_while(|part| !is_glob(part))
            .cloned()
            .collect();
        let start = fixed
            .iter()
            .fold(base.to_path_buf(), |path, part| path.join(part));
        let modules = pattern.contains("node_modules");
        let mut files = Vec::new();
        walk(&start, &mut Vec::new(), &mut files);
        for below in files {
            let mut candidate = fixed.clone();
            candidate.extend(below);
            if !modules && candidate.iter().any(|part| part == "node_modules") {
                continue;
            }
            if !matches_segments(&parts, &candidate)
                || excluded
                    .iter()
                    .any(|excluded| matches_segments(excluded, &candidate))
            {
                continue;
            }
            let joined = candidate.join("/");
            let key = if absolute {
                format!("/{joined}")
            } else if joined.starts_with("../") {
                joined
            } else {
                format!("./{joined}")
            };
            let specifier = if absolute {
                lexical_relative(
                    directory,
                    &candidate
                        .iter()
                        .fold(root.to_path_buf(), |path, part| path.join(part)),
                )
            } else {
                key.clone()
            };
            found.push((key, specifier));
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Every file below a directory, as the names from it.
fn walk(directory: &Path, prefix: &mut Vec<String>, files: &mut Vec<Vec<String>>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        prefix.push(name);
        if path.is_dir() {
            walk(&path, prefix, files);
        } else {
            files.push(prefix.clone());
        }
        prefix.pop();
    }
}

/// An import path from `directory` to `file`, both absolute and compared
/// without resolving links.
fn lexical_relative(directory: &Path, file: &Path) -> String {
    let normalize = |path: &Path| -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    parts.pop();
                }
                std::path::Component::CurDir => {}
                component => {
                    parts.push(component.as_os_str().to_string_lossy().to_ascii_lowercase())
                }
            }
        }
        parts
    };
    let from = normalize(directory);
    let to: Vec<String> = file
        .components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let lower: Vec<String> = normalize(file);
    let common = from
        .iter()
        .zip(&lower)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec!["..".to_owned(); from.len() - common];
    parts.extend(to[to.len() - (lower.len() - common)..].iter().cloned());
    let joined = parts.join("/");
    if joined.starts_with("../") {
        joined
    } else {
        format!("./{joined}")
    }
}

/// Path segments without `.` and empty parts; `..` stays.
fn segments(pattern: &str) -> Vec<String> {
    pattern
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_owned)
        .collect()
}

fn is_glob(part: &str) -> bool {
    part.contains(['*', '?', '[', '{'])
}

/// The patterns `{a,b}` alternatives stand for.
fn braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_owned()];
    };
    let mut depth = 0;
    let mut close = None;
    let mut commas = Vec::new();
    for (index, character) in pattern[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + index);
                    break;
                }
            }
            ',' if depth == 1 => commas.push(open + index),
            _ => {}
        }
    }
    let Some(close) = close else {
        return vec![pattern.to_owned()];
    };
    let mut bounds = vec![open];
    bounds.extend(commas);
    bounds.push(close);
    bounds
        .windows(2)
        .flat_map(|bound| {
            let alternative = &pattern[bound[0] + 1..bound[1]];
            braces(&format!(
                "{}{alternative}{}",
                &pattern[..open],
                &pattern[close + 1..]
            ))
        })
        .collect()
}

/// Whether path segments match pattern segments; `**` matches any number of
/// segments.
fn matches_segments(pattern: &[String], path: &[String]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=path.len()).any(|skip| matches_segments(rest, &path[skip..]))
        }
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(part, others)| wildcard(first, part) && matches_segments(rest, others)),
    }
}

/// Whether one segment matches a pattern of `*`, `?` and `[...]` classes.
fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some(('*', rest)) => (0..=text.len()).any(|skip| matches(rest, &text[skip..])),
            Some(('?', rest)) => !text.is_empty() && matches(rest, &text[1..]),
            Some(('[', rest)) => {
                let Some(close) = rest.iter().position(|character| *character == ']') else {
                    return text.first() == Some(&'[') && matches(rest, &text[1..]);
                };
                let Some(character) = text.first() else {
                    return false;
                };
                let (negated, class) = match rest[..close].split_first() {
                    Some(('!' | '^', class)) => (true, class),
                    _ => (false, &rest[..close]),
                };
                let mut found = false;
                let mut index = 0;
                while index < class.len() {
                    if index + 2 < class.len() && class[index + 1] == '-' {
                        found |= (class[index]..=class[index + 2]).contains(character);
                        index += 3;
                    } else {
                        found |= class[index] == *character;
                        index += 1;
                    }
                }
                found != negated && matches(&rest[close + 1..], &text[1..])
            }
            Some((character, rest)) => text.first() == Some(character) && matches(rest, &text[1..]),
        }
    }
    matches(&pattern, &text)
}

#[cfg(test)]
mod tests;
