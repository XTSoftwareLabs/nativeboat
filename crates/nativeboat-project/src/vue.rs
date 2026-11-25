//! The top-level blocks of Vue single-file components.

/// What a top-level block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// `<template>`
    Template,
    /// `<script>` or `<script setup>`
    Script,
    /// `<style>`
    Style,
    /// Any other block, such as `<i18n>`.
    Custom,
}

/// A top-level block of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'s> {
    /// The kind of block.
    pub kind: BlockKind,
    /// The `lang` attribute, lowercased.
    pub lang: Option<String>,
    /// Whether the block has a `src` attribute, naming its content elsewhere.
    pub src: Option<String>,
    /// Whether a style block is `scoped`, so Vue's style compiler scopes
    /// its selectors and rewrites `:deep()`, `:slotted()` and `:global()`.
    pub scoped: bool,
    /// The byte offset of the content in the component source.
    pub start: usize,
    /// The content between the opening and closing tags.
    pub content: &'s str,
}

impl Block<'_> {
    /// The 1-based line of the component on which the content starts.
    pub fn line(&self, source: &str) -> u32 {
        source[..self.start].matches('\n').count() as u32 + 1
    }
}

/// The end of the opening tag starting at `open`, after its `>`, skipping
/// quoted attribute values.
fn opening_end(source: &str, open: usize) -> Option<usize> {
    let mut quote = None;
    for (index, character) in source[open..].char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open_quote), _) if character == open_quote => quote = None,
            (None, '>') => return Some(open + index + 1),
            _ => {}
        }
    }
    None
}

fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    while let Some(start) = rest.find(|c: char| c.is_ascii_alphabetic()) {
        rest = &rest[start..];
        let end = rest
            .find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let key = &rest[..end];
        rest = rest[end..].trim_start();
        let value = if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (value, remaining) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let close = after[1..].find(quote).map_or(after.len(), |at| at + 1);
                    (&after[1..close], after.get(close + 1..).unwrap_or_default())
                }
                _ => {
                    let close = after
                        .find(|c: char| c.is_ascii_whitespace() || c == '>')
                        .unwrap_or(after.len());
                    (&after[..close], &after[close..])
                }
            };
            rest = remaining;
            value
        } else {
            ""
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value.to_owned());
        }
    }
    None
}

/// Where `</name` closes the block whose content starts at `start`. Nested
/// `<template>` elements inside a template are counted.
fn closing(source: &str, name: &str, start: usize) -> Option<usize> {
    let lower = source.to_ascii_lowercase();
    let (open, close) = (format!("<{name}"), format!("</{name}"));
    let mut depth = 1;
    let mut at = start;
    loop {
        let next_close = lower[at..].find(&close)? + at;
        let nested = (name == "template")
            .then(|| lower[at..next_close].find(&open).map(|found| found + at))
            .flatten()
            .filter(|found| {
                lower[found + open.len()..]
                    .starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            });
        match nested {
            Some(found) => {
                depth += 1;
                at = found + open.len();
            }
            None => {
                depth -= 1;
                if depth == 0 {
                    return Some(next_close);
                }
                at = next_close + close.len();
            }
        }
    }
}

/// The top-level blocks of a component, in source order.
pub fn blocks(source: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut at = 0;
    while let Some(found) = source[at..].find('<') {
        let open = at + found;
        let rest = &source[open + 1..];
        if rest.starts_with("!--") {
            at = source[open..]
                .find("-->")
                .map_or(source.len(), |end| open + end + 3);
            continue;
        }
        let length = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .unwrap_or(rest.len());
        let Some(end) = (length > 0).then(|| opening_end(source, open)).flatten() else {
            at = open + 1;
            continue;
        };
        let name = rest[..length].to_ascii_lowercase();
        let attributes = &source[open + 1 + length..end - 1];
        if attributes.trim_end().ends_with('/') {
            at = end;
            continue;
        }
        let Some(close) = closing(source, &name, end) else {
            break;
        };
        blocks.push(Block {
            kind: match name.as_str() {
                "template" => BlockKind::Template,
                "script" => BlockKind::Script,
                "style" => BlockKind::Style,
                _ => BlockKind::Custom,
            },
            lang: attribute(attributes, "lang").map(|lang| lang.to_ascii_lowercase()),
            src: attribute(attributes, "src"),
            scoped: attribute(attributes, "scoped").is_some(),
            start: end,
            content: &source[end..close],
        });
        at = source[close..]
            .find('>')
            .map_or(source.len(), |found| close + found + 1);
    }
    blocks
}

#[cfg(test)]
mod tests;
