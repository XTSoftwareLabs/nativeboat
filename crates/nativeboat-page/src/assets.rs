//! The files a page loads by URL, such as images and fonts: `data:` URLs,
//! `file:` URLs and Windows paths, URLs relative to the page's own `file:`
//! URL, and URLs of schemes the main process serves through Electron's
//! `protocol`, or relative to a page of such a scheme. Other schemes load
//! nothing.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

/// The bytes a URL names, for a page whose URL is `page`.
pub(crate) fn fetch(page: &str, url: &str) -> Option<Vec<u8>> {
    let url = url.trim();
    if let Some(data) = strip_prefix_ignore_case(url, "data:") {
        return data_url(data);
    }
    if let Some(served) = served_url(page, url) {
        return match nativeboat_native::electron_protocol::serve(&served)? {
            Served::File(path) => nativeboat_native::platform::read_file(&path),
            Served::Bytes(bytes) => Some(bytes),
        };
    }
    nativeboat_native::platform::read_file(&file_path(page, url)?)
}

use nativeboat_native::electron_protocol::Served;

/// Where an absolute URL's scheme ends: its colon, after two or more
/// characters, so `C:` stays a drive.
fn scheme_end(url: &str) -> Option<usize> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    (colon > 1
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte)))
    .then_some(colon)
}

/// `url` resolved against `base`, an absolute URL with a path, as URLs of
/// hierarchical schemes resolve: a path from `/` replaces the base's path,
/// another replaces its last segment, and `.` and `..` segments go.
fn join_url(base: &str, url: &str) -> Option<String> {
    let colon = scheme_end(base)?;
    let after = &base[colon + 1..];
    let (origin, path) = match after.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            (&base[..colon + 3 + end], &rest[end..])
        }
        None => (&base[..colon + 1], after),
    };
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let (relative, suffix) = match url.find(['?', '#']) {
        Some(at) => (&url[..at], &url[at..]),
        None => (url, ""),
    };
    let mut segments: Vec<&str> = if relative.starts_with('/') {
        Vec::new()
    } else {
        let mut segments: Vec<&str> = path.split('/').collect();
        segments.pop();
        segments
    };
    if segments.is_empty() {
        segments.push("");
    }
    let last = relative.rsplit('/').next().unwrap_or_default();
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            segment => segments.push(segment),
        }
    }
    // `dir/` and `dir/..` name directories.
    if relative.is_empty() || matches!(last, "" | "." | "..") {
        segments.push("");
    }
    let mut joined = String::from(origin);
    let path = segments.join("/");
    if !path.starts_with('/') {
        joined.push('/');
    }
    joined.push_str(&path);
    joined.push_str(suffix);
    Some(joined)
}

/// Whether text is an absolute Windows path: `C:\…`, `C:/…` or `\\server`.
fn windows_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    (bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || text.starts_with("\\\\")
}

/// The absolute URL of a resource the main process serves (Electron's
/// `protocol`): `url` when its scheme is served, or `url` resolved against
/// a page whose scheme is, such as VS Code's `vscode-file:` pages.
pub(crate) fn served_url(page: &str, url: &str) -> Option<String> {
    let url = url.trim();
    // Windows paths, such as those style sheets' `url()`s are rebased to,
    // name files, never paths of the page's URL.
    if url.is_empty() || windows_path(url) {
        return None;
    }
    let absolute = if scheme_end(url).is_some() {
        String::from(url)
    } else if nativeboat_native::electron_protocol::serves(page) {
        join_url(page, url)?
    } else {
        return None;
    };
    nativeboat_native::electron_protocol::serves(&absolute).then_some(absolute)
}

/// Where a style sheet or other resource is, for caching and for resolving
/// what it names: its file, or the URL the main process serves it at when
/// it has none.
pub(crate) fn locate(page: &str, url: &str) -> Option<String> {
    file_path(page, url).or_else(|| served_url(page, url))
}

/// Whether a URL is an http(s) one, which the page downloads instead.
pub(crate) fn is_network(url: &str) -> bool {
    let url = url.trim();
    strip_prefix_ignore_case(url, "http://").is_some()
        || strip_prefix_ignore_case(url, "https://").is_some()
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

/// The data of a `data:` URL after its scheme: base64 or percent-encoded.
fn data_url(rest: &str) -> Option<Vec<u8>> {
    let (meta, data) = rest.split_once(',')?;
    if meta.to_ascii_lowercase().ends_with(";base64") {
        base64(data)
    } else {
        Some(percent_decode(data))
    }
}

/// Decodes base64 text, ignoring white space and padding.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\t' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    Some(bytes)
}

fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |byte: u8| char::from(byte).to_digit(16);
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).copied().and_then(hex),
                bytes.get(index + 2).copied().and_then(hex),
            )
        {
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    decoded
}

/// The Windows path a URL names: a `file:` URL, an absolute path, or a
/// relative URL resolved against the page's `file:` URL.
/// The `@import`s that start a style sheet, with their URLs and media
/// queries, and where the rest of the sheet begins.
pub(crate) fn leading_imports(css: &str) -> (Vec<(String, String)>, usize) {
    let mut imports = Vec::new();
    let mut at = 0;
    loop {
        let rest = &css[at..];
        let trimmed = rest.trim_start();
        let skipped = rest.len() - trimmed.len();
        if trimmed.starts_with("/*") {
            match trimmed.find("*/") {
                Some(end) => at += skipped + end + 2,
                None => return (imports, css.len()),
            }
            continue;
        }
        let statement = |name: &str| {
            trimmed
                .get(..name.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(name))
        };
        if !statement("@import") && !statement("@charset") {
            return (imports, at);
        }
        let Some(end) = statement_end(trimmed) else {
            return (imports, css.len());
        };
        if statement("@import") {
            let body = trimmed[7..end].trim();
            let (url, media) = if body
                .get(..4)
                .is_some_and(|start| start.eq_ignore_ascii_case("url("))
            {
                let inner = &body[4..];
                let close = inner.find(')').unwrap_or(inner.len());
                (
                    inner[..close].trim(),
                    &inner[(close + 1).min(inner.len())..],
                )
            } else {
                let quote = body.chars().next().unwrap_or(' ');
                let inner = body.get(1..).unwrap_or_default();
                let close = inner.find(quote).unwrap_or(inner.len());
                (&inner[..close], &inner[(close + 1).min(inner.len())..])
            };
            let url = url.trim_matches(['"', '\'']);
            if !url.is_empty() {
                imports.push((String::from(url), String::from(media.trim())));
            }
        }
        at += skipped + end + 1;
    }
}

/// Where a statement at-rule ends: its semicolon outside strings.
fn statement_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in text.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), _) if open == character => quote = None,
            (None, ';') => return Some(index),
            _ => {}
        }
    }
    None
}

/// Style sheet text whose relative `url()`s are made absolute paths from the
/// sheet's own file, as browsers resolve them against the sheet's URL.
pub(crate) fn rebase_urls(css: &str, sheet: &str) -> String {
    let mut rebased = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest
        .as_bytes()
        .windows(4)
        .position(|window| window.eq_ignore_ascii_case(b"url("))
    {
        rebased.push_str(&rest[..at + 4]);
        rest = &rest[at + 4..];
        let spaces = rest.len() - rest.trim_start().len();
        let quote = rest[spaces..]
            .chars()
            .next()
            .filter(|quote| matches!(quote, '"' | '\''));
        let start = spaces + quote.map_or(0, char::len_utf8);
        let body = &rest[start..];
        let Some(end) = body.find(quote.unwrap_or(')')) else {
            break;
        };
        let url = body[..end].trim();
        let relative = !url.is_empty()
            && !url.starts_with('#')
            && !url.starts_with('/')
            && !url.starts_with('\\')
            && url.as_bytes().get(1) != Some(&b':')
            && !url.find(':').is_some_and(|colon| {
                url[..colon]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
            });
        rebased.push_str(&rest[..start]);
        match relative.then(|| locate(sheet, url)).flatten() {
            Some(path) => rebased.push_str(&path.replace('\\', "/")),
            None => rebased.push_str(&body[..end]),
        }
        rest = &body[end..];
    }
    rebased.push_str(rest);
    rebased
}

