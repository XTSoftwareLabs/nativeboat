//! Messages are V8's; results follow ECMAScript 2024 and Node 22.

use super::*;
use crate::realm::ErrorKind;

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn reflect(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let global = Value::Object(realm.global.clone());
    let namespace = ops::get(realm, &global, &Key::name("Reflect")).unwrap();
    let method = ops::get(realm, &namespace, &Key::name(name)).unwrap();
    ops::call(realm, &method, namespace, args)
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

fn flag(result: Completion) -> bool {
    matches!(result.unwrap(), Value::Boolean(true))
}

#[test]
fn methods_reject_non_object_targets() {
    let mut realm = Realm::new();
    for name in [
        "defineProperty",
        "deleteProperty",
        "get",
        "getOwnPropertyDescriptor",
        "getPrototypeOf",
        "has",
        "isExtensible",
        "ownKeys",
        "preventExtensions",
        "set",
        "setPrototypeOf",
    ] {
        let result = reflect(&mut realm, name, &[n(1.0)]);
        assert_eq!(
            message(&mut realm, result),
            format!("TypeError: Reflect.{name} called on non-object")
        );
    }
    let result = reflect(&mut realm, "apply", &[n(1.0)]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Function.prototype.apply was called on 1, which is a number and not a function"
    );
    let result = reflect(&mut realm, "apply", &[Value::Undefined]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Function.prototype.apply was called on undefined, which is an undefined and not a function"
    );
    let max = {
        let global = Value::Object(realm.global.clone());
        let math = ops::get(&mut realm, &global, &Key::name("Math")).unwrap();
        ops::get(&mut realm, &math, &Key::name("max")).unwrap()
    };
    let result = reflect(&mut realm, "apply", &[max.clone(), Value::Null, n(1.0)]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: CreateListFromArrayLike called on non-object"
    );
    let result = reflect(&mut realm, "construct", &[max.clone(), Value::Undefined]);
    let text = ops::to_string(&mut realm, &max).unwrap().to_rust_lossy();
    assert_eq!(
        message(&mut realm, result),
        format!("TypeError: {text} is not a constructor")
    );
    let constructor = Value::Object(realm.function("C", 0, true, |_, this, _| Ok(this)));
    let empty = Value::Object(realm.array(vec![]));
    let result = reflect(
        &mut realm,
        "construct",
        &[constructor.clone(), empty, n(1.0)],
    );
    assert_eq!(
        message(&mut realm, result),
        "TypeError: 1 is not a constructor"
    );
    let result = reflect(&mut realm, "construct", &[constructor, n(1.0)]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: CreateListFromArrayLike called on non-object"
    );
    let plain = Value::Object(realm.object());
    let result = reflect(&mut realm, "setPrototypeOf", &[plain, n(1.0)]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Object prototype may only be an Object or null: 1"
    );
}

#[test]
fn set_reports_refusals_and_uses_receivers() {
    let mut realm = Realm::new();
    let getter_only = realm.object();
    let getter = realm.function("get", 0, false, |_, _, _| Ok(n(1.0)));
    getter_only.define(
        Key::name("g"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter.clone()),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let target = Value::Object(getter_only.clone());
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[target.clone(), s("g"), n(2.0)]
    )));
    // Assignments whose receiver is a primitive, or has an accessor or a
    // read-only property of its own, are refused.
    let empty = Value::Object(realm.object());
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[empty.clone(), s("x"), n(1.0), n(1.0)]
    )));
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[empty.clone(), s("g"), n(1.0), target.clone()]
    )));
    let fixed = realm.object();
    fixed.define(
        Key::name("x"),
        Property {
            slot: Slot::Data {
                value: n(1.0),
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let fixed = Value::Object(fixed);
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[empty.clone(), s("x"), n(2.0), fixed]
    )));
    // An existing writable data property is updated in place.
    let existing = realm.object();
    existing.define(Key::name("x"), Property::hidden(n(1.0)));
    let existing = Value::Object(existing);
    assert!(flag(reflect(
        &mut realm,
        "set",
        &[empty.clone(), s("x"), n(5.0), existing.clone()]
    )));
    let descriptor = existing
        .as_object()
        .unwrap()
        .own_property(&Key::name("x"))
        .unwrap();
    assert!(!descriptor.enumerable);
    assert!(matches!(
        descriptor.slot,
        Slot::Data {
            value: Value::Number(5.0),
            ..
        }
    ));
    // A throwing setter propagates.
    let thrower = realm.function("set", 1, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "setter"))
    });
    let guarded = realm.object();
    guarded.define(
        Key::name("x"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: Some(thrower),
            },
            enumerable: true,
            configurable: true,
        },
    );
    let result = reflect(&mut realm, "set", &[Value::Object(guarded), s("x"), n(1.0)]);
    assert_eq!(message(&mut realm, result), "RangeError: setter");
    // Receiver-aware reads find getters and data along the chain.
    let child = Object::new(Some(getter_only), Kind::Ordinary);
    let child = Value::Object(child);
    let got = reflect(&mut realm, "get", &[child, s("g"), empty]).unwrap();
    assert!(matches!(got, Value::Number(1.0)));
}

