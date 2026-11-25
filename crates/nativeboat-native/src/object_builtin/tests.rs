use super::*;

fn global(realm: &mut Realm) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name("Object")).unwrap()
}

/// Calls `Object.<name>(...args)`.
fn invoke(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let constructor = global(realm);
    let function = ops::get(realm, &constructor, &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

/// Calls `Object.prototype.<name>` with a receiver.
fn method(realm: &mut Realm, name: &str, this: Value, args: &[Value]) -> Completion {
    let prototype = Value::Object(realm.object_prototype.clone());
    let function = ops::get(realm, &prototype, &Key::name(name)).unwrap();
    ops::call(realm, &function, this, args)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    text(realm, &thrown.0)
}

fn get(realm: &mut Realm, value: &Value, key: &str) -> Value {
    ops::get(realm, value, &Key::name(key)).unwrap()
}

fn object(realm: &Realm, entries: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (key, value) in entries {
        object.define(Key::name(key), Property::data(value.clone()));
    }
    Value::Object(object)
}

fn function(
    realm: &Realm,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 0, false, code))
}

fn throwing_getter(realm: &Realm, target: &Object, key: Key) {
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "getter"))
    });
    target.define(
        key,
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
fn object_calls_and_construction_box_values() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    for value in [Value::Undefined, Value::Null] {
        let created = ops::call(&mut realm, &constructor, Value::Undefined, &[value]).unwrap();
        assert_eq!(text(&mut realm, &created), "[object Object]");
    }
    let existing = object(&realm, &[]);
    let same = ops::call(
        &mut realm,
        &constructor,
        Value::Undefined,
        std::slice::from_ref(&existing),
    )
    .unwrap();
    assert!(ops::strict_equals(&same, &existing));
    for (value, tag, prototype) in [
        (
            Value::Boolean(true),
            "true",
            realm.boolean_prototype.clone(),
        ),
        (Value::Number(1.0), "1", realm.number_prototype.clone()),
        (Value::str("ab"), "ab", realm.string_prototype.clone()),
    ] {
        let boxed = ops::construct(&mut realm, &constructor, &[value]).unwrap();
        assert_eq!(text(&mut realm, &boxed), tag);
        assert_eq!(boxed.as_object().unwrap().data().prototype, Some(prototype));
    }
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let boxed = ops::construct(&mut realm, &constructor, &[symbol]).unwrap();
    assert!(matches!(
        boxed.as_object().unwrap().data().kind,
        Kind::SymbolObject(_)
    ));
    let empty = ops::construct(&mut realm, &constructor, &[]).unwrap();
    assert_eq!(
        empty.as_object().unwrap().data().prototype,
        Some(realm.object_prototype.clone())
    );
    // A different new target supplies the prototype; a primitive one falls back.
    let derived = realm.function("Derived", 0, true, |_, _, _| Ok(Value::Undefined));
    let custom = ops::construct_target(
        &mut realm,
        &constructor,
        &[Value::Number(1.0)],
        &Value::Object(derived.clone()),
    )
    .unwrap();
    let custom_prototype = get(&mut realm, &Value::Object(derived.clone()), "prototype");
    assert_eq!(
        custom.as_object().unwrap().data().prototype,
        custom_prototype.as_object().cloned()
    );
    ops::set(
        &mut realm,
        &Value::Object(derived.clone()),
        &Key::name("prototype"),
        Value::Null,
    )
    .unwrap();
    let fallback = ops::construct_target(
        &mut realm,
        &constructor,
        &[],
        &Value::Object(derived.clone()),
    )
    .unwrap();
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        Some(realm.object_prototype.clone())
    );
    throwing_getter(&realm, &derived, Key::name("prototype"));
    let other = realm.function("Other", 0, true, |_, _, _| Ok(Value::Undefined));
    throwing_getter(&realm, &other, Key::name("prototype"));
    assert!(ops::construct_target(&mut realm, &constructor, &[], &Value::Object(other)).is_err());
}

