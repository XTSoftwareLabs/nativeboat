//! Expected values are Node 22's results.

use super::*;

fn call(realm: &mut Realm, name: &str, args: &[Value]) -> String {
    let global = Value::Object(realm.global.clone());
    let function = ops::get(realm, &global, &Key::name(name)).unwrap();
    match ops::call(realm, &function, Value::Undefined, args) {
        Ok(value) => ops::to_string(realm, &value).unwrap().to_rust_lossy(),
        Err(thrown) => format!(
            "throws {}",
            ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
        ),
    }
}

fn units(text: &[u16]) -> Value {
    Value::String(JsStr::from_units(text))
}

fn s(text: &str) -> Value {
    Value::str(text)
}

#[test]
fn encoding_escapes_utf8_and_rejects_lone_surrogates() {
    let mut realm = Realm::new();
    assert_eq!(
        call(&mut realm, "encodeURIComponent", &[s("a b&c/d?é😀")]),
        "a%20b%26c%2Fd%3F%C3%A9%F0%9F%98%80"
    );
    assert_eq!(
        call(&mut realm, "encodeURI", &[s("http://x.y/a b?q=1&r=é#h")]),
        "http://x.y/a%20b?q=1&r=%C3%A9#h"
    );
    assert_eq!(
        call(&mut realm, "encodeURIComponent", &[units(&[0xD800])]),
        "throws URIError: URI malformed"
    );
    assert_eq!(call(&mut realm, "encodeURIComponent", &[]), "undefined");
}

#[test]
fn decoding_is_strict_and_preserves_reserved_escapes() {
    let mut realm = Realm::new();
    assert_eq!(
        call(
            &mut realm,
            "decodeURIComponent",
            &[s("a%20b%26%F0%9F%98%80")]
        ),
        "a b&😀"
    );
    assert_eq!(
        call(
            &mut realm,
            "decodeURI",
            &[s("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23%20")]
        ),
        "%3B%2F%3F%3A%40%26%3D%2B%24%2C%23 "
    );
    for malformed in ["%", "%C0%80", "%E0%A4%A", "%ED%A0%80", "%FF", "%80", "%C3x"] {
        assert_eq!(
            call(&mut realm, "decodeURIComponent", &[s(malformed)]),
            "throws URIError: URI malformed",
            "{malformed}"
        );
    }
    assert_eq!(
        call(&mut realm, "decodeURIComponent", &[Value::Null]),
        "null"
    );
}

#[test]
fn annex_b_escape_and_unescape() {
    let mut realm = Realm::new();
    assert_eq!(
        call(&mut realm, "escape", &[s("a b+é😀@*_-./")]),
        "a%20b+%E9%uD83D%uDE00@*_-./"
    );
    assert_eq!(
        call(&mut realm, "unescape", &[s("%u0041%41%zz%u12")]),
        "AA%zz%u12"
    );
}

#[test]
fn global_this_names_the_global_object() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    for name in ["globalThis", "global"] {
        let value = ops::get(&mut realm, &global, &Key::name(name)).unwrap();
        assert_eq!(value.as_object(), Some(&realm.global));
        let property = realm.global.own_property(&Key::name(name)).unwrap();
        assert!(!property.enumerable && property.configurable);
    }
    let length = {
        let encode = ops::get(&mut realm, &global, &Key::name("encodeURI")).unwrap();
        ops::get(&mut realm, &encode, &Key::name("length")).unwrap()
    };
    assert!(matches!(length, Value::Number(1.0)));
}
