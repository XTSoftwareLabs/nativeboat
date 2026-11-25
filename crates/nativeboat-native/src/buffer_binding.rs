//! Node's native `buffer` binding (`internalBinding('buffer')`), as
//! Node.js v22.15.0's `src/node_buffer.cc` defines it, for the bundled
//! `buffer` library module: byte comparison, copying, filling, searching
//! and swapping, and string conversion in Node's encodings.

use crate::encoding::{self, Encoding};
use crate::node_error::node_error;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::typed_array_builtin::{Bytes, Element, buffer_of, new_typed_array, view_range};
use crate::{Completion, ErrorKind, JsStr, Key, Object, Property, Realm, Thrown, Value, ops};

/// `v8::Uint8Array::kMaxLength` on 64-bit systems.
const MAX_LENGTH: f64 = 9_007_199_254_740_991.0;
/// `v8::String::kMaxLength`.
const MAX_STRING_LENGTH: f64 = 536_870_888.0;

type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

/// `copyArrayBuffer(destination, destinationOffset, source, sourceOffset,
/// length)`: copies bytes between array buffers, as Node's web streams do
/// for byte streams. The caller has checked the ranges.
fn copy_array_buffer(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let buffer = |index: usize| {
        args.get(index)
            .and_then(crate::typed_array_builtin::buffer_of)
    };
    let (Some(destination), Some(source)) = (buffer(0), buffer(2)) else {
        return Err(realm.throw(ErrorKind::Type, "copyArrayBuffer needs two ArrayBuffers"));
    };
    let mut number = |index: usize| -> Result<usize, Thrown> {
        let value = args.get(index).cloned().unwrap_or(Value::Undefined);
        Ok(ops::to_number(realm, &value)?.max(0.0) as usize)
    };
    let (to, from, length) = (number(1)?, number(3)?, number(4)?);
    if alloc::rc::Rc::ptr_eq(&destination, &source) {
        let mut bytes = destination.borrow_mut();
        if from + length <= bytes.len() && to + length <= bytes.len() {
            bytes.copy_within(from..from + length, to);
        }
    } else {
        let source = source.borrow();
        let mut destination = destination.borrow_mut();
        if from + length <= source.len() && to + length <= destination.len() {
            destination[to..to + length].copy_from_slice(&source[from..from + length]);
        }
    }
    Ok(Value::Undefined)
}

/// The binding's exports.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let functions: [(&str, u32, Native); 31] = [
        ("byteLengthUtf8", 1, byte_length_utf8),
        ("copyArrayBuffer", 5, copy_array_buffer),
        ("compare", 2, compare),
        ("compareOffset", 6, compare_offset),
        ("copy", 5, copy),
        ("fill", 5, fill),
        ("isAscii", 1, is_ascii),
        ("isUtf8", 1, is_utf8),
        ("indexOfBuffer", 5, index_of_buffer),
        ("indexOfNumber", 4, index_of_number),
        ("indexOfString", 5, index_of_string),
        ("swap16", 1, |realm, _, args| swap(realm, args, 2)),
        ("swap32", 1, |realm, _, args| swap(realm, args, 4)),
        ("swap64", 1, |realm, _, args| swap(realm, args, 8)),
        ("atob", 1, atob),
        ("btoa", 1, btoa),
        ("asciiSlice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Ascii)
        }),
        ("base64Slice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Base64)
        }),
        ("base64urlSlice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Base64Url)
        }),
        ("latin1Slice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Latin1)
        }),
        ("hexSlice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Hex)
        }),
        ("ucs2Slice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Utf16Le)
        }),
        ("utf8Slice", 2, |realm, this, args| {
            slice(realm, &this, args, Encoding::Utf8)
        }),
        ("base64Write", 3, |realm, this, args| {
            write(realm, &this, args, Encoding::Base64)
        }),
        ("base64urlWrite", 3, |realm, this, args| {
            write(realm, &this, args, Encoding::Base64Url)
        }),
        ("hexWrite", 3, |realm, this, args| {
            write(realm, &this, args, Encoding::Hex)
        }),
        ("ucs2Write", 3, |realm, this, args| {
            write(realm, &this, args, Encoding::Utf16Le)
        }),
        ("asciiWriteStatic", 4, |realm, _, args| {
            write(
                realm,
                &arg(args, 0),
                &args[1.min(args.len())..],
                Encoding::Ascii,
            )
        }),
        ("latin1WriteStatic", 4, |realm, _, args| {
            write(
                realm,
                &arg(args, 0),
                &args[1.min(args.len())..],
                Encoding::Latin1,
            )
        }),
        ("utf8WriteStatic", 4, |realm, _, args| {
            write(
                realm,
                &arg(args, 0),
                &args[1.min(args.len())..],
                Encoding::Utf8,
            )
        }),
        ("getZeroFillToggle", 0, |realm, _, _| {
            // Buffers are always zero-filled, so the toggle changes nothing.
            Ok(Value::Object(new_typed_array(realm, Element::Uint32, 1)))
        }),
    ];
    for (name, length, function) in functions {
        let function = realm.function(name, length, false, function);
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let set_prototype = realm.function("setBufferPrototype", 1, false, |realm, _, args| {
        realm.buffer_prototype = arg(args, 0).as_object().cloned();
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("setBufferPrototype"),
        Property::data(Value::Object(set_prototype)),
    );
    for (name, value) in [
        ("kMaxLength", MAX_LENGTH),
        ("kStringMaxLength", MAX_STRING_LENGTH),
    ] {
        exports.define(Key::name(name), Property::data(Value::Number(value)));
    }
    exports
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// The bytes of an `ArrayBufferView`, or Node's "argument must be a buffer".
fn buffer(realm: &mut Realm, value: &Value) -> Result<(Bytes, usize, usize), Thrown> {
    view_range(value).ok_or_else(|| {
        node_error(
            realm,
            ErrorKind::Type,
            "ERR_INVALID_ARG_TYPE",
            "argument must be a buffer",
        )
    })
}

/// A copy of a view's bytes.
fn contents(range: &(Bytes, usize, usize)) -> Vec<u8> {
    let (bytes, offset, length) = range;
    bytes.borrow()[*offset..*offset + *length].to_vec()
}

fn string(realm: &mut Realm, value: &Value) -> Result<JsStr, Thrown> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => Err(node_error(
            realm,
            ErrorKind::Type,
            "ERR_INVALID_ARG_TYPE",
            "argument must be a string",
        )),
    }
}