#[test]
fn enumeration_assignment_and_entries() {
    let mut realm = Realm::new();
    let parent = object(&realm, &[("inherited", Value::Null)]);
    let source = Object::new(parent.as_object().cloned(), Kind::Ordinary);
    source.define(Key::name("b"), Property::data(Value::Number(2.0)));
    source.define(Key::Index(1), Property::data(Value::Number(1.0)));
    source.define(Key::name("hidden"), Property::hidden(Value::Null));
    source.define(
        Key::Symbol(crate::Symbol::new(None)),
        Property::data(Value::Null),
    );
    let source = Value::Object(source);
    let keys = invoke(&mut realm, "keys", std::slice::from_ref(&source)).unwrap();
    assert_eq!(text(&mut realm, &keys), "1,b");
    let values = invoke(&mut realm, "values", std::slice::from_ref(&source)).unwrap();
    assert_eq!(text(&mut realm, &values), "1,2");
    let entries = invoke(&mut realm, "entries", std::slice::from_ref(&source)).unwrap();
    assert_eq!(text(&mut realm, &entries), "1,1,b,2");
    let letters = invoke(&mut realm, "keys", &[Value::str("hi")]).unwrap();
    assert_eq!(text(&mut realm, &letters), "0,1");
    assert_eq!(
        {
            let __result = invoke(&mut realm, "keys", &[Value::Null]);
            {
                let __result = __result;
                message(&mut realm, __result)
            }
        },
        "TypeError: Cannot convert undefined or null to object"
    );
    let failing = realm.object();
    throwing_getter(&realm, &failing, Key::name("trap"));
    for name in ["values", "entries"] {
        assert!(invoke(&mut realm, name, &[Value::Object(failing.clone())]).is_err());
    }
    // A getter that hides a later property excludes it.
    let changing = realm.object();
    let target = changing.clone();
    let hide = realm.function("get", 0, false, move |_, _, _| {
        target.define(Key::name("later"), Property::hidden(Value::Null));
        Ok(Value::Null)
    });
    changing.define(
        Key::name("first"),
        Property {
            slot: Slot::Accessor {
                get: Some(hide),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    changing.define(Key::name("later"), Property::data(Value::Null));
    let seen = invoke(&mut realm, "values", &[Value::Object(changing)]).unwrap();
    assert!(get(&mut realm, &seen, "length").as_object().is_none());
    assert!(ops::strict_equals(
        &get(&mut realm, &seen, "length"),
        &Value::Number(1.0)
    ));
    let target = object(&realm, &[("a", Value::Number(0.0))]);
    let assigned = invoke(
        &mut realm,
        "assign",
        &[target.clone(), Value::Null, source.clone(), Value::str("x")],
    )
    .unwrap();
    assert!(ops::strict_equals(&assigned, &target));
    let merged = invoke(&mut realm, "keys", std::slice::from_ref(&target)).unwrap();
    assert_eq!(text(&mut realm, &merged), "0,1,a,b");
    let boxed = invoke(&mut realm, "assign", &[Value::Number(1.0)]).unwrap();
    assert_eq!(text(&mut realm, &boxed), "1");
    assert!(invoke(&mut realm, "assign", &[Value::Undefined]).is_err());
    assert!(
        invoke(
            &mut realm,
            "assign",
            &[target.clone(), Value::Object(failing.clone())]
        )
        .is_err()
    );
    let frozen = object(&realm, &[]);
    invoke(&mut realm, "freeze", std::slice::from_ref(&frozen)).unwrap();
    assert!(invoke(&mut realm, "assign", &[frozen, source]).is_err());
}

#[test]
fn defining_properties_from_descriptors() {
    let mut realm = Realm::new();
    let target = object(&realm, &[]);
    let getter = function(&realm, |_, _, _| Ok(Value::Number(5.0)));
    let descriptor = object(
        &realm,
        &[
            ("get", getter.clone()),
            ("enumerable", Value::Boolean(true)),
        ],
    );
    let result = invoke(
        &mut realm,
        "defineProperty",
        &[target.clone(), Value::str("x"), descriptor],
    )
    .unwrap();
    assert!(ops::strict_equals(&result, &target));
    assert!(ops::strict_equals(
        &get(&mut realm, &target, "x"),
        &Value::Number(5.0)
    ));
    let described = invoke(
        &mut realm,
        "getOwnPropertyDescriptor",
        &[target.clone(), Value::str("x")],
    )
    .unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &described, "get"),
        &getter
    ));
    assert!(matches!(
        get(&mut realm, &described, "set"),
        Value::Undefined
    ));
    assert!(ops::to_boolean(&get(&mut realm, &described, "enumerable")));
    assert!(!ops::to_boolean(&get(
        &mut realm,
        &described,
        "configurable"
    )));
    let data = object(
        &realm,
        &[
            ("value", Value::Number(1.0)),
            ("writable", Value::Boolean(true)),
        ],
    );
    invoke(
        &mut realm,
        "defineProperty",
        &[target.clone(), Value::str("y"), data],
    )
    .unwrap();
    let described = invoke(
        &mut realm,
        "getOwnPropertyDescriptor",
        &[target.clone(), Value::str("y")],
    )
    .unwrap();
    assert!(ops::to_boolean(&get(&mut realm, &described, "writable")));
    let configurable = object(
        &realm,
        &[
            ("value", Value::Null),
            ("configurable", Value::Boolean(true)),
        ],
    );
    invoke(
        &mut realm,
        "defineProperty",
        &[target.clone(), Value::str("loose"), configurable],
    )
    .unwrap();
    assert!(
        target
            .as_object()
            .unwrap()
            .own_property(&Key::name("loose"))
            .unwrap()
            .configurable
    );
    assert!(target.as_object().unwrap().delete(&Key::name("loose")));
    let missing = invoke(
        &mut realm,
        "getOwnPropertyDescriptor",
        &[target.clone(), Value::str("none")],
    )
    .unwrap();
    assert!(matches!(missing, Value::Undefined));
    let all = invoke(
        &mut realm,
        "getOwnPropertyDescriptors",
        std::slice::from_ref(&target),
    )
    .unwrap();
    let keys = invoke(&mut realm, "keys", &[all]).unwrap();
    assert_eq!(text(&mut realm, &keys), "x,y");
    // Redefining a non-configurable property throws.
    let again = object(&realm, &[("value", Value::Number(1.0))]);
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[target.clone(), Value::str("x"), again.clone()],
            );
            message(&mut realm, __result)
        },
        "TypeError: Cannot redefine property: x"
    );
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[Value::Number(1.0), Value::str("x"), again.clone()],
            );
            message(&mut realm, __result)
        },
        "TypeError: Object.defineProperty called on non-object"
    );
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[target.clone(), Value::str("z"), Value::Null],
            );
            message(&mut realm, __result)
        },
        "TypeError: Property description must be an object"
    );
    let bad_getter = object(&realm, &[("get", Value::Number(1.0))]);
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[target.clone(), Value::str("z"), bad_getter],
            );
            message(&mut realm, __result)
        },
        "TypeError: Property descriptor get must be a function"
    );
    let bad_setter = object(&realm, &[("set", Value::Null)]);
    assert!(
        invoke(
            &mut realm,
            "defineProperty",
            &[target.clone(), Value::str("z"), bad_setter]
        )
        .is_err()
    );
    let mixed = object(&realm, &[("set", Value::Undefined), ("value", Value::Null)]);
    assert!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[target.clone(), Value::str("z"), mixed],
            );
            message(&mut realm, __result)
        }
        .contains("Cannot both specify")
    );
    let failing = realm.object();
    throwing_getter(&realm, &failing, Key::name("value"));
    assert!(
        invoke(
            &mut realm,
            "defineProperty",
            &[
                target.clone(),
                Value::str("z"),
                Value::Object(failing.clone())
            ]
        )
        .is_err()
    );
    let inherited = Object::new(Some(realm.object()), Kind::Ordinary);
    inherited.data().prototype.as_ref().unwrap().define(
        Key::name("value"),
        Property::data(Value::str("from prototype")),
    );
    invoke(
        &mut realm,
        "defineProperty",
        &[
            target.clone(),
            Value::str("inherited"),
            Value::Object(inherited),
        ],
    )
    .unwrap();
    assert_eq!(
        {
            let __result = get(&mut realm, &target, "inherited");
            text(&mut realm, &__result)
        },
        "from prototype"
    );
    // Array lengths are validated before definition.
    let list = Value::Object(realm.array(vec![Value::Null, Value::Null]));
    let shorter = object(&realm, &[("value", Value::str("1"))]);
    invoke(
        &mut realm,
        "defineProperty",
        &[list.clone(), Value::str("length"), shorter],
    )
    .unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &list, "length"),
        &Value::Number(1.0)
    ));
    let invalid = object(&realm, &[("value", Value::Number(-1.0))]);
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "defineProperty",
                &[list.clone(), Value::str("length"), invalid],
            );
            message(&mut realm, __result)
        },
        "RangeError: Invalid array length"
    );
    let symbol_length = object(
        &realm,
        &[("value", Value::Symbol(crate::Symbol::new(None)))],
    );
    assert!(
        invoke(
            &mut realm,
            "defineProperty",
            &[list.clone(), Value::str("length"), symbol_length]
        )
        .is_err()
    );
    let plain = object(&realm, &[("value", Value::Number(1.0))]);
    invoke(
        &mut realm,
        "defineProperty",
        &[list.clone(), Value::str("other"), plain],
    )
    .unwrap();
    // defineProperties reads every descriptor before applying any.
    let descriptors = Value::Object(realm.object());
    let first = object(
        &realm,
        &[
            ("value", Value::Number(1.0)),
            ("enumerable", Value::Boolean(true)),
        ],
    );
    descriptors
        .as_object()
        .unwrap()
        .define(Key::name("a"), Property::data(first));
    descriptors
        .as_object()
        .unwrap()
        .define(Key::name("skipped"), Property::hidden(Value::Null));
    let fresh = object(&realm, &[]);
    invoke(
        &mut realm,
        "defineProperties",
        &[fresh.clone(), descriptors.clone()],
    )
    .unwrap();
    let keys = invoke(&mut realm, "keys", std::slice::from_ref(&fresh)).unwrap();
    assert_eq!(text(&mut realm, &keys), "a");
    descriptors
        .as_object()
        .unwrap()
        .define(Key::name("bad"), Property::data(Value::Null));
    let untouched = object(&realm, &[]);
    assert!(
        invoke(
            &mut realm,
            "defineProperties",
            &[untouched.clone(), descriptors.clone()]
        )
        .is_err()
    );
    let none = invoke(&mut realm, "keys", &[untouched]).unwrap();
    assert_eq!(text(&mut realm, &none), "");
    assert!(
        invoke(
            &mut realm,
            "defineProperties",
            &[Value::Null, descriptors.clone()]
        )
        .is_err()
    );
    assert!(
        invoke(
            &mut realm,
            "defineProperties",
            &[fresh.clone(), Value::Null]
        )
        .is_err()
    );
    let trapping = realm.object();
    throwing_getter(&realm, &trapping, Key::name("a"));
    assert!(
        invoke(
            &mut realm,
            "defineProperties",
            &[fresh.clone(), Value::Object(trapping)]
        )
        .is_err()
    );
    let conflicting = Value::Object(realm.object());
    let fixed_again = object(&realm, &[("value", Value::Number(2.0))]);
    conflicting
        .as_object()
        .unwrap()
        .define(Key::name("a"), Property::data(fixed_again));
    assert!(invoke(&mut realm, "defineProperties", &[fresh, conflicting]).is_err());
    assert!(invoke(&mut realm, "getOwnPropertyDescriptor", &[Value::Null]).is_err());
    assert!(invoke(&mut realm, "getOwnPropertyDescriptors", &[Value::Null]).is_err());
}

