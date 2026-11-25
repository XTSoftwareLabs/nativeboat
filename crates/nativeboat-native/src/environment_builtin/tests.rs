use super::*;
use crate::JsStr;

fn read(realm: &mut Realm, value: &Value, name: &str) -> Value {
    ops::get(realm, value, &Key::name(name)).unwrap()
}

fn text(value: Value) -> JsStr {
    let Value::String(value) = value else {
        panic!("expected a string")
    };
    value
}

#[test]
fn environment_handles_keep_identity_and_native_object_protocols() {
    let mut realm = Realm::new();
    let env = environment(&mut realm);
    assert!(crate::same_value(&env, &environment(&mut realm)));
    let mut sibling = realm.sibling();
    assert!(!crate::same_value(&env, &environment(&mut sibling)));
    assert_eq!(
        format!("{:?}", env.as_object().unwrap().data().kind),
        "Environment"
    );
    let constructor = read(&mut realm, &env, "constructor");
    assert_eq!(
        text(read(&mut realm, &constructor, "name")),
        JsStr::from("")
    );
    assert!(matches!(
        read(&mut realm, &constructor, "length"),
        Value::Number(0.0)
    ));
    let global = ops::call(&mut realm, &constructor, Value::Undefined, &[]).unwrap();
    assert!(crate::same_value(
        &global,
        &Value::Object(realm.global.clone())
    ));
    let receiver = Value::Object(realm.object());
    assert!(crate::same_value(
        &ops::call(&mut realm, &constructor, receiver.clone(), &[]).unwrap(),
        &receiver
    ));
    assert!(matches!(
        ops::construct(&mut realm, &constructor, &[]).unwrap(),
        Value::Object(_)
    ));
    assert!(matches!(
        ops::call(&mut realm, &constructor, Value::Number(7.0), &[]).unwrap(),
        Value::Object(_)
    ));
    let symbol = Key::Symbol(crate::Symbol::new(Some("native env".into())));
    env.as_object()
        .unwrap()
        .data()
        .prototype
        .as_ref()
        .unwrap()
        .define(symbol.clone(), Property::data(Value::Number(7.0)));
    assert!(matches!(
        ops::get(&mut realm, &env, &symbol).unwrap(),
        Value::Undefined
    ));
    assert!(ops::has_property(&mut realm, &symbol, &env).unwrap());
    let inherited = Value::Object(Object::new(
        Some(env.as_object().unwrap().clone()),
        Kind::Ordinary,
    ));
    assert!(matches!(
        ops::get(&mut realm, &inherited, &symbol).unwrap(),
        Value::Undefined
    ));
    assert!(ops::has_property(&mut realm, &symbol, &inherited).unwrap());
    assert!(env.as_object().unwrap().own_property(&symbol).is_none());
    assert!(ops::set(&mut realm, &env, &symbol, Value::str("x")).is_err());
    assert!(ops::delete(&mut realm, &env, &symbol).unwrap());
    let ordinary = Value::Object(realm.object());
    ops::set(&mut realm, &ordinary, &symbol, Value::Number(9.0)).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &ordinary, &symbol).unwrap(),
        Value::Number(9.0)
    ));
}

#[test]
fn descriptors_and_integrity_operations_keep_environment_rules() {
    let mut realm = Realm::new();
    let env = environment(&mut realm);
    let name = Key::name(&format!(
        "NATIVEBOAT_ENV_DESCRIPTOR_{}",
        platform::process_id()
    ));
    let previous = platform::environment_value(&name.to_js());
    ops::define_property(
        &mut realm,
        env.as_object().unwrap(),
        &name,
        Descriptor {
            value: Some(Value::Number(42.0)),
            writable: Some(true),
            enumerable: Some(true),
            configurable: Some(true),
            ..Descriptor::default()
        },
    )
    .unwrap();
    assert_eq!(
        text(ops::get(&mut realm, &env, &name).unwrap()),
        JsStr::from("42")
    );
    let property = env.as_object().unwrap().own_property(&name).unwrap();
    assert!(env.as_object().unwrap().own_keys().contains(&name));
    assert!(
        property.enumerable
            && property.configurable
            && matches!(property.slot, crate::Slot::Data { writable: true, .. })
    );
    for descriptor in [
        Descriptor::default(),
        Descriptor {
            value: Some(Value::Number(1.0)),
            ..Descriptor::default()
        },
        Descriptor {
            get: Some(None),
            ..Descriptor::default()
        },
    ] {
        let error = ops::define_property(&mut realm, env.as_object().unwrap(), &name, descriptor)
            .unwrap_err()
            .0;
        assert_eq!(
            text(read(&mut realm, &error, "code")),
            JsStr::from("ERR_INVALID_OBJECT_DEFINE_PROPERTY")
        );
    }
    let object = realm.global.own_property(&Key::name("Object")).unwrap();
    let crate::Slot::Data { value: object, .. } = object.slot else {
        panic!("constructor")
    };
    for operation in ["freeze", "seal", "preventExtensions"] {
        let function = read(&mut realm, &object, operation);
        assert!(
            ops::call(
                &mut realm,
                &function,
                Value::Undefined,
                core::slice::from_ref(&env)
            )
            .is_err()
        );
        assert!(env.as_object().unwrap().is_extensible());
    }
    assert!(ops::delete(&mut realm, &env, &name).unwrap());
    assert!(!env.as_object().unwrap().own_keys().contains(&name));
    platform::set_environment(&name.to_js(), previous.as_ref());
}

#[test]
fn native_environment_keys_sort_indexes_without_reordering_named_keys() {
    let mut session = shimforge::Session::new();
    let query = shimforge::mock!(session, platform::environment_keys, fn() -> Vec<JsStr>);
    query
        .expect()
        .returning(|| vec!["20".into(), "3".into(), "b".into(), "a".into()]);
    assert_eq!(
        keys(),
        vec![
            Key::Index(3),
            Key::Index(20),
            Key::name("b"),
            Key::name("a")
        ]
    );
    let query = shimforge::mock!(
        session,
        platform::environment_value,
        fn(&JsStr) -> Option<JsStr>
    );
    query.expect().returning(|name| {
        assert_eq!(name, &JsStr::from("123"));
        Some("value".into())
    });
    let property = property(&Key::Index(123)).unwrap();
    assert!(property.enumerable);
}