/// `ParseArrayIndex`: undefined gives the default; a negative index is out
/// of range, which Node reports as `Index out of range`.
fn index(realm: &mut Realm, value: &Value, default: usize) -> Result<usize, Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok(default);
    }
    let number = ops::to_number(realm, value)?;
    let number = if number.is_nan() {
        0.0
    } else {
        number - crate::float::fract(number)
    };
    if number < 0.0 {
        return Err(out_of_range(realm));
    }
    Ok(if number >= usize::MAX as f64 {
        usize::MAX
    } else {
        number as usize
    })
}

fn out_of_range(realm: &mut Realm) -> Thrown {
    node_error(
        realm,
        ErrorKind::Range,
        "ERR_OUT_OF_RANGE",
        "Index out of range",
    )
}

/// A number argument as `Integer::Value` reads it.
fn integer(value: &Value) -> i64 {
    match value {
        Value::Number(number) if number.is_finite() => *number as i64,
        Value::Number(number) if *number > 0.0 => i64::MAX,
        Value::Number(number) if *number < 0.0 => i64::MIN,
        _ => 0,
    }
}

fn byte_length_utf8(_: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let length = match args.first() {
        Some(Value::String(text)) => encoding::utf8_length(text.units()),
        _ => 0,
    };
    Ok(Value::Number(length as f64))
}

/// `memcmp` normalized to -1, 0 or 1, then the shorter range first.
fn ordering(left: &[u8], right: &[u8], left_length: usize, right_length: usize) -> Value {
    let order = left.cmp(right).then(left_length.cmp(&right_length));
    Value::Number(match order {
        core::cmp::Ordering::Less => -1.0,
        core::cmp::Ordering::Equal => 0.0,
        core::cmp::Ordering::Greater => 1.0,
    })
}

fn compare(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let left = contents(&buffer(realm, &arg(args, 0))?);
    let right = contents(&buffer(realm, &arg(args, 1))?);
    let length = left.len().min(right.len());
    Ok(ordering(
        &left[..length],
        &right[..length],
        left.len(),
        right.len(),
    ))
}

