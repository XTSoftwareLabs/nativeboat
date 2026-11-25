use super::*;

fn converter(encoding: Encoding, fatal: bool, ignore_bom: bool) -> Converter {
    Converter {
        encoding,
        fatal,
        ignore_bom,
        pending: Vec::new(),
        started: false,
    }
}

fn text(units: Option<Vec<u16>>) -> String {
    String::from_utf16(&units.unwrap()).unwrap()
}

#[test]
fn utf8_characters_cut_by_chunks_wait_for_their_bytes() {
    let mut decoder = converter(Encoding::Utf8, false, false);
    let bytes = "\u{feff}h\u{e9}\u{20ac}\u{1f600}".as_bytes();
    let mut out = String::new();
    for byte in bytes {
        out.push_str(&text(decoder.decode(&[*byte], false)));
    }
    out.push_str(&text(decoder.decode(&[], true)));
    // The byte order mark goes once, at the start.
    assert_eq!(out, "h\u{e9}\u{20ac}\u{1f600}");
    // A stream that ends inside a character ends with a replacement.
    assert_eq!(text(decoder.decode(&[0xe2, 0x82], false)), "");
    assert_eq!(text(decoder.decode(&[], true)), "\u{fffd}");
    // Fatal converters reject invalid input.
    let mut fatal = converter(Encoding::Utf8, true, false);
    assert!(fatal.decode(&[0xff], true).is_none());
    // Kept marks stay.
    let mut keep = converter(Encoding::Utf8, false, true);
    assert_eq!(
        text(keep.decode(&[0xef, 0xbb, 0xbf, b'a'], true)),
        "\u{feff}a"
    );
}

#[test]
fn utf16_units_and_surrogate_pairs_span_chunks() {
    let mut decoder = converter(Encoding::Utf16Le, false, false);
    let units: Vec<u8> = "a\u{1f600}"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut out = String::new();
    for byte in &units {
        out.push_str(&text(decoder.decode(&[*byte], false)));
    }
    assert_eq!(out, "a\u{1f600}");
    let mut big = converter(Encoding::Utf16Be, false, false);
    assert_eq!(text(big.decode(&[0xfe, 0xff, 0, b'z'], true)), "z");
    let mut odd = converter(Encoding::Utf16Le, false, false);
    assert_eq!(text(odd.decode(&[b'x', 0, b'y'], true)), "x\u{fffd}");
    let mut windows = converter(Encoding::Windows1252, false, false);
    assert_eq!(text(windows.decode(&[0x80, b'!'], true)), "\u{20ac}!");
}

#[test]
fn binding_decodes_through_converter_objects() {
    let realm = &mut Realm::new();
    let exports = realm.object();
    install(realm, &exports);
    let get = ops::get_named(realm, &Value::Object(exports.clone()), "getConverter").unwrap();
    let decode = ops::get_named(realm, &Value::Object(exports), "decode").unwrap();
    let handle = ops::call(
        realm,
        &get,
        Value::Undefined,
        &[Value::str("utf-8"), Value::Number(0.0)],
    )
    .unwrap();
    let first = crate::typed_array_builtin::new_bytes(realm, alloc::vec![0xe2, 0x82], None);
    let rest = crate::typed_array_builtin::new_bytes(realm, alloc::vec![0xac], None);
    let start = ops::call(
        realm,
        &decode,
        Value::Undefined,
        &[handle.clone(), Value::Object(first), Value::Number(0.0)],
    )
    .unwrap();
    let end = ops::call(
        realm,
        &decode,
        Value::Undefined,
        &[handle, Value::Object(rest), Value::Number(f64::from(FLUSH))],
    )
    .unwrap();
    assert!(crate::same_value(&start, &Value::str("")));
    assert!(crate::same_value(&end, &Value::str("\u{20ac}")));
    let unknown = ops::call(realm, &get, Value::Undefined, &[Value::str("koi8-r")]).unwrap();
    assert!(matches!(unknown, Value::Undefined));
    let fatal = ops::call(
        realm,
        &get,
        Value::Undefined,
        &[Value::str("utf-8"), Value::Number(f64::from(FATAL))],
    )
    .unwrap();
    let invalid = crate::typed_array_builtin::new_bytes(realm, alloc::vec![0xff], None);
    let error = ops::call(
        realm,
        &decode,
        Value::Undefined,
        &[
            fatal,
            Value::Object(invalid),
            Value::Number(f64::from(FLUSH)),
            Value::str("utf-8"),
        ],
    )
    .unwrap_err();
    let code = ops::get_named(realm, &error.0, "code").unwrap();
    assert!(crate::same_value(
        &code,
        &Value::str("ERR_ENCODING_INVALID_ENCODED_DATA")
    ));
}
