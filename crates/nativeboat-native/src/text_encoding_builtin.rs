//! The UTF-8 and UTF-16 text codecs used by Node's file and plist sources.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::typed_array_builtin::Element;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Symbol, Value, ops};

/// Installs the Web-compatible text codec constructors on the global object.
pub(crate) fn install(realm: &mut Realm) {
    install_encoder(realm);
    install_decoder(realm);
}

fn install_encoder(realm: &mut Realm) {
    let prototype = realm.object();
    prototype.define(Key::name("encoding"), Property::hidden(Value::str("utf-8")));
    realm.method(&prototype, "encode", 1, |realm, _, args| {
        let input = match args.first() {
            None | Some(Value::Undefined) => Value::str(""),
            Some(value) => value.clone(),
        };
        let text = ops::to_string(realm, &input)?;
        let bytes = crate::encoding::encode(text.units(), crate::encoding::Encoding::Utf8);
        Ok(Value::Object(crate::typed_array_builtin::new_bytes(
            realm, bytes, None,
        )))
    });
    realm.method(&prototype, "encodeInto", 2, |realm, _, args| {
        let text = ops::to_string(realm, &arg(args, 0))?;
        let destination = arg(args, 1);
        let Some(view) = crate::typed_array_builtin::view_of(&destination) else {
            return Err(crate::node_error::invalid_arg_type(
                realm,
                "destination",
                &["Uint8Array"],
                &destination,
            ));
        };
        if view.element != Element::Uint8 {
            return Err(crate::node_error::invalid_arg_type(
                realm,
                "destination",
                &["Uint8Array"],
                &destination,
            ));
        }
        let (read, written) = encode_into(text.units(), &view);
        let result = realm.object();
        result.define(
            Key::name("read"),
            Property::data(Value::Number(read as f64)),
        );
        result.define(
            Key::name("written"),
            Property::data(Value::Number(written as f64)),
        );
        Ok(Value::Object(result))
    });
    let instance_prototype = prototype.clone();
    let constructor = realm.builtin_constructor(
        "TextEncoder",
        0,
        &prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "TextEncoder must be constructed")),
        move |_, _, _| {
            Ok(Object::new(
                Some(instance_prototype.clone()),
                Kind::Ordinary,
            ))
        },
    );
    realm.set_global("TextEncoder", Value::Object(constructor));
}

