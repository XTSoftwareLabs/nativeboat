use super::*;
use crate::object::NativeCode;
use std::rc::Rc;

fn text(realm: &mut Realm, value: &Value) -> String {
    to_string(realm, value).unwrap().to_rust_lossy()
}

fn thrown(realm: &mut Realm, error: Thrown) -> String {
    text(realm, &error.0)
}

#[test]
fn sloppy_this_coerces_nullish_and_primitive_receivers() {
    let mut realm = Realm::new();
    assert!(matches!(
        sloppy_this(&mut realm, Value::Undefined),
        Value::Object(object) if object == realm.global
    ));
    assert!(matches!(
        sloppy_this(&mut realm, Value::Null),
        Value::Object(object) if object == realm.global
    ));

    let object = Value::Object(realm.object());
    assert!(matches!(
        sloppy_this(&mut realm, object.clone()),
        Value::Object(result) if Some(&result) == object.as_object()
    ));

    let symbol = Value::Symbol(crate::Symbol::new(None));
    let bigint = Value::BigInt(alloc::rc::Rc::new(BigInt::from_i64(3)));
    for primitive in [
        Value::Boolean(true),
        Value::Number(3.0),
        Value::str("three"),
        symbol,
        bigint,
    ] {
        assert!(sloppy_this(&mut realm, primitive).as_object().is_some());
    }
}

#[test]
fn global_stores_follow_node_strictness_and_preserve_setter_errors() {
    let mut realm = Realm::new();
    let reference = global_reference(&realm, "ambient", true);
    assert!(format!("{reference:?}").contains("GlobalReference"));
    assert!(reference.get(&mut realm).is_err());
    assert!(reference.set(&mut realm, Value::Number(1.0)).is_err());
    realm.set_global("ambient", Value::Number(2.0));
    reference.set(&mut realm, Value::Number(3.0)).unwrap();
    assert!(matches!(
        reference.get(&mut realm).unwrap(),
        Value::Number(3.0)
    ));
    realm.global.delete(&Key::name("ambient"));
    assert!(reference.set(&mut realm, Value::Number(4.0)).is_err());
    let sloppy = global_reference(&realm, "ambient", false);
    sloppy.set(&mut realm, Value::Number(5.0)).unwrap();
    let property = realm.global.own_property(&Key::name("ambient")).unwrap();
    assert!(property.configurable && property.enumerable);
    realm.global.define(
        Key::name("ambient"),
        Property {
            slot: Slot::Data {
                value: Value::Number(6.0),
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(reference.set(&mut realm, Value::Number(7.0)).is_err());
    sloppy.set(&mut realm, Value::Number(8.0)).unwrap();
    assert!(matches!(
        sloppy.get(&mut realm).unwrap(),
        Value::Number(6.0)
    ));
    let setter = realm.function("set ambient", 1, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "setter"))
    });
    realm.global.define(
        Key::name("ambient"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
    for reference in [&reference, &sloppy] {
        let error = reference.set(&mut realm, Value::Null).unwrap_err();
        assert_eq!(thrown(&mut realm, error), "RangeError: setter");
    }
    realm.global.data_mut().extensible = false;
    let ignored = global_reference(&realm, "newSloppyGlobal", false);
    ignored.set(&mut realm, Value::Null).unwrap();
    assert!(
        realm
            .global
            .own_property(&Key::name("newSloppyGlobal"))
            .is_none()
    );
}

#[test]
fn global_identifier_reads_and_typeof_keep_reference_semantics() {
    let mut realm = Realm::new();
    let missing_type = type_of_global(&mut realm, "absent").unwrap();
    assert_eq!(text(&mut realm, &missing_type), "undefined");
    let error = get_global(&mut realm, "absent").unwrap_err();
    assert!(matches!(
        error.0.as_object().unwrap().data().kind,
        Kind::Error
    ));
    let name = get(&mut realm, &error.0, &Key::name("name")).unwrap();
    assert_eq!(text(&mut realm, &name), "ReferenceError");
    assert_eq!(
        thrown(&mut realm, error),
        "ReferenceError: absent is not defined"
    );

    realm.set_global("answer", Value::Number(42.0));
    let answer_type = type_of_global(&mut realm, "answer").unwrap();
    assert_eq!(text(&mut realm, &answer_type), "number");
    let answer = get_global(&mut realm, "answer").unwrap();
    assert_eq!(to_number(&mut realm, &answer).unwrap(), 42.0);
}

fn function(
    realm: &Realm,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 0, true, code))
}

/// An object whose `valueOf`/`toString` return the given values.
fn convertible(realm: &mut Realm, value_of: Option<Value>, to_string: Option<Value>) -> Value {
    let object = realm.object();
    for (name, result) in [("valueOf", value_of), ("toString", to_string)] {
        let property = match result {
            Some(result) => {
                Value::Object(realm.function(name, 0, false, move |_, _, _| Ok(result.clone())))
            }
            None => Value::Null,
        };
        object.define(Key::name(name), Property::data(property));
    }
    Value::Object(object)
}

#[test]
fn booleans_and_types() {
    let realm = Realm::new();
    for (value, truthy) in [
        (Value::Undefined, false),
        (Value::Null, false),
        (Value::Boolean(true), true),
        (Value::Number(f64::NAN), false),
        (Value::Number(-0.0), false),
        (Value::Number(2.0), true),
        (Value::str(""), false),
        (Value::str("0"), true),
        (Value::Object(realm.object()), true),
    ] {
        assert_eq!(to_boolean(&value), truthy, "{value:?}");
    }
    let types: Vec<_> = [
        Value::Undefined,
        Value::Null,
        Value::Boolean(false),
        Value::Number(1.0),
        Value::str("s"),
        Value::Object(realm.object()),
        function(&realm, |_, _, _| Ok(Value::Undefined)),
    ]
    .iter()
    .map(type_of)
    .collect();
    assert_eq!(
        types,
        [
            "undefined",
            "object",
            "boolean",
            "number",
            "string",
            "object",
            "function"
        ]
    );
}