#[test]
fn creation_prototypes_and_integrity_levels() {
    let mut realm = Realm::new();
    let parent = object(&realm, &[("p", Value::Number(1.0))]);
    let child = invoke(&mut realm, "create", std::slice::from_ref(&parent)).unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &child, "p"),
        &Value::Number(1.0)
    ));
    let bare = invoke(&mut realm, "create", &[Value::Null]).unwrap();
    assert!(bare.as_object().unwrap().data().prototype.is_none());
    let props = object(
        &realm,
        &[("q", object(&realm, &[("value", Value::Number(2.0))]))],
    );
    let described = invoke(&mut realm, "create", &[parent.clone(), props]).unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &described, "q"),
        &Value::Number(2.0)
    ));
    assert_eq!(
        {
            let __result = invoke(&mut realm, "create", &[Value::Number(1.0)]);
            message(&mut realm, __result)
        },
        "TypeError: Object prototype may only be an Object or null"
    );
    assert!(invoke(&mut realm, "create", &[Value::Null, Value::Null]).is_err());
    let prototype = invoke(&mut realm, "getPrototypeOf", std::slice::from_ref(&child)).unwrap();
    assert!(ops::strict_equals(&prototype, &parent));
    let none = invoke(&mut realm, "getPrototypeOf", std::slice::from_ref(&bare)).unwrap();
    assert!(matches!(none, Value::Null));
    let string_prototype = invoke(&mut realm, "getPrototypeOf", &[Value::str("")]).unwrap();
    assert!(ops::strict_equals(
        &string_prototype,
        &Value::Object(realm.string_prototype.clone())
    ));
    assert!(invoke(&mut realm, "getPrototypeOf", &[Value::Undefined]).is_err());
    let moved = invoke(
        &mut realm,
        "setPrototypeOf",
        &[bare.clone(), parent.clone()],
    )
    .unwrap();
    assert!(ops::strict_equals(&moved, &bare));
    assert!(ops::strict_equals(
        &get(&mut realm, &bare, "p"),
        &Value::Number(1.0)
    ));
    let primitive = invoke(
        &mut realm,
        "setPrototypeOf",
        &[Value::Number(1.0), Value::Null],
    )
    .unwrap();
    assert!(ops::strict_equals(&primitive, &Value::Number(1.0)));
    assert!(invoke(&mut realm, "setPrototypeOf", &[Value::Null, Value::Null]).is_err());
    assert!(
        invoke(
            &mut realm,
            "setPrototypeOf",
            &[bare.clone(), Value::Number(1.0)]
        )
        .is_err()
    );
    assert_eq!(
        {
            let __result = invoke(
                &mut realm,
                "setPrototypeOf",
                &[parent.clone(), bare.clone()],
            );
            message(&mut realm, __result)
        },
        "TypeError: Cannot set the prototype of this object"
    );
    // Integrity levels.
    let accessor = realm.object();
    accessor.define(
        Key::name("a"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    accessor.define(Key::name("d"), Property::data(Value::Null));
    let accessor = Value::Object(accessor);
    for (name, value) in [
        ("isFrozen", true),
        ("isSealed", true),
        ("isExtensible", false),
    ] {
        let result = invoke(&mut realm, name, &[Value::Number(1.0)]).unwrap();
        assert_eq!(ops::to_boolean(&result), value, "{name}");
    }
    let sealed = invoke(&mut realm, "seal", std::slice::from_ref(&accessor)).unwrap();
    assert!(ops::strict_equals(&sealed, &accessor));
    let check = |realm: &mut Realm, name: &str, value: &Value| {
        ops::to_boolean(&invoke(realm, name, std::slice::from_ref(value)).unwrap())
    };
    assert!(check(&mut realm, "isSealed", &accessor));
    assert!(!check(&mut realm, "isFrozen", &accessor));
    assert!(!check(&mut realm, "isExtensible", &accessor));
    ops::set(&mut realm, &accessor, &Key::name("d"), Value::Number(1.0)).unwrap();
    invoke(&mut realm, "freeze", std::slice::from_ref(&accessor)).unwrap();
    assert!(check(&mut realm, "isFrozen", &accessor));
    assert!(ops::set(&mut realm, &accessor, &Key::name("d"), Value::Number(2.0)).is_err());
    let frozen_primitive = invoke(&mut realm, "freeze", &[Value::str("x")]).unwrap();
    assert_eq!(text(&mut realm, &frozen_primitive), "x");
    let list = Value::Object(realm.array(vec![Value::Null]));
    invoke(&mut realm, "freeze", std::slice::from_ref(&list)).unwrap();
    assert!(check(&mut realm, "isFrozen", &list));
    assert!(ops::set(&mut realm, &list, &Key::Index(0), Value::Number(1.0)).is_err());
    assert!(ops::set(&mut realm, &list, &Key::name("length"), Value::Number(0.0)).is_err());
    let open = object(&realm, &[("x", Value::Null)]);
    assert!(check(&mut realm, "isExtensible", &open));
    invoke(&mut realm, "preventExtensions", std::slice::from_ref(&open)).unwrap();
    assert!(!check(&mut realm, "isFrozen", &open));
    assert!(!check(&mut realm, "isSealed", &open));
    let kept = invoke(&mut realm, "preventExtensions", &[Value::Number(2.0)]).unwrap();
    assert!(ops::strict_equals(&kept, &Value::Number(2.0)));
    let empty = object(&realm, &[]);
    invoke(
        &mut realm,
        "preventExtensions",
        std::slice::from_ref(&empty),
    )
    .unwrap();
    assert!(check(&mut realm, "isFrozen", &empty));
}

#[test]
fn entries_groups_names_and_identity() {
    let mut realm = Realm::new();
    let pairs = Value::Object(realm.array(vec![
        Value::Object(realm.array(vec![Value::str("a"), Value::Number(1.0)])),
        Value::Object(realm.array(vec![Value::Number(2.0), Value::str("two")])),
    ]));
    let built = invoke(&mut realm, "fromEntries", &[pairs]).unwrap();
    let keys = invoke(&mut realm, "keys", std::slice::from_ref(&built)).unwrap();
    assert_eq!(text(&mut realm, &keys), "2,a");
    assert!(invoke(&mut realm, "fromEntries", &[Value::Undefined]).is_err());
    assert!(invoke(&mut realm, "fromEntries", &[Value::Number(1.0)]).is_err());
    let bad = Value::Object(realm.array(vec![Value::Number(1.0)]));
    assert_eq!(
        {
            let __result = invoke(&mut realm, "fromEntries", &[bad]);
            {
                let __result = __result;
                message(&mut realm, __result)
            }
        },
        "TypeError: Iterator value is not an entry object"
    );
    let failing = realm.object();
    throwing_getter(&realm, &failing, Key::Index(0));
    let failing_entries = Value::Object(realm.array(vec![Value::Object(failing.clone())]));
    assert!(invoke(&mut realm, "fromEntries", &[failing_entries]).is_err());
    let items = Value::Object(realm.array(vec![
        Value::Number(1.0),
        Value::Number(2.0),
        Value::Number(3.0),
    ]));
    let parity = function(&realm, |realm, _, args| {
        let odd = ops::arithmetic(
            realm,
            ops::Arithmetic::Remainder,
            &args[0],
            &Value::Number(2.0),
        )?;
        Ok(Value::str(
            if ops::strict_equals(&odd, &Value::Number(1.0)) {
                "odd"
            } else {
                "even"
            },
        ))
    });
    let groups = invoke(&mut realm, "groupBy", &[items.clone(), parity]).unwrap();
    assert!(groups.as_object().unwrap().data().prototype.is_none());
    let odd = get(&mut realm, &groups, "odd");
    assert_eq!(text(&mut realm, &odd), "1,3");
    let even = get(&mut realm, &groups, "even");
    assert_eq!(text(&mut realm, &even), "2");
    assert!(invoke(&mut realm, "groupBy", &[Value::Null, Value::Null]).is_err());
    assert_eq!(
        {
            let __result = invoke(&mut realm, "groupBy", &[items.clone(), Value::Null]);
            message(&mut realm, __result)
        },
        "TypeError: Object.groupBy callback is not a function"
    );
    let throwing = function(&realm, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "group"))
    });
    assert!(invoke(&mut realm, "groupBy", &[items.clone(), throwing]).is_err());
    let not_iterable = function(&realm, |_, _, _| Ok(Value::Null));
    assert!(invoke(&mut realm, "groupBy", &[Value::Number(1.0), not_iterable]).is_err());
    let source = object(&realm, &[("b", Value::Null)]);
    let symbol = crate::Symbol::new(None);
    source
        .as_object()
        .unwrap()
        .define(Key::Symbol(symbol.clone()), Property::hidden(Value::Null));
    let names = invoke(
        &mut realm,
        "getOwnPropertyNames",
        std::slice::from_ref(&source),
    )
    .unwrap();
    assert_eq!(text(&mut realm, &names), "b");
    let symbols = invoke(
        &mut realm,
        "getOwnPropertySymbols",
        std::slice::from_ref(&source),
    )
    .unwrap();
    let first = get(&mut realm, &symbols, "0");
    assert!(ops::strict_equals(&first, &Value::Symbol(symbol)));
    let string_names = invoke(&mut realm, "getOwnPropertyNames", &[Value::str("ab")]).unwrap();
    assert_eq!(text(&mut realm, &string_names), "0,1,length");
    assert!(invoke(&mut realm, "getOwnPropertyNames", &[Value::Null]).is_err());
    let has = invoke(&mut realm, "hasOwn", &[source.clone(), Value::str("b")]).unwrap();
    assert!(ops::to_boolean(&has));
    let lacks = invoke(
        &mut realm,
        "hasOwn",
        &[source.clone(), Value::str("toString")],
    )
    .unwrap();
    assert!(!ops::to_boolean(&lacks));
    assert!(invoke(&mut realm, "hasOwn", &[Value::Null, Value::str("b")]).is_err());
    let nan = invoke(
        &mut realm,
        "is",
        &[Value::Number(f64::NAN), Value::Number(f64::NAN)],
    )
    .unwrap();
    assert!(ops::to_boolean(&nan));
    let zeros = invoke(&mut realm, "is", &[Value::Number(0.0), Value::Number(-0.0)]).unwrap();
    assert!(!ops::to_boolean(&zeros));
}