fn compare_offset(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let source = contents(&buffer(realm, &arg(args, 0))?);
    let target = contents(&buffer(realm, &arg(args, 1))?);
    let target_start = index(realm, &arg(args, 2), 0)?;
    let source_start = index(realm, &arg(args, 3), 0)?;
    let target_end = index(realm, &arg(args, 4), target.len())?;
    let source_end = index(realm, &arg(args, 5), source.len())?;
    for (start, length, name) in [
        (source_start, source.len(), "sourceStart"),
        (target_start, target.len(), "targetStart"),
    ] {
        if start > length {
            let message = format!("The value of \"{name}\" is out of range.");
            return Err(node_error(
                realm,
                ErrorKind::Range,
                "ERR_OUT_OF_RANGE",
                &message,
            ));
        }
    }
    let source_length = source_end.saturating_sub(source_start);
    let target_length = target_end.saturating_sub(target_start);
    let length = source_length
        .min(target_length)
        .min(source.len() - source_start)
        .min(target.len() - target_start);
    Ok(ordering(
        &source[source_start..source_start + length],
        &target[target_start..target_start + length],
        source_length,
        target_length,
    ))
}

fn copy(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let source = contents(&buffer(realm, &arg(args, 0))?);
    let (target, offset, length) = buffer(realm, &arg(args, 1))?;
    let target_start = ops::to_uint32(realm, &arg(args, 2))? as usize;
    let source_start = ops::to_uint32(realm, &arg(args, 3))? as usize;
    let count = ops::to_uint32(realm, &arg(args, 4))? as usize;
    // The library validates the ranges; stay within both views anyway.
    let count = count
        .min(source.len().saturating_sub(source_start))
        .min(length.saturating_sub(target_start));
    if count > 0 {
        let start = offset + target_start;
        target.borrow_mut()[start..start + count]
            .copy_from_slice(&source[source_start..source_start + count]);
    }
    Ok(Value::Number(count as f64))
}

fn fill(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let (target, offset, length) = buffer(realm, &arg(args, 0))?;
    let start = index(realm, &arg(args, 2), 0)?;
    let end = index(realm, &arg(args, 3), 0)?;
    if start > end || end > length {
        return Ok(Value::Number(-2.0));
    }
    let fill_length = end - start;
    let value = arg(args, 1);
    let pattern = if let Some(range) = view_range(&value) {
        contents(&range)
    } else if let Value::String(text) = &value {
        let selected = match arg(args, 4) {
            Value::String(name) => encoding::named(name.units()).unwrap_or(Encoding::Utf8),
            _ => Encoding::Utf8,
        };
        match selected {
            Encoding::Utf8 | Encoding::Utf16Le => encoding::encode(text.units(), selected),
            _ => encoding::write(text.units(), selected, fill_length),
        }
    } else {
        let byte = ops::to_uint32(realm, &value)? as u8;
        target.borrow_mut()[offset + start..offset + end].fill(byte);
        return Ok(Value::Undefined);
    };
    if pattern.is_empty() && fill_length > 0 {
        return Ok(Value::Number(-1.0));
    }
    let mut bytes = target.borrow_mut();
    for (position, byte) in bytes[offset + start..offset + end].iter_mut().enumerate() {
        *byte = pattern[position % pattern.len()];
    }
    Ok(Value::Undefined)
}

/// The bytes of a view or `ArrayBuffer`, for validation.
fn validated(realm: &mut Realm, value: &Value) -> Result<Vec<u8>, Thrown> {
    if let Some(bytes) = buffer_of(value) {
        return Ok(bytes.borrow().clone());
    }
    Ok(contents(&buffer(realm, value)?))
}

fn is_ascii(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    Ok(Value::Boolean(validated(realm, &arg(args, 0))?.is_ascii()))
}

fn is_utf8(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let bytes = validated(realm, &arg(args, 0))?;
    Ok(Value::Boolean(core::str::from_utf8(&bytes).is_ok()))
}

/// Node's `IndexOfOffset`: where a search starts, or -1 for no match.
fn search_offset(length: usize, offset: i64, needle: usize, forward: bool) -> i64 {
    let length = length as i64;
    let needle = needle as i64;
    if offset < 0 {
        if offset + length >= 0 {
            length + offset
        } else if forward || needle == 0 {
            0
        } else {
            -1
        }
    } else if offset.saturating_add(needle) <= length {
        offset
    } else if needle == 0 {
        length
    } else if forward {
        -1
    } else {
        length - 1
    }
}

/// The first match at or after `from`, or the last at or before it, in
/// units of `width` bytes.
fn search(
    haystack: &[u8],
    needle: &[u8],
    from: usize,
    forward: bool,
    width: usize,
) -> Option<usize> {
    let units = haystack.len() / width;
    let needle_units = needle.len() / width;
    let matches = |unit: usize| {
        unit + needle_units <= units
            && haystack[unit * width..(unit + needle_units) * width]
                == needle[..needle_units * width]
    };
    let from = from / width;
    if forward {
        (from..units)
            .find(|unit| matches(*unit))
            .map(|unit| unit * width)
    } else {
        (0..=from.min(units.saturating_sub(1)))
            .rev()
            .find(|unit| matches(*unit))
            .map(|unit| unit * width)
    }
}

