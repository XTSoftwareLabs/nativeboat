use super::*;

fn global(realm: &mut Realm, name: &str) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name(name),
    )
    .unwrap()
}

fn construct(realm: &mut Realm, constructor: &Value, args: &[Value]) -> Value {
    ops::construct(realm, constructor, args).unwrap()
}

fn method(realm: &mut Realm, receiver: &Value, name: &str, args: &[Value]) -> Value {
    let function = ops::get(realm, receiver, &Key::name(name)).unwrap();
    assert!(
        function
            .as_object()
            .is_some_and(|object| matches!(object.data().kind, Kind::Function { .. })),
        "{name}: {function:?}"
    );
    let result = ops::call(realm, &function, receiver.clone(), args);
    result.unwrap_or_else(|error| panic!("{}", text(realm, &error.0)))
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

#[test]
fn text_encoder_encodes_usv_strings_and_respects_code_point_capacity() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextEncoder");
    let encoder = construct(&mut realm, &constructor, &[]);
    let encoding = ops::get(&mut realm, &encoder, &Key::name("encoding")).unwrap();
    assert_eq!(text(&mut realm, &encoding), "utf-8");
    let encoded = method(&mut realm, &encoder, "encode", &[Value::str("A🦀")]);
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&encoded).unwrap(),
        [0x41, 0xf0, 0x9f, 0xa6, 0x80]
    );
    let destination = Value::Object(crate::typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        3,
    ));
    let result = method(
        &mut realm,
        &encoder,
        "encodeInto",
        &[Value::str("🦀"), destination.clone()],
    );
    assert!(matches!(
        ops::get(&mut realm, &result, &Key::name("read")).unwrap(),
        Value::Number(0.0)
    ));
    assert!(matches!(
        ops::get(&mut realm, &result, &Key::name("written")).unwrap(),
        Value::Number(0.0)
    ));
    let result = method(
        &mut realm,
        &encoder,
        "encodeInto",
        &[Value::str("A🦀"), destination.clone()],
    );
    assert!(matches!(
        ops::get(&mut realm, &result, &Key::name("read")).unwrap(),
        Value::Number(1.0)
    ));
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&destination).unwrap(),
        [0x41, 0, 0]
    );
}

#[test]
fn text_decoder_supports_bom_options_utf16_and_fatal_utf8() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextDecoder");
    let utf8 = construct(&mut realm, &constructor, &[]);
    let bytes = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![0xef, 0xbb, 0xbf, b'o', b'k'],
        None,
    ));
    let decoded = method(&mut realm, &utf8, "decode", core::slice::from_ref(&bytes));
    assert_eq!(text(&mut realm, &decoded), "ok");
    let keep_bom = realm.object();
    keep_bom.define(Key::name("ignoreBOM"), Property::data(Value::Boolean(true)));
    let decoder = construct(
        &mut realm,
        &constructor,
        &[Value::str("utf-8"), Value::Object(keep_bom)],
    );
    let decoded = method(&mut realm, &decoder, "decode", &[bytes]);
    assert_eq!(text(&mut realm, &decoded), "\u{feff}ok");

    let fatal = realm.object();
    fatal.define(Key::name("fatal"), Property::data(Value::Boolean(true)));
    let decoder = construct(
        &mut realm,
        &constructor,
        &[Value::str("utf-8"), Value::Object(fatal)],
    );
    let malformed = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![0xff],
        None,
    ));
    let decode = ops::get(&mut realm, &decoder, &Key::name("decode")).unwrap();
    assert!(ops::call(&mut realm, &decode, decoder.clone(), &[malformed]).is_err());

    let utf16 = construct(
        &mut realm,
        &constructor,
        &[Value::str("utf-16"), Value::Undefined],
    );
    let bytes = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![0xff, 0xfe, b'h', 0, b'i', 0],
        None,
    ));
    let decoded = method(&mut realm, &utf16, "decode", &[bytes]);
    assert_eq!(text(&mut realm, &decoded), "hi");
    assert!(ops::construct(&mut realm, &constructor, &[Value::str("not-an-encoding")]).is_err());
}

