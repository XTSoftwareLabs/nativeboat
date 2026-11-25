//! HTML tokenizer.

use crate::prelude::*;
use crate::{
    Attribute, Position, decode_entities, is_escapable_raw_text_element, is_raw_text_element,
};

/// A token and the byte offset where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: TokenKind,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenKind {
    Doctype(String),
    StartTag {
        name: String,
        attributes: Vec<Attribute>,
        self_closing: bool,
    },
    EndTag(String),
    Text(String),
    Comment(String),
}

/// Splits `source` into tokens.
pub(crate) fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut text_start = 0;
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] != b'<' {
            pos += 1;
            continue;
        }
        let Some((kind, end)) = markup(source, pos) else {
            pos += 1;
            continue;
        };
        push_text(&mut tokens, source, text_start, pos, true);
        let start = pos;
        pos = end;
        if let Some(kind) = kind {
            let raw = raw_text_kind(&kind);
            tokens.push(Token {
                kind,
                offset: start,
            });
            if let Some((name, decode)) = raw {
                let content_end = find_raw_end(source, pos, &name);
                push_text(&mut tokens, source, pos, content_end, decode);
                pos = content_end;
            }
        }
        text_start = pos;
    }
    push_text(&mut tokens, source, text_start, bytes.len(), true);
    tokens
}

fn raw_text_kind(kind: &TokenKind) -> Option<(String, bool)> {
    match kind {
        TokenKind::StartTag { name, .. } if is_raw_text_element(name) => {
            Some((name.clone(), false))
        }
        TokenKind::StartTag { name, .. } if is_escapable_raw_text_element(name) => {
            Some((name.clone(), true))
        }
        _ => None,
    }
}

fn push_text(tokens: &mut Vec<Token>, source: &str, start: usize, end: usize, decode: bool) {
    if start >= end {
        return;
    }
    let raw = &source[start..end];
    let text = if decode {
        decode_entities(raw).into_owned()
    } else {
        raw.to_owned()
    };
    if let Some(Token {
        kind: TokenKind::Text(previous),
        ..
    }) = tokens.last_mut()
    {
        previous.push_str(&text);
        return;
    }
    tokens.push(Token {
        kind: TokenKind::Text(text),
        offset: start,
    });
}

/// Finds the end of raw text content: the start of the matching end tag.
fn find_raw_end(source: &str, from: usize, name: &str) -> usize {
    let bytes = source.as_bytes();
    let mut pos = from;
    while let Some(found) = source[pos..].find("</") {
        let start = pos + found;
        let name_end = start + 2 + name.len();
        if name_end <= bytes.len()
            // Bytes: text such as `</中` may follow, which a string slice
            // could cut inside a character.
            && bytes[start + 2..name_end].eq_ignore_ascii_case(name.as_bytes())
            && (name_end == bytes.len()
                || matches!(bytes[name_end], b'>' | b'/')
                || bytes[name_end].is_ascii_whitespace())
        {
            return start;
        }
        pos = start + 2;
    }
    bytes.len()
}

/// Parses the markup starting with `<` at `pos`.
///
/// Returns `None` when the `<` is literal text, otherwise the token (if any)
/// and the offset after the markup.
fn markup(source: &str, pos: usize) -> Option<(Option<TokenKind>, usize)> {
    let bytes = source.as_bytes();
    let next = *bytes.get(pos + 1)?;
    if source[pos + 1..].starts_with("!--") {
        let body = pos + 4;
        return Some(match source[body..].find("-->") {
            Some(end) => (
                Some(TokenKind::Comment(source[body..body + end].to_owned())),
                body + end + 3,
            ),
            None => (
                Some(TokenKind::Comment(source[body..].to_owned())),
                bytes.len(),
            ),
        });
    }
    if next == b'!' || next == b'?' {
        let end = find_byte(bytes, pos + 2, b'>');
        let content = &source[pos + 2..end.min(bytes.len())];
        let token = match content.get(..7) {
            Some(keyword) if next == b'!' && keyword.eq_ignore_ascii_case("doctype") => {
                let name = content[7..]
                    .split_ascii_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                TokenKind::Doctype(name)
            }
            _ => TokenKind::Comment(content.to_owned()),
        };
        return Some((Some(token), (end + 1).min(bytes.len())));
    }
    if next == b'/' {
        let after = *bytes.get(pos + 2)?;
        if after == b'>' {
            return Some((None, pos + 3));
        }
        let end = find_byte(bytes, pos + 2, b'>');
        let content = &source[pos + 2..end.min(bytes.len())];
        let token = if after.is_ascii_alphabetic() {
            let name_end = content
                .find(|c: char| c.is_ascii_whitespace() || c == '/')
                .unwrap_or(content.len());
            TokenKind::EndTag(content[..name_end].to_ascii_lowercase())
        } else {
            TokenKind::Comment(content.to_owned())
        };
        return Some((Some(token), (end + 1).min(bytes.len())));
    }
    if next.is_ascii_alphabetic() {
        let (token, end) = start_tag(source, pos + 1);
        return Some((Some(token), end));
    }
    None
}