/// A search for a non-empty needle, after Node's offset rules.
fn find(haystack: &[u8], needle: &[u8], offset: i64, forward: bool, width: usize) -> Value {
    let start = search_offset(haystack.len(), offset, needle.len(), forward);
    if needle.is_empty() {
        return Value::Number(start as f64);
    }
    if haystack.is_empty()
        || start <= -1
        || (forward && needle.len() + start as usize > haystack.len())
        || needle.len() > haystack.len()
        || (width == 2 && (haystack.len() < 2 || needle.len() < 2))
    {
        return Value::Number(-1.0);
    }
    let found = search(haystack, needle, start as usize, forward, width);
    Value::Number(found.map_or(-1.0, |index| index as f64))
}

fn index_of_buffer(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let haystack = contents(&buffer(realm, &arg(args, 0))?);
    let needle = contents(&buffer(realm, &arg(args, 1))?);
    let width = if integer(&arg(args, 3)) == 3 { 2 } else { 1 };
    let forward = ops::to_boolean(&arg(args, 4));
    Ok(find(
        &haystack,
        &needle,
        integer(&arg(args, 2)),
        forward,
        width,
    ))
}

fn index_of_number(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let haystack = contents(&buffer(realm, &arg(args, 0))?);
    let needle = [ops::to_uint32(realm, &arg(args, 1))? as u8];
    let forward = ops::to_boolean(&arg(args, 3));
    Ok(find(&haystack, &needle, integer(&arg(args, 2)), forward, 1))
}

fn index_of_string(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let mut haystack = contents(&buffer(realm, &arg(args, 0))?);
    let needle = string(realm, &arg(args, 1))?;
    let selected = encoding::numbered(integer(&arg(args, 3)) as i32).unwrap_or(Encoding::Utf8);
    let width = if selected == Encoding::Utf16Le {
        haystack.truncate(haystack.len() & !1);
        2
    } else {
        1
    };
    let needle = match selected {
        Encoding::Utf16Le | Encoding::Latin1 => encoding::encode(needle.units(), selected),
        _ => encoding::encode(needle.units(), Encoding::Utf8),
    };
    let forward = ops::to_boolean(&arg(args, 4));
    Ok(find(
        &haystack,
        &needle,
        integer(&arg(args, 2)),
        forward,
        width,
    ))
}

fn swap(realm: &mut Realm, args: &[Value], width: usize) -> Completion {
    let target = arg(args, 0);
    let (bytes, offset, length) = buffer(realm, &target)?;
    let mut bytes = bytes.borrow_mut();
    for unit in bytes[offset..offset + length].chunks_exact_mut(width) {
        unit.reverse();
    }
    Ok(target)
}

fn atob(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let text = string(realm, &arg(args, 0))?;
    Ok(match encoding::forgiving_base64(text.units()) {
        Ok(bytes) => Value::String(JsStr::from_units(&encoding::decode(
            &bytes,
            Encoding::Latin1,
        ))),
        Err(code) => Value::Number(f64::from(code)),
    })
}

fn btoa(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let text = string(realm, &arg(args, 0))?;
    if text.units().iter().any(|unit| *unit > 0xff) {
        return Ok(Value::Number(-1.0));
    }
    let bytes = encoding::encode(text.units(), Encoding::Latin1);
    Ok(Value::str(&encoding::base64_encode(&bytes, false)))
}

/// `buffer.<encoding>Slice(start, end)`.
fn slice(realm: &mut Realm, this: &Value, args: &[Value], selected: Encoding) -> Completion {
    let bytes = contents(&buffer(realm, this)?);
    if bytes.is_empty() {
        return Ok(Value::str(""));
    }
    let start = index(realm, &arg(args, 0), 0)?;
    let end = index(realm, &arg(args, 1), bytes.len())?.max(start);
    if end > bytes.len() {
        return Err(out_of_range(realm));
    }
    let units = encoding::decode(&bytes[start..end], selected);
    Ok(Value::String(JsStr::from_units(&units)))
}

