//! Character reference decoding.

use crate::prelude::*;
use alloc::borrow::Cow;

/// Named character references supported by the parser.
const NAMED: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("middot", "·"),
    ("bull", "•"),
    ("times", "×"),
    ("divide", "÷"),
    ("deg", "°"),
    ("plusmn", "±"),
    ("para", "¶"),
    ("sect", "§"),
    ("cent", "¢"),
    ("pound", "£"),
    ("yen", "¥"),
    ("euro", "€"),
    ("larr", "←"),
    ("rarr", "→"),
    ("uarr", "↑"),
    ("darr", "↓"),
    ("harr", "↔"),
    ("hearts", "♥"),
    ("shy", "\u{ad}"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("zwj", "\u{200d}"),
    ("zwnj", "\u{200c}"),
];

/// Decodes character references such as `&amp;`, `&#38;` and `&#x26;`.
///
/// Unknown or malformed references are kept as written.
pub fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        decoded.push_str(&rest[..index]);
        rest = &rest[index..];
        match decode_reference(&rest[1..]) {
            Some((value, consumed)) => {
                decoded.push_str(&value);
                rest = &rest[1 + consumed..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// Decodes the reference after `&`, returning the text and consumed length.
fn decode_reference(text: &str) -> Option<(String, usize)> {
    let end = text.find(';')?;
    let name = &text[..end];
    let value = match name.strip_prefix('#') {
        Some(number) => decode_numeric(number)?,
        None => NAMED
            .iter()
            .find(|(entity, _)| *entity == name)
            .map(|(_, value)| (*value).to_owned())?,
    };
    Some((value, end + 1))
}

fn decode_numeric(number: &str) -> Option<String> {
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => number.parse().ok()?,
    };
    let character = match code {
        0 => '\u{fffd}',
        _ => char::from_u32(code).unwrap_or('\u{fffd}'),
    };
    Some(character.to_string())
}