#[test]
fn conversions_follow_primitive_hints() {
    let mut realm = Realm::new();
    for (value, number) in [
        (Value::Null, 0.0),
        (Value::Boolean(true), 1.0),
        (Value::str(" 12 "), 12.0),
        (Value::Number(3.5), 3.5),
    ] {
        assert_eq!(to_number(&mut realm, &value).unwrap(), number);
    }
    assert!(to_number(&mut realm, &Value::Undefined).unwrap().is_nan());
    let both = convertible(
        &mut realm,
        Some(Value::Number(7.0)),
        Some(Value::str("seven")),
    );
    assert_eq!(to_number(&mut realm, &both).unwrap(), 7.0);
    assert_eq!(text(&mut realm, &both), "seven");
    // A non-primitive or missing method falls through to the other one.
    let object_valued = Value::Object(realm.object());
    let fallback = convertible(&mut realm, Some(object_valued), Some(Value::str("9")));
    assert_eq!(to_number(&mut realm, &fallback).unwrap(), 9.0);
    let neither = convertible(&mut realm, None, None);
    let error = to_number(&mut realm, &neither).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot convert object to primitive value"
    );
    for (value, string) in [
        (Value::Undefined, "undefined"),
        (Value::Null, "null"),
        (Value::Boolean(true), "true"),
        (Value::Boolean(false), "false"),
        (Value::Number(-1.5), "-1.5"),
        (Value::str("s"), "s"),
    ] {
        assert_eq!(text(&mut realm, &value), string);
    }
    let plain = Value::Object(realm.object());
    assert_eq!(text(&mut realm, &plain), "[object Object]");
    let failing = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "boom"))
    });
    let object = realm.object();
    object.define(Key::name("toString"), Property::data(failing));
    let error = to_string(&mut realm, &Value::Object(object)).unwrap_err();
    assert_eq!(thrown(&mut realm, error), "Error: boom");
}

#[test]
fn keys_and_32_bit_integers() {
    let mut realm = Realm::new();
    assert_eq!(
        to_key(&mut realm, &Value::Number(3.0)).unwrap(),
        Key::Index(3)
    );
    assert_eq!(
        to_key(&mut realm, &Value::Number(-0.0)).unwrap(),
        Key::Index(0)
    );
    assert_eq!(
        to_key(&mut realm, &Value::Number(1.5)).unwrap(),
        Key::name("1.5")
    );
    assert_eq!(
        to_key(&mut realm, &Value::Number(-1.0)).unwrap(),
        Key::name("-1")
    );
    assert_eq!(
        to_key(&mut realm, &Value::Number(4_294_967_295.0)).unwrap(),
        Key::name("4294967295")
    );
    assert_eq!(to_key(&mut realm, &Value::str("2")).unwrap(), Key::Index(2));
    let neither = convertible(&mut realm, None, None);
    assert!(to_key(&mut realm, &neither).is_err());
    for (value, int, uint) in [
        (4_294_967_296.0 + 5.0, 5, 5),
        (-1.0, -1, u32::MAX),
        (2_147_483_648.0, i32::MIN, 2_147_483_648),
        (f64::NAN, 0, 0),
        (f64::INFINITY, 0, 0),
        (-3.7, -3, 4_294_967_293),
    ] {
        assert_eq!(to_int32(&mut realm, &Value::Number(value)).unwrap(), int);
        assert_eq!(to_uint32(&mut realm, &Value::Number(value)).unwrap(), uint);
    }
}

#[test]
fn equality_operators() {
    let mut realm = Realm::new();
    let object = Value::Object(realm.object());
    let other = Value::Object(realm.object());
    assert!(strict_equals(&Value::Undefined, &Value::Undefined));
    assert!(strict_equals(&Value::Null, &Value::Null));
    assert!(strict_equals(&Value::Boolean(true), &Value::Boolean(true)));
    assert!(strict_equals(&Value::Number(0.0), &Value::Number(-0.0)));
    assert!(!strict_equals(
        &Value::Number(f64::NAN),
        &Value::Number(f64::NAN)
    ));
    assert!(strict_equals(&Value::str("a"), &Value::str("a")));
    assert!(strict_equals(&object, &object));
    assert!(!strict_equals(&object, &other));
    assert!(!strict_equals(&Value::Null, &Value::Undefined));
    let seven = convertible(&mut realm, Some(Value::Number(7.0)), None);
    for (left, right, equal) in [
        (Value::Null, Value::Undefined, true),
        (Value::Undefined, Value::Number(0.0), false),
        (Value::Number(0.0), Value::Null, false),
        (Value::Number(1.0), Value::str(" 1 "), true),
        (Value::str("2"), Value::Number(2.0), true),
        (Value::Boolean(true), Value::Number(1.0), true),
        (Value::str("0"), Value::Boolean(false), true),
        (object.clone(), object.clone(), true),
        (object.clone(), other.clone(), false),
        (seven.clone(), Value::Number(7.0), true),
        (Value::str("7"), seven.clone(), true),
        (Value::str("a"), Value::str("a"), true),
        (Value::Number(f64::NAN), Value::Number(f64::NAN), false),
    ] {
        assert_eq!(
            loose_equals(&mut realm, &left, &right).unwrap(),
            equal,
            "{left:?} == {right:?}"
        );
    }
    let neither = convertible(&mut realm, None, None);
    assert!(loose_equals(&mut realm, &neither, &Value::Number(1.0)).is_err());
    assert!(loose_equals(&mut realm, &Value::Number(1.0), &neither).is_err());
    assert!(loose_equals(&mut realm, &Value::Boolean(true), &neither).is_err());
    assert!(loose_equals(&mut realm, &neither, &Value::Boolean(true)).is_err());
}