/// `buffer.<encoding>Write(string, offset, length)`: the bytes written.
fn write(realm: &mut Realm, target: &Value, args: &[Value], selected: Encoding) -> Completion {
    let (bytes, offset, length) = buffer(realm, target)?;
    let text = string(realm, &arg(args, 0))?;
    let start = index(realm, &arg(args, 1), 0)?;
    if start > length {
        return Err(node_error(
            realm,
            ErrorKind::Range,
            "ERR_BUFFER_OUT_OF_BOUNDS",
            "\"offset\" is outside of buffer bounds",
        ));
    }
    let available = length - start;
    let limit = index(realm, &arg(args, 2), available)?.min(available);
    let written = encoding::write(text.units(), selected, limit);
    let first = offset + start;
    bytes.borrow_mut()[first..first + written.len()].copy_from_slice(&written);
    Ok(Value::Number(written.len() as f64))
}

/// ICU's error codes for the conversions `transcode` reports.
const ILLEGAL_ARGUMENT: i32 = 1;
const INVALID_CHAR_FOUND: i32 = 10;

/// `internalBinding('icu')`: `transcode` between ASCII, Latin-1, UTF-8
/// and UTF-16LE as Node's ICU converters do: unmappable characters become
/// `?`, invalid input decodes to U+FFFD, and the strict UTF-8 and UTF-16
/// conversions fail on invalid input.
pub(crate) fn icu(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let transcode = realm.function("transcode", 3, false, |realm, _, args| {
        let source = contents(&buffer(realm, &arg(args, 0))?);
        let from = encoding_of(realm, &arg(args, 1))?;
        let to = encoding_of(realm, &arg(args, 2))?;
        Ok(match convert(&source, from, to) {
            Ok(bytes) => {
                let prototype = realm.buffer_prototype.clone();
                Value::Object(crate::typed_array_builtin::new_bytes(
                    realm, bytes, prototype,
                ))
            }
            Err(code) => Value::Number(f64::from(code)),
        })
    });
    exports.define(
        Key::name("transcode"),
        Property::data(Value::Object(transcode)),
    );
    let name = realm.function("icuErrName", 1, false, |realm, _, args| {
        let code = ops::to_int32(realm, &arg(args, 0))?;
        Ok(Value::str(match code {
            ILLEGAL_ARGUMENT => "U_ILLEGAL_ARGUMENT_ERROR",
            INVALID_CHAR_FOUND => "U_INVALID_CHAR_FOUND",
            _ => "U_ZERO_ERROR",
        }))
    });
    exports.define(Key::name("icuErrName"), Property::data(Value::Object(name)));
    crate::icu_converter::install(realm, &exports);
    exports
}

/// An encoding name as Node's `ParseEncoding` reads it; others cannot be
/// transcoded.
fn encoding_of(realm: &mut Realm, value: &Value) -> Result<Option<Encoding>, Thrown> {
    let name = ops::to_string(realm, value)?;
    Ok(encoding::named(name.units()))
}

fn convert(source: &[u8], from: Option<Encoding>, to: Option<Encoding>) -> Result<Vec<u8>, i32> {
    let supported = |encoding: Option<Encoding>| match encoding {
        Some(
            encoding @ (Encoding::Ascii | Encoding::Latin1 | Encoding::Utf8 | Encoding::Utf16Le),
        ) => Ok(encoding),
        _ => Err(ILLEGAL_ARGUMENT),
    };
    let (from, to) = (supported(from)?, supported(to)?);
    let units: Vec<u16> = match from {
        Encoding::Ascii => source
            .iter()
            .map(|byte| {
                if *byte < 0x80 {
                    u16::from(*byte)
                } else {
                    0xfffd
                }
            })
            .collect(),
        Encoding::Utf8 if to == Encoding::Utf16Le => core::str::from_utf8(source)
            .map_err(|_| INVALID_CHAR_FOUND)?
            .encode_utf16()
            .collect(),
        Encoding::Utf8 => String::from_utf8_lossy(source).encode_utf16().collect(),
        Encoding::Utf16Le => encoding::decode(source, Encoding::Utf16Le),
        _ => encoding::decode(source, Encoding::Latin1),
    };
    Ok(match to {
        Encoding::Utf16Le => encoding::encode(&units, Encoding::Utf16Le),
        Encoding::Utf8 => {
            let text = String::from_utf16(&units).map_err(|_| INVALID_CHAR_FOUND)?;
            text.into_bytes()
        }
        _ => {
            let limit = if to == Encoding::Ascii { 0x7f } else { 0xff };
            char::decode_utf16(units.iter().copied())
                .map(|unit| match unit {
                    Ok(character) if u32::from(character) <= limit => u32::from(character) as u8,
                    _ => b'?',
                })
                .collect()
        }
    })
}

#[cfg(test)]
mod tests;
