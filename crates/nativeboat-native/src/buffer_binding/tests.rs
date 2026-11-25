//! Expected results follow Node 22's `src/node_buffer.cc`.

use super::*;
use crate::typed_array_builtin::new_bytes;

fn bytes(realm: &Realm, data: &[u8]) -> Value {
    Value::Object(new_bytes(realm, data.to_vec(), None))
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn data(value: &Value) -> Vec<u8> {
    crate::typed_array_builtin::view_bytes(value).unwrap()
}

fn shown(realm: &mut Realm, result: Completion) -> String {
    match result {
        Ok(Value::Object(object))
            if crate::typed_array_builtin::view_range(&Value::Object(object.clone())).is_some() =>
        {
            let bytes = data(&Value::Object(object));
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        }
        Ok(value) => ops::to_string(realm, &value).unwrap().to_rust_lossy(),
        Err(Thrown(error)) => {
            let code = ops::get(realm, &error, &Key::name("code")).unwrap();
            let code = ops::to_string(realm, &code).unwrap().to_rust_lossy();
            let text = ops::to_string(realm, &error).unwrap().to_rust_lossy();
            format!("{code}|{text}")
        }
    }
}

struct Binding {
    realm: Realm,
    exports: Value,
}

impl Binding {
    fn new() -> Binding {
        let mut realm = Realm::new();
        let exports = Value::Object(binding(&mut realm));
        Binding { realm, exports }
    }

    fn call(&mut self, name: &str, this: Value, args: &[Value]) -> String {
        let function = ops::get(&mut self.realm, &self.exports, &Key::name(name)).unwrap();
        let result = ops::call(&mut self.realm, &function, this, args);
        shown(&mut self.realm, result)
    }
}

#[test]
fn lengths_comparisons_copies_and_swaps() {
    let mut binding = Binding::new();
    let realm = &binding.realm;
    let (a, b, empty) = (
        bytes(realm, b"abc"),
        bytes(realm, b"abd"),
        bytes(realm, b""),
    );
    let not_buffer = Value::Object(binding.realm.object());
    assert_eq!(
        binding.call("byteLengthUtf8", Value::Undefined, &[s("\u{e9}a")]),
        "3"
    );
    assert_eq!(
        binding.call("byteLengthUtf8", Value::Undefined, &[n(1.0)]),
        "0"
    );
    for (left, right, expected) in [
        (a.clone(), b.clone(), "-1"),
        (b.clone(), a.clone(), "1"),
        (a.clone(), a.clone(), "0"),
        (a.clone(), empty.clone(), "1"),
        (empty.clone(), a.clone(), "-1"),
    ] {
        assert_eq!(
            binding.call("compare", Value::Undefined, &[left, right]),
            expected
        );
    }
    assert_eq!(
        binding.call(
            "compare",
            Value::Undefined,
            &[not_buffer.clone(), a.clone()]
        ),
        "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a buffer"
    );
    assert_eq!(
        binding.call(
            "compare",
            Value::Undefined,
            &[a.clone(), not_buffer.clone()]
        ),
        "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a buffer"
    );
    for (args, expected) in [
        (
            vec![a.clone(), b.clone(), n(0.0), n(0.0), n(2.0), n(2.0)],
            "0",
        ),
        (vec![a.clone(), b.clone()], "-1"),
        (
            vec![a.clone(), b.clone(), n(4.0)],
            "ERR_OUT_OF_RANGE|RangeError [ERR_OUT_OF_RANGE]: The value of \"targetStart\" is out of range.",
        ),
        (
            vec![a.clone(), b.clone(), n(0.0), n(4.0)],
            "ERR_OUT_OF_RANGE|RangeError [ERR_OUT_OF_RANGE]: The value of \"sourceStart\" is out of range.",
        ),
        (
            vec![a.clone(), b.clone(), n(-1.0)],
            "ERR_OUT_OF_RANGE|RangeError [ERR_OUT_OF_RANGE]: Index out of range",
        ),
        (
            vec![
                a.clone(),
                b.clone(),
                Value::Undefined,
                n(f64::NAN),
                n(1e30),
                n(1.0),
            ],
            "-1",
        ),
        (
            vec![a.clone(), not_buffer.clone()],
            "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a buffer",
        ),
        (
            vec![not_buffer.clone(), a.clone()],
            "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a buffer",
        ),
    ] {
        assert_eq!(
            binding.call("compareOffset", Value::Undefined, &args),
            expected
        );
    }
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(
        binding
            .call(
                "compareOffset",
                Value::Undefined,
                &[a.clone(), b.clone(), symbol.clone()]
            )
            .starts_with("undefined|TypeError")
    );
    let target = bytes(&binding.realm, b"......");
    assert_eq!(
        binding.call(
            "copy",
            Value::Undefined,
            &[a.clone(), target.clone(), n(1.0), n(1.0), n(9.0)]
        ),
        "2"
    );
    assert_eq!(data(&target), b".bc...");
    assert_eq!(
        binding.call(
            "copy",
            Value::Undefined,
            &[a.clone(), target.clone(), n(0.0), n(0.0), n(0.0)]
        ),
        "0"
    );
    for args in [
        vec![not_buffer.clone(), target.clone()],
        vec![a.clone(), not_buffer.clone()],
    ] {
        assert!(
            binding
                .call("copy", Value::Undefined, &args)
                .starts_with("ERR_INVALID_ARG_TYPE")
        );
    }
    for index in 2..5 {
        let mut args = vec![a.clone(), target.clone(), n(0.0), n(0.0), n(0.0)];
        args[index] = symbol.clone();
        assert!(
            binding
                .call("copy", Value::Undefined, &args)
                .starts_with("undefined|TypeError")
        );
    }
    for (name, expected) in [
        ("swap16", b"badcfehg"),
        ("swap32", b"dcbahgfe"),
        ("swap64", b"hgfedcba"),
    ] {
        let swapped = bytes(&binding.realm, b"abcdefgh");
        binding.call(name, Value::Undefined, std::slice::from_ref(&swapped));
        assert_eq!(&data(&swapped), expected, "{name}");
    }
    assert!(
        binding
            .call("swap16", Value::Undefined, &[not_buffer])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
}

#[test]
fn fills_follow_node_rules() {
    let mut binding = Binding::new();
    let target = bytes(&binding.realm, &[0; 7]);
    let pattern = bytes(&binding.realm, b"xy");
    let empty = bytes(&binding.realm, b"");
    for (args, expected_result, expected) in [
        (
            vec![target.clone(), pattern.clone(), n(1.0), n(6.0)],
            "undefined",
            "00787978797800",
        ),
        (
            vec![target.clone(), s("\u{e9}"), n(0.0), n(7.0)],
            "undefined",
            "c3a9c3a9c3a9c3",
        ),
        (
            vec![target.clone(), s("ab"), n(0.0), n(3.0), s("ucs2")],
            "undefined",
            "610062a9c3a9c3",
        ),
        (
            vec![target.clone(), s("abcd"), n(0.0), n(7.0), s("hex")],
            "undefined",
            "abcdabcdabcdab",
        ),
        (
            vec![target.clone(), s("zz"), n(0.0), n(2.0), s("hex")],
            "-1",
            "abcdabcdabcdab",
        ),
        (
            vec![target.clone(), s("x"), n(0.0), n(1.0), n(5.0)],
            "undefined",
            "78cdabcdabcdab",
        ),
        (
            vec![target.clone(), n(257.0), n(0.0), n(7.0)],
            "undefined",
            "01010101010101",
        ),
        (
            vec![target.clone(), empty.clone(), n(0.0), n(0.0)],
            "undefined",
            "01010101010101",
        ),
        (
            vec![target.clone(), empty, n(0.0), n(1.0)],
            "-1",
            "01010101010101",
        ),
        (
            vec![target.clone(), n(1.0), n(5.0), n(4.0)],
            "-2",
            "01010101010101",
        ),
        (
            vec![target.clone(), n(1.0), n(0.0), n(8.0)],
            "-2",
            "01010101010101",
        ),
    ] {
        assert_eq!(
            binding.call("fill", Value::Undefined, &args),
            expected_result
        );
        let bytes: String = data(&target)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(bytes, expected);
    }
    let not_buffer = Value::Object(binding.realm.object());
    assert!(
        binding
            .call("fill", Value::Undefined, &[not_buffer, n(1.0)])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    assert!(
        binding
            .call("fill", Value::Undefined, &[target.clone(), n(1.0), n(-1.0)])
            .starts_with("ERR_OUT_OF_RANGE")
    );
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(
        binding
            .call(
                "fill",
                Value::Undefined,
                &[target.clone(), symbol, n(0.0), n(1.0)]
            )
            .starts_with("undefined|TypeError")
    );
}

#[test]
fn validation_and_searches() {
    let mut binding = Binding::new();
    let realm = &binding.realm;
    let (valid, invalid) = (bytes(realm, &[0xc3, 0xa9]), bytes(realm, &[0xff]));
    let buffer = binding.realm.array_buffer_prototype.clone();
    let array_buffer = Value::Object(crate::Object::new(
        Some(buffer),
        crate::Kind::ArrayBuffer(alloc::rc::Rc::new(core::cell::RefCell::new(
            b"abc".to_vec(),
        ))),
    ));
    for (name, value, expected) in [
        ("isUtf8", valid.clone(), "true"),
        ("isUtf8", invalid.clone(), "false"),
        ("isAscii", valid, "false"),
        ("isAscii", array_buffer, "true"),
    ] {
        assert_eq!(binding.call(name, Value::Undefined, &[value]), expected);
    }
    assert!(
        binding
            .call("isUtf8", Value::Undefined, &[n(1.0)])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    let haystack = bytes(&binding.realm, b"abcabc");
    let needle = bytes(&binding.realm, b"bc");
    let empty = bytes(&binding.realm, b"");
    let wide = bytes(&binding.realm, b"a\0b\0c\0b\0");
    let wide_needle = bytes(&binding.realm, b"b\0");
    for (args, expected) in [
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(0.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(2.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "4",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(-1.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "4",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(3.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(-10.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(-10.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "-1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(9.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "-1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(9.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "4",
        ),
        (
            vec![
                haystack.clone(),
                empty.clone(),
                n(2.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "2",
        ),
        (
            vec![
                haystack.clone(),
                empty.clone(),
                n(9.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "6",
        ),
        (
            vec![
                haystack.clone(),
                empty.clone(),
                n(-9.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "0",
        ),
        (
            vec![
                empty.clone(),
                needle.clone(),
                n(0.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "-1",
        ),
        (
            vec![
                needle.clone(),
                haystack.clone(),
                n(0.0),
                n(1.0),
                Value::Boolean(false),
            ],
            "-1",
        ),
        (
            vec![
                wide.clone(),
                wide_needle.clone(),
                n(0.0),
                n(3.0),
                Value::Boolean(true),
            ],
            "2",
        ),
        (
            vec![
                wide.clone(),
                wide_needle.clone(),
                n(7.0),
                n(3.0),
                Value::Boolean(false),
            ],
            "6",
        ),
        (
            vec![
                wide.clone(),
                bytes(&binding.realm, b"b"),
                n(0.0),
                n(3.0),
                Value::Boolean(true),
            ],
            "-1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(f64::INFINITY),
                n(1.0),
                Value::Boolean(false),
            ],
            "4",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                n(f64::NEG_INFINITY),
                n(1.0),
                Value::Boolean(true),
            ],
            "1",
        ),
        (
            vec![
                haystack.clone(),
                needle.clone(),
                Value::Undefined,
                n(1.0),
                Value::Boolean(true),
            ],
            "1",
        ),
    ] {
        assert_eq!(
            binding.call("indexOfBuffer", Value::Undefined, &args),
            expected
        );
    }
    for (args, expected) in [
        (
            vec![haystack.clone(), n(98.0), n(0.0), Value::Boolean(true)],
            "1",
        ),
        (
            vec![
                haystack.clone(),
                n(98.0 + 256.0),
                n(-1.0),
                Value::Boolean(false),
            ],
            "4",
        ),
        (
            vec![haystack.clone(), n(122.0), n(0.0), Value::Boolean(true)],
            "-1",
        ),
    ] {
        assert_eq!(
            binding.call("indexOfNumber", Value::Undefined, &args),
            expected
        );
    }
    for (args, expected) in [
        (
            vec![
                haystack.clone(),
                s("ca"),
                n(0.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "2",
        ),
        (
            vec![
                haystack.clone(),
                s("b"),
                n(5.0),
                n(4.0),
                Value::Boolean(false),
            ],
            "4",
        ),
        (
            vec![
                haystack.clone(),
                s("b"),
                n(0.0),
                n(0.0),
                Value::Boolean(true),
            ],
            "1",
        ),
        (
            vec![
                bytes(&binding.realm, b"a\0b\0c"),
                s("b"),
                n(0.0),
                n(3.0),
                Value::Boolean(true),
            ],
            "2",
        ),
        (
            vec![
                haystack.clone(),
                n(1.0),
                n(0.0),
                n(1.0),
                Value::Boolean(true),
            ],
            "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a string",
        ),
    ] {
        assert_eq!(
            binding.call("indexOfString", Value::Undefined, &args),
            expected
        );
    }
    let not_buffer = Value::Object(binding.realm.object());
    for name in ["indexOfBuffer", "indexOfNumber", "indexOfString"] {
        assert!(
            binding
                .call(name, Value::Undefined, std::slice::from_ref(&not_buffer))
                .starts_with("ERR_INVALID_ARG_TYPE")
        );
    }
    assert!(
        binding
            .call(
                "indexOfBuffer",
                Value::Undefined,
                &[haystack.clone(), not_buffer]
            )
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(
        binding
            .call("indexOfNumber", Value::Undefined, &[haystack, symbol])
            .starts_with("undefined|TypeError")
    );
}

#[test]
fn strings_slice_and_write_in_every_encoding() {
    let mut binding = Binding::new();
    let buffer = bytes(&binding.realm, &[0xe9, 0x41, 0xff, 0x80]);
    for (name, args, expected) in [
        ("asciiSlice", vec![], "iA\u{7f}\u{0}"),
        ("latin1Slice", vec![n(1.0), n(2.0)], "A"),
        ("hexSlice", vec![n(2.0)], "ff80"),
        ("base64Slice", vec![], "6UH/gA=="),
        ("base64urlSlice", vec![], "6UH_gA"),
        ("ucs2Slice", vec![], "\u{41e9}\u{80ff}"),
        ("utf8Slice", vec![n(1.0), n(2.0)], "A"),
        ("hexSlice", vec![n(3.0), n(1.0)], ""),
        (
            "hexSlice",
            vec![n(0.0), n(5.0)],
            "ERR_OUT_OF_RANGE|RangeError [ERR_OUT_OF_RANGE]: Index out of range",
        ),
    ] {
        assert_eq!(
            binding.call(name, buffer.clone(), &args),
            expected,
            "{name}"
        );
    }
    let empty = bytes(&binding.realm, b"");
    assert_eq!(binding.call("hexSlice", empty, &[]), "");
    assert!(
        binding
            .call("hexSlice", n(1.0), &[])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    let target = bytes(&binding.realm, &[0; 6]);
    for (name, this, args, expected) in [
        ("hexWrite", target.clone(), vec![s("abcd"), n(1.0)], "2"),
        (
            "base64Write",
            target.clone(),
            vec![s("YWJj"), n(3.0), n(2.0)],
            "2",
        ),
        ("base64urlWrite", target.clone(), vec![s("_w"), n(5.0)], "1"),
        (
            "ucs2Write",
            target.clone(),
            vec![s("ab"), n(0.0), n(3.0)],
            "2",
        ),
        ("hexWrite", target.clone(), vec![s("ab"), n(6.0)], "0"),
        (
            "hexWrite",
            target.clone(),
            vec![s("ab"), n(7.0)],
            "ERR_BUFFER_OUT_OF_BOUNDS|RangeError [ERR_BUFFER_OUT_OF_BOUNDS]: \"offset\" is outside of buffer bounds",
        ),
        (
            "hexWrite",
            target.clone(),
            vec![n(1.0)],
            "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a string",
        ),
        (
            "hexWrite",
            target.clone(),
            vec![s("ab"), n(-1.0)],
            "ERR_OUT_OF_RANGE|RangeError [ERR_OUT_OF_RANGE]: Index out of range",
        ),
    ] {
        assert_eq!(binding.call(name, this, &args), expected, "{name}");
    }
    assert_eq!(data(&target), [0x61, 0x00, 0xcd, 0x61, 0x62, 0xff]);
    for (name, expected) in [
        ("asciiWriteStatic", "2"),
        ("latin1WriteStatic", "2"),
        ("utf8WriteStatic", "2"),
    ] {
        let args = [target.clone(), s("\u{e9}z"), n(4.0)];
        assert_eq!(
            binding.call(name, Value::Undefined, &args),
            expected,
            "{name}"
        );
    }
    assert_eq!(
        binding.call("utf8WriteStatic", Value::Undefined, &[]),
        "ERR_INVALID_ARG_TYPE|TypeError [ERR_INVALID_ARG_TYPE]: argument must be a buffer"
    );
    assert_eq!(
        binding.call("btoa", Value::Undefined, &[s("\u{ff}\u{e9}")]),
        "/+k="
    );
    assert_eq!(
        binding.call("btoa", Value::Undefined, &[s("\u{100}")]),
        "-1"
    );
    assert_eq!(binding.call("atob", Value::Undefined, &[s("YQ==")]), "a");
    assert_eq!(binding.call("atob", Value::Undefined, &[s("Y")]), "-1");
    assert!(
        binding
            .call("atob", Value::Undefined, &[n(1.0)])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    assert!(
        binding
            .call("btoa", Value::Undefined, &[n(1.0)])
            .starts_with("ERR_INVALID_ARG_TYPE")
    );
    let toggle = binding.call("getZeroFillToggle", Value::Undefined, &[]);
    assert_eq!(toggle, "00000000");
    for (name, expected) in [
        ("kMaxLength", "9007199254740991"),
        ("kStringMaxLength", "536870888"),
    ] {
        let value = ops::get(&mut binding.realm, &binding.exports, &Key::name(name)).unwrap();
        assert_eq!(shown(&mut binding.realm, Ok(value)), expected);
    }
}

#[test]
fn transcoding_matches_node_icu() {
    let mut realm = Realm::new();
    let icu = Value::Object(icu(&mut realm));
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let function = ops::get(realm, &icu, &Key::name(name)).unwrap();
        let result = ops::call(realm, &function, Value::Undefined, args);
        shown(realm, result)
    };
    let text: Vec<u8> = "a\u{e9}\u{20ac}\u{1f600}\u{ff}".bytes().collect();
    let wide: Vec<u8> = "a\u{e9}\u{20ac}\u{1f600}"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for (source, from, to, expected) in [
        (text.clone(), "utf8", "latin1", "61e93f3fff"),
        (text.clone(), "utf8", "ascii", "613f3f3f3f"),
        (text.clone(), "utf8", "ucs2", "6100e900ac203dd800deff00"),
        (wide, "ucs2", "utf8", "61c3a9e282acf09f9880"),
        (vec![0x61, 0xe9, 0xff], "latin1", "utf8", "61c3a9c3bf"),
        (vec![0x61, 0xe9], "ascii", "utf8", "61efbfbd"),
        (vec![0xff, 0x41], "utf8", "latin1", "3f41"),
        (vec![0x61, 0xe9], "binary", "ucs2", "6100e900"),
        (text.clone(), "utf8", "hex", "1"),
        (text, "nope", "utf8", "1"),
        (vec![0xff, 0xfe, 0x41], "utf8", "ucs2", "10"),
        (vec![0x41, 0x00, 0x00, 0xd8], "ucs2", "utf8", "10"),
    ] {
        let source = bytes(&realm, &source);
        let result = call(&mut realm, "transcode", &[source, s(from), s(to)]);
        assert_eq!(result, expected, "{from} -> {to}");
    }
    let prototype = realm.object();
    realm.buffer_prototype = Some(prototype.clone());
    let source = bytes(&realm, b"a");
    let function = ops::get(&mut realm, &icu, &Key::name("transcode")).unwrap();
    let result = ops::call(
        &mut realm,
        &function,
        Value::Undefined,
        &[source, s("utf8"), s("ucs2")],
    )
    .unwrap();
    assert_eq!(
        result.as_object().unwrap().data().prototype,
        Some(prototype)
    );
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let source = bytes(&realm, b"a");
    assert!(call(&mut realm, "transcode", &[source, symbol]).starts_with("undefined|TypeError"));
    for (code, expected) in [
        (1.0, "U_ILLEGAL_ARGUMENT_ERROR"),
        (10.0, "U_INVALID_CHAR_FOUND"),
        (0.0, "U_ZERO_ERROR"),
    ] {
        assert_eq!(call(&mut realm, "icuErrName", &[n(code)]), expected);
    }
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(call(&mut realm, "icuErrName", &[symbol]).starts_with("undefined|TypeError"));
    let mut binding = Binding::new();
    let prototype = Value::Object(binding.realm.object());
    binding.call(
        "setBufferPrototype",
        Value::Undefined,
        std::slice::from_ref(&prototype),
    );
    assert_eq!(
        binding.realm.buffer_prototype,
        prototype.as_object().cloned()
    );
}
