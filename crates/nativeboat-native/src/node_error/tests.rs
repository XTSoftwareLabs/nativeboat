//! Expected messages are Node 22's.

use super::*;
use crate::{JsStr, Object};

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn object_with(realm: &Realm, entries: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (name, value) in entries {
        object.define(Key::name(name), Property::data(value.clone()));
    }
    Value::Object(object)
}

fn throwing_getter(realm: &Realm, object: &Object, name: &str) {
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "getter"))
    });
    object.define(
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

#[test]
fn received_values_are_described_like_node() {
    let mut realm = Realm::new();
    let named = realm.function("foo", 0, false, |_, _, _| Ok(Value::Undefined));
    let null_prototype = Object::new(None, Kind::Ordinary);
    let filled = Object::new(None, Kind::Ordinary);
    filled.define(Key::name("a"), Property::data(n(1.0)));
    let hidden_only = Object::new(None, Kind::Ordinary);
    hidden_only.define(Key::name("a"), Property::hidden(n(1.0)));
    let nameless = object_with(&realm, &[]);
    let nameless_constructor = Value::Object(realm.object());
    nameless.as_object().unwrap().define(
        Key::name("constructor"),
        Property::hidden(nameless_constructor.clone()),
    );
    let array = |values: Vec<Value>| {
        let array = realm.array(values);
        array.define(Key::name("constructor"), Property::hidden(Value::Undefined));
        Value::Object(array)
    };
    let empty_array = array(Vec::new());
    let full_array = array(vec![n(1.0)]);
    let full_object = object_with(&realm, &[("constructor", Value::Object(realm.object()))]);
    let symbol = Value::Symbol(crate::Symbol::new(Some(JsStr::from("s"))));
    let cases = [
        (Value::Undefined, "undefined"),
        (Value::Null, "null"),
        (Value::Boolean(true), "type boolean (true)"),
        (n(-0.0), "type number (-0)"),
        (n(f64::NAN), "type number (NaN)"),
        (n(1.5), "type number (1.5)"),
        (Value::str("abc"), "type string ('abc')"),
        (Value::str("a'b"), "type string (\"a'b\")"),
        (
            Value::str(&"x".repeat(30)),
            "type string ('xxxxxxxxxxxxxxxxxxxxxxxxx...')",
        ),
        (symbol, "type symbol (Symbol(s))"),
        (Value::Object(named), "function foo"),
        (object_with(&realm, &[]), "an instance of Object"),
        (Value::Object(null_prototype), "[Object: null prototype] {}"),
        (Value::Object(filled), "[Object: null prototype]"),
        (Value::Object(hidden_only), "[Object: null prototype] {}"),
        (nameless, "{}"),
        (full_object, "[Object]"),
        (empty_array, "[]"),
        (full_array, "[Array]"),
    ];
    for (value, expected) in cases {
        assert_eq!(specific_type(&mut realm, &value).unwrap(), expected);
    }
    // Getters run and their exceptions propagate, as do `in` checks on
    // primitive constructors.
    let failing = realm.object();
    throwing_getter(&realm, &failing, "constructor");
    assert!(specific_type(&mut realm, &Value::Object(failing)).is_err());
    let primitive = object_with(&realm, &[("constructor", Value::str("text"))]);
    let thrown = specific_type(&mut realm, &primitive).unwrap_err();
    assert!(text(&mut realm, &thrown.0).starts_with("TypeError"));
    let unnamed = realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined));
    throwing_getter(&realm, &unnamed, "name");
    assert!(specific_type(&mut realm, &Value::Object(unnamed)).is_err());
    let _ = nameless_constructor;
}

#[test]
fn coded_errors_show_their_code() {
    let mut realm = Realm::new();
    let thrown = invalid_arg_type(&mut realm, "code", &["number"], &Value::str("abc"));
    let error = thrown.0.clone();
    assert_eq!(
        text(&mut realm, &error),
        "TypeError [ERR_INVALID_ARG_TYPE]: The \"code\" argument must be of type number. Received type string ('abc')"
    );
    let keys: Vec<String> = error
        .as_object()
        .unwrap()
        .own_keys()
        .iter()
        .filter(|key| {
            error
                .as_object()
                .unwrap()
                .own_property(key)
                .unwrap()
                .enumerable
        })
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["code"]);
    for (name, expected) in [
        ("options.mode", "The \"options.mode\" property must be"),
        ("first argument", "The first argument must be"),
    ] {
        let thrown = invalid_arg_type(&mut realm, name, &["string"], &n(1.0));
        let message = ops::get(&mut realm, &thrown.0, &Key::name("message")).unwrap();
        assert!(text(&mut realm, &message).starts_with(expected), "{name}");
    }
    let failing = realm.object();
    throwing_getter(&realm, &failing, "constructor");
    let thrown = invalid_arg_type(&mut realm, "x", &["string"], &Value::Object(failing));
    assert_eq!(text(&mut realm, &thrown.0), "RangeError: getter");
    // `toString` reads the current name and message.
    let error = invalid_arg_type(&mut realm, "x", &["string"], &n(1.0)).0;
    let object = error.as_object().unwrap().clone();
    throwing_getter(&realm, &object, "message");
    assert!(ops::to_string(&mut realm, &error).is_err());
    throwing_getter(&realm, &object, "name");
    assert!(ops::to_string(&mut realm, &error).is_err());
}