fn install_decoder(realm: &mut Realm) {
    let prototype = realm.object();
    let encoding_key = Symbol::private("TextDecoder.encoding");
    let fatal_key = Symbol::private("TextDecoder.fatal");
    let ignore_bom_key = Symbol::private("TextDecoder.ignoreBOM");
    for (name, key) in [
        ("encoding", encoding_key.clone()),
        ("fatal", fatal_key.clone()),
        ("ignoreBOM", ignore_bom_key.clone()),
    ] {
        let getter = realm.function("get", 0, false, move |realm, this, _| {
            this.as_object()
                .and_then(|object| private_value(object, &key))
                .ok_or_else(|| {
                    realm.throw(
                        ErrorKind::Type,
                        "TextDecoder getter called on an invalid receiver",
                    )
                })
        });
        prototype.define(
            Key::name(name),
            Property {
                slot: Slot::Accessor {
                    get: Some(getter),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    let encoding_for_decode = encoding_key.clone();
    let fatal_for_decode = fatal_key.clone();
    let ignore_bom_for_decode = ignore_bom_key.clone();
    realm.method(&prototype, "decode", 1, move |realm, this, args| {
        let Some(object) = this.as_object() else {
            return Err(realm.throw(
                ErrorKind::Type,
                "TextDecoder.decode called on an invalid receiver",
            ));
        };
        let encoding = private_text(object, &encoding_for_decode).ok_or_else(|| {
            realm.throw(
                ErrorKind::Type,
                "TextDecoder.decode called on an invalid receiver",
            )
        })?;
        let fatal = private_bool(object, &fatal_for_decode);
        let ignore_bom = private_bool(object, &ignore_bom_for_decode);
        let input = args.first().cloned().unwrap_or(Value::Undefined);
        let bytes = bytes_from_value(realm, &input)?;
        let units = decode_bytes(realm, &encoding, &bytes, fatal, ignore_bom)?;
        Ok(Value::String(crate::JsStr::from_units(&units)))
    });
    let encoding_for_construct = encoding_key.clone();
    let fatal_for_construct = fatal_key.clone();
    let ignore_bom_for_construct = ignore_bom_key.clone();
    let instance_prototype = prototype.clone();
    let constructor = realm.builtin_constructor(
        "TextDecoder",
        0,
        &prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "TextDecoder must be constructed")),
        move |realm, _, args| {
            let label = match args.first() {
                None | Some(Value::Undefined) => "utf-8".to_owned(),
                Some(value) => ops::to_string(realm, value)?.to_rust_lossy(),
            };
            let encoding = canonical_encoding(&label).ok_or_else(|| {
                realm.throw(
                    ErrorKind::Range,
                    &format!("The \"{label}\" encoding is not supported"),
                )
            })?;
            let options = args.get(1).cloned().unwrap_or(Value::Undefined);
            let fatal = option_bool(realm, &options, "fatal")?;
            let ignore_bom = option_bool(realm, &options, "ignoreBOM")?;
            let object = Object::new(Some(instance_prototype.clone()), Kind::Ordinary);
            object.set_private(
                &encoding_for_construct,
                Property::data(Value::str(encoding)),
            );
            object.set_private(&fatal_for_construct, Property::data(Value::Boolean(fatal)));
            object.set_private(
                &ignore_bom_for_construct,
                Property::data(Value::Boolean(ignore_bom)),
            );
            Ok(object)
        },
    );
    realm.set_global("TextDecoder", Value::Object(constructor));
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn option_bool(realm: &mut Realm, options: &Value, name: &str) -> Result<bool, crate::Thrown> {
    if matches!(options, Value::Undefined | Value::Null) {
        return Ok(false);
    }
    let value = ops::get(realm, options, &Key::name(name))?;
    Ok(ops::to_boolean(&value))
}

fn private_value(object: &Object, key: &Symbol) -> Option<Value> {
    object
        .private_property(key)
        .and_then(|property| match property.slot {
            Slot::Data { value, .. } => Some(value),
            _ => None,
        })
}

fn private_text(object: &Object, key: &Symbol) -> Option<String> {
    match private_value(object, key)? {
        Value::String(value) => Some(value.to_rust_lossy()),
        _ => None,
    }
}

fn private_bool(object: &Object, key: &Symbol) -> bool {
    matches!(private_value(object, key), Some(Value::Boolean(true)))
}

fn canonical_encoding(label: &str) -> Option<&'static str> {
    match label.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => Some("utf-8"),
        "utf-16" | "utf-16le" => Some("utf-16le"),
        "iso-8859-1" | "latin1" | "us-ascii" | "ascii" | "windows-1252" => Some("windows-1252"),
        _ => None,
    }
}

fn bytes_from_value(realm: &mut Realm, value: &Value) -> Result<Vec<u8>, crate::Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok(Vec::new());
    }
    if let Some(bytes) = crate::typed_array_builtin::view_bytes(value) {
        return Ok(bytes);
    }
    if let Some(bytes) = crate::typed_array_builtin::buffer_of(value) {
        return Ok(bytes.borrow().clone());
    }
    Err(crate::node_error::invalid_arg_type(
        realm,
        "input",
        &["ArrayBuffer", "ArrayBufferView"],
        value,
    ))
}

fn decode_bytes(
    realm: &mut Realm,
    encoding: &str,
    bytes: &[u8],
    fatal: bool,
    ignore_bom: bool,
) -> Result<Vec<u16>, crate::Thrown> {
    match encoding {
        "utf-8" => {
            let bytes = if !ignore_bom && bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                &bytes[3..]
            } else {
                bytes
            };
            if fatal && core::str::from_utf8(bytes).is_err() {
                return Err(realm.throw(ErrorKind::Type, "The encoded data was not valid UTF-8"));
            }
            Ok(crate::encoding::decode(
                bytes,
                crate::encoding::Encoding::Utf8,
            ))
        }
        "utf-16le" => {
            let byte_length = bytes.len();
            let even = byte_length & !1;
            let mut units: Vec<u16> = bytes[..even]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            if even != byte_length {
                units.push(0xfffd);
            }
            if !ignore_bom && units.first() == Some(&0xfeff) {
                units.remove(0);
            }
            if fatal && (even != byte_length || String::from_utf16(&units).is_err()) {
                return Err(realm.throw(ErrorKind::Type, "The encoded data was not valid UTF-16"));
            }
            Ok(String::from_utf16_lossy(&units).encode_utf16().collect())
        }
        "windows-1252" => Ok(windows_1252(bytes)),
        _ => Err(realm.throw(ErrorKind::Range, "Unsupported TextDecoder encoding")),
    }
}

