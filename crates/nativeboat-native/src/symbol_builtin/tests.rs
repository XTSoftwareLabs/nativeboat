use super::*;

fn global(realm: &mut Realm) -> Value {
    let object = Value::Object(realm.global.clone());
    ops::get(realm, &object, &Key::name("Symbol")).unwrap()
}

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap()
}

fn equal(actual: &Value, expected: &Value) {
    assert!(
        ops::strict_equals(actual, expected),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn symbols_construct_native_identity_descriptions_metadata_and_registry_values() {
    let mut realm = Realm::new();
    let function = global(&mut realm);
    equal(&get(&mut realm, &function, "name"), &Value::str("Symbol"));
    equal(&get(&mut realm, &function, "length"), &Value::Number(0.0));
    let prototype = get(&mut realm, &function, "prototype");
    equal(&get(&mut realm, &prototype, "constructor"), &function);
    assert!(ops::construct(&mut realm, &function, &[]).is_err());
    for name in ["prototype", "iterator", "asyncDispose", "unscopables"] {
        let descriptor = function
            .as_object()
            .unwrap()
            .own_property(&Key::name(name))
            .unwrap();
        assert!(!descriptor.enumerable && !descriptor.configurable);
        assert!(matches!(
            descriptor.slot,
            Slot::Data {
                writable: false,
                ..
            }
        ));
        assert!(ops::set(&mut realm, &function, &Key::name(name), Value::Undefined).is_err());
        assert!(ops::delete(&mut realm, &function, &Key::name(name)).is_err());
    }
    let anonymous = ops::call(&mut realm, &function, Value::Null, &[]).unwrap();
    let empty = ops::call(&mut realm, &function, Value::Null, &[Value::str("")]).unwrap();
    assert_eq!(ops::type_of(&anonymous), "symbol");
    equal(
        &get(&mut realm, &anonymous, "description"),
        &Value::Undefined,
    );
    equal(&get(&mut realm, &empty, "description"), &Value::str(""));
    let lone = Value::String(JsStr::from_units(&[0xd800]));
    let named = ops::call(
        &mut realm,
        &function,
        Value::Undefined,
        std::slice::from_ref(&lone),
    )
    .unwrap();
    let other = ops::call(
        &mut realm,
        &function,
        Value::Undefined,
        std::slice::from_ref(&lone),
    )
    .unwrap();
    assert!(!ops::strict_equals(&named, &other));
    equal(&get(&mut realm, &named, "description"), &lone);
    let stringify = get(&mut realm, &named, "toString");
    equal(
        &ops::call(&mut realm, &stringify, named.clone(), &[]).unwrap(),
        &Value::String(
            JsStr::from("Symbol(")
                .concat(&JsStr::from_units(&[0xd800]))
                .concat(&JsStr::from(")")),
        ),
    );
    assert!(
        ops::call(
            &mut realm,
            &function,
            Value::Null,
            std::slice::from_ref(&named)
        )
        .is_err()
    );
    let fail = realm.object();
    realm.method(&fail, "toString", 0, |_, _, _| {
        Err(Thrown(Value::str("description")))
    });
    equal(
        &ops::call(&mut realm, &function, Value::Null, &[Value::Object(fail)])
            .unwrap_err()
            .0,
        &Value::str("description"),
    );
    let for_symbol = get(&mut realm, &function, "for");
    let key_for = get(&mut realm, &function, "keyFor");
    for method in [&for_symbol, &key_for] {
        equal(&get(&mut realm, method, "length"), &Value::Number(1.0));
        assert!(ops::construct(&mut realm, method, &[]).is_err());
    }
    let registered = ops::call(
        &mut realm,
        &for_symbol,
        Value::Null,
        std::slice::from_ref(&lone),
    )
    .unwrap();
    equal(
        &ops::call(
            &mut realm,
            &for_symbol,
            Value::Undefined,
            std::slice::from_ref(&lone),
        )
        .unwrap(),
        &registered,
    );
    equal(
        &ops::call(
            &mut realm,
            &key_for,
            Value::Undefined,
            std::slice::from_ref(&registered),
        )
        .unwrap(),
        &lone,
    );
    equal(
        &ops::call(&mut realm, &key_for, Value::Undefined, &[named]).unwrap(),
        &Value::Undefined,
    );
    let iterator = get(&mut realm, &function, "iterator");
    equal(
        &ops::call(&mut realm, &key_for, Value::Undefined, &[iterator]).unwrap(),
        &Value::Undefined,
    );
    assert!(ops::call(&mut realm, &key_for, Value::Undefined, &[]).is_err());
    assert!(ops::call(&mut realm, &key_for, Value::Undefined, &[Value::str("key")]).is_err());
    assert!(ops::call(&mut realm, &for_symbol, Value::Undefined, &[anonymous]).is_err());
    let undefined = ops::call(&mut realm, &for_symbol, Value::Undefined, &[]).unwrap();
    equal(
        &get(&mut realm, &undefined, "description"),
        &Value::str("undefined"),
    );
    let reentrant = realm.object();
    let captured = for_symbol.clone();
    realm.method(&reentrant, "toString", 0, move |realm, _, _| {
        ops::call(realm, &captured, Value::Undefined, &[Value::str("inner")])?;
        Ok(Value::str("outer"))
    });
    let symbol = ops::call(
        &mut realm,
        &for_symbol,
        Value::Undefined,
        &[Value::Object(reentrant)],
    )
    .unwrap();
    equal(
        &ops::call(&mut realm, &key_for, Value::Undefined, &[symbol]).unwrap(),
        &Value::str("outer"),
    );
}

#[test]
fn symbol_boxes_have_intrinsic_brand_conversions_and_receiver_checks() {
    let mut realm = Realm::new();
    let function = global(&mut realm);
    let prototype = get(&mut realm, &function, "prototype");
    let primitive = ops::call(
        &mut realm,
        &function,
        Value::Undefined,
        &[Value::str("box")],
    )
    .unwrap();
    let Value::Symbol(symbol) = &primitive else {
        panic!("symbol")
    };
    let object = realm.symbol_object(symbol.clone());
    let boxed = Value::Object(object.clone());
    assert_eq!(
        format!("{:?}", object.data().kind),
        "SymbolObject(Symbol(box))"
    );
    let key = ops::to_key(&mut realm, &boxed).unwrap();
    assert_eq!(key, Key::Symbol(symbol.clone()));
    assert!(ops::to_number(&mut realm, &boxed).is_err());
    assert!(ops::to_string(&mut realm, &boxed).is_err());
    equal(&get(&mut realm, &boxed, "description"), &Value::str("box"));
    let primitive_key = Key::Symbol(realm.symbols.to_primitive.clone());
    let to_primitive = ops::get(&mut realm, &prototype, &primitive_key).unwrap();
    let value_of = get(&mut realm, &prototype, "valueOf");
    let stringify = get(&mut realm, &prototype, "toString");
    for receiver in [&primitive, &boxed] {
        equal(
            &ops::call(&mut realm, &to_primitive, receiver.clone(), &[Value::Null]).unwrap(),
            &primitive,
        );
        equal(
            &ops::call(&mut realm, &value_of, receiver.clone(), &[]).unwrap(),
            &primitive,
        );
        equal(
            &ops::call(&mut realm, &stringify, receiver.clone(), &[]).unwrap(),
            &Value::str("Symbol(box)"),
        );
    }
    let description = prototype
        .as_object()
        .unwrap()
        .own_property(&Key::name("description"))
        .unwrap();
    assert!(!description.enumerable && description.configurable);
    let Slot::Accessor {
        get: Some(getter),
        set: None,
    } = description.slot
    else {
        panic!("getter")
    };
    equal(
        &get(&mut realm, &Value::Object(getter.clone()), "name"),
        &Value::str("get description"),
    );
    equal(
        &get(&mut realm, &Value::Object(getter.clone()), "length"),
        &Value::Number(0.0),
    );
    for receiver in [
        prototype.clone(),
        Value::Null,
        Value::Object(realm.object()),
        Value::Number(0.0),
    ] {
        for method in [
            &to_primitive,
            &value_of,
            &stringify,
            &Value::Object(getter.clone()),
        ] {
            assert!(ops::call(&mut realm, method, receiver.clone(), &[]).is_err());
        }
    }
    for key in [
        &Key::Symbol(realm.symbols.to_primitive.clone()),
        &Key::Symbol(realm.symbols.to_string_tag.clone()),
    ] {
        let descriptor = prototype.as_object().unwrap().own_property(key).unwrap();
        assert!(!descriptor.enumerable && descriptor.configurable);
        assert!(matches!(
            descriptor.slot,
            Slot::Data {
                writable: false,
                ..
            }
        ));
    }
    let tag_method = {
        let prototype = Value::Object(realm.object_prototype.clone());
        get(&mut realm, &prototype, "toString")
    };
    equal(
        &ops::call(&mut realm, &tag_method, boxed.clone(), &[]).unwrap(),
        &Value::str("[object Symbol]"),
    );
    assert!(ops::instance_of(&mut realm, &boxed, &function).unwrap());
    let expected = primitive.clone();
    let tag_getter = realm.function("tag", 0, false, move |realm, this, _| {
        assert!(matches!(this, Value::Object(_)));
        equal(&super::primitive(realm, this, &[])?, &expected);
        Ok(Value::str("Boxed"))
    });
    let tag_key = Key::Symbol(realm.symbols.to_string_tag.clone());
    ops::define_accessor(
        prototype.as_object().unwrap(),
        tag_key.clone(),
        Some(tag_getter),
        None,
    );
    equal(
        &ops::call(&mut realm, &tag_method, primitive.clone(), &[]).unwrap(),
        &Value::str("[object Boxed]"),
    );
    prototype
        .as_object()
        .unwrap()
        .define(tag_key, fixed(Value::str("Symbol"), true));
    let tag_key = Key::Symbol(realm.symbols.to_string_tag.clone());
    assert!(prototype.as_object().unwrap().delete(&tag_key));
    equal(
        &ops::call(&mut realm, &tag_method, primitive.clone(), &[]).unwrap(),
        &Value::str("[object Object]"),
    );
    prototype
        .as_object()
        .unwrap()
        .define(tag_key, fixed(Value::str("Symbol"), true));
    // Removing a prototype cannot remove the native wrapper brand.
    object.data_mut().prototype = None;
    equal(
        &ops::call(&mut realm, &tag_method, boxed.clone(), &[]).unwrap(),
        &Value::str("[object Object]"),
    );
    equal(
        &ops::call(&mut realm, &value_of, boxed, &[]).unwrap(),
        &primitive,
    );
    let key_for = get(&mut realm, &function, "keyFor");
    assert!(
        ops::call(
            &mut realm,
            &key_for,
            Value::Undefined,
            &[Value::Object(object)]
        )
        .is_err()
    );
}

#[test]
fn same_agent_realms_share_protocol_symbols_and_registry_without_sharing_prototypes() {
    let mut first = Realm::new();
    let mut second = first.sibling();
    let first_symbol = global(&mut first);
    let second_symbol = global(&mut second);
    assert_ne!(first.symbol_prototype, second.symbol_prototype);
    let entries = first.symbols.entries();
    assert_eq!(entries.len(), 15);
    for (name, symbol) in entries {
        equal(
            &get(&mut second, &second_symbol, name),
            &Value::Symbol(symbol.clone()),
        );
        assert_eq!(
            symbol.descriptive(),
            JsStr::from(&format!("Symbol(Symbol.{name})")[..])
        );
    }
    let for_first = get(&mut first, &first_symbol, "for");
    let for_second = get(&mut second, &second_symbol, "for");
    let registered = ops::call(
        &mut first,
        &for_first,
        Value::Undefined,
        &[Value::str("across realms")],
    )
    .unwrap();
    equal(
        &ops::call(
            &mut second,
            &for_second,
            Value::Undefined,
            &[Value::str("across realms")],
        )
        .unwrap(),
        &registered,
    );
    let object = first.object();
    object.define(
        Key::Symbol(first.symbols.iterator.clone()),
        Property::data(Value::Number(42.0)),
    );
    let key = Key::Symbol(second.symbols.iterator.clone());
    equal(
        &ops::get(&mut second, &Value::Object(object), &key).unwrap(),
        &Value::Number(42.0),
    );
    let target =
        Value::Object(first.function("Constructor", 0, true, |_, _, _| Ok(Value::Undefined)));
    let instance = ops::construct(&mut first, &target, &[]).unwrap();
    let instance_key = Key::Symbol(second.symbols.has_instance.clone());
    let intrinsic = ops::get(&mut second, &target, &instance_key).unwrap();
    equal(
        &get(&mut second, &intrinsic, "name"),
        &Value::str("[Symbol.hasInstance]"),
    );
    equal(
        &ops::call(
            &mut second,
            &intrinsic,
            target.clone(),
            std::slice::from_ref(&instance),
        )
        .unwrap(),
        &Value::Boolean(true),
    );
    equal(
        &ops::call(
            &mut second,
            &intrinsic,
            Value::Null,
            std::slice::from_ref(&instance),
        )
        .unwrap(),
        &Value::Boolean(false),
    );
    let descriptor = first
        .function_prototype
        .own_property(&Key::Symbol(first.symbols.has_instance.clone()))
        .unwrap();
    assert!(!descriptor.enumerable && !descriptor.configurable);
    assert!(matches!(
        descriptor.slot,
        Slot::Data {
            writable: false,
            ..
        }
    ));
    assert!(ops::set(&mut second, &target, &instance_key, Value::Null).is_err());
    let callable_without_prototype = {
        let object = first.function("bare", 0, true, |_, _, _| Ok(Value::Undefined));
        object.data_mut().prototype = None;
        Value::Object(object)
    };
    let bare_instance = ops::construct(&mut first, &callable_without_prototype, &[]).unwrap();
    assert!(ops::instance_of(&mut first, &bare_instance, &callable_without_prototype).unwrap());
    let bound = ops::get(&mut first, &target, &Key::name("bind")).unwrap();
    let bound = ops::call(&mut first, &bound, target, &[]).unwrap();
    assert!(ops::instance_of(&mut first, &instance, &bound).unwrap());
}