#[test]
fn prototype_methods() {
    let mut realm = Realm::new();
    for (value, tag) in [
        (Value::Undefined, "[object Undefined]"),
        (Value::Null, "[object Null]"),
        (Value::Object(realm.array(Vec::new())), "[object Array]"),
        (
            function(&realm, |_, _, _| Ok(Value::Undefined)),
            "[object Function]",
        ),
        (
            Value::Object(realm.error(ErrorKind::Error, "e")),
            "[object Error]",
        ),
        (Value::Object(realm.arguments(&[])), "[object Arguments]"),
        (Value::Boolean(false), "[object Boolean]"),
        (Value::Number(0.0), "[object Number]"),
        (Value::str(""), "[object String]"),
        (Value::Symbol(crate::Symbol::new(None)), "[object Symbol]"),
        (object(&realm, &[]), "[object Object]"),
        (
            crate::regexp_builtin::literal(&mut realm, &[97], "g").unwrap(),
            "[object RegExp]",
        ),
        (
            Value::Object(crate::object::Object::new(
                None,
                crate::object::Kind::Date(0.0),
            )),
            "[object Date]",
        ),
    ] {
        let result = method(&mut realm, "toString", value, &[]).unwrap();
        assert_eq!(text(&mut realm, &result), tag);
    }
    let tagged = object(&realm, &[]);
    tagged.as_object().unwrap().define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property::data(Value::str("Custom")),
    );
    let result = method(&mut realm, "toString", tagged.clone(), &[]).unwrap();
    assert_eq!(text(&mut realm, &result), "[object Custom]");
    let trap = realm.object();
    throwing_getter(
        &realm,
        &trap,
        Key::Symbol(realm.symbols.to_string_tag.clone()),
    );
    assert!(method(&mut realm, "toString", Value::Object(trap), &[]).is_err());
    let local = method(&mut realm, "toLocaleString", tagged.clone(), &[]).unwrap();
    assert_eq!(text(&mut realm, &local), "[object Custom]");
    assert!(method(&mut realm, "toLocaleString", Value::Null, &[]).is_err());
    let boxed = method(&mut realm, "valueOf", Value::Number(3.0), &[]).unwrap();
    assert_eq!(text(&mut realm, &boxed), "3");
    assert!(method(&mut realm, "valueOf", Value::Undefined, &[]).is_err());
    let parent = object(&realm, &[("p", Value::Null)]);
    let child = Value::Object(Object::new(parent.as_object().cloned(), Kind::Ordinary));
    let object_prototype = Value::Object(realm.object_prototype.clone());
    let is = method(
        &mut realm,
        "isPrototypeOf",
        object_prototype,
        std::slice::from_ref(&child),
    )
    .unwrap();
    assert!(ops::to_boolean(&is));
    let not = method(
        &mut realm,
        "isPrototypeOf",
        child.clone(),
        std::slice::from_ref(&parent),
    )
    .unwrap();
    assert!(!ops::to_boolean(&not));
    let primitive = method(
        &mut realm,
        "isPrototypeOf",
        Value::Null,
        &[Value::Number(1.0)],
    )
    .unwrap();
    assert!(!ops::to_boolean(&primitive));
    assert!(
        method(
            &mut realm,
            "isPrototypeOf",
            Value::Null,
            std::slice::from_ref(&child)
        )
        .is_err()
    );
    let enumerable = method(
        &mut realm,
        "propertyIsEnumerable",
        parent.clone(),
        &[Value::str("p")],
    )
    .unwrap();
    assert!(ops::to_boolean(&enumerable));
    let inherited = method(
        &mut realm,
        "propertyIsEnumerable",
        child.clone(),
        &[Value::str("p")],
    )
    .unwrap();
    assert!(!ops::to_boolean(&inherited));
    assert!(
        method(
            &mut realm,
            "propertyIsEnumerable",
            Value::Null,
            &[Value::str("p")]
        )
        .is_err()
    );
    assert!(
        method(
            &mut realm,
            "hasOwnProperty",
            parent.clone(),
            &[Value::Symbol(crate::Symbol::new(None))]
        )
        .is_ok()
    );
}