#[test]
fn out_of_range_numbers_use_node_separators() {
    let mut realm = Realm::new();
    for (received, expected) in [
        (1.5, "Received 1.5"),
        (-0.0, "Received -0"),
        (4_294_967_296.0, "Received 4294967296"),
        (9_007_199_254_740_992.0, "Received 9_007_199_254_740_992"),
        (-9_007_199_254_740_992.0, "Received -9_007_199_254_740_992"),
        (-4_294_967_297.0, "Received -4_294_967_297"),
        (1e21, "Received 1e_+21"),
        (f64::INFINITY, "Received Infinity"),
    ] {
        let thrown = out_of_range(&mut realm, "code", "an integer", &n(received));
        let shown = text(&mut realm, &thrown.0);
        assert!(shown.starts_with("RangeError [ERR_OUT_OF_RANGE]: The value of \"code\" is out of range. It must be an integer."), "{shown}");
        assert!(shown.ends_with(expected), "{shown}");
    }
}

#[test]
fn errors_share_a_prototype_per_code_like_node_classes() {
    let mut realm = Realm::new();
    let first = invalid_arg_type(&mut realm, "a", &["string"], &n(1.0)).0;
    let second = invalid_arg_type(&mut realm, "b", &["string"], &n(2.0)).0;
    let prototype = |value: &Value| value.as_object().unwrap().data().prototype.clone().unwrap();
    assert_eq!(prototype(&first), prototype(&second));
    let ranged = out_of_range(&mut realm, "a", "an integer", &n(1.5)).0;
    assert_ne!(prototype(&first), prototype(&ranged));
    assert_eq!(
        prototype(&first).data().prototype.clone(),
        Some(realm.type_error_prototype.clone())
    );
    let names: Vec<String> = first
        .as_object()
        .unwrap()
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(names, ["stack", "code", "message"]);
    let constructor = ops::get(&mut realm, &first, &Key::name("constructor")).unwrap();
    let global = Value::Object(realm.global.clone());
    let type_error = ops::get(&mut realm, &global, &Key::name("TypeError")).unwrap();
    assert!(crate::same_value(&constructor, &type_error));
    // Base prototypes without a data `constructor` give `undefined`.
    let mut realm = Realm::new();
    throwing_getter(&realm, &realm.uri_error_prototype.clone(), "constructor");
    let error = node_error(&mut realm, ErrorKind::Uri, "ERR_X", "x").0;
    let constructor = ops::get(&mut realm, &error, &Key::name("constructor")).unwrap();
    assert!(matches!(constructor, Value::Undefined));
    realm.eval_error_prototype.delete(&Key::name("constructor"));
    let error = node_error(&mut realm, ErrorKind::Eval, "ERR_Y", "y").0;
    let constructor = ops::get(&mut realm, &error, &Key::name("constructor")).unwrap();
    assert!(matches!(constructor, Value::Undefined));
}

#[test]
fn expected_types_are_listed_like_node() {
    let mut realm = Realm::new();
    for (expected, text) in [
        (&["object", "Error"][..], "an instance of Error or Object"),
        (&["a dictionary"][..], "a dictionary"),
        (
            &["Object", "URL with x"][..],
            "of type object or an URL with x",
        ),
        (&["x y", "z w"][..], "one of x y or z w"),
        (
            &["boolean", "bigint", "symbol"][..],
            "one of type boolean, bigint, or symbol",
        ),
        (&["Error", "x"][..], "an instance of Error or x"),
        (&[][..], ""),
    ] {
        let thrown = invalid_arg_type(&mut realm, "a", expected, &Value::Null);
        let message = ops::get(&mut realm, &thrown.0, &Key::name("message")).unwrap();
        assert_eq!(
            text_of(&mut realm, &message),
            format!("The \"a\" argument must be {text}. Received null")
        );
    }
}

fn text_of(realm: &mut Realm, value: &Value) -> String {
    text(realm, value)
}

#[test]
fn received_values_that_cannot_be_inspected_propagate() {
    let mut realm = Realm::new();
    let tagged = realm.object();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "tag"))
    });
    tagged.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let tagged = Value::Object(tagged);
    for thrown in [
        out_of_range(&mut realm, "a", "b", &tagged),
        invalid_arg_value(&mut realm, "a", &tagged, "is invalid"),
    ] {
        assert_eq!(text(&mut realm, &thrown.0), "RangeError: tag");
    }
}
