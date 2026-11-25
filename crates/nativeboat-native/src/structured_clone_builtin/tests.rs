use super::*;
use crate::{Property, Slot, Symbol};

#[test]
fn structured_clone_copies_plain_graphs_between_realms() {
    let mut source_realm = Realm::new();
    let mut target_realm = Realm::new();
    let shared = source_realm.object();
    shared.define(Key::name("value"), Property::data(Value::Number(19.0)));
    let source = source_realm.object();
    source.define(
        Key::name("first"),
        Property::data(Value::Object(shared.clone())),
    );
    source.define(Key::name("second"), Property::data(Value::Object(shared)));
    source.define(
        Key::name("self"),
        Property::data(Value::Object(source.clone())),
    );
    source.define(
        Key::name("items"),
        Property::data(Value::Object(source_realm.array(vec![Value::Number(3.0)]))),
    );

    let cloned = clone_between(&mut source_realm, &mut target_realm, &Value::Object(source))
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    let first = ops::get(
        &mut target_realm,
        &Value::Object(cloned.clone()),
        &Key::name("first"),
    )
    .unwrap();
    let second = ops::get(
        &mut target_realm,
        &Value::Object(cloned.clone()),
        &Key::name("second"),
    )
    .unwrap();
    assert!(ops::strict_equals(&first, &second));
    assert!(matches!(
        ops::get(&mut target_realm, &first, &Key::name("value")).unwrap(),
        Value::Number(19.0)
    ));
    assert!(ops::strict_equals(
        &ops::get(
            &mut target_realm,
            &Value::Object(cloned.clone()),
            &Key::name("self")
        )
        .unwrap(),
        &Value::Object(cloned)
    ));
}

fn global(realm: &mut Realm) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name("structuredClone"),
    )
    .unwrap()
}