#[test]
fn legacy_accessor_methods_and_proto() {
    let mut realm = Realm::new();
    let target = object(&realm, &[("data", Value::Null)]);
    let getter = function(&realm, |_, _, _| Ok(Value::Number(4.0)));
    let setter = function(&realm, |_, _, _| Ok(Value::Undefined));
    method(
        &mut realm,
        "__defineGetter__",
        target.clone(),
        &[Value::str("x"), getter.clone()],
    )
    .unwrap();
    method(
        &mut realm,
        "__defineSetter__",
        target.clone(),
        &[Value::str("x"), setter.clone()],
    )
    .unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &target, "x"),
        &Value::Number(4.0)
    ));
    let found = method(
        &mut realm,
        "__lookupGetter__",
        target.clone(),
        &[Value::str("x")],
    )
    .unwrap();
    assert!(ops::strict_equals(&found, &getter));
    let child = Value::Object(Object::new(target.as_object().cloned(), Kind::Ordinary));
    let inherited = method(
        &mut realm,
        "__lookupSetter__",
        child.clone(),
        &[Value::str("x")],
    )
    .unwrap();
    assert!(ops::strict_equals(&inherited, &setter));
    let data = method(
        &mut realm,
        "__lookupGetter__",
        child.clone(),
        &[Value::str("data")],
    )
    .unwrap();
    assert!(matches!(data, Value::Undefined));
    let missing = method(
        &mut realm,
        "__lookupSetter__",
        child.clone(),
        &[Value::str("nothing")],
    )
    .unwrap();
    assert!(matches!(missing, Value::Undefined));
    let only_getter = object(&realm, &[]);
    method(
        &mut realm,
        "__defineGetter__",
        only_getter.clone(),
        &[Value::str("g"), getter.clone()],
    )
    .unwrap();
    let no_setter = method(
        &mut realm,
        "__lookupSetter__",
        only_getter,
        &[Value::str("g")],
    )
    .unwrap();
    assert!(matches!(no_setter, Value::Undefined));
    assert_eq!(
        {
            let __result = method(
                &mut realm,
                "__defineGetter__",
                target.clone(),
                &[Value::str("y"), Value::Null],
            );
            message(&mut realm, __result)
        },
        "TypeError: Accessor must be a function"
    );
    assert!(
        method(
            &mut realm,
            "__defineGetter__",
            Value::Null,
            &[Value::str("y"), getter.clone()]
        )
        .is_err()
    );
    assert!(
        method(
            &mut realm,
            "__lookupGetter__",
            Value::Null,
            &[Value::str("y")]
        )
        .is_err()
    );
    let frozen = object(&realm, &[]);
    frozen.as_object().unwrap().prevent_extensions();
    assert!(
        method(
            &mut realm,
            "__defineGetter__",
            frozen.clone(),
            &[Value::str("y"), getter]
        )
        .is_err()
    );
    // __proto__ reads and changes prototypes.
    let proto = get(&mut realm, &child, "__proto__");
    assert!(ops::strict_equals(&proto, &target));
    let other = object(&realm, &[("o", Value::Null)]);
    ops::set(&mut realm, &child, &Key::name("__proto__"), other.clone()).unwrap();
    assert!(ops::strict_equals(
        &get(&mut realm, &child, "__proto__"),
        &other
    ));
    ops::set(
        &mut realm,
        &child,
        &Key::name("__proto__"),
        Value::Number(1.0),
    )
    .unwrap();
    ops::set(&mut realm, &child, &Key::name("__proto__"), Value::Null).unwrap();
    assert!(child.as_object().unwrap().data().prototype.is_none());
    let string_proto = get(&mut realm, &Value::str("s"), "__proto__");
    assert!(ops::strict_equals(
        &string_proto,
        &Value::Object(realm.string_prototype.clone())
    ));
    let prototype = Value::Object(realm.object_prototype.clone());
    let accessor = ops::get(&mut realm, &prototype, &Key::name("__proto__"));
    let _ = accessor;
    let descriptor = realm
        .object_prototype
        .own_property(&Key::name("__proto__"))
        .unwrap();
    let Slot::Accessor {
        get: Some(getter),
        set: Some(setter),
    } = descriptor.slot
    else {
        panic!("__proto__ is an accessor");
    };
    let (getter, setter) = (Value::Object(getter), Value::Object(setter));
    assert!(ops::call(&mut realm, &getter, Value::Null, &[]).is_err());
    assert!(ops::call(&mut realm, &setter, Value::Undefined, &[Value::Null]).is_err());
    let kept = ops::call(&mut realm, &setter, Value::Number(1.0), &[Value::Null]).unwrap();
    assert!(matches!(kept, Value::Undefined));
    assert!(
        ops::call(
            &mut realm,
            &setter,
            other.clone(),
            std::slice::from_ref(&child)
        )
        .is_ok()
    );
    assert!(ops::call(&mut realm, &setter, child, &[other]).is_err());
}