pub(crate) fn file_path(page: &str, url: &str) -> Option<String> {
    // A URL the main process serves is the file its handler names.
    if let Some(served) = served_url(page, url) {
        return match nativeboat_native::electron_protocol::serve(&served)? {
            Served::File(path) => Some(path),
            Served::Bytes(_) => None,
        };
    }
    let url = url.split(['?', '#']).next()?;
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = strip_prefix_ignore_case(url, "file:") {
        let path = String::from_utf8(percent_decode(rest)).ok()?;
        let path = path.trim_start_matches('/');
        // `file:///C:/x` names a drive; `file://server/share` a share.
        let drive = path.as_bytes().get(1) == Some(&b':');
        let host = rest.starts_with("//") && !rest.starts_with("///") && !drive;
        let path = path.replace('/', "\\");
        return Some(if host {
            alloc::format!("\\\\{path}")
        } else {
            path
        });
    }
    let bytes = url.as_bytes();
    let absolute = bytes.get(1) == Some(&b':') || url.starts_with("\\\\");
    if absolute {
        return Some(url.replace('/', "\\"));
    }
    // A scheme other than `file:` loads nothing.
    if let Some(colon) = url.find(':')
        && url[..colon]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte))
    {
        return None;
    }
    let base = file_path(page, page)?;
    let mut parts: Vec<&str> = base.split('\\').collect();
    parts.pop();
    let relative = String::from_utf8(percent_decode(url)).ok()?;
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            part => parts.push(part),
        }
    }
    let mut path = parts.join("\\");
    path.shrink_to_fit();
    Some(path)
}

/// The font an `@font-face` rule declares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FontFace {
    pub family: String,
    /// The weights it serves: one, or a variable font's range.
    pub weights: (u16, u16),
    pub italic: bool,
    /// Its `unicode-range`: inclusive code point ranges, all when empty.
    pub ranges: Vec<(u32, u32)>,
    /// The URLs of its sources in order of preference.
    pub urls: Vec<String>,
}

/// The font an `@font-face` rule declares.
pub(crate) fn font_face(declarations: &[nativeboat_css::parser::Declaration]) -> Option<FontFace> {
    let value = |name: &str| {
        declarations
            .iter()
            .rev()
            .find(|declaration| declaration.name == name)
            .map(|declaration| declaration.value.trim())
    };
    let family = value("font-family")?
        .trim_matches(|character| character == '"' || character == '\'')
        .to_owned();
    let weight = |text: &str| match text {
        "normal" => 400,
        "bold" => 700,
        weight => weight
            .parse::<f32>()
            .map_or(400, |weight| weight.clamp(1.0, 1000.0) as u16),
    };
    // A variable font serves a range of weights.
    let mut weights = value("font-weight")
        .unwrap_or("400")
        .split_whitespace()
        .map(weight);
    let lightest = weights.next().unwrap_or(400);
    let weights = (lightest, weights.next().unwrap_or(lightest).max(lightest));
    let italic = value("font-style")
        .is_some_and(|style| style.contains("italic") || style.contains("oblique"));
    let mut urls = Vec::new();
    let mut rest = value("src")?;
    while let Some(start) = rest.find("url(") {
        rest = &rest[start + 4..];
        let end = rest.find(')')?;
        let url = rest[..end]
            .trim()
            .trim_matches(|character| character == '"' || character == '\'');
        urls.push(url.to_owned());
        rest = &rest[end..];
    }
    let ranges = value("unicode-range")
        .map(unicode_ranges)
        .unwrap_or_default();
    Some(FontFace {
        family,
        weights,
        italic,
        ranges,
        urls,
    })
}

/// The code point ranges of a `unicode-range` list: `U+26`, `U+0-7F` and
/// `U+4??` forms; invalid parts are left out.
fn unicode_ranges(list: &str) -> Vec<(u32, u32)> {
    list.split(',')
        .filter_map(|part| {
            // Normalized values may keep spaces between a range's parts.
            let part: String = part
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            let digits = part
                .strip_prefix("U+")
                .or_else(|| part.strip_prefix("u+"))?;
            let (first, last) = match digits.split_once('-') {
                Some((first, last)) => (
                    u32::from_str_radix(first, 16).ok()?,
                    u32::from_str_radix(last, 16).ok()?,
                ),
                // Question marks stand for any hexadecimal digit.
                None if digits.contains('?') => (
                    u32::from_str_radix(&digits.replace('?', "0"), 16).ok()?,
                    u32::from_str_radix(&digits.replace('?', "F"), 16).ok()?,
                ),
                None => {
                    let point = u32::from_str_radix(digits, 16).ok()?;
                    (point, point)
                }
            };
            (first <= last && last <= 0x10_FFFF).then_some((first, last))
        })
        .collect()
}

#[cfg(test)]
mod tests;