fn find_byte(bytes: &[u8], from: usize, byte: u8) -> usize {
    bytes[from.min(bytes.len())..]
        .iter()
        .position(|candidate| *candidate == byte)
        .map_or(bytes.len(), |index| from + index)
}

fn is_tag_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || byte == b'/' || byte == b'>'
}

/// Parses a start tag whose name begins at `pos`.
fn start_tag(source: &str, mut pos: usize) -> (TokenKind, usize) {
    let bytes = source.as_bytes();
    let name_start = pos;
    while pos < bytes.len() && !is_tag_delimiter(bytes[pos]) {
        pos += 1;
    }
    let name = source[name_start..pos].to_ascii_lowercase();
    let mut attributes: Vec<Attribute> = Vec::new();
    let mut self_closing = false;
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let Some(&byte) = bytes.get(pos) else {
            break;
        };
        if byte == b'>' {
            pos += 1;
            break;
        }
        if byte == b'/' {
            pos += 1;
            if bytes.get(pos) == Some(&b'>') {
                self_closing = true;
                pos += 1;
                break;
            }
            continue;
        }
        let attribute_start = pos;
        pos += 1;
        while pos < bytes.len() && !is_tag_delimiter(bytes[pos]) && bytes[pos] != b'=' {
            pos += 1;
        }
        let attribute_name = source[attribute_start..pos].to_ascii_lowercase();
        let mut after_name = pos;
        while after_name < bytes.len() && bytes[after_name].is_ascii_whitespace() {
            after_name += 1;
        }
        let mut value = String::new();
        if bytes.get(after_name) == Some(&b'=') {
            pos = after_name + 1;
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            let (raw, end) = attribute_value(source, pos);
            value = decode_entities(raw).into_owned();
            pos = end;
        }
        if !attributes
            .iter()
            .any(|existing| existing.name == attribute_name)
        {
            attributes.push(Attribute {
                name: attribute_name,
                value,
            });
        }
    }
    (
        TokenKind::StartTag {
            name,
            attributes,
            self_closing,
        },
        pos,
    )
}

/// Reads an attribute value starting at `pos`; returns the raw value and end.
fn attribute_value(source: &str, pos: usize) -> (&str, usize) {
    let bytes = source.as_bytes();
    match bytes.get(pos) {
        Some(&quote) if quote == b'"' || quote == b'\'' => {
            let end = find_byte(bytes, pos + 1, quote);
            (&source[pos + 1..end], (end + 1).min(bytes.len()))
        }
        _ => {
            let mut end = pos;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && bytes[end] != b'>' {
                end += 1;
            }
            (&source[pos..end], end)
        }
    }
}

/// Maps byte offsets to line and column positions.
#[derive(Debug)]
pub(crate) struct LineIndex<'a> {
    source: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self { source, starts }
    }

    pub fn position(&self, offset: usize) -> Position {
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let column = self.source[self.starts[line]..offset].chars().count();
        Position {
            line: line as u32 + 1,
            column: column as u32 + 1,
        }
    }
}

#[cfg(test)]
mod tests;