#[test]
fn environment_handles_use_their_own_operations() {
    let mut realm = Realm::new();
    let environment = crate::environment_builtin::environment(&mut realm);
    let name = s("NATIVEBOAT_REFLECT_TEST");
    assert!(flag(reflect(
        &mut realm,
        "set",
        &[environment.clone(), name.clone(), s("1")]
    )));
    let value = reflect(&mut realm, "get", &[environment.clone(), name.clone()]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &value).unwrap().to_rust_lossy(),
        "1"
    );
    assert!(flag(reflect(
        &mut realm,
        "deleteProperty",
        &[environment.clone(), name]
    )));
    assert!(!flag(reflect(
        &mut realm,
        "preventExtensions",
        &[environment]
    )));
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let global = Value::Object(realm.global.clone());
    let namespace = ops::get(&mut realm, &global, &Key::name("Reflect")).unwrap();
    let tag = ops::get(&mut realm, &namespace, &tag).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &tag).unwrap().to_rust_lossy(),
        "Reflect"
    );
}

#[test]
fn methods_reflect_internal_operations() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    let math = ops::get(&mut realm, &global, &Key::name("Math")).unwrap();
    let max = ops::get(&mut realm, &math, &Key::name("max")).unwrap();
    let values = Value::Object(realm.array(vec![n(1.0), n(5.0), n(3.0)]));
    let largest = reflect(&mut realm, "apply", &[max, Value::Null, values]).unwrap();
    assert!(matches!(largest, Value::Number(5.0)));
    // Construction with and without a separate new target.
    let constructor = Value::Object(realm.function("C", 0, true, |_, this, _| Ok(this)));
    let other = Value::Object(realm.function("O", 0, true, |_, this, _| Ok(this)));
    let none = Value::Object(realm.array(vec![]));
    let made = reflect(
        &mut realm,
        "construct",
        &[constructor.clone(), none.clone(), other.clone()],
    )
    .unwrap();
    let other_prototype = ops::get(&mut realm, &other, &Key::name("prototype")).unwrap();
    assert_eq!(
        made.as_object().unwrap().data().prototype.as_ref(),
        other_prototype.as_object()
    );
    let plain = reflect(&mut realm, "construct", &[constructor.clone(), none]).unwrap();
    let own_prototype = ops::get(&mut realm, &constructor, &Key::name("prototype")).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype.as_ref(),
        own_prototype.as_object()
    );
    // Definition, deletion and description.
    let object = Value::Object(realm.object());
    let descriptor = Value::Object(realm.object());
    ops::set(&mut realm, &descriptor, &Key::name("value"), n(2.0)).unwrap();
    assert!(flag(reflect(
        &mut realm,
        "defineProperty",
        &[object.clone(), s("a"), descriptor.clone()]
    )));
    let shown = reflect(
        &mut realm,
        "getOwnPropertyDescriptor",
        &[object.clone(), s("a")],
    )
    .unwrap();
    let writable = ops::get(&mut realm, &shown, &Key::name("writable")).unwrap();
    assert!(matches!(writable, Value::Boolean(false)));
    let missing = reflect(
        &mut realm,
        "getOwnPropertyDescriptor",
        &[object.clone(), s("zz")],
    )
    .unwrap();
    assert!(matches!(missing, Value::Undefined));
    let redefined = Value::Object(realm.object());
    ops::set(&mut realm, &redefined, &Key::name("value"), n(3.0)).unwrap();
    assert!(!flag(reflect(
        &mut realm,
        "defineProperty",
        &[object.clone(), s("a"), redefined]
    )));
    assert!(!flag(reflect(
        &mut realm,
        "deleteProperty",
        &[object.clone(), s("a")]
    )));
    ops::set(&mut realm, &object, &Key::name("b"), n(1.0)).unwrap();
    assert!(flag(reflect(
        &mut realm,
        "deleteProperty",
        &[object.clone(), s("b")]
    )));
    // Reads and writes without explicit receivers.
    let read = reflect(&mut realm, "get", &[object.clone(), s("a")]).unwrap();
    assert!(matches!(read, Value::Number(2.0)));
    let absent = reflect(
        &mut realm,
        "get",
        &[object.clone(), s("absent"), object.clone()],
    )
    .unwrap();
    assert!(matches!(absent, Value::Undefined));
    assert!(flag(reflect(
        &mut realm,
        "set",
        &[object.clone(), s("c"), n(4.0)]
    )));
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[object.clone(), s("a"), n(4.0)]
    )));
    // Keys in property order, including indexes and symbols.
    let symbol = Value::Symbol(realm.symbols.iterator.clone());
    ops::set(&mut realm, &object, &Key::Index(2), n(0.0)).unwrap();
    let key = ops::to_key(&mut realm, &symbol).unwrap();
    ops::set(&mut realm, &object, &key, n(0.0)).unwrap();
    let keys = reflect(&mut realm, "ownKeys", std::slice::from_ref(&object)).unwrap();
    let keys = crate::inspect::inspect(&mut realm, &keys, 2).unwrap();
    assert_eq!(keys, "[ '2', 'a', 'c', Symbol(Symbol.iterator) ]");
    assert!(flag(reflect(
        &mut realm,
        "has",
        &[object.clone(), s("toString")]
    )));
    // Prototypes and extensibility.
    let prototype = reflect(&mut realm, "getPrototypeOf", std::slice::from_ref(&object)).unwrap();
    assert_eq!(prototype.as_object(), Some(&realm.object_prototype));
    assert!(flag(reflect(
        &mut realm,
        "setPrototypeOf",
        &[object.clone(), Value::Null]
    )));
    let prototype = reflect(&mut realm, "getPrototypeOf", std::slice::from_ref(&object)).unwrap();
    assert!(matches!(prototype, Value::Null));
    assert!(flag(reflect(
        &mut realm,
        "isExtensible",
        std::slice::from_ref(&object)
    )));
    assert!(flag(reflect(
        &mut realm,
        "preventExtensions",
        std::slice::from_ref(&object)
    )));
    assert!(!flag(reflect(
        &mut realm,
        "isExtensible",
        std::slice::from_ref(&object)
    )));
    let base = Value::Object(realm.object());
    assert!(!flag(reflect(
        &mut realm,
        "setPrototypeOf",
        &[object.clone(), base]
    )));
    assert!(!flag(reflect(
        &mut realm,
        "set",
        &[object, s("new"), n(1.0)]
    )));
}

#[test]
fn receivers_reach_data_and_setters_and_invalid_lengths_throw() {
    let mut realm = Realm::new();
    let source = realm.object();
    source.define(Key::name("data"), Property::data(n(1.0)));
    let setter = realm.function("set", 1, false, |_, _, _| Ok(Value::Undefined));
    source.define(
        Key::name("x"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
    let source = Value::Object(source);
    let receiver = Value::Object(realm.object());
    let data = reflect(
        &mut realm,
        "get",
        &[source.clone(), s("data"), receiver.clone()],
    )
    .unwrap();
    assert!(matches!(data, Value::Number(1.0)));
    assert!(flag(reflect(
        &mut realm,
        "set",
        &[source, s("x"), n(1.0), receiver]
    )));
    let array = Value::Object(realm.array(vec![]));
    let descriptor = Value::Object(realm.object());
    ops::set(&mut realm, &descriptor, &Key::name("value"), n(-1.0)).unwrap();
    let result = reflect(
        &mut realm,
        "defineProperty",
        &[array, s("length"), descriptor],
    );
    assert_eq!(
        message(&mut realm, result),
        "RangeError: Invalid array length"
    );
}