#[test]
fn structured_clone_copies_nested_arrays_shared_references_and_cycles() {
    let mut realm = Realm::new();
    let clone = global(&mut realm);

    let shared = realm.object();
    shared.define(Key::name("value"), Property::data(Value::Number(7.0)));
    let source = realm.object();
    source.define(
        Key::name("left"),
        Property::data(Value::Object(shared.clone())),
    );
    source.define(
        Key::name("right"),
        Property::data(Value::Object(shared.clone())),
    );
    source.define(
        Key::name("self"),
        Property::data(Value::Object(source.clone())),
    );
    source.define(
        Key::name("hidden"),
        Property {
            slot: Slot::Data {
                value: Value::Number(8.0),
                writable: true,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let symbol = Symbol::new(Some(crate::JsStr::from("hidden-symbol")));
    source.define(
        Key::Symbol(symbol.clone()),
        Property::data(Value::Number(9.0)),
    );
    let array = realm.array(vec![Value::Number(1.0), Value::Object(source.clone())]);
    let result = ops::call(
        &mut realm,
        &clone,
        Value::Undefined,
        &[Value::Object(array.clone())],
    )
    .unwrap();

    let Value::Object(result) = result else {
        panic!("array clone should be an object")
    };
    assert_ne!(result, array);
    let Value::Object(cloned_source) =
        ops::get(&mut realm, &Value::Object(result), &Key::Index(1)).unwrap()
    else {
        panic!("nested source should be an object")
    };
    assert_ne!(cloned_source, source);
    assert!(cloned_source.own_property(&Key::name("hidden")).is_none());
    assert!(cloned_source.own_property(&Key::Symbol(symbol)).is_none());
    assert!(ops::strict_equals(
        &ops::get(
            &mut realm,
            &Value::Object(cloned_source.clone()),
            &Key::name("self")
        )
        .unwrap(),
        &Value::Object(cloned_source.clone())
    ));
    let left = ops::get(
        &mut realm,
        &Value::Object(cloned_source.clone()),
        &Key::name("left"),
    )
    .unwrap();
    let right = ops::get(
        &mut realm,
        &Value::Object(cloned_source),
        &Key::name("right"),
    )
    .unwrap();
    assert!(ops::strict_equals(&left, &right));
    assert!(!ops::strict_equals(&left, &Value::Object(shared)));
}

#[test]
fn structured_clone_rejects_symbols_unsupported_objects_and_transfer_options() {
    let mut realm = Realm::new();
    let clone = global(&mut realm);
    for value in [
        Value::Symbol(Symbol::new(Some(crate::JsStr::from("not-cloneable")))),
        Value::Object(realm.function("callback", 0, false, |_, _, _| Ok(Value::Undefined))),
        Value::Object(Object::new(None, Kind::Environment)),
    ] {
        assert!(ops::call(&mut realm, &clone, Value::Undefined, &[value]).is_err());
    }
    let transfer_options = Value::Object(realm.object());
    assert!(
        ops::call(
            &mut realm,
            &clone,
            Value::Undefined,
            &[Value::Null, transfer_options],
        )
        .is_err()
    );
}

#[test]
fn structured_clone_getters_can_mutate_their_source_without_borrow_panics() {
    let mut realm = Realm::new();
    let clone = global(&mut realm);
    for source in [realm.object(), realm.array(vec![])] {
        let changed = source.clone();
        let getter = realm.function("get value", 0, false, move |_, _, _| {
            changed.define(Key::name("added"), Property::data(Value::Number(8.0)));
            Ok(Value::Number(7.0))
        });
        source.define(
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
        let result = ops::call(
            &mut realm,
            &clone,
            Value::Undefined,
            &[Value::Object(source.clone())],
        )
        .unwrap();
        assert!(source.own_property(&Key::name("added")).is_some());
        let result = result.as_object().unwrap();
        assert!(result.own_property(&Key::name("added")).is_none());
        assert!(matches!(
            ops::get(
                &mut realm,
                &Value::Object(result.clone()),
                &Key::name("value")
            )
            .unwrap(),
            Value::Number(7.0)
        ));
    }
}

#[test]
fn structured_clone_copies_dates_patterns_collections_errors_and_buffers() {
    let mut realm = Realm::new();
    let clone = global(&mut realm);
    let date = Value::Object(Object::new(
        Some(realm.date_prototype.clone()),
        Kind::Date(5.0),
    ));
    let pattern = crate::regexp_builtin::literal(&mut realm, &[97, 40, 98, 41], "gi").unwrap();
    let cloned = |realm: &mut Realm, value: &Value| {
        ops::call(
            realm,
            &clone,
            Value::Undefined,
            core::slice::from_ref(value),
        )
        .unwrap()
    };
    let date_clone = cloned(&mut realm, &date);
    assert!(!ops::strict_equals(&date, &date_clone));
    assert!(matches!(date_clone.as_object().unwrap().data().kind, Kind::Date(time) if time == 5.0));
    let pattern_clone = cloned(&mut realm, &pattern);
    let source_text = ops::get(&mut realm, &pattern_clone, &Key::name("source")).unwrap();
    let flags = ops::get(&mut realm, &pattern_clone, &Key::name("flags")).unwrap();
    assert_eq!(text(&mut realm, &source_text), "a(b)");
    assert_eq!(text(&mut realm, &flags), "gi");
    // Collections, errors and buffers are built with the realm's own
    // constructors.
    let global_object = Value::Object(realm.global_object());
    let construct = |realm: &mut Realm, name: &str, args: &[Value]| {
        let constructor = ops::get(realm, &global_object, &Key::name(name)).unwrap();
        ops::construct(realm, &constructor, args).unwrap()
    };
    let map = construct(&mut realm, "Map", &[]);
    let set = ops::get(&mut realm, &map, &Key::name("set")).unwrap();
    let inner = Value::Object(realm.object());
    ops::call(
        &mut realm,
        &set,
        map.clone(),
        &[Value::Number(1.0), inner.clone()],
    )
    .unwrap();
    ops::call(&mut realm, &set, map.clone(), &[Value::Number(2.0), inner]).unwrap();
    let map_clone = cloned(&mut realm, &map);
    let get = ops::get(&mut realm, &map_clone, &Key::name("get")).unwrap();
    let first = ops::call(&mut realm, &get, map_clone.clone(), &[Value::Number(1.0)]).unwrap();
    let second = ops::call(&mut realm, &get, map_clone, &[Value::Number(2.0)]).unwrap();
    assert!(first.as_object().is_some() && ops::strict_equals(&first, &second));
    let error = construct(&mut realm, "RangeError", &[Value::str("bad")]);
    let error_clone = cloned(&mut realm, &error);
    let name = ops::get(&mut realm, &error_clone, &Key::name("name")).unwrap();
    let message = ops::get(&mut realm, &error_clone, &Key::name("message")).unwrap();
    assert_eq!(
        (text(&mut realm, &name), text(&mut realm, &message)),
        ("RangeError".to_owned(), "bad".to_owned())
    );
    let numbers = construct(&mut realm, "Uint16Array", &[Value::Number(2.0)]);
    ops::set(&mut realm, &numbers, &Key::Index(1), Value::Number(513.0)).unwrap();
    let numbers_clone = cloned(&mut realm, &numbers);
    let element = ops::get(&mut realm, &numbers_clone, &Key::Index(1)).unwrap();
    assert!(matches!(element, Value::Number(value) if value == 513.0));
    ops::set(&mut realm, &numbers, &Key::Index(1), Value::Number(1.0)).unwrap();
    let element = ops::get(&mut realm, &numbers_clone, &Key::Index(1)).unwrap();
    assert!(matches!(element, Value::Number(value) if value == 513.0));
    let wrapper = Value::Object(ops::to_object(&mut realm, &Value::Number(3.0)).unwrap());
    let wrapper_clone = cloned(&mut realm, &wrapper);
    assert!(matches!(
        wrapper_clone.as_object().unwrap().data().kind,
        Kind::Primitive(crate::object::Wrapped::Number(value)) if value == 3.0
    ));
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

#[test]
fn nested_arrays_keep_their_own_values_between_realms() {
    let mut source_realm = Realm::new();
    let mut target_realm = Realm::new();
    let pair = |realm: &Realm, a: f64, b: f64| {
        Value::Object(realm.array(vec![Value::Number(a), Value::Number(b)]))
    };
    let range = Value::Object(source_realm.array(vec![
        pair(&source_realm, 0.0, 2.0),
        pair(&source_realm, 0.0, 6.0),
    ]));
    let cloned = clone_between(&mut source_realm, &mut target_realm, &range).unwrap();
    let end = ops::get(&mut target_realm, &cloned, &Key::Index(1)).unwrap();
    let column = ops::get(&mut target_realm, &end, &Key::Index(1)).unwrap();
    assert!(matches!(column, Value::Number(6.0)), "{column:?}");
}