#[test]
fn addition_and_arithmetic() {
    let mut realm = Realm::new();
    let sum = add(&mut realm, &Value::Number(1.0), &Value::Number(2.0)).unwrap();
    assert!(strict_equals(&sum, &Value::Number(3.0)));
    let joined = add(&mut realm, &Value::str("a"), &Value::Number(1.0)).unwrap();
    assert_eq!(text(&mut realm, &joined), "a1");
    let joined = add(&mut realm, &Value::Null, &Value::str("b")).unwrap();
    assert_eq!(text(&mut realm, &joined), "nullb");
    let list = Value::Object(realm.array(vec![Value::Number(1.0), Value::Number(2.0)]));
    let joined = add(&mut realm, &list, &Value::Number(3.0)).unwrap();
    assert_eq!(text(&mut realm, &joined), "1,23");
    let total = add(&mut realm, &Value::Boolean(true), &Value::Null).unwrap();
    assert!(strict_equals(&total, &Value::Number(1.0)));
    let neither = convertible(&mut realm, None, None);
    assert!(add(&mut realm, &neither, &Value::Null).is_err());
    assert!(add(&mut realm, &Value::Null, &neither).is_err());
    let object_string = convertible(&mut realm, None, Some(Value::str("x")));
    let object_valued = Value::Object(realm.object());
    let failing = convertible(&mut realm, Some(object_valued), None);
    assert!(add(&mut realm, &object_string, &failing).is_err());
    let number = |realm: &mut Realm, operator, a: f64, b: f64| match arithmetic(
        realm,
        operator,
        &Value::Number(a),
        &Value::Number(b),
    )
    .unwrap()
    {
        Value::Number(value) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(number(&mut realm, Arithmetic::Subtract, 5.0, 3.0), 2.0);
    assert_eq!(number(&mut realm, Arithmetic::Multiply, 5.0, 3.0), 15.0);
    assert_eq!(
        number(&mut realm, Arithmetic::Divide, 1.0, 0.0),
        f64::INFINITY
    );
    assert_eq!(number(&mut realm, Arithmetic::Remainder, -5.5, 2.0), -1.5);
    assert!(number(&mut realm, Arithmetic::Remainder, 1.0, 0.0).is_nan());
    assert_eq!(number(&mut realm, Arithmetic::Exponent, 2.0, 10.0), 1024.0);
    assert!(number(&mut realm, Arithmetic::Exponent, 1.0, f64::INFINITY).is_nan());
    assert!(number(&mut realm, Arithmetic::Exponent, -1.0, f64::NEG_INFINITY).is_nan());
    assert!(number(&mut realm, Arithmetic::Exponent, 1.0, f64::NAN).is_nan());
    assert_eq!(number(&mut realm, Arithmetic::Exponent, 0.0, 0.0), 1.0);
    assert_eq!(number(&mut realm, Arithmetic::And, 6.0, 3.0), 2.0);
    assert_eq!(number(&mut realm, Arithmetic::Or, 6.0, 3.0), 7.0);
    assert_eq!(number(&mut realm, Arithmetic::Xor, 6.0, 3.0), 5.0);
    assert_eq!(number(&mut realm, Arithmetic::ShiftLeft, 1.0, 33.0), 2.0);
    assert_eq!(
        number(&mut realm, Arithmetic::ShiftLeft, 1.0, 31.0),
        -2_147_483_648.0
    );
    assert_eq!(number(&mut realm, Arithmetic::ShiftRight, -8.0, 1.0), -4.0);
    assert_eq!(
        number(&mut realm, Arithmetic::ShiftRightUnsigned, -8.0, 1.0),
        2_147_483_644.0
    );
    assert!(arithmetic(&mut realm, Arithmetic::Subtract, &neither, &Value::Null).is_err());
    assert!(arithmetic(&mut realm, Arithmetic::Subtract, &Value::Null, &neither).is_err());
}

#[test]
fn relational_comparison() {
    let mut realm = Realm::new();
    let compare = |realm: &mut Realm, a: Value, b: Value| less_than(realm, &a, &b).unwrap();
    assert_eq!(
        compare(&mut realm, Value::Number(1.0), Value::Number(2.0)),
        Some(true)
    );
    assert_eq!(
        compare(&mut realm, Value::str("10"), Value::str("9")),
        Some(true)
    );
    assert_eq!(
        compare(&mut realm, Value::str("10"), Value::Number(9.0)),
        Some(false)
    );
    assert_eq!(
        compare(&mut realm, Value::Number(f64::NAN), Value::Number(1.0)),
        None
    );
    assert_eq!(
        compare(&mut realm, Value::Number(1.0), Value::str("x")),
        None
    );
    let neither = convertible(&mut realm, None, None);
    assert!(less_than(&mut realm, &neither, &Value::Null).is_err());
    assert!(less_than(&mut realm, &Value::Null, &neither).is_err());
    let relate = |realm: &mut Realm, relation, a: f64, b: f64| {
        relational(realm, relation, &Value::Number(a), &Value::Number(b)).unwrap()
    };
    let nan = f64::NAN;
    for (relation, results) in [
        (Relation::Less, [true, false, false, false]),
        (Relation::Greater, [false, true, false, false]),
        (Relation::LessOrEqual, [true, false, true, false]),
        (Relation::GreaterOrEqual, [false, true, true, false]),
    ] {
        let actual = [
            relate(&mut realm, relation, 1.0, 2.0),
            relate(&mut realm, relation, 2.0, 1.0),
            relate(&mut realm, relation, 1.0, 1.0),
            relate(&mut realm, relation, nan, 1.0),
        ];
        assert_eq!(actual, results, "{relation:?}");
    }
    assert!(relational(&mut realm, Relation::Less, &neither, &Value::Null).is_err());
    assert!(relational(&mut realm, Relation::Less, &Value::Null, &neither).is_err());
    let strings = relational(
        &mut realm,
        Relation::GreaterOrEqual,
        &Value::str("b"),
        &Value::str("a"),
    );
    assert!(strings.unwrap());
    let symbolic = convertible(&mut realm, Some(Value::str("x")), None);
    assert!(!relational(&mut realm, Relation::Less, &symbolic, &Value::Number(1.0)).unwrap());
    let object_valued = Value::Object(realm.object());
    let throwing = convertible(&mut realm, Some(object_valued), None);
    let error = relational(&mut realm, Relation::Less, &throwing, &Value::Number(1.0));
    assert!(error.is_err());
}

#[test]
fn property_reads_follow_prototypes_accessors_and_primitives() {
    let mut realm = Realm::new();
    let parent = realm.object();
    parent.define(Key::name("inherited"), Property::data(Value::Number(1.0)));
    let getter = realm.function("get", 0, false, |realm, this, _| {
        get(realm, &this, &Key::name("own"))
    });
    parent.define(
        Key::name("computed"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    parent.define(
        Key::name("writeOnly"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let child = Object::new(Some(parent), Kind::Ordinary);
    child.define(Key::name("own"), Property::data(Value::str("mine")));
    let child = Value::Object(child);
    let read =
        |realm: &mut Realm, base: &Value, key: &str| get(realm, base, &Key::name(key)).unwrap();
    let inherited = read(&mut realm, &child, "inherited");
    assert!(strict_equals(&inherited, &Value::Number(1.0)));
    let computed = read(&mut realm, &child, "computed");
    assert_eq!(text(&mut realm, &computed), "mine");
    assert!(matches!(
        read(&mut realm, &child, "writeOnly"),
        Value::Undefined
    ));
    assert!(matches!(
        read(&mut realm, &child, "missing"),
        Value::Undefined
    ));
    let word = Value::str("a😀");
    let length = read(&mut realm, &word, "length");
    assert!(strict_equals(&length, &Value::Number(3.0)));
    let unit = get(&mut realm, &word, &Key::Index(1)).unwrap();
    assert_eq!(to_string(&mut realm, &unit).unwrap().units(), &[0xD83D]);
    assert!(matches!(
        get(&mut realm, &word, &Key::Index(3)).unwrap(),
        Value::Undefined
    ));
    realm
        .string_prototype
        .define(Key::name("shout"), Property::data(Value::Number(5.0)));
    let shout = read(&mut realm, &word, "shout");
    assert!(strict_equals(&shout, &Value::Number(5.0)));
    for base in [Value::Number(1.0), Value::Boolean(true)] {
        assert!(matches!(
            read(&mut realm, &base, "missing"),
            Value::Undefined
        ));
    }
    let error = get(&mut realm, &Value::Undefined, &Key::name("x")).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot read properties of undefined (reading 'x')"
    );
    let error = get(&mut realm, &Value::Null, &Key::name("y")).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot read properties of null (reading 'y')"
    );
}

#[test]
fn property_writes_are_strict() {
    let mut realm = Realm::new();
    let parent = realm.object();
    parent.define(
        Key::name("locked"),
        Property {
            slot: Slot::Data {
                value: Value::Null,
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    parent.define(
        Key::name("getterOnly"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let log = realm.object();
    let target = log.clone();
    let setter = realm.function("set", 1, false, move |_, _, args| {
        target.define(Key::name("seen"), Property::data(args[0].clone()));
        Ok(Value::Undefined)
    });
    parent.define(
        Key::name("observed"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
    parent.define(Key::name("shadow"), Property::data(Value::Number(1.0)));
    let child_object = Object::new(Some(parent.clone()), Kind::Ordinary);
    let child = Value::Object(child_object.clone());
    let write = |realm: &mut Realm, base: &Value, key: &str, value: Value| {
        set(realm, base, &Key::name(key), value)
    };
    write(&mut realm, &child, "observed", Value::Number(4.0)).unwrap();
    assert!(log.own_property(&Key::name("seen")).is_some());
    write(&mut realm, &child, "shadow", Value::Number(2.0)).unwrap();
    assert!(child_object.own_property(&Key::name("shadow")).is_some());
    let error = write(&mut realm, &child, "locked", Value::Null).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot assign to read only property 'locked' of object"
    );
    let error = write(&mut realm, &child, "getterOnly", Value::Null).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot set property 'getterOnly' which has only a getter"
    );
    let error = write(&mut realm, &Value::str("s"), "x", Value::Null).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot create property 'x' on a primitive"
    );
    let error = write(&mut realm, &Value::Null, "x", Value::Null).unwrap_err();
    assert!(thrown(&mut realm, error).starts_with("TypeError: Cannot set properties of null"));
    child_object.data_mut().extensible = false;
    let error = write(&mut realm, &child, "fresh", Value::Null).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot add property 'fresh', object is not extensible"
    );
    let list = Value::Object(realm.array(vec![Value::Number(1.0)]));
    write(&mut realm, &list, "length", Value::str("3")).unwrap();
    let length = get(&mut realm, &list, &Key::name("length")).unwrap();
    assert!(strict_equals(&length, &Value::Number(3.0)));
    let error = write(&mut realm, &list, "length", Value::Number(-1.0)).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "RangeError: Invalid array length"
    );
    let neither = convertible(&mut realm, None, None);
    assert!(write(&mut realm, &list, "length", neither).is_err());
    // A partial shrink stops above a non-configurable element.
    let pinned = realm.array(vec![Value::Null, Value::Null]);
    pinned.define(
        Key::Index(0),
        Property {
            slot: Slot::Data {
                value: Value::Null,
                writable: true,
            },
            enumerable: true,
            configurable: false,
        },
    );
    let error = set(
        &mut realm,
        &Value::Object(pinned),
        &Key::name("length"),
        Value::Number(0.0),
    )
    .unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot assign to read only property 'length' of object"
    );
    // Far indexes are kept sparsely and extend the length.
    set(&mut realm, &list, &Key::Index(u32::MAX - 1), Value::Null).unwrap();
    let length = get(&mut realm, &list, &Key::name("length")).unwrap();
    assert!(strict_equals(&length, &Value::Number(4_294_967_295.0)));
    let frozen = realm.array(vec![Value::Null]);
    frozen.define_own_property(
        &Key::name("length"),
        &crate::Descriptor {
            writable: Some(false),
            ..crate::Descriptor::default()
        },
    );
    let error = set(
        &mut realm,
        &Value::Object(frozen),
        &Key::Index(1),
        Value::Null,
    )
    .unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot add property '1', object is not extensible"
    );
}

#[test]
fn deletion_membership_calls_and_construction() {
    let mut realm = Realm::new();
    let object = realm.object();
    object.define(Key::name("a"), Property::data(Value::Null));
    object.define(
        Key::name("fixed"),
        Property {
            slot: Slot::Data {
                value: Value::Null,
                writable: true,
            },
            enumerable: true,
            configurable: false,
        },
    );
    let value = Value::Object(object.clone());
    assert!(delete(&mut realm, &value, &Key::name("a")).unwrap());
    let error = delete(&mut realm, &value, &Key::name("fixed")).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot delete property 'fixed'"
    );
    assert!(delete(&mut realm, &Value::str("s"), &Key::name("length")).unwrap());
    assert!(delete(&mut realm, &Value::Undefined, &Key::name("x")).is_err());
    assert!(has_property(&mut realm, &Key::name("fixed"), &value).unwrap());
    assert!(has_property(&mut realm, &Key::name("toString"), &value).unwrap());
    assert!(!has_property(&mut realm, &Key::name("a"), &value).unwrap());
    assert!(has_property(&mut realm, &Key::name("x"), &Value::str("s")).is_err());
    assert!(in_operator(&mut realm, &Value::str("fixed"), &value).unwrap());
    let neither = convertible(&mut realm, None, None);
    let error = in_operator(&mut realm, &neither, &Value::Number(1.0)).unwrap_err();
    assert!(thrown(&mut realm, error).contains("'in' operator"));
    assert!(in_operator(&mut realm, &neither, &value).is_err());
    let error = call(&mut realm, &Value::Number(1.0), Value::Undefined, &[]).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: value is not a function"
    );
    let error = call(&mut realm, &value, Value::Undefined, &[]).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: value is not a function"
    );
    let point = function(&realm, |realm, this, args| {
        set(realm, &this, &Key::name("x"), args[0].clone())?;
        Ok(Value::Undefined)
    });
    let instance = construct(&mut realm, &point, &[Value::Number(3.0)]).unwrap();
    let x = get(&mut realm, &instance, &Key::name("x")).unwrap();
    assert!(strict_equals(&x, &Value::Number(3.0)));
    assert!(instance_of(&mut realm, &instance, &point).unwrap());
    let other = function(&realm, |_, _, _| Ok(Value::Undefined));
    assert!(!instance_of(&mut realm, &instance, &other).unwrap());
    assert!(!instance_of(&mut realm, &Value::Number(1.0), &point).unwrap());
    let replacement = Value::Object(realm.object());
    let returned = replacement.clone();
    let factory = function(&realm, move |_, _, _| Ok(returned.clone()));
    let made = construct(&mut realm, &factory, &[]).unwrap();
    assert!(strict_equals(&made, &replacement));
    // A non-object prototype falls back to Object.prototype.
    set(&mut realm, &other, &Key::name("prototype"), Value::Null).unwrap();
    let plain = construct(&mut realm, &other, &[]).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype,
        Some(realm.object_prototype.clone())
    );
    let error = instance_of(&mut realm, &plain, &other).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Function has non-object prototype in instanceof check"
    );
    let arrow = Value::Object(realm.function("arrow", 0, false, |_, _, _| Ok(Value::Undefined)));
    let error = construct(&mut realm, &arrow, &[]).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: value is not a constructor"
    );
    let error = construct_site(&mut realm, &arrow, &[], "Intl.DisplayNames").unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Intl.DisplayNames is not a constructor"
    );
    let made = construct_site(&mut realm, &factory, &[], "factory").unwrap();
    assert!(strict_equals(&made, &replacement));
    let not_callable = Value::Object(realm.object());
    let error = instance_of(&mut realm, &plain, &not_callable).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Right-hand side of 'instanceof' is not callable"
    );
    let failing = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "no"))
    });
    assert!(construct(&mut realm, &failing, &[]).is_err());
    let getter: NativeCode = Rc::new(|realm, _, _| Err(realm.throw(ErrorKind::Error, "get")));
    let trap = Object::new(
        Some(realm.function_prototype.clone()),
        Kind::Function {
            code: Rc::new(|_, _, _| Ok(Value::Undefined)),
            constructor: true,
        },
    );
    trap.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Accessor {
                get: Some(Object::new(
                    None,
                    Kind::Function {
                        code: getter,
                        constructor: false,
                    },
                )),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let trap = Value::Object(trap);
    assert!(construct(&mut realm, &trap, &[]).is_err());
    assert!(instance_of(&mut realm, &plain, &trap).is_err());
    let literal = realm.object();
    define_data(&literal, Key::name("k"), Value::Number(1.0));
    assert!(literal.own_property(&Key::name("k")).unwrap().enumerable);
    let getter = realm.function("get", 0, false, |_, _, _| Ok(Value::Number(5.0)));
    let setter = realm.function("set", 1, false, |_, _, _| Ok(Value::Undefined));
    define_accessor(&literal, Key::name("k"), Some(getter.clone()), None);
    define_accessor(&literal, Key::name("k"), None, Some(setter.clone()));
    let Some(Property {
        slot: Slot::Accessor { get, set },
        ..
    }) = literal.own_property(&Key::name("k"))
    else {
        panic!("accessor expected");
    };
    assert_eq!(get, Some(getter));
    assert_eq!(set, Some(setter));
}

