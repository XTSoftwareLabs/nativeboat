//! The decoding half of ICU's converters, as `internalBinding('icu')` gives
//! it to Node's `TextDecoder` for streaming decodes: `getConverter(encoding,
//! flags)` and `decode(converter, input, flags, encoding)`. A converter
//! keeps the bytes of a character that a chunk cuts, and the byte order
//! mark is dropped once at the start of each stream unless asked to keep it.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Value, ops};

const FLUSH: u32 = 0x1;
const FATAL: u32 = 0x2;
const IGNORE_BOM: u32 = 0x4;

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1252,
}

struct Converter {
    encoding: Encoding,
    fatal: bool,
    ignore_bom: bool,
    pending: Vec<u8>,
    /// Whether the stream's start, where a byte order mark may be, has
    /// been decoded.
    started: bool,
}

/// Bytes at the end of `bytes` that start a UTF-8 sequence they do not
/// finish.
fn utf8_incomplete(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let needed = match byte {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return 0,
        };
        return if needed > back { back } else { 0 };
    }
    0
}

impl Converter {
    /// Decodes the pending bytes and `input`; unless `flush`, keeps the
    /// bytes of a character the input cuts. `None` when fatal and invalid.
    fn decode(&mut self, input: &[u8], flush: bool) -> Option<Vec<u16>> {
        let mut bytes = core::mem::take(&mut self.pending);
        bytes.extend_from_slice(input);
        let keep = if flush {
            0
        } else {
            match self.encoding {
                Encoding::Utf8 => utf8_incomplete(&bytes),
                Encoding::Utf16Le | Encoding::Utf16Be => {
                    let odd = bytes.len() % 2;
                    // A lead surrogate may pair with the next chunk's unit.
                    let end = bytes.len() - odd;
                    let lead = end >= 2 && {
                        let pair = [bytes[end - 2], bytes[end - 1]];
                        let unit = if self.encoding == Encoding::Utf16Le {
                            u16::from_le_bytes(pair)
                        } else {
                            u16::from_be_bytes(pair)
                        };
                        (0xd800..0xdc00).contains(&unit)
                    };
                    odd + if lead { 2 } else { 0 }
                }
                Encoding::Windows1252 => 0,
            }
        };
        self.pending = bytes.split_off(bytes.len() - keep);
        let mut units = match self.encoding {
            Encoding::Utf8 => {
                if self.fatal && core::str::from_utf8(&bytes).is_err() {
                    return None;
                }
                crate::encoding::decode(&bytes, crate::encoding::Encoding::Utf8)
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let mut units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|pair| {
                        if self.encoding == Encoding::Utf16Le {
                            u16::from_le_bytes([pair[0], pair[1]])
                        } else {
                            u16::from_be_bytes([pair[0], pair[1]])
                        }
                    })
                    .collect();
                let odd = bytes.len() % 2 == 1;
                if (odd || char::decode_utf16(units.iter().copied()).any(|unit| unit.is_err()))
                    && self.fatal
                {
                    return None;
                }
                units = char::decode_utf16(units.iter().copied())
                    .map(|unit| unit.unwrap_or('\u{fffd}'))
                    .collect::<String>()
                    .encode_utf16()
                    .collect();
                if odd {
                    units.push(0xfffd);
                }
                units
            }
            Encoding::Windows1252 => crate::text_encoding_builtin::windows_1252(&bytes),
        };
        if !self.started && !units.is_empty() {
            self.started = true;
            if !self.ignore_bom && units[0] == 0xfeff {
                units.remove(0);
            }
        }
        if flush {
            self.started = false;
        }
        Some(units)
    }
}

pub(crate) fn install(realm: &Realm, exports: &Object) {
    let get = realm.function("getConverter", 2, false, |realm, _, args| {
        let name = args.first().cloned().unwrap_or(Value::Undefined);
        let name = ops::to_string(realm, &name)?.to_rust_lossy();
        let encoding = match name.as_str() {
            "utf-8" => Encoding::Utf8,
            "utf-16le" => Encoding::Utf16Le,
            "utf-16be" => Encoding::Utf16Be,
            "windows-1252" | "latin1" | "iso-8859-1" => Encoding::Windows1252,
            _ => return Ok(Value::Undefined),
        };
        let flags = match args.get(1) {
            Some(value) => ops::to_uint32(realm, value)?,
            None => 0,
        };
        let converter = Converter {
            encoding,
            fatal: flags & FATAL != 0,
            ignore_bom: flags & IGNORE_BOM != 0,
            pending: Vec::new(),
            started: false,
        };
        let data: Rc<dyn core::any::Any> = Rc::new(RefCell::new(converter));
        Ok(Value::Object(Object::new(
            Some(realm.object_prototype.clone()),
            Kind::Native(data),
        )))
    });
    exports.define(
        Key::name("getConverter"),
        Property::data(Value::Object(get)),
    );
    let decode = realm.function("decode", 4, false, |realm, _, args| {
        let converter =
            args.first()
                .and_then(Value::as_object)
                .and_then(|object| match &object.data().kind {
                    Kind::Native(data) => data.clone().downcast::<RefCell<Converter>>().ok(),
                    _ => None,
                });
        let Some(converter) = converter else {
            return Err(realm.throw(ErrorKind::Type, "Illegal invocation"));
        };
        let input = args
            .get(1)
            .and_then(|value| {
                crate::typed_array_builtin::view_bytes(value).or_else(|| {
                    crate::typed_array_builtin::buffer_of(value).map(|bytes| bytes.borrow().clone())
                })
            })
            .unwrap_or_default();
        let flags = match args.get(2) {
            Some(value) => ops::to_uint32(realm, value)?,
            None => 0,
        };
        let decoded = converter.borrow_mut().decode(&input, flags & FLUSH != 0);
        match decoded {
            Some(units) => Ok(Value::String(crate::JsStr::from_units(&units))),
            None => {
                let encoding = args.get(3).cloned().unwrap_or(Value::Undefined);
                let encoding = ops::to_string(realm, &encoding)?.to_rust_lossy();
                Err(crate::node_error::node_error(
                    realm,
                    ErrorKind::Type,
                    "ERR_ENCODING_INVALID_ENCODED_DATA",
                    &alloc::format!("The encoded data was not valid for encoding {encoding}"),
                ))
            }
        }
    });
    exports.define(Key::name("decode"), Property::data(Value::Object(decode)));
}

#[cfg(test)]
mod tests;
