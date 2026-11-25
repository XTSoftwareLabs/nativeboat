use super::*;

#[test]
fn non_array_object_replacers_are_ignored_without_reading_their_properties() {
    let mut realm = Realm::new();
    let ignored = realm.object();
    let getter = realm.function("length", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "must not read"))
    });
    ignored.define(
        Key::name("length"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let value = Value::Object(realm.array(vec![n(1.0)]));
    let serialized = json(&mut realm, "stringify", &[value, Value::Object(ignored)]);
    assert_eq!(text(&mut realm, serialized), "[1]");
}

#[test]
fn indentation_preserves_utf16_even_when_truncated_between_surrogates() {
    let mut realm = Realm::new();
    for units in [
        vec![0xd800],
        vec![0xd801, 0xdc01],
        [vec![b' ' as u16; 9], vec![0xd800, 0xdc00]].concat(),
    ] {
        let array = Value::Object(realm.array(vec![n(1.0)]));
        let space = Value::String(JsStr::from_units(&units));
        let serialized = json(&mut realm, "stringify", &[array, Value::Null, space]).unwrap();
        let mut expected = vec![b'[' as u16, b'\n' as u16];
        expected.extend_from_slice(&units[..units.len().min(10)]);
        expected.extend([b'1' as u16, b'\n' as u16, b']' as u16]);
        assert!(matches!(serialized, Value::String(text) if text.units() == expected));
    }
}

#[test]
fn recursive_input_and_callback_mutation_raise_range_errors_without_poisoning_state() {
    // Values nest as deep as on the application's main thread's stack.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(nesting_limits)
        .unwrap()
        .join()
        .unwrap();
}

fn nesting_limits() {
    let mut realm = Realm::new();
    let boundary = format!("{}0{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    let parsed = json(&mut realm, "parse", &[s(&boundary)]).unwrap();
    let serialized = json(&mut realm, "stringify", &[parsed]);
    assert_eq!(text(&mut realm, serialized), boundary);
    let excessive = format!("[{boundary}]");
    let failed = json(&mut realm, "parse", &[s(&excessive)]);
    assert_eq!(
        message(&mut realm, failed),
        "RangeError: Maximum JSON nesting depth exceeded"
    );
    let mut chain = Value::Number(0.0);
    for _ in 0..=MAX_DEPTH {
        chain = Value::Object(realm.array(vec![chain]));
    }
    let failed = json(&mut realm, "stringify", &[chain]);
    assert_eq!(
        message(&mut realm, failed),
        "RangeError: Maximum JSON nesting depth exceeded"
    );
    let reviver = function(&realm, |_, this, args| {
        if matches!(&args[0], Value::String(key) if key.units() == JsStr::from("first").units()) {
            ops::define_data(this.as_object().unwrap(), Key::name("later"), this.clone());
        }
        Ok(args[1].clone())
    });
    let failed = json(
        &mut realm,
        "parse",
        &[s(r#"{"first":0,"later":{}}"#), reviver],
    );
    assert_eq!(
        message(&mut realm, failed),
        "RangeError: Maximum JSON nesting depth exceeded"
    );
    assert_eq!(round_trip(&mut realm, "[1]"), "[1]");
}

/// The text of a result computed with the realm.
macro_rules! t {
    ($realm:ident, $value:expr) => {{
        let value = $value;
        text(&mut $realm, value)
    }};
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn json(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let global = Value::Object(realm.global.clone());
    let json = ops::get(realm, &global, &Key::name("JSON")).unwrap();
    let function = ops::get(realm, &json, &Key::name(name)).unwrap();
    ops::call(realm, &function, json, args)
}

fn text(realm: &mut Realm, value: Completion) -> String {
    let value = value.unwrap();
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

/// Parses and stringifies again.
fn round_trip(realm: &mut Realm, source: &str) -> String {
    let parsed = json(realm, "parse", &[s(source)]).unwrap();
    let result = json(realm, "stringify", &[parsed]);
    text(realm, result)
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

fn function(
    realm: &Realm,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 0, false, code))
}

#[test]
fn parsing_accepts_the_json_grammar() {
    let mut realm = Realm::new();
    let source = r#" {"a": [1, -0.5e2, 2E+1, 3e-1, 0, true, false, null, "x\"\\\/\b\f\n\r\t\u0041\ud800"], "b": {}, "c": [], "__proto__": 1, "a2": "z", "a2": "w"} "#;
    assert_eq!(
        round_trip(&mut realm, source),
        r#"{"a":[1,-50,20,0.3,0,true,false,null,"x\"\\/\b\f\n\r\tA\ud800"],"b":{},"c":[],"__proto__":1,"a2":"w"}"#
    );
    let parsed = json(&mut realm, "parse", &[s(r#"{"__proto__": {"x": 1}}"#)]).unwrap();
    let prototype = parsed.as_object().unwrap().data().prototype.clone();
    assert_eq!(prototype, Some(realm.object_prototype.clone()));
    assert_eq!(
        t!(realm, json(&mut realm, "parse", &[s("\"\\u00e9\"")])),
        "é"
    );
    assert_eq!(t!(realm, json(&mut realm, "parse", &[n(42.0)])), "42");
    let symbol = Value::Symbol(realm.symbols.iterator.clone());
    let thrown = json(&mut realm, "parse", &[symbol]);
    assert!(message(&mut realm, thrown).starts_with("TypeError"));
}

#[test]
fn parsing_reports_native_syntax_diagnostics() {
    let mut realm = Realm::new();
    for (source, expected) in [
        ("", "SyntaxError: Unexpected end of JSON input"),
        ("{", "SyntaxError: Unexpected end of JSON input"),
        (
            "[1,]",
            "SyntaxError: Unexpected token ']', \"[1,]\" is not valid JSON",
        ),
        (
            "{'a':1}",
            "SyntaxError: Unexpected token ''', \"{'a':1}\" is not valid JSON",
        ),
        (
            "{\"a\" 1}",
            "SyntaxError: Unexpected token '1', \"{\"a\" 1}\" is not valid JSON",
        ),
        (
            "[1 2]",
            "SyntaxError: Unexpected token '2', \"[1 2]\" is not valid JSON",
        ),
        ("tru", "SyntaxError: Unexpected end of JSON input"),
        (
            "nul!",
            "SyntaxError: Unexpected token '!', \"nul!\" is not valid JSON",
        ),
        (
            "01",
            "SyntaxError: Unexpected token '1', \"01\" is not valid JSON",
        ),
        ("-", "SyntaxError: Unexpected end of JSON input"),
        ("1.", "SyntaxError: Unexpected end of JSON input"),
        ("1e", "SyntaxError: Unexpected end of JSON input"),
        (
            "\"\\x\"",
            "SyntaxError: Unexpected token 'x', \"\"\\x\"\" is not valid JSON",
        ),
        (
            "\"\\u12g4\"",
            "SyntaxError: Unexpected token 'g', \"\"\\u12g4\"\" is not valid JSON",
        ),
        (
            "\"a\nb\"",
            "SyntaxError: Unexpected token '\n', \"\"a\nb\"\" is not valid JSON",
        ),
        ("\"open", "SyntaxError: Unexpected end of JSON input"),
        (
            "+1",
            "SyntaxError: Unexpected token '+', \"+1\" is not valid JSON",
        ),
        (
            "1 1",
            "SyntaxError: Unexpected token '1', \"1 1\" is not valid JSON",
        ),
    ] {
        let thrown = json(&mut realm, "parse", &[s(source)]);
        assert_eq!(message(&mut realm, thrown), expected, "{source:?}");
    }
}

#[test]
fn revivers_transform_and_delete_values_bottom_up() {
    let mut realm = Realm::new();
    let log = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new()));
    let seen = log.clone();
    let reviver = function(&realm, move |realm, _, args| {
        let key = ops::to_string(realm, &args[0])?.to_rust_lossy();
        seen.borrow_mut().push(key.clone());
        Ok(match (key.as_str(), &args[1]) {
            ("drop", _) | ("1", _) => Value::Undefined,
            (_, Value::Number(number)) => Value::Number(number * 10.0),
            (_, value) => value.clone(),
        })
    });
    let parsed = json(
        &mut realm,
        "parse",
        &[s(r#"{"a": 1, "drop": 2, "list": [3, 4, 5]}"#), reviver],
    )
    .unwrap();
    assert_eq!(*log.borrow(), ["a", "drop", "0", "1", "2", "list", ""]);
    let result = json(&mut realm, "stringify", &[parsed]);
    assert_eq!(t!(realm, result), r#"{"a":10,"list":[30,null,50]}"#);
    // Properties that cannot change are left as they are: the first key's
    // reviver call locks its later siblings before they are revived.
    let locker = function(&realm, |realm, this, args| {
        let key = ops::to_string(realm, &args[0])?.to_rust_lossy();
        if key == "a" {
            let holder = this.as_object().unwrap();
            for (name, value) in [("fixed", "kept"), ("locked", "same")] {
                holder.define(
                    Key::name(name),
                    Property {
                        slot: Slot::Data {
                            value: Value::str(value),
                            writable: false,
                        },
                        enumerable: true,
                        configurable: false,
                    },
                );
            }
        }
        Ok(match key.as_str() {
            "fixed" => Value::Undefined,
            "locked" => Value::str("changed"),
            _ => args[1].clone(),
        })
    });
    let parsed = json(
        &mut realm,
        "parse",
        &[s(r#"{"inner": {"a": 0, "fixed": 1, "locked": 2}}"#), locker],
    )
    .unwrap();
    let result = json(&mut realm, "stringify", &[parsed]);
    assert_eq!(
        t!(realm, result),
        r#"{"inner":{"a":0,"fixed":"kept","locked":"same"}}"#
    );
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "reviver"))
    });
    let thrown = json(&mut realm, "parse", &[s("[1]"), thrower]);
    assert_eq!(message(&mut realm, thrown), "RangeError: reviver");
    let ignored = json(&mut realm, "parse", &[s("[1]"), n(1.0)]).unwrap();
    assert!(ignored.as_object().unwrap().is_array());
}

#[test]
fn stringify_serializes_values_with_replacers_and_gaps() {
    let mut realm = Realm::new();
    let object = realm.object();
    ops::define_data(&object, Key::name("b"), n(2.0));
    ops::define_data(&object, Key::name("a"), Value::str("x\u{1}"));
    ops::define_data(
        &object,
        Key::name("f"),
        function(&realm, |_, _, _| Ok(Value::Undefined)),
    );
    ops::define_data(&object, Key::name("u"), Value::Undefined);
    ops::define_data(&object, Key::name("nan"), n(f64::NAN));
    ops::define_data(&object, Key::Symbol(realm.symbols.iterator.clone()), n(1.0));
    let list = realm.array(vec![
        n(1.0),
        Value::Undefined,
        function(&realm, |_, _, _| Ok(Value::Undefined)),
        Value::Object(realm.primitive_object(Wrapped::Number(3.0))),
        Value::Object(realm.primitive_object(Wrapped::String("s".into()))),
        Value::Object(realm.primitive_object(Wrapped::Boolean(true))),
        Value::Object(realm.array(vec![])),
    ]);
    ops::define_data(&object, Key::name("list"), Value::Object(list));
    let value = Value::Object(object.clone());
    let compact = json(&mut realm, "stringify", core::slice::from_ref(&value));
    assert_eq!(
        t!(realm, compact),
        r#"{"b":2,"a":"x\u0001","nan":null,"list":[1,null,null,3,"s",true,[]]}"#
    );
    let pretty = json(
        &mut realm,
        "stringify",
        &[value.clone(), Value::Null, n(2.0)],
    );
    assert_eq!(
        t!(realm, pretty),
        "{\n  \"b\": 2,\n  \"a\": \"x\\u0001\",\n  \"nan\": null,\n  \"list\": [\n    1,\n    null,\n    null,\n    3,\n    \"s\",\n    true,\n    []\n  ]\n}"
    );
    let keys = Value::Object(realm.array(vec![
        Value::str("a"),
        n(1.0),
        Value::str("a"),
        Value::Object(realm.primitive_object(Wrapped::String("b".into()))),
        Value::Object(realm.primitive_object(Wrapped::Number(7.0))),
        Value::Boolean(true),
    ]));
    let picked = json(&mut realm, "stringify", &[value.clone(), keys, s("--")]);
    assert_eq!(
        t!(realm, picked),
        "{\n--\"a\": \"x\\u0001\",\n--\"b\": 2\n}"
    );
    let tab =
        Value::Object(realm.primitive_object(Wrapped::String("\t\t\t\t\t\t\t\t\t\t\t\t".into())));
    let one = Value::Object(realm.array(vec![n(1.0)]));
    let tabbed = json(&mut realm, "stringify", &[one, Value::Null, tab]);
    assert_eq!(t!(realm, tabbed), "[\n\t\t\t\t\t\t\t\t\t\t1\n]");
    let wide = Value::Object(realm.primitive_object(Wrapped::Number(20.0)));
    let one = Value::Object(realm.array(vec![n(1.0)]));
    let spaced = json(&mut realm, "stringify", &[one, Value::Null, wide]);
    assert_eq!(t!(realm, spaced), format!("[\n{}1\n]", " ".repeat(10)));
    let plain = Value::Object(realm.object());
    let other = json(&mut realm, "stringify", &[n(1.0), Value::Null, plain]);
    assert_eq!(t!(realm, other), "1");
    // A replacer function sees every key, with the holder as receiver.
    let replacer = function(&realm, |realm, _, args| {
        let key = ops::to_string(realm, &args[0])?.to_rust_lossy();
        Ok(if key == "b" {
            Value::str("replaced")
        } else {
            args[1].clone()
        })
    });
    let replaced = json(&mut realm, "stringify", &[value, replacer]);
    assert!(t!(realm, replaced).contains(r#""b":"replaced""#));
    // toJSON runs with the key.
    let dated = realm.object();
    let to_json = function(&realm, |realm, _, args| {
        let key = ops::to_string(realm, &args[0])?;
        Ok(Value::String(JsStr::from("at ").concat(&key)))
    });
    ops::define_data(&dated, Key::name("toJSON"), to_json);
    let holder = realm.object();
    ops::define_data(&holder, Key::name("when"), Value::Object(dated));
    let result = json(&mut realm, "stringify", &[Value::Object(holder)]);
    assert_eq!(t!(realm, result), r#"{"when":"at when"}"#);
    for value in [
        Value::Undefined,
        function(&realm, |_, _, _| Ok(Value::Undefined)),
    ] {
        assert!(matches!(
            json(&mut realm, "stringify", &[value]).unwrap(),
            Value::Undefined
        ));
    }
    assert_eq!(
        t!(
            realm,
            json(&mut realm, "stringify", &[Value::Boolean(false)])
        ),
        "false"
    );
    let lone = Value::String(JsStr::from_units(&[0xD800]));
    assert_eq!(
        t!(realm, json(&mut realm, "stringify", &[lone])),
        "\"\\ud800\""
    );
    assert_eq!(
        t!(realm, json(&mut realm, "stringify", &[n(f64::INFINITY)])),
        "null"
    );
}

#[test]
fn stringify_rejects_cycles_and_propagates_abrupt_completions() {
    let mut realm = Realm::new();
    let cyclic = realm.object();
    ops::define_data(&cyclic, Key::name("self"), Value::Object(cyclic.clone()));
    let thrown = json(&mut realm, "stringify", &[Value::Object(cyclic)]);
    assert_eq!(
        message(&mut realm, thrown),
        "TypeError: Converting circular structure to JSON"
    );
    let thrower = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "abrupt"))
    });
    let throwing = realm.object();
    ops::define_data(&throwing, Key::name("toJSON"), thrower.clone());
    let thrown = json(&mut realm, "stringify", &[Value::Object(throwing)]);
    assert_eq!(message(&mut realm, thrown), "RangeError: abrupt");
    let thrown = json(&mut realm, "stringify", &[n(1.0), thrower.clone()]);
    assert_eq!(message(&mut realm, thrown), "RangeError: abrupt");
    let failing = realm.object();
    ops::define_data(&failing, Key::name("valueOf"), thrower.clone());
    let wrapped = realm.primitive_object(Wrapped::Number(1.0));
    wrapped.set_prototype(Some(failing.clone()));
    let thrown = json(&mut realm, "stringify", &[Value::Object(wrapped)]);
    assert_eq!(message(&mut realm, thrown), "RangeError: abrupt");
    let space = realm.primitive_object(Wrapped::Number(1.0));
    space.set_prototype(Some(failing));
    let thrown = json(
        &mut realm,
        "stringify",
        &[n(1.0), Value::Null, Value::Object(space)],
    );
    assert_eq!(message(&mut realm, thrown), "RangeError: abrupt");
    // Property-list elements are read and converted in order.
    let list = realm.array(vec![]);
    list.define(
        Key::Index(0),
        Property {
            slot: Slot::Accessor {
                get: thrower.as_object().cloned(),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let keys = Value::Object(list);
    let thrown = json(&mut realm, "stringify", &[n(1.0), keys]);
    assert!(message(&mut realm, thrown).starts_with("RangeError"));
    let unnamed = realm.primitive_object(Wrapped::String("k".into()));
    let failing_text = realm.object();
    ops::define_data(&failing_text, Key::name("toString"), thrower.clone());
    unnamed.set_prototype(Some(failing_text));
    let keys = Value::Object(realm.array(vec![Value::Object(unnamed)]));
    let thrown = json(&mut realm, "stringify", &[n(1.0), keys]);
    assert!(message(&mut realm, thrown).starts_with("RangeError"));
}