#[test]
fn symbols_convert_compare_and_key_properties() {
    let mut realm = Realm::new();
    let symbol = crate::Symbol::new(Some(JsStr::from("s")));
    let value = Value::Symbol(symbol.clone());
    assert!(to_boolean(&value));
    assert_eq!(type_of(&value), "symbol");
    assert!(strict_equals(&value, &value.clone()));
    assert!(!strict_equals(
        &value,
        &Value::Symbol(crate::Symbol::new(None))
    ));
    assert!(loose_equals(&mut realm, &value, &value.clone()).unwrap());
    assert!(!loose_equals(&mut realm, &value, &Value::str("Symbol(s)")).unwrap());
    let error = to_number(&mut realm, &value).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot convert a Symbol value to a number"
    );
    let error = to_string(&mut realm, &value).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot convert a Symbol value to a string"
    );
    assert!(add(&mut realm, &value, &Value::str("")).is_err());
    assert_eq!(
        to_key(&mut realm, &value).unwrap(),
        Key::Symbol(symbol.clone())
    );
    let object = realm.object();
    object.define(Key::name("b"), Property::data(Value::Null));
    object.define(
        Key::Symbol(symbol.clone()),
        Property::data(Value::Number(1.0)),
    );
    object.define(Key::name("a"), Property::data(Value::Null));
    object.define(Key::Index(1), Property::data(Value::Null));
    let keys: Vec<_> = object
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["1", "b", "a", "Symbol(s)"]);
    let target = Value::Object(object);
    let read = get(&mut realm, &target, &Key::Symbol(symbol.clone())).unwrap();
    assert!(strict_equals(&read, &Value::Number(1.0)));
    realm
        .symbol_prototype
        .define(Key::name("kind"), Property::data(Value::str("symbolic")));
    let kind = get(&mut realm, &value, &Key::name("kind")).unwrap();
    assert_eq!(text(&mut realm, &kind), "symbolic");
    let wrapper = convertible(&mut realm, Some(value.clone()), None);
    assert_eq!(to_key(&mut realm, &wrapper).unwrap(), Key::Symbol(symbol));
}

