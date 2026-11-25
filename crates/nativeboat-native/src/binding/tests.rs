use super::*;
use crate::ops;

fn message(realm: &mut Realm, thrown: Thrown) -> String {
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

#[test]
fn bindings_have_a_temporal_dead_zone() {
    let mut realm = Realm::new();
    let binding = new(None);
    let error = read(&realm, &binding, "x").unwrap_err();
    assert_eq!(
        message(&mut realm, error),
        "ReferenceError: Cannot access 'x' before initialization"
    );
    let error = write(&realm, &binding, "x", Value::Null).unwrap_err();
    assert!(message(&mut realm, error).starts_with("ReferenceError"));
    initialize(&binding, Value::Number(1.0));
    write(&realm, &binding, "x", Value::Number(2.0)).unwrap();
    let value = read(&realm, &binding, "x").unwrap();
    assert!(ops::strict_equals(&value, &Value::Number(2.0)));
    let var = new(Some(Value::Undefined));
    assert!(matches!(read(&realm, &var, "v").unwrap(), Value::Undefined));
}

#[test]
fn exports_are_live_and_read_only() {
    let mut realm = Realm::new();
    let exports = realm.object();
    let binding = new(None);
    export(&realm, &exports, "answer", &binding);
    let module = Value::Object(exports.clone());
    let key = Key::name("answer");
    assert!(ops::get(&mut realm, &module, &key).is_err());
    initialize(&binding, Value::Number(41.0));
    write(&realm, &binding, "answer", Value::Number(42.0)).unwrap();
    let value = ops::get(&mut realm, &module, &key).unwrap();
    assert!(ops::strict_equals(&value, &Value::Number(42.0)));
    assert!(ops::set(&mut realm, &module, &key, Value::Null).is_err());
    assert!(ops::delete(&mut realm, &module, &key).is_err());
}

fn guarded(fail: bool) -> (Option<Thrown>, f64) {
    let mut realm = Realm::new();
    let mut slot: Option<Thrown> = None;
    let mut reached = 0.0;
    'region: {
        let operation = if fail {
            Err(realm.throw(crate::ErrorKind::Error, "inside"))
        } else {
            Ok(Value::Number(1.0))
        };
        let value = crate::attempt!(operation, slot, 'region);
        reached = ops::to_number(&mut realm, &value).unwrap();
    }
    (slot, reached)
}

#[test]
fn attempt_leaves_the_region_with_the_thrown_value() {
    let (slot, reached) = guarded(false);
    assert!(slot.is_none());
    assert_eq!(reached, 1.0);
    let (slot, reached) = guarded(true);
    assert!(slot.is_some());
    assert_eq!(reached, 0.0);
}

#[test]
fn namespaces_have_no_prototype_and_reject_additions() {
    let mut realm = Realm::new();
    let first = new(Some(Value::Number(1.0)));
    let second = new(None);
    let namespace = namespace(&realm, &[("a", &first), ("b", &second)]);
    assert!(namespace.data().prototype.is_none());
    assert!(!namespace.data().extensible);
    let value = Value::Object(namespace);
    let a = ops::get(&mut realm, &value, &Key::name("a")).unwrap();
    assert!(ops::strict_equals(&a, &Value::Number(1.0)));
    assert!(ops::get(&mut realm, &value, &Key::name("b")).is_err());
    assert!(ops::set(&mut realm, &value, &Key::name("c"), Value::Null).is_err());
    assert!(matches!(
        ops::get(&mut realm, &value, &Key::name("missing")).unwrap(),
        Value::Undefined
    ));
}

#[test]
fn commonjs_namespaces_snapshot_enumerable_string_keys() {
    let mut realm = Realm::new();
    let exports = realm.object();
    exports.define(Key::name("b"), Property::data(Value::Number(2.0)));
    exports.define(Key::name("a"), Property::data(Value::Number(1.0)));
    exports.define(Key::name("hidden"), Property::hidden(Value::Number(3.0)));
    exports.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(Value::Null),
    );
    let exports = Value::Object(exports);
    let namespace = Value::Object(commonjs_namespace(&mut realm, &exports).unwrap());
    let global = Value::Object(realm.global.clone());
    let keys = ops::get(&mut realm, &global, &Key::name("Object"))
        .and_then(|object| {
            let keys = ops::get(&mut realm, &object, &Key::name("keys"))?;
            ops::call(&mut realm, &keys, object, std::slice::from_ref(&namespace))
        })
        .unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &keys).unwrap().to_rust_lossy(),
        "a,b,default"
    );
    ops::set(&mut realm, &exports, &Key::name("a"), Value::Number(9.0)).unwrap();
    let a = ops::get(&mut realm, &namespace, &Key::name("a")).unwrap();
    assert!(crate::same_value(&a, &Value::Number(1.0)));
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = ops::get(&mut realm, &namespace, &tag).unwrap();
    assert!(crate::same_value(&tag, &Value::str("Module")));
    // Primitive exports only give a default; getters run and may throw.
    let primitive = commonjs_namespace(&mut realm, &Value::Number(5.0)).unwrap();
    assert_eq!(primitive.own_keys().len(), 2);
    let failing = realm.object();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "getter"))
    });
    failing.define(
        Key::name("x"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let error = commonjs_namespace(&mut realm, &Value::Object(failing)).unwrap_err();
    assert_eq!(message(&mut realm, error), "RangeError: getter");
}
