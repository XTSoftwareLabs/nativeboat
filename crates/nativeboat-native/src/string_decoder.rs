//! Streaming decoder needed by the unchanged filesystem promise/stream sources.
//! Port of Node v22.15.0 src/string_decoder.cc, retaining its seven-byte state.
// Ported from Node v22.15.0's src/string_decoder.cc.
// Copyright Joyent, Inc. and other Node contributors. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to
// deal in the Software without restriction, including without limitation the
// rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
// sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
// IN THE SOFTWARE.

use crate::encoding::{self, Encoding};
use crate::prelude::*;
use crate::{Completion, ErrorKind, JsStr, Key, Object, Property, Realm, Value};

const MISSING: usize = 4;
const BUFFERED: usize = 5;
const ENCODING: usize = 6;

fn decode(state: &mut [u8; 7], mut data: &[u8], encoding: Encoding) -> Vec<u16> {
    if matches!(encoding, Encoding::Ascii | Encoding::Hex | Encoding::Latin1) {
        return encoding::decode(data, encoding);
    }
    let mut prefix = Vec::new();
    if state[MISSING] > 0 {
        if encoding == Encoding::Utf8 {
            let needed = usize::from(state[MISSING]);
            for index in 0..data.len().min(needed) {
                if data[index] & 0xc0 != 0x80 {
                    let start = usize::from(state[BUFFERED]);
                    state[start..start + index].copy_from_slice(&data[..index]);
                    state[BUFFERED] += index as u8;
                    state[MISSING] = 0;
                    data = &data[index..];
                    break;
                }
            }
        }
        let found = data.len().min(usize::from(state[MISSING]));
        let start = usize::from(state[BUFFERED]);
        state[start..start + found].copy_from_slice(&data[..found]);
        state[MISSING] -= found as u8;
        state[BUFFERED] += found as u8;
        data = &data[found..];
        if state[MISSING] == 0 {
            prefix = encoding::decode(&state[..usize::from(state[BUFFERED])], encoding);
            state[BUFFERED] = 0;
        }
    }
    if data.is_empty() {
        return prefix;
    }
    match encoding {
        Encoding::Utf8 if data[data.len() - 1] & 0x80 != 0 => {
            for index in (0..data.len()).rev() {
                state[BUFFERED] += 1;
                let byte = data[index];
                if byte & 0xc0 == 0x80 {
                    if state[BUFFERED] >= 4 || index == 0 {
                        state[BUFFERED] = 0;
                        break;
                    }
                } else {
                    state[MISSING] = if byte & 0xe0 == 0xc0 {
                        2
                    } else if byte & 0xf0 == 0xe0 {
                        3
                    } else if byte & 0xf8 == 0xf0 {
                        4
                    } else {
                        state[BUFFERED] = 0;
                        break;
                    };
                    if state[BUFFERED] >= state[MISSING] {
                        state[MISSING] = 0;
                        state[BUFFERED] = 0;
                    }
                    state[MISSING] -= state[BUFFERED];
                    break;
                }
            }
        }
        Encoding::Utf16Le => {
            if data.len() % 2 == 1 {
                state[BUFFERED] = 1;
                state[MISSING] = 1;
            } else if data[data.len() - 1] & 0xfc == 0xd8 {
                state[BUFFERED] = 2;
                state[MISSING] = 2;
            }
        }
        Encoding::Base64 | Encoding::Base64Url => {
            state[BUFFERED] = (data.len() % 3) as u8;
            if state[BUFFERED] > 0 {
                state[MISSING] = 3 - state[BUFFERED];
            }
        }
        _ => {}
    }
    let end = data.len() - usize::from(state[BUFFERED]);
    if state[BUFFERED] > 0 {
        let buffered = usize::from(state[BUFFERED]);
        state[..buffered].copy_from_slice(&data[end..]);
    }
    prefix.extend(encoding::decode(&data[..end], encoding));
    prefix
}

fn flush(state: &mut [u8; 7], encoding: Encoding) -> Vec<u16> {
    if encoding == Encoding::Utf16Le && state[BUFFERED] % 2 == 1 {
        state[MISSING] -= 1;
        state[BUFFERED] -= 1;
    }
    if state[BUFFERED] == 0 {
        return Vec::new();
    }
    let result = encoding::decode(&state[..usize::from(state[BUFFERED])], encoding);
    state[MISSING] = 0;
    state[BUFFERED] = 0;
    result
}

fn operation(realm: &Realm, args: &[Value], end: bool) -> Completion {
    let state = args.first().unwrap_or(&Value::Undefined);
    let (bytes, offset, length) = crate::typed_array_builtin::view_range(state)
        .ok_or_else(|| realm.throw(ErrorKind::Type, "StringDecoder requires a state buffer"))?;
    if length < 7 {
        return Err(realm.throw(ErrorKind::Range, "StringDecoder state requires seven bytes"));
    }
    let mut state: [u8; 7] = bytes.borrow()[offset..offset + 7].try_into().unwrap();
    let encoding = encoding::numbered(i32::from(state[ENCODING]))
        .ok_or_else(|| realm.throw(ErrorKind::Type, "Invalid StringDecoder encoding"))?;
    if state[MISSING] > 4 || state[BUFFERED] > 4 || state[MISSING] + state[BUFFERED] > 4 {
        return Err(realm.throw(ErrorKind::Range, "Invalid StringDecoder state"));
    }
    let output = if end {
        flush(&mut state, encoding)
    } else {
        let value = args.get(1).unwrap_or(&Value::Undefined);
        let (input, input_offset, input_length) = crate::typed_array_builtin::view_range(value)
            .ok_or_else(|| {
                realm.throw(ErrorKind::Type, "StringDecoder requires an ArrayBufferView")
            })?;
        let data = input.borrow()[input_offset..input_offset + input_length].to_vec();
        decode(&mut state, &data, encoding)
    };
    bytes.borrow_mut()[offset..offset + 7].copy_from_slice(&state);
    Ok(Value::String(JsStr::from_units(&output)))
}

pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let encodings = [
        "ascii",
        "utf8",
        "base64",
        "utf16le",
        "latin1",
        "hex",
        "buffer",
        "base64url",
    ]
    .map(Value::str)
    .to_vec();
    exports.define(
        Key::name("encodings"),
        Property::data(Value::Object(realm.array(encodings))),
    );
    for (name, value) in [
        ("kIncompleteCharactersStart", 0),
        ("kIncompleteCharactersEnd", 4),
        ("kMissingBytes", 4),
        ("kBufferedBytes", 5),
        ("kEncodingField", 6),
        ("kNumFields", 7),
        ("kSize", 7),
    ] {
        exports.define(Key::name(name), Property::data(Value::Number(value as f64)));
    }
    for (name, length, end) in [("decode", 2, false), ("flush", 1, true)] {
        let function = realm.function(name, length, false, move |realm, _, args| {
            operation(realm, args, end)
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

#[cfg(test)]
mod tests;