#[test]
fn well_known_symbols_customize_conversion_and_instanceof() {
    let mut realm = Realm::new();
    let hints = realm.array(Vec::new());
    let log = hints.clone();
    let exotic = realm.function("toPrimitive", 1, false, move |realm, _, args| {
        array_push_value(realm, &log, args[0].clone());
        Ok(Value::Number(5.0))
    });
    let object = realm.object();
    object.define(
        Key::Symbol(realm.symbols.to_primitive.clone()),
        Property::data(Value::Object(exotic)),
    );
    let value = Value::Object(object.clone());
    let sum = add(&mut realm, &value, &Value::Number(1.0)).unwrap();
    assert!(strict_equals(&sum, &Value::Number(6.0)));
    assert_eq!(to_number(&mut realm, &value).unwrap(), 5.0);
    assert_eq!(text(&mut realm, &value), "5");
    let recorded = Value::Object(hints);
    assert_eq!(text(&mut realm, &recorded), "default,number,string");
    let bad = realm.function("toPrimitive", 1, false, |realm, _, _| {
        Ok(Value::Object(realm.object()))
    });
    object.define(
        Key::Symbol(realm.symbols.to_primitive.clone()),
        Property::data(Value::Object(bad)),
    );
    let error = to_number(&mut realm, &value).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Cannot convert object to primitive value"
    );
    let checker = realm.object();
    let always = realm.function("hasInstance", 1, false, |_, _, args| {
        Ok(Value::Boolean(matches!(args[0], Value::Number(_))))
    });
    checker.define(
        Key::Symbol(realm.symbols.has_instance.clone()),
        Property::data(Value::Object(always)),
    );
    let checker = Value::Object(checker);
    assert!(instance_of(&mut realm, &Value::Number(1.0), &checker).unwrap());
    assert!(!instance_of(&mut realm, &Value::Null, &checker).unwrap());
    let failing = realm.function("hasInstance", 1, false, |_, _, _| {
        Err(Thrown(Value::str("instance hook threw")))
    });
    checker.as_object().unwrap().define(
        Key::Symbol(realm.symbols.has_instance.clone()),
        Property::data(Value::Object(failing)),
    );
    let error = instance_of(&mut realm, &Value::Null, &checker).unwrap_err();
    assert!(strict_equals(&error.0, &Value::str("instance hook threw")));
    checker.as_object().unwrap().define(
        Key::Symbol(realm.symbols.has_instance.clone()),
        Property::data(Value::Number(1.0)),
    );
    assert!(instance_of(&mut realm, &Value::Null, &checker).is_err());
    let error = instance_of(&mut realm, &Value::Null, &Value::Number(1.0)).unwrap_err();
    assert_eq!(
        thrown(&mut realm, error),
        "TypeError: Right-hand side of 'instanceof' is not an object"
    );
}

