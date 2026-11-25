//! Node's character encodings for strings written as bytes (`Buffer.from`
//! with an encoding, stream writes).

#[allow(unused_imports)]
use crate::prelude::*;

/// A character encoding Node accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Latin1,
    Ascii,
    Base64,
    Base64Url,
    Hex,
}

/// The encoding an encoding name selects, ignoring ASCII case, like Node's
/// `normalizeEncoding`.
pub fn named(name: &[u16]) -> Option<Encoding> {
    let lower: String = String::from_utf16_lossy(name).to_ascii_lowercase();
    Some(match lower.as_str() {
        "utf8" | "utf-8" => Encoding::Utf8,
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => Encoding::Utf16Le,
        "latin1" | "binary" => Encoding::Latin1,
        "ascii" => Encoding::Ascii,
        "base64" => Encoding::Base64,
        "base64url" => Encoding::Base64Url,
        "hex" => Encoding::Hex,
        _ => return None,
    })
}

/// The bytes of a string in an encoding. UTF-8 replaces unpaired
/// surrogates; Latin-1 and ASCII keep each unit's low byte; base64 accepts
/// both alphabets, skips other characters and stops at `=`; hex stops at
/// the first pair that is not two hex digits.
pub fn encode(units: &[u16], encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => char::decode_utf16(units.iter().copied())
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
            .into_bytes(),
        Encoding::Utf16Le => units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        Encoding::Latin1 | Encoding::Ascii => units.iter().map(|unit| *unit as u8).collect(),
        Encoding::Base64 | Encoding::Base64Url => base64(units),
        Encoding::Hex => units
            .chunks_exact(2)
            .map_while(|pair| Some(hex_digit(pair[0])? << 4 | hex_digit(pair[1])?))
            .collect(),
    }
}

fn hex_digit(unit: u16) -> Option<u8> {
    char::from_u32(u32::from(unit))?
        .to_digit(16)
        .map(|digit| digit as u8)
}

fn base64_digit(unit: u16) -> Option<u32> {
    Some(match u8::try_from(unit).ok()? {
        letter @ b'A'..=b'Z' => letter - b'A',
        letter @ b'a'..=b'z' => letter - b'a' + 26,
        digit @ b'0'..=b'9' => digit - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    } as u32)
}

fn base64(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    for &unit in units.iter().take_while(|unit| **unit != u16::from(b'=')) {
        let Some(digit) = base64_digit(unit) else {
            continue;
        };
        bits = bits << 6 | digit;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    bytes
}

/// The encoding with Node's native encoding number (`encodingsMap`):
/// ascii 0, utf8 1, base64 2, utf16le 3, latin1 4, hex 5, base64url 7.
pub fn numbered(number: i32) -> Option<Encoding> {
    Some(match number {
        0 => Encoding::Ascii,
        1 => Encoding::Utf8,
        2 => Encoding::Base64,
        3 => Encoding::Utf16Le,
        4 => Encoding::Latin1,
        5 => Encoding::Hex,
        7 => Encoding::Base64Url,
        _ => return None,
    })
}

/// The bytes of a string that fit in `capacity` bytes, as Node writes them
/// into a buffer: UTF-8 never splits a character, UTF-16 never splits a
/// unit, and other encodings truncate.
pub fn write(units: &[u16], encoding: Encoding, capacity: usize) -> Vec<u8> {
    let mut bytes = encode(units, encoding);
    let mut end = capacity.min(bytes.len());
    match encoding {
        Encoding::Utf8 => {
            while end < bytes.len() && end > 0 && bytes[end] & 0xc0 == 0x80 {
                end -= 1;
            }
        }
        Encoding::Utf16Le => end &= !1,
        _ => {}
    }
    bytes.truncate(end);
    bytes
}

/// The number of UTF-8 bytes of a string; unpaired surrogates take three.
pub fn utf8_length(units: &[u16]) -> usize {
    char::decode_utf16(units.iter().copied())
        .map(|unit| unit.map_or(3, char::len_utf8))
        .sum()
}

/// The string bytes decode to in an encoding, as `buffer.toString` gives
/// it: UTF-8 replaces invalid sequences, ASCII clears the high bit, UTF-16
/// drops an odd final byte, hex is lowercase, base64 is padded and
/// base64url is not.
pub fn decode(bytes: &[u8], encoding: Encoding) -> Vec<u16> {
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(bytes).encode_utf16().collect(),
        Encoding::Ascii => bytes.iter().map(|byte| u16::from(byte & 0x7f)).collect(),
        Encoding::Latin1 => bytes.iter().map(|byte| u16::from(*byte)).collect(),
        Encoding::Utf16Le => bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
        Encoding::Hex => bytes
            .iter()
            .flat_map(|byte| {
                let digit = |value: u8| u16::from(b"0123456789abcdef"[usize::from(value)]);
                [digit(byte >> 4), digit(byte & 15)]
            })
            .collect(),
        Encoding::Base64 => base64_encode(bytes, false).encode_utf16().collect(),
        Encoding::Base64Url => base64_encode(bytes, true).encode_utf16().collect(),
    }
}

/// Base64 text of bytes: the standard alphabet with padding, or the URL
/// alphabet without it.
pub fn base64_encode(bytes: &[u8], url: bool) -> String {
    let alphabet: &[u8; 64] = if url {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, byte)| {
            bits | u32::from(*byte) << (16 - 8 * index)
        });
        for index in 0..=chunk.len() {
            text.push(char::from(
                alphabet[(bits >> (18 - 6 * index) & 63) as usize],
            ));
        }
        if !url {
            for _ in chunk.len()..3 {
                text.push('=');
            }
        }
    }
    text
}

/// `atob`'s forgiving-base64 decode: ASCII whitespace is ignored and up to
/// two `=` may end text whose length is a multiple of four. Errors are
/// Node's codes: -2 for an invalid character, -1 for a lone final
/// character.
pub fn forgiving_base64(units: &[u16]) -> Result<Vec<u8>, i32> {
    let mut text: Vec<u16> = units
        .iter()
        .copied()
        .filter(|unit| !matches!(unit, 0x09 | 0x0a | 0x0c | 0x0d | 0x20))
        .collect();
    if text.len().is_multiple_of(4) {
        for _ in 0..2 {
            if text.last() == Some(&u16::from(b'=')) {
                text.pop();
            }
        }
    }
    let valid = |unit: &u16| {
        u8::try_from(*unit)
            .is_ok_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    };
    if !text.iter().all(valid) {
        return Err(-2);
    }
    if text.len() % 4 == 1 {
        return Err(-1);
    }
    Ok(base64(&text))
}

#[cfg(test)]
mod tests;