#[test]
fn text_encoder_defaults_coerces_usv_text_and_rejects_non_byte_destinations() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextEncoder");
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
    let encoder = construct(&mut realm, &constructor, &[]);
    for args in [&[][..], &[Value::Undefined][..]] {
        let empty = method(&mut realm, &encoder, "encode", args);
        assert_eq!(crate::typed_array_builtin::view_bytes(&empty).unwrap(), []);
    }
    let number = method(&mut realm, &encoder, "encode", &[Value::Number(42.0)]);
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&number).unwrap(),
        b"42"
    );

    let lone_surrogate = Value::String(crate::JsStr::from_units(&[0xd800]));
    let replacement = method(
        &mut realm,
        &encoder,
        "encode",
        core::slice::from_ref(&lone_surrogate),
    );
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&replacement).unwrap(),
        [0xef, 0xbf, 0xbd]
    );
    let destination = Value::Object(crate::typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        3,
    ));
    let result = method(
        &mut realm,
        &encoder,
        "encodeInto",
        &[lone_surrogate, destination.clone()],
    );
    assert!(matches!(
        ops::get(&mut realm, &result, &Key::name("read")).unwrap(),
        Value::Number(1.0)
    ));
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&destination).unwrap(),
        [0xef, 0xbf, 0xbd]
    );

    let invalid = ops::get(&mut realm, &encoder, &Key::name("encodeInto")).unwrap();
    assert!(
        ops::call(
            &mut realm,
            &invalid,
            encoder.clone(),
            &[Value::str("x"), Value::Null],
        )
        .is_err()
    );
    let wide = Value::Object(crate::typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint16,
        1,
    ));
    assert!(ops::call(&mut realm, &invalid, encoder, &[Value::str("x"), wide],).is_err());
}

#[test]
fn text_decoder_covers_windows_1252_aliases_and_unmapped_bytes() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextDecoder");
    let labels = [
        "utf8",
        "unicode-1-1-utf-8",
        "utf-16le",
        "iso-8859-1",
        "latin1",
        "us-ascii",
        "ascii",
        "windows-1252",
    ];
    for label in labels {
        assert!(ops::construct(&mut realm, &constructor, &[Value::str(label)]).is_ok());
    }

    let decoder = construct(&mut realm, &constructor, &[Value::str("windows-1252")]);
    let bytes = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        (0x80..=0x9f).collect(),
        None,
    ));
    let decoded = method(&mut realm, &decoder, "decode", &[bytes]);
    let expected = [
        0x20ac, 0x0081, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008d, 0x017d, 0x008f, 0x0090, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x009d, 0x017e, 0x0178,
    ];
    assert_eq!(
        text(&mut realm, &decoded),
        String::from_utf16(&expected).unwrap()
    );
}

#[test]
fn text_decoder_validates_receiver_inputs_options_and_malformed_utf16() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextDecoder");
    let decoder = construct(&mut realm, &constructor, &[]);
    let decode = ops::get(&mut realm, &decoder, &Key::name("decode")).unwrap();
    assert!(ops::call(&mut realm, &decode, Value::Undefined, &[]).is_err());
    assert!(ops::call(&mut realm, &decode, decoder.clone(), &[Value::Null]).is_err());
    let empty = method(&mut realm, &decoder, "decode", &[]);
    assert_eq!(text(&mut realm, &empty), "");

    let throwing_getter = realm.function("get fatal", 0, false, |realm, _, _| {
        Err(realm.throw(crate::ErrorKind::Type, "fatal option getter failed"))
    });
    let options = realm.object();
    options.define(
        Key::name("fatal"),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(throwing_getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(
        ops::construct(
            &mut realm,
            &constructor,
            &[Value::str("utf-8"), Value::Object(options)],
        )
        .is_err()
    );

    let utf16 = construct(&mut realm, &constructor, &[Value::str("utf-16le")]);
    let odd_bytes = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![b'a', 0, 0xff],
        None,
    ));
    let decoded = method(
        &mut realm,
        &utf16,
        "decode",
        core::slice::from_ref(&odd_bytes),
    );
    assert_eq!(text(&mut realm, &decoded), "a�");
    let fatal_options = realm.object();
    fatal_options.define(Key::name("fatal"), Property::data(Value::Boolean(true)));
    let fatal_utf16 = construct(
        &mut realm,
        &constructor,
        &[Value::str("utf-16le"), Value::Object(fatal_options)],
    );
    let fatal_decode = ops::get(&mut realm, &fatal_utf16, &Key::name("decode")).unwrap();
    assert!(ops::call(&mut realm, &fatal_decode, fatal_utf16, &[odd_bytes]).is_err());
}

