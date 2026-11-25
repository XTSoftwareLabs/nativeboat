use super::*;
use crate::Kind;

fn method(realm: &mut Realm, this: &Value, name: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, this, &Key::name(name))?;
    ops::call(realm, &function, this.clone(), args)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    to_string(realm, value).unwrap().to_rust_lossy()
}

#[test]
fn object_methods() {
    let mut realm = Realm::new();
    let object = Value::Object(realm.object());
    let to_string_method = ops::get(&mut realm, &object, &Key::name("toString")).unwrap();
    let error = Value::Object(realm.error(ErrorKind::Error, "e"));
    let function = Value::Object(realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined)));
    let array = Value::Object(realm.array(Vec::new()));
    for (this, tag) in [
        (Value::Undefined, "[object Undefined]"),
        (Value::Null, "[object Null]"),
        (object.clone(), "[object Object]"),
        (array, "[object Array]"),
        (function, "[object Function]"),
        (error, "[object Error]"),
        (Value::Boolean(true), "[object Boolean]"),
        (Value::Number(1.0), "[object Number]"),
        (Value::str("s"), "[object String]"),
    ] {
        let result = ops::call(&mut realm, &to_string_method, this, &[]).unwrap();
        assert_eq!(text(&mut realm, &result), tag);
    }
    let same = method(&mut realm, &object, "valueOf", &[]).unwrap();
    assert!(ops::strict_equals(&same, &object));
    object
        .as_object()
        .unwrap()
        .define(Key::Index(1), crate::Property::data(Value::Null));
    let own = method(&mut realm, &object, "hasOwnProperty", &[Value::Number(1.0)]).unwrap();
    assert!(ops::to_boolean(&own));
    let inherited = method(
        &mut realm,
        &object,
        "hasOwnProperty",
        &[Value::str("toString")],
    )
    .unwrap();
    assert!(!ops::to_boolean(&inherited));
    // Primitives are converted to objects first.
    let own = method(
        &mut realm,
        &Value::str("s"),
        "hasOwnProperty",
        &[Value::str("length")],
    )
    .unwrap();
    assert!(ops::to_boolean(&own));
    let has_own = ops::get(&mut realm, &object, &Key::name("hasOwnProperty")).unwrap();
    let error = ops::call(&mut realm, &has_own, Value::Null, &[Value::str("x")]).unwrap_err();
    assert_eq!(
        text(&mut realm, &error.0),
        "TypeError: Cannot convert undefined or null to object"
    );
}

#[test]
fn function_call_passes_receiver_and_arguments() {
    let mut realm = Realm::new();
    let function = Value::Object(realm.function("f", 2, false, |realm, this, args| {
        let total = ops::add(realm, &this, &args.first().cloned().unwrap_or_default())?;
        Ok(total)
    }));
    let result = method(
        &mut realm,
        &function,
        "call",
        &[Value::Number(1.0), Value::Number(2.0)],
    )
    .unwrap();
    assert!(ops::strict_equals(&result, &Value::Number(3.0)));
    let result = method(&mut realm, &function, "call", &[]).unwrap();
    assert!(ops::to_number(&mut realm, &result).unwrap().is_nan());
}

#[test]
fn array_methods() {
    let mut realm = Realm::new();
    let list = Value::Object(realm.array(vec![Value::Number(1.0), Value::Null, Value::str("x")]));
    let joined = method(&mut realm, &list, "join", &[]).unwrap();
    assert_eq!(text(&mut realm, &joined), "1,,x");
    let joined = method(&mut realm, &list, "join", &[Value::str(" - ")]).unwrap();
    assert_eq!(text(&mut realm, &joined), "1 -  - x");
    assert_eq!(text(&mut realm, &list), "1,,x");
    let length = method(
        &mut realm,
        &list,
        "push",
        &[Value::Number(4.0), Value::Number(5.0)],
    )
    .unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(5.0)));
    let last = method(&mut realm, &list, "pop", &[]).unwrap();
    assert!(ops::strict_equals(&last, &Value::Number(5.0)));
    let empty = Value::Object(realm.array(Vec::new()));
    let none = method(&mut realm, &empty, "pop", &[]).unwrap();
    assert!(matches!(none, Value::Undefined));
    // A read-only length makes pop fail after removing the element.
    let fixed_array = realm.array(vec![Value::Null]);
    let read_only = crate::Descriptor {
        writable: Some(false),
        ..crate::Descriptor::default()
    };
    assert!(fixed_array.define_own_property(&Key::name("length"), &read_only));
    let fixed = Value::Object(fixed_array);
    assert!(method(&mut realm, &fixed, "pop", &[]).is_err());
    let throwing = Value::Object(realm.function("f", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "element"))
    }));
    let bad = realm.object();
    bad.define(Key::name("toString"), crate::Property::data(throwing));
    let failing = Value::Object(realm.array(vec![Value::Object(bad)]));
    assert!(method(&mut realm, &failing, "join", &[]).is_err());
    let neither = Value::Object(realm.object());
    for name in ["valueOf", "toString"] {
        neither
            .as_object()
            .unwrap()
            .define(Key::name(name), crate::Property::data(Value::Null));
    }
    assert!(method(&mut realm, &list, "join", &[neither]).is_err());
}