/// Windows-1252 bytes as UTF-16 units, as WHATWG's single-byte decoder
/// maps them.
pub(crate) fn windows_1252(bytes: &[u8]) -> Vec<u16> {
    bytes
        .iter()
        .map(|byte| match byte {
            0x80 => 0x20ac,
            0x82 => 0x201a,
            0x83 => 0x0192,
            0x84 => 0x201e,
            0x85 => 0x2026,
            0x86 => 0x2020,
            0x87 => 0x2021,
            0x88 => 0x02c6,
            0x89 => 0x2030,
            0x8a => 0x0160,
            0x8b => 0x2039,
            0x8c => 0x0152,
            0x8e => 0x017d,
            0x91 => 0x2018,
            0x92 => 0x2019,
            0x93 => 0x201c,
            0x94 => 0x201d,
            0x95 => 0x2022,
            0x96 => 0x2013,
            0x97 => 0x2014,
            0x98 => 0x02dc,
            0x99 => 0x2122,
            0x9a => 0x0161,
            0x9b => 0x203a,
            0x9c => 0x0153,
            0x9e => 0x017e,
            0x9f => 0x0178,
            _ => u16::from(*byte),
        })
        .collect()
}

/// Encodes whole characters of `units` as UTF-8 into the view while they
/// fit, returning the units read and the bytes written.
fn encode_into(units: &[u16], view: &crate::typed_array_builtin::View) -> (usize, usize) {
    let mut encoded = Vec::new();
    let mut read = 0;
    let mut written = 0;
    while read < units.len() {
        let unit = units[read];
        let width = if (0xd800..=0xdbff).contains(&unit)
            && units
                .get(read + 1)
                .is_some_and(|next| (0xdc00..=0xdfff).contains(next))
        {
            2
        } else {
            1
        };
        let next =
            crate::encoding::encode(&units[read..read + width], crate::encoding::Encoding::Utf8);
        if written + next.len() > view.length {
            break;
        }
        encoded.extend_from_slice(&next);
        written += next.len();
        read += width;
    }
    // The destination is a validated Uint8Array, and no source callbacks
    // run after acquiring its view. Copy into that view directly rather
    // than dispatching one dynamic property assignment for every byte.
    view.bytes.borrow_mut()[view.offset..view.offset + encoded.len()].copy_from_slice(&encoded);
    (read, written)
}

/// The encoding functions of `internalBinding('encoding_binding')` that
/// Node's `TextEncoder` and `TextDecoder` call: `encodeUtf8String`,
/// `encodeInto` (which reports through `encodeIntoResults`), `decodeUTF8`
/// and `decodeLatin1`.
pub(crate) fn install_binding(realm: &Realm, exports: &Object) {
    let encode = realm.function("encodeUtf8String", 1, false, |realm, _, args| {
        let text = ops::to_string(realm, &arg(args, 0))?;
        let bytes = crate::encoding::encode(text.units(), crate::encoding::Encoding::Utf8);
        Ok(Value::Object(crate::typed_array_builtin::new_bytes(
            realm, bytes, None,
        )))
    });
    exports.define(
        Key::name("encodeUtf8String"),
        Property::data(Value::Object(encode)),
    );
    let results = realm.array(alloc::vec![Value::Number(0.0), Value::Number(0.0)]);
    exports.define(
        Key::name("encodeIntoResults"),
        Property::data(Value::Object(results.clone())),
    );
    let into = realm.function("encodeInto", 2, false, move |realm, _, args| {
        let text = ops::to_string(realm, &arg(args, 0))?;
        let destination = arg(args, 1);
        let Some(view) = crate::typed_array_builtin::view_of(&destination) else {
            return Err(crate::node_error::invalid_arg_type(
                realm,
                "dest",
                &["Uint8Array"],
                &destination,
            ));
        };
        let (read, written) = encode_into(text.units(), &view);
        results.define(Key::Index(0), Property::data(Value::Number(read as f64)));
        results.define(Key::Index(1), Property::data(Value::Number(written as f64)));
        Ok(Value::Undefined)
    });
    exports.define(Key::name("encodeInto"), Property::data(Value::Object(into)));
    for (name, encoding) in [("decodeUTF8", "utf-8"), ("decodeLatin1", "windows-1252")] {
        let decode = realm.function(name, 3, false, move |realm, _, args| {
            let bytes = bytes_from_value(realm, &arg(args, 0))?;
            let ignore_bom = ops::to_boolean(&arg(args, 1));
            let fatal = ops::to_boolean(&arg(args, 2));
            let units = decode_bytes(realm, encoding, &bytes, fatal, ignore_bom)?;
            Ok(Value::String(crate::JsStr::from_units(&units)))
        });
        exports.define(Key::name(name), Property::data(Value::Object(decode)));
    }
}

#[cfg(test)]
mod tests;