fn array_push_value(realm: &mut Realm, array: &Object, value: Value) {
    let length = array.own_keys().len() - 1;
    set(
        realm,
        &Value::Object(array.clone()),
        &Key::Index(length as u32),
        value,
    )
    .unwrap();
}

#[test]
fn well_known_symbol_lookups_propagate_errors() {
    let mut realm = Realm::new();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "lookup"))
    });
    let trap = |realm: &Realm, key: crate::Symbol| {
        let object = realm.object();
        object.define(
            Key::Symbol(key),
            Property {
                slot: Slot::Accessor {
                    get: Some(getter.clone()),
                    set: None,
                },
                enumerable: false,
                configurable: true,
            },
        );
        Value::Object(object)
    };
    let primitive = trap(&realm, realm.symbols.to_primitive.clone());
    assert!(to_number(&mut realm, &primitive).is_err());
    let checker = trap(&realm, realm.symbols.has_instance.clone());
    assert!(instance_of(&mut realm, &Value::Null, &checker).is_err());
}

#[test]
fn ordinary_invocations_keep_new_target_local_and_restore_it_after_calls_and_errors() {
    let mut realm = Realm::new();
    let read = function(&realm, |realm, _, _| Ok(realm.new_target()));
    assert!(matches!(
        call(&mut realm, &read, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    ));
    assert!(strict_equals(
        &construct(&mut realm, &read, &[]).unwrap(),
        &read
    ));
    let target = function(&realm, |_, _, _| Ok(Value::Undefined));
    assert!(strict_equals(
        &construct_target(&mut realm, &read, &[], &target).unwrap(),
        &target
    ));
    assert!(matches!(realm.new_target(), Value::Undefined));
    let inner = read.clone();
    let outer = function(&realm, move |realm, _, _| {
        let before = realm.new_target();
        assert!(matches!(
            call(realm, &inner, Value::Undefined, &[])?,
            Value::Undefined
        ));
        assert!(strict_equals(&realm.new_target(), &before));
        let fail = function(realm, |realm, _, _| {
            Err(realm.throw(ErrorKind::Range, "inner"))
        });
        assert!(construct(realm, &fail, &[]).is_err());
        assert!(strict_equals(&realm.new_target(), &before));
        let limit = realm.max_depth;
        realm.max_depth = 0;
        assert!(call(realm, &inner, Value::Undefined, &[]).is_err());
        realm.max_depth = limit;
        assert!(strict_equals(&realm.new_target(), &before));
        Ok(before)
    });
    assert!(strict_equals(
        &construct(&mut realm, &outer, &[]).unwrap(),
        &outer
    ));
    assert!(matches!(realm.new_target(), Value::Undefined));
    let prototype = realm.object();
    let builtin = Value::Object(realm.builtin_constructor(
        "Target",
        0,
        &prototype,
        |realm, _, _| Ok(realm.new_target()),
        |realm, target, _| {
            assert!(strict_equals(
                &realm.new_target(),
                &Value::Object(target.clone())
            ));
            Ok(target)
        },
    ));
    assert!(strict_equals(
        &construct_target(&mut realm, &builtin, &[], &target).unwrap(),
        &target
    ));
    assert!(matches!(
        call(&mut realm, &builtin, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    ));
    assert!(matches!(realm.new_target(), Value::Undefined));
}

#[test]
fn static_reads_follow_their_cache_only_while_it_holds() {
    use crate::string::StaticStr;
    static NAME: StaticStr<4> = StaticStr::new("name");
    static LENGTH: StaticStr<6> = StaticStr::new("length");
    static VALUE: StaticStr<5> = StaticStr::new("value");
    let mut realm = Realm::new();
    let read = |realm: &mut Realm, base: &Object, name: &'static StaticStr<4>| {
        let value = get_static(realm, &Value::Object(base.clone()), &name.header).unwrap();
        text(realm, &value)
    };
    // A prototype's property, through a child that lacks it.
    let grand = realm.object();
    grand.define(Key::name("name"), Property::data(Value::str("grand")));
    let parent = Object::new(Some(grand.clone()), Kind::Ordinary);
    parent.define(Key::name("other"), Property::data(Value::Number(1.0)));
    let child = Object::new(Some(parent.clone()), Kind::Ordinary);
    assert_eq!(read(&mut realm, &child, &NAME), "grand");
    assert_eq!(read(&mut realm, &child, &NAME), "grand");
    // Another object with the same prototype finds it where the cache says.
    let sibling = Object::new(Some(parent.clone()), Kind::Ordinary);
    assert_eq!(read(&mut realm, &sibling, &NAME), "grand");
    // A property that comes to shadow it on the way wins.
    parent.define(Key::name("name"), Property::data(Value::str("parent")));
    assert_eq!(read(&mut realm, &child, &NAME), "parent");
    child.define(Key::name("name"), Property::data(Value::str("child")));
    assert_eq!(read(&mut realm, &child, &NAME), "child");
    assert_eq!(read(&mut realm, &sibling, &NAME), "parent");
    // Removed properties are not read from their old slots.
    child.delete(&Key::name("name"));
    parent.delete(&Key::name("name"));
    grand.delete(&Key::name("name"));
    assert_eq!(read(&mut realm, &child, &NAME), "undefined");
    // Getters get the receiver.
    let getter = realm.function("get", 0, false, |realm, this, _| {
        get(realm, &this, &Key::name("own"))
    });
    parent.define(
        Key::name("value"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    child.define(Key::name("own"), Property::data(Value::str("mine")));
    let value = get_static(&mut realm, &Value::Object(child.clone()), &VALUE.header).unwrap();
    assert_eq!(text(&mut realm, &value), "mine");
    // Arrays' lengths, proxies and primitives take the general reads.
    let array = realm.array(vec![Value::Number(1.0), Value::Number(2.0)]);
    let length = get_static(&mut realm, &Value::Object(array), &LENGTH.header).unwrap();
    assert!(strict_equals(&length, &Value::Number(2.0)));
    let length = get_static(&mut realm, &Value::str("abc"), &LENGTH.header).unwrap();
    assert!(strict_equals(&length, &Value::Number(3.0)));
    let global = Value::Object(realm.global.clone());
    let proxy_constructor = get(&mut realm, &global, &Key::name("Proxy")).unwrap();
    let target = realm.object();
    target.define(Key::name("name"), Property::data(Value::str("target")));
    let handler = Value::Object(realm.object());
    let proxy = construct(
        &mut realm,
        &proxy_constructor,
        &[Value::Object(target), handler],
    )
    .unwrap();
    let Value::Object(proxy) = proxy else {
        panic!("a proxy is an object");
    };
    assert_eq!(read(&mut realm, &proxy, &NAME), "target");
    // A chain longer than the cached walk is read in full.
    let mut deep = realm.object();
    deep.define(Key::name("name"), Property::data(Value::str("deep")));
    for _ in 0..10 {
        deep = Object::new(Some(deep), Kind::Ordinary);
    }
    assert_eq!(read(&mut realm, &deep, &NAME), "deep");
}

#[test]
fn site_caches_remember_two_places() {
    use crate::string::{LookupCache, StaticStr};
    static NAME: StaticStr<4> = StaticStr::new("name");
    static SITE: LookupCache = LookupCache::new();
    let mut realm = Realm::new();
    let read = |realm: &mut Realm, base: &Object| {
        let value =
            get_static_at(realm, &Value::Object(base.clone()), &NAME.header, &SITE).unwrap();
        text(realm, &value)
    };
    // Two layouts, the name in different slots.
    let first = realm.object();
    first.define(Key::name("x"), Property::data(Value::Number(1.0)));
    first.define(Key::name("name"), Property::data(Value::str("first")));
    let second = realm.object();
    second.define(Key::name("name"), Property::data(Value::str("second")));
    for _ in 0..2 {
        assert_eq!(read(&mut realm, &first), "first");
        assert_eq!(read(&mut realm, &second), "second");
    }
    // Remembering the latest place again keeps the other.
    let latest = SITE.get();
    SITE.set(latest[0].0, latest[0].1);
    assert_eq!(SITE.get(), latest);
    let mut places = SITE.get();
    places.sort_unstable();
    assert_eq!(places, [(0, 0), (0, 1)]);
    // Two prototypes, as subclasses each with the method.
    let base = realm.object();
    let left = Object::new(Some(base.clone()), Kind::Ordinary);
    left.define(Key::name("name"), Property::data(Value::str("left")));
    let right = Object::new(Some(base.clone()), Kind::Ordinary);
    right.define(Key::name("name"), Property::data(Value::str("right")));
    let (a, b) = (
        Object::new(Some(left.clone()), Kind::Ordinary),
        Object::new(Some(right.clone()), Kind::Ordinary),
    );
    for _ in 0..2 {
        assert_eq!(read(&mut realm, &a), "left");
        assert_eq!(read(&mut realm, &b), "right");
    }
    // An own property shadows a remembered prototype's.
    a.define(Key::name("name"), Property::data(Value::str("own")));
    assert_eq!(read(&mut realm, &a), "own");
}

#[test]
fn keyed_reads_cache_string_keys_only() {
    use crate::string::LookupCache;
    static SITE: LookupCache = LookupCache::new();
    let mut realm = Realm::new();
    let object = realm.object();
    object.define(Key::name("a"), Property::data(Value::str("first")));
    object.define(Key::name("b"), Property::data(Value::str("second")));
    let base = Value::Object(object);
    for (name, expected) in [("a", "first"), ("b", "second"), ("a", "first")] {
        let value = get_at(&mut realm, &base, &Key::name(name), &SITE).unwrap();
        assert_eq!(text(&mut realm, &value), expected);
    }
    // Indexes and primitives take the general read.
    let array = Value::Object(realm.array(vec![Value::str("zero")]));
    let value = get_at(&mut realm, &array, &Key::Index(0), &SITE).unwrap();
    assert_eq!(text(&mut realm, &value), "zero");
    let length = get_at(&mut realm, &Value::str("abc"), &Key::name("length"), &SITE).unwrap();
    assert!(strict_equals(&length, &Value::Number(3.0)));
}

#[test]
fn writes_replace_values_where_their_site_cache_says() {
    use crate::string::LookupCache;
    static SITE: LookupCache = LookupCache::new();
    let mut realm = Realm::new();
    let first = realm.object();
    first.define(Key::name("x"), Property::data(Value::Number(0.0)));
    first.define(Key::name("value"), Property::data(Value::Number(0.0)));
    let second = realm.object();
    second.define(Key::name("value"), Property::data(Value::Number(0.0)));
    for round in 1..3 {
        for object in [&first, &second] {
            let base = Value::Object(object.clone());
            set_at(
                &mut realm,
                &base,
                &Key::name("value"),
                Value::Number(f64::from(round)),
                &SITE,
            )
            .unwrap();
            let read = get(&mut realm, &base, &Key::name("value")).unwrap();
            assert!(strict_equals(&read, &Value::Number(f64::from(round))));
        }
    }
    let mut places = SITE.get();
    places.sort_unstable();
    assert_eq!(places, [(0, 0), (0, 1)]);
    // New and read-only properties take the general write.
    let base = Value::Object(second.clone());
    set_sloppy_at(&mut realm, &base, &Key::name("added"), Value::Null, &SITE).unwrap();
    assert!(second.own_property(&Key::name("added")).is_some());
    second.define(
        Key::name("fixed"),
        Property {
            slot: Slot::Data {
                value: Value::Null,
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(
        set_at(
            &mut realm,
            &base,
            &Key::name("fixed"),
            Value::Undefined,
            &SITE
        )
        .is_err()
    );
}