#[test]
fn error_to_string_combines_name_and_message() {
    let mut realm = Realm::new();
    let error = Value::Object(realm.error(ErrorKind::Type, "bad"));
    let result = method(&mut realm, &error, "toString", &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "TypeError: bad");
    let quiet = Value::Object(realm.error(ErrorKind::Error, ""));
    let result = method(&mut realm, &quiet, "toString", &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "Error");
    let unnamed = realm.error(ErrorKind::Error, "only message");
    unnamed.define(Key::name("name"), crate::Property::data(Value::str("")));
    let result = method(&mut realm, &Value::Object(unnamed), "toString", &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "only message");
    let fallback = Object::new(Some(realm.error_prototype.clone()), Kind::Ordinary);
    fallback.define(Key::name("name"), crate::Property::data(Value::Undefined));
    fallback.define(
        Key::name("message"),
        crate::Property::data(Value::Number(1.0)),
    );
    let result = method(&mut realm, &Value::Object(fallback), "toString", &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "Error: 1");
    let prototype = Value::Object(realm.error_prototype.clone());
    let to_string_method = ops::get(&mut realm, &prototype, &Key::name("toString")).unwrap();
    assert!(ops::call(&mut realm, &to_string_method, Value::Number(1.0), &[]).is_err());
    let broken = realm.error(ErrorKind::Error, "x");
    let throwing = Value::Object(realm.function("f", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "name"))
    }));
    broken.define(
        Key::name("name"),
        crate::Property {
            slot: crate::Slot::Accessor {
                get: throwing.as_object().cloned(),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert!(method(&mut realm, &Value::Object(broken), "toString", &[]).is_err());
}

#[test]
fn object_to_string_reads_the_string_tag() {
    let mut realm = Realm::new();
    let object = realm.object();
    let tagged = Value::Object(object.clone());
    let to_string_method = ops::get(&mut realm, &tagged, &Key::name("toString")).unwrap();
    object.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        crate::Property::data(Value::str("Custom")),
    );
    let result = ops::call(&mut realm, &to_string_method, tagged.clone(), &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "[object Custom]");
    object.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        crate::Property::data(Value::Number(1.0)),
    );
    let result = ops::call(&mut realm, &to_string_method, tagged, &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "[object Object]");
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let result = ops::call(&mut realm, &to_string_method, symbol, &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "[object Symbol]");
    let throwing = realm.object();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "tag"))
    });
    throwing.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        crate::Property {
            slot: crate::Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert!(ops::call(&mut realm, &to_string_method, Value::Object(throwing), &[]).is_err());
}

#[test]
fn iterator_base_metadata_keeps_node_abstract_construction_and_setter_rules() {
    let mut realm = Realm::new();
    let prototype = realm.iterator_prototype.clone();
    let base = Value::Object(prototype.clone());
    let constructor = ops::get(&mut realm, &base, &Key::name("constructor")).unwrap();
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
    assert!(ops::construct(&mut realm, &constructor, &[]).is_err());
    let child = realm.function("Child", 0, true, |_, _, _| Ok(Value::Undefined));
    let built = ops::construct_target(&mut realm, &constructor, &[], &Value::Object(child.clone()))
        .unwrap();
    let expected = ops::get(
        &mut realm,
        &Value::Object(child.clone()),
        &Key::name("prototype"),
    )
    .unwrap();
    assert_eq!(
        built.as_object().unwrap().data().prototype,
        expected.as_object().cloned()
    );
    child.define(Key::name("prototype"), crate::Property::data(Value::Null));
    let fallback =
        ops::construct_target(&mut realm, &constructor, &[], &Value::Object(child.clone()))
            .unwrap();
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        Some(prototype.clone())
    );
    for key in [
        Key::name("constructor"),
        Key::Symbol(realm.symbols.to_string_tag.clone()),
    ] {
        let property = prototype.own_property(&key).unwrap();
        let crate::Slot::Accessor {
            get: Some(getter),
            set: Some(setter),
        } = property.slot
        else {
            panic!()
        };
        let got = ops::call(&mut realm, &Value::Object(getter), Value::Null, &[]).unwrap();
        assert!(crate::same_value(
            &got,
            &ops::get(&mut realm, &base, &key).unwrap()
        ));
        let setter = Value::Object(setter);
        assert!(ops::call(&mut realm, &setter, base.clone(), &[Value::Number(1.0)]).is_err());
        assert!(ops::call(&mut realm, &setter, Value::Null, &[]).is_err());
        let ordinary = Value::Object(realm.object());
        ops::call(
            &mut realm,
            &setter,
            ordinary.clone(),
            &[Value::Number(42.0)],
        )
        .unwrap();
        assert!(crate::same_value(
            &ops::get(&mut realm, &ordinary, &key).unwrap(),
            &Value::Number(42.0)
        ));
        assert!(
            ordinary
                .as_object()
                .unwrap()
                .own_property(&key)
                .unwrap()
                .enumerable
        );
        let sealed = realm.object();
        sealed.prevent_extensions();
        assert!(ops::call(&mut realm, &setter, Value::Object(sealed), &[]).is_err());
        let fixed = realm.object();
        fixed.define(
            key.clone(),
            crate::Property {
                configurable: false,
                enumerable: false,
                slot: crate::Slot::Data {
                    value: Value::Undefined,
                    writable: false,
                },
            },
        );
        assert!(ops::call(&mut realm, &setter, Value::Object(fixed), &[]).is_err());
    }
}