#[test]
fn text_decoder_exposes_options_and_reads_array_buffers() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "TextDecoder");
    let options = realm.object();
    options.define(Key::name("fatal"), Property::data(Value::Boolean(true)));
    options.define(Key::name("ignoreBOM"), Property::data(Value::Boolean(true)));
    let decoder = construct(
        &mut realm,
        &constructor,
        &[Value::str("utf-8"), Value::Object(options)],
    );
    for (name, expected) in [("encoding", 0), ("fatal", 1), ("ignoreBOM", 2)] {
        let value = ops::get(&mut realm, &decoder, &Key::name(name)).unwrap();
        match expected {
            0 => assert_eq!(text(&mut realm, &value), "utf-8"),
            1 | 2 => assert!(matches!(value, Value::Boolean(true))),
            _ => unreachable!(),
        }
    }
    let array_buffer_constructor = global(&mut realm, "ArrayBuffer");
    let buffer = construct(&mut realm, &array_buffer_constructor, &[Value::Number(2.0)]);
    let decoded = method(&mut realm, &decoder, "decode", &[buffer]);
    assert_eq!(text(&mut realm, &decoded), "\0\0");
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
}

#[test]
fn text_decoder_internal_helpers_cover_private_accessors_and_unknown_encodings() {
    let key = Symbol::private("TextDecoder.encoding");
    let object = Object::new(None, Kind::Ordinary);
    object.set_private(
        &key,
        Property {
            slot: crate::Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert_eq!(private_text(&object, &key), None);
    object.set_private(&key, Property::data(Value::Boolean(false)));
    assert_eq!(private_text(&object, &key), None);
    let mut realm = Realm::new();
    assert!(decode_bytes(&mut realm, "unknown", &[], false, false).is_err());
    let encoder = global(&mut realm, "TextEncoder");
    assert!(ops::call(&mut realm, &encoder, Value::Undefined, &[]).is_err());
    let decoder = global(&mut realm, "TextDecoder");
    let prototype = ops::get(&mut realm, &decoder, &Key::name("prototype")).unwrap();
    let property = prototype
        .as_object()
        .unwrap()
        .own_property(&Key::name("encoding"))
        .unwrap();
    let Slot::Accessor {
        get: Some(getter), ..
    } = property.slot
    else {
        panic!("encoding getter")
    };
    for receiver in [Value::Null, Value::Object(realm.object())] {
        assert!(ops::call(&mut realm, &Value::Object(getter.clone()), receiver, &[]).is_err());
    }
    let decode = ops::get(&mut realm, &prototype, &Key::name("decode")).unwrap();
    let invalid_receiver = Value::Object(realm.object());
    assert!(ops::call(&mut realm, &decode, invalid_receiver, &[]).is_err());
}

#[test]
fn text_encoder_writes_only_inside_the_destination_view() {
    let mut realm = Realm::new();
    let encoder_constructor = global(&mut realm, "TextEncoder");
    let encoder = construct(&mut realm, &encoder_constructor, &[]);
    let backing = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![9; 6],
        None,
    ));
    let buffer = ops::get(&mut realm, &backing, &Key::name("buffer")).unwrap();
    let view_constructor = global(&mut realm, "Uint8Array");
    let destination = construct(
        &mut realm,
        &view_constructor,
        &[buffer, Value::Number(2.0), Value::Number(3.0)],
    );
    method(
        &mut realm,
        &encoder,
        "encodeInto",
        &[Value::str("Aé"), destination],
    );
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&backing).unwrap(),
        [9, 9, 65, 0xc3, 0xa9, 9]
    );
}
