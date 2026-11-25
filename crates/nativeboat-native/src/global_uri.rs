//! `encodeURI`, `encodeURIComponent`, `decodeURI`, `decodeURIComponent` and
//! Annex B's `escape`/`unescape`, as ECMAScript 2024 specifies them: UTF-8
//! percent-encoding of UTF-16 text, strict UTF-8 decoding that rejects
//! malformed, overlong, surrogate and out-of-range sequences with `URIError`,
//! and `decodeURI`'s preserved reserved characters. Also `globalThis` and
//! Node's `global` alias.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};

/// Characters that are never escaped: letters, digits and `-_.!~*'()`.
const UNRESERVED: &[u8] = b"-_.!~*'()";
/// Reserved characters `encodeURI` keeps and `decodeURI` preserves.
const RESERVED: &[u8] = b";/?:@&=+$,#";

fn text(realm: &mut Realm, args: &[Value]) -> Result<JsStr, Thrown> {
    ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))
}

fn malformed(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Uri, "URI malformed")
}

fn hex(byte: u8) -> [u16; 3] {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    [
        u16::from(b'%'),
        u16::from(DIGITS[usize::from(byte >> 4)]),
        u16::from(DIGITS[usize::from(byte & 15)]),
    ]
}

/// The spec's `Encode`.
fn encode(realm: &Realm, units: &[u16], extra: &[u8]) -> Result<JsStr, Thrown> {
    let mut result = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        let kept = u8::try_from(unit).is_ok_and(|byte| {
            byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) || extra.contains(&byte)
        });
        if kept {
            result.push(unit);
            index += 1;
            continue;
        }
        let point = match char::decode_utf16(units[index..].iter().copied()).next() {
            Some(Ok(character)) => character,
            _ => return Err(malformed(realm)),
        };
        index += point.len_utf16();
        let mut bytes = [0; 4];
        for byte in point.encode_utf8(&mut bytes).bytes() {
            result.extend_from_slice(&hex(byte));
        }
    }
    Ok(JsStr::from_units(&result))
}

/// The two hexadecimal digits at `index`, if both are present.
fn byte_at(units: &[u16], index: usize) -> Option<u8> {
    let digit = |unit: u16| char::from_u32(u32::from(unit))?.to_digit(16);
    let high = digit(*units.get(index)?)?;
    let low = digit(*units.get(index + 1)?)?;
    Some((high * 16 + low) as u8)
}

/// The spec's `Decode`.
fn decode(realm: &Realm, units: &[u16], preserve: &[u8]) -> Result<JsStr, Thrown> {
    let mut result = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        if units[index] != u16::from(b'%') {
            result.push(units[index]);
            index += 1;
            continue;
        }
        let first = byte_at(units, index + 1).ok_or_else(|| malformed(realm))?;
        if first < 0x80 {
            if preserve.contains(&first) {
                result.extend_from_slice(&units[index..index + 3]);
            } else {
                result.push(u16::from(first));
            }
            index += 3;
            continue;
        }
        let length = first.leading_ones() as usize;
        if !(2..=4).contains(&length) {
            return Err(malformed(realm));
        }
        let mut bytes = vec![first];
        for continuation in 1..length {
            let position = index + 3 * continuation;
            let byte = match (units.get(position), byte_at(units, position + 1)) {
                (Some(&unit), Some(byte)) if unit == u16::from(b'%') && byte & 0xC0 == 0x80 => byte,
                _ => return Err(malformed(realm)),
            };
            bytes.push(byte);
        }
        // Rust's decoder rejects overlong forms, surrogates and values above
        // U+10FFFF, as the specification requires.
        let decoded = core::str::from_utf8(&bytes).map_err(|_| malformed(realm))?;
        result.extend(decoded.encode_utf16());
        index += 3 * length;
    }
    Ok(JsStr::from_units(&result))
}

/// Annex B `escape`.
fn escape(units: &[u16]) -> JsStr {
    const KEPT: &[u8] = b"@*_+-./";
    let mut result = Vec::with_capacity(units.len());
    for &unit in units {
        match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() || KEPT.contains(&byte) => result.push(unit),
            Ok(byte) => result.extend_from_slice(&hex(byte)),
            Err(_) => {
                result.extend("%u".encode_utf16());
                result.extend(format!("{unit:04X}").encode_utf16());
            }
        }
    }
    JsStr::from_units(&result)
}

/// Annex B `unescape`.
fn unescape(units: &[u16]) -> JsStr {
    let mut result = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        if units[index] == u16::from(b'%') {
            if units.get(index + 1) == Some(&u16::from(b'u')) {
                if let (Some(high), Some(low)) =
                    (byte_at(units, index + 2), byte_at(units, index + 4))
                {
                    result.push(u16::from(high) << 8 | u16::from(low));
                    index += 6;
                    continue;
                }
            } else if let Some(byte) = byte_at(units, index + 1) {
                result.push(u16::from(byte));
                index += 3;
                continue;
            }
        }
        result.push(units[index]);
        index += 1;
    }
    JsStr::from_units(&result)
}

/// A native function body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

const FUNCTIONS: [(&str, Native); 6] = [
    ("encodeURI", |realm, _, args| {
        let text = text(realm, args)?;
        encode(realm, text.units(), RESERVED).map(Value::String)
    }),
    ("encodeURIComponent", |realm, _, args| {
        let text = text(realm, args)?;
        encode(realm, text.units(), &[]).map(Value::String)
    }),
    ("decodeURI", |realm, _, args| {
        let text = text(realm, args)?;
        decode(realm, text.units(), RESERVED).map(Value::String)
    }),
    ("decodeURIComponent", |realm, _, args| {
        let text = text(realm, args)?;
        decode(realm, text.units(), &[]).map(Value::String)
    }),
    ("escape", |realm, _, args| {
        Ok(Value::String(escape(text(realm, args)?.units())))
    }),
    ("unescape", |realm, _, args| {
        Ok(Value::String(unescape(text(realm, args)?.units())))
    }),
];

pub(crate) fn install(realm: &Realm) {
    for (name, code) in FUNCTIONS {
        let function = realm.function(name, 1, false, code);
        realm.set_global(name, Value::Object(function));
    }
    // `globalThis` (and Node's `global`) is writable and configurable but
    // not enumerable.
    for name in ["globalThis", "global"] {
        realm.global.define(
            Key::name(name),
            Property {
                slot: Slot::Data {
                    value: Value::Object(realm.global.clone()),
                    writable: true,
                },
                enumerable: false,
                configurable: true,
            },
        );
    }
}

#[cfg(test)]
mod tests;
