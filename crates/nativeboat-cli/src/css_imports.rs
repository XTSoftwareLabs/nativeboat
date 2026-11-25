//! Stylesheet `@import`s, inlined at build time as Vite inlines them: the
//! imported sheet's rules take the place of the rule, inside `@layer`,
//! `@supports` and `@media` blocks when the rule names a layer, a support
//! condition or media. Specifiers are relative to the sheet, or packages,
//! resolved with the `style` condition and field. Each sheet's local
//! `url()`s are inlined against its own folder before it is inlined.

use nativeboat_bundler::resolve::{Alias, Resolved, Resolver, Target};
use std::path::{Path, PathBuf};

/// A sheet with its imports inlined, and the problems found.
pub(crate) fn inline(
    path: &Path,
    text: &str,
    root: &Path,
    aliases: &[Alias],
) -> (String, Vec<String>) {
    let resolver = Resolver {
        root: root.to_path_buf(),
        target: Target::Style,
        aliases: aliases.to_vec(),
    };
    let mut errors = Vec::new();
    let mut stack = vec![nativeboat_bundler::resolve::realpath(path)];
    let text = expand(path, text, &resolver, &mut stack, &mut errors);
    (text, errors)
}

fn expand(
    path: &Path,
    text: &str,
    resolver: &Resolver,
    stack: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> String {
    let text = crate::native::inline_css_assets(path, text);
    let mut output = String::with_capacity(text.len());
    let mut rest = text.as_str();
    loop {
        let (space, statement) = leading(rest);
        output.push_str(space);
        rest = &rest[space.len()..];
        let Some(statement) = statement else {
            break;
        };
        rest = &rest[statement.len()..];
        if statement.starts_with("@charset") {
            continue;
        }
        if !statement.starts_with("@import") {
            output.push_str(statement);
            continue;
        }
        match import(path, statement, resolver, stack, errors) {
            Some(inlined) => output.push_str(&inlined),
            // Remote sheets stay, for the stylesheet checks to report.
            None => output.push_str(statement),
        }
    }
    output.push_str(rest);
    output
}

/// The whitespace and comments before the next statement, and the next
/// statement if it is an `@charset`, `@import` or `@layer` statement: the
/// rules that may precede imports.
fn leading(text: &str) -> (&str, Option<&str>) {
    let mut index = 0;
    let bytes = text.as_bytes();
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if text[index..].starts_with("/*") {
            index = text[index + 2..]
                .find("*/")
                .map_or(text.len(), |end| index + 2 + end + 2);
            continue;
        }
        break;
    }
    let rest = &text[index..];
    let lower: String = rest
        .chars()
        .take(9)
        .collect::<String>()
        .to_ascii_lowercase();
    let statement = ["@charset", "@import", "@layer"]
        .iter()
        .any(|keyword| lower.starts_with(keyword))
        .then(|| statement_end(rest))
        .flatten()
        .map(|end| &rest[..end]);
    // `@layer name { ... }` is a block, which ends the leading statements.
    (
        &text[..index],
        statement.filter(|statement| !statement.contains('{')),
    )
}

/// The length of a statement up to and including its `;`, skipping strings
/// and parentheses.
fn statement_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut characters = text.char_indices();
    while let Some((index, character)) = characters.next() {
        match (quote, character) {
            (Some(_), '\\') => {
                characters.next();
            }
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, '{') if depth == 0 => return Some(index),
            (None, ';') if depth == 0 => return Some(index + 1),
            _ => {}
        }
    }
    None
}

/// The parts of an `@import` rule: its URL, layer, support condition and
/// media.
struct Import {
    url: String,
    layer: Option<String>,
    supports: Option<String>,
    media: String,
}

fn parse(statement: &str) -> Option<Import> {
    let body = statement["@import".len()..]
        .trim()
        .trim_end_matches(';')
        .trim();
    let (url, rest) = if let Some(rest) = body.strip_prefix("url(") {
        let end = rest.find(')')?;
        let url = rest[..end].trim().trim_matches(|c| c == '"' || c == '\'');
        (url.to_owned(), &rest[end + 1..])
    } else {
        let quote = body.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let end = body[1..].find(quote)? + 1;
        (body[1..end].to_owned(), &body[end + 1..])
    };
    let mut rest = rest.trim();
    let mut layer = None;
    if let Some(after) = rest.strip_prefix("layer(") {
        let end = after.find(')')?;
        layer = Some(after[..end].trim().to_owned());
        rest = after[end + 1..].trim();
    } else if rest == "layer" || rest.starts_with("layer ") {
        layer = Some(String::new());
        rest = rest["layer".len()..].trim();
    }
    let mut supports = None;
    if let Some(after) = rest.strip_prefix("supports(") {
        let mut depth = 1usize;
        let end = after.char_indices().find_map(|(index, character)| {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(index)
        })?;
        supports = Some(after[..end].trim().to_owned());
        rest = after[end + 1..].trim();
    }
    Some(Import {
        url,
        layer,
        supports,
        media: rest.to_owned(),
    })
}

fn import(
    path: &Path,
    statement: &str,
    resolver: &Resolver,
    stack: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> Option<String> {
    let Some(rule) = parse(statement) else {
        errors.push(format!("{}: invalid @import {statement}", path.display()));
        return Some(String::new());
    };
    if rule.url.contains("://") || rule.url.starts_with("//") || rule.url.starts_with("data:") {
        return None;
    }
    let Some(file) = resolve(&rule.url, path, resolver) else {
        errors.push(format!(
            "{}: cannot resolve stylesheet \"{}\"",
            path.display(),
            rule.url
        ));
        return Some(String::new());
    };
    let file = nativeboat_bundler::resolve::realpath(&file);
    // A sheet imported again inside itself adds nothing.
    if stack.contains(&file) {
        return Some(String::new());
    }
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("{}: {error}", file.display()));
            return Some(String::new());
        }
    };
    stack.push(file.clone());
    let mut inlined = expand(&file, &text, resolver, stack, errors);
    stack.pop();
    if !rule.media.is_empty() {
        inlined = format!("@media {} {{\n{inlined}\n}}", rule.media);
    }
    if let Some(condition) = rule.supports {
        inlined = format!("@supports ({condition}) {{\n{inlined}\n}}");
    }
    if let Some(layer) = rule.layer {
        let name = if layer.is_empty() {
            String::new()
        } else {
            format!(" {layer}")
        };
        inlined = format!("@layer{name} {{\n{inlined}\n}}");
    }
    Some(format!("\n{inlined}\n"))
}

/// A sheet's file: relative to the importing sheet first, as a URL is, then
/// a module specifier, such as a package or an alias.
fn resolve(url: &str, from: &Path, resolver: &Resolver) -> Option<PathBuf> {
    let url = url.strip_prefix('~').unwrap_or(url);
    let directory = from.parent().unwrap_or_else(|| Path::new(""));
    let relative = directory.join(url);
    if relative.is_file() {
        return Some(relative);
    }
    if !url.ends_with(".css") {
        let with_extension = directory.join(format!("{url}.css"));
        if with_extension.is_file() {
            return Some(with_extension);
        }
    }
    match resolver.resolve(url, from) {
        Ok(Resolved::File(file)) => Some(file),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
