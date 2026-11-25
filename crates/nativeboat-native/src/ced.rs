//! Engine-free replacement for MarkText's `ced` encoding detector package.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Object, Realm, Value, typed_array_builtin};
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

const MAX_INPUT: usize = i32::MAX as usize;

/// The callable CommonJS export supplied by the `ced` package.
pub(crate) fn module(realm: &mut Realm) -> Object {
    realm.function("ced", 1, false, |realm, _, args| {
        let value = args.first().cloned().unwrap_or(Value::Undefined);
        if !is_buffer(realm, &value) {
            return Err(realm.throw(ErrorKind::Type, "Expected a buffer"));
        }
        let Some((bytes, offset, length)) = typed_array_builtin::view_range(&value) else {
            return Err(realm.throw(ErrorKind::Type, "Expected a buffer"));
        };
        if length > MAX_INPUT {
            return Err(realm.throw(ErrorKind::Range, "Encoding detection buffer exceeds 2 GiB"));
        }
        let bytes = bytes.borrow();
        let slice = &bytes[offset..offset + length];
        Ok(Value::str(detect(slice)))
    })
}

fn is_buffer(realm: &Realm, value: &Value) -> bool {
    let Some(buffer_prototype) = &realm.buffer_prototype else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let mut prototype = object.data().prototype.clone();
    while let Some(current) = prototype {
        if current == *buffer_prototype {
            return true;
        }
        prototype = current.data().prototype.clone();
    }
    false
}

fn detect(bytes: &[u8]) -> &'static str {
    if bytes.is_empty() {
        return "ASCII";
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return "UTF8";
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        return "UTF-16LE";
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return "UTF-16BE";
    }
    if let Some(label) = bomless_utf16(bytes) {
        return label;
    }
    if bytes.is_ascii() {
        return "ASCII-7-bit";
    }

    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    ced_label(detector.guess(None, Utf8Detection::Allow).name())
}

fn bomless_utf16(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.len() / 2;
    let even_nuls = bytes.chunks_exact(2).filter(|pair| pair[0] == 0).count();
    let odd_nuls = bytes.chunks_exact(2).filter(|pair| pair[1] == 0).count();
    if odd_nuls * 2 >= pairs {
        Some("UTF-16LE")
    } else if even_nuls * 2 >= pairs {
        Some("UTF-16BE")
    } else {
        None
    }
}

/// Keep the Google CED labels consumed by MarkText's existing iconv map.
fn ced_label(name: &str) -> &'static str {
    match name {
        "UTF-8" => "UTF8",
        "Big5" => "BIG5",
        "EUC-KR" => "KSC",
        "GBK" => "GB",
        "Shift_JIS" => "SJS",
        "EUC-JP" => "EUC-JP",
        "ISO-2022-JP" => "ISO-2022-JP",
        "windows-1250" => "CP1250",
        "windows-1251" => "CP1251",
        "windows-1252" => "CP1252",
        "windows-1253" => "CP1253",
        "windows-1254" => "CP1254",
        "windows-1255" => "CP1255",
        "windows-1256" => "CP1256",
        "windows-1257" => "CP1257",
        "windows-874" => "CP874",
        "IBM866" => "CP866",
        "KOI8-R" => "KOI8R",
        "KOI8-U" => "KOI8U",
        "ISO-8859-2" => "Latin2",
        "ISO-8859-4" => "Latin4",
        "ISO-8859-5" => "ISO-8859-5",
        "ISO-8859-6" => "Arabic",
        "ISO-8859-7" => "Greek",
        "ISO-8859-8" => "Hebrew",
        "ISO-8859-13" => "ISO-8859-13",
        "ISO-8859-15" => "ISO-8859-15",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests;
