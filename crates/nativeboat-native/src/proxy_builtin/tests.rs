use super::*;
use crate::{Slot, Symbol};
use alloc::rc::Rc;
use core::cell::RefCell;

fn global(realm: &mut Realm) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name("Proxy"),
    )
    .unwrap()
}

#[test]
fn proxy_get_trap_receives_target_property_and_receiver() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    let target = realm.object();
    target.define(Key::name("value"), Property::data(Value::Number(2.0)));
    target.define(Key::name("other"), Property::data(Value::Number(3.0)));
    let receiver = Rc::new(RefCell::new(Value::Undefined));
    let observed_receiver = receiver.clone();
    let handler = realm.object();
    let get = realm.function("get", 3, false, move |realm, _, args| {
        *observed_receiver.borrow_mut() = args.get(2).cloned().unwrap_or_default();
        let property = args.first().cloned().unwrap_or_default();
        let key = args.get(1).cloned().unwrap_or_default();
        if matches!(&key, Value::String(key) if key.to_rust_lossy() == "value") {
            return Ok(Value::Number(7.0));
        }
        let key = match key {
            Value::Symbol(symbol) => Key::Symbol(symbol),
            Value::String(text) => Key::from_js(&text),
            _ => return Err(realm.throw(ErrorKind::Type, "Invalid Proxy property key")),
        };
        ops::get(realm, &property, &key)
    });
    handler.define(Key::name("get"), Property::data(Value::Object(get)));
    let proxy = ops::construct(
        &mut realm,
        &constructor,
        &[Value::Object(target), Value::Object(handler)],
    )
    .unwrap();
    assert!(matches!(
        ops::get(&mut realm, &proxy, &Key::name("value")).unwrap(),
        Value::Number(7.0)
    ));
    assert!(matches!(
        ops::get(&mut realm, &proxy, &Key::name("other")).unwrap(),
        Value::Number(3.0)
    ));
    assert!(ops::strict_equals(&receiver.borrow(), &proxy));
}

#[test]
fn proxy_without_get_trap_reads_from_target_and_rejects_invalid_construction() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    let handler = realm.object();
    assert!(
        ops::construct(
            &mut realm,
            &constructor,
            &[Value::Null, Value::Object(handler.clone())],
        )
        .is_err()
    );
    let target = realm.object();
    assert!(
        ops::construct(
            &mut realm,
            &constructor,
            &[Value::Object(target), Value::Null],
        )
        .is_err()
    );

    let target = realm.object();
    target.define(Key::name("present"), Property::data(Value::Boolean(true)));
    let handler = realm.object();
    let proxy = ops::construct(
        &mut realm,
        &constructor,
        &[Value::Object(target), Value::Object(handler)],
    )
    .unwrap();
    assert!(matches!(
        ops::get(&mut realm, &proxy, &Key::name("present")).unwrap(),
        Value::Boolean(true)
    ));
}

#[test]
fn proxy_validates_callability_and_passes_symbol_keys_to_the_trap() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
    let target = realm.object();
    let handler = realm.object();
    handler.define(Key::name("get"), Property::data(Value::Number(1.0)));
    let proxy = ops::construct(
        &mut realm,
        &constructor,
        &[Value::Object(target), Value::Object(handler.clone())],
    )
    .unwrap();
    assert!(ops::get(&mut realm, &proxy, &Key::name("value")).is_err());
    let trap = realm.function("get", 3, false, |_, _, args| {
        assert!(matches!(&args[1], Value::Symbol(_)));
        Ok(Value::Number(8.0))
    });
    handler.define(Key::name("get"), Property::data(Value::Object(trap)));
    let key = Key::Symbol(Symbol::new(None));
    assert!(matches!(
        ops::get(&mut realm, &proxy, &key).unwrap(),
        Value::Number(8.0)
    ));
}

#[test]
fn proxy_set_and_has_traps_receive_native_property_operations() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    let target = realm.object();
    let target_for_set = target.clone();
    let set_receiver = Rc::new(RefCell::new(Value::Undefined));
    let observed_receiver = set_receiver.clone();
    let handler = realm.object();
    let set = realm.function("set", 4, false, move |realm, _, args| {
        *observed_receiver.borrow_mut() = args.get(3).cloned().unwrap_or_default();
        let key = match args.get(1) {
            Some(Value::String(text)) => Key::from_js(text),
            _ => return Err(realm.throw(ErrorKind::Type, "invalid set key")),
        };
        ops::set(
            realm,
            &Value::Object(target_for_set.clone()),
            &key,
            args.get(2).cloned().unwrap_or_default(),
        )?;
        Ok(Value::Boolean(true))
    });
    handler.define(Key::name("set"), Property::data(Value::Object(set)));
    let has = realm.function("has", 2, false, |_, _, args| {
        Ok(Value::Boolean(matches!(
            args.get(1),
            Some(Value::String(key)) if key.to_rust_lossy() == "virtual"
        )))
    });
    handler.define(Key::name("has"), Property::data(Value::Object(has)));
    let proxy = ops::construct(
        &mut realm,
        &constructor,
        &[Value::Object(target.clone()), Value::Object(handler)],
    )
    .unwrap();

    ops::set(
        &mut realm,
        &proxy,
        &Key::name("answer"),
        Value::Number(42.0),
    )
    .unwrap();
    assert!(ops::strict_equals(&set_receiver.borrow(), &proxy));
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(target), &Key::name("answer")).unwrap(),
        Value::Number(42.0)
    ));
    assert!(ops::has_property(&mut realm, &Key::name("virtual"), &proxy).unwrap());
    assert!(!ops::has_property(&mut realm, &Key::name("missing"), &proxy).unwrap());
}

#[test]
fn proxy_rejects_false_and_non_callable_set_traps() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm);
    let target = realm.object();
    let handler = realm.object();
    let proxy = ops::construct(
        &mut realm,
        &constructor,
        &[
            Value::Object(target.clone()),
            Value::Object(handler.clone()),
        ],
    )
    .unwrap();
    ops::set(&mut realm, &proxy, &Key::name("plain"), Value::Number(1.0)).unwrap();
    assert!(matches!(
        ops::get(
            &mut realm,
            &Value::Object(target.clone()),
            &Key::name("plain")
        )
        .unwrap(),
        Value::Number(1.0)
    ));

    let false_set = realm.function("set", 4, false, |_, _, _| Ok(Value::Boolean(false)));
    handler.define(Key::name("set"), Property::data(Value::Object(false_set)));
    assert!(ops::set(&mut realm, &proxy, &Key::name("blocked"), Value::Null).is_err());

    handler.define(Key::name("set"), Property::data(Value::Number(1.0)));
    assert!(ops::set(&mut realm, &proxy, &Key::name("blocked"), Value::Null).is_err());
    handler.define(Key::name("has"), Property::data(Value::Number(1.0)));
    assert!(ops::has_property(&mut realm, &Key::name("anything"), &proxy).is_err());
}

/// A global's method, such as `Object.keys`.
fn method(realm: &mut Realm, global: &str, name: &str) -> Value {
    let global_object = Value::Object(realm.global.clone());
    let owner = ops::get(realm, &global_object, &Key::name(global)).unwrap();
    ops::get(realm, &owner, &Key::name(name)).unwrap()
}

fn invoke(realm: &mut Realm, global: &str, name: &str, args: &[Value]) -> crate::Completion {
    let function = method(realm, global, name);
    ops::call(realm, &function, Value::Undefined, args)
}

/// A proxy of `target` whose handler has the given traps.
fn proxy(realm: &mut Realm, target: &Object, traps: Vec<(&str, Object)>) -> (Value, Object) {
    let constructor = global(realm);
    let handler = realm.object();
    for (name, trap) in traps {
        handler.define(Key::name(name), Property::data(Value::Object(trap)));
    }
    let proxy = ops::construct(
        realm,
        &constructor,
        &[
            Value::Object(target.clone()),
            Value::Object(handler.clone()),
        ],
    )
    .unwrap();
    (proxy, handler)
}

fn returning(realm: &Realm, value: Value) -> Object {
    realm.function("trap", 0, false, move |_, _, _| Ok(value.clone()))
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => *number,
        other => panic!("not a number: {other:?}"),
    }
}

#[test]
fn traps_answer_the_object_internal_methods() {
    let mut realm = Realm::new();
    let target = realm.object();
    let prototype = realm.object();
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let log = |realm: &Realm, name: &'static str, result: Value| {
        let calls = calls.clone();
        realm.function(name, 0, false, move |_, _, _| {
            calls.borrow_mut().push(name.into());
            Ok(result.clone())
        })
    };
    let descriptor = realm.object();
    descriptor.define(Key::name("value"), Property::data(Value::Number(1.0)));
    descriptor.define(
        Key::name("enumerable"),
        Property::data(Value::Boolean(true)),
    );
    descriptor.define(
        Key::name("configurable"),
        Property::data(Value::Boolean(true)),
    );
    let symbol = Symbol::new(None);
    let keys = realm.array(vec![
        Value::str("0"),
        Value::str("b"),
        Value::Symbol(symbol),
    ]);
    let defined = Rc::new(RefCell::new(Value::Undefined));
    let seen = defined.clone();
    let define = realm.function("defineProperty", 3, false, move |_, _, args| {
        *seen.borrow_mut() = args[2].clone();
        Ok(Value::Boolean(true))
    });
    let traps = vec![
        (
            "getPrototypeOf",
            log(&realm, "getPrototypeOf", Value::Object(prototype.clone())),
        ),
        (
            "setPrototypeOf",
            log(&realm, "setPrototypeOf", Value::Boolean(true)),
        ),
        (
            "isExtensible",
            log(&realm, "isExtensible", Value::Boolean(true)),
        ),
        (
            "getOwnPropertyDescriptor",
            log(
                &realm,
                "getOwnPropertyDescriptor",
                Value::Object(descriptor),
            ),
        ),
        ("defineProperty", define),
        ("has", log(&realm, "has", Value::Boolean(true))),
        (
            "deleteProperty",
            log(&realm, "deleteProperty", Value::Boolean(true)),
        ),
        ("ownKeys", log(&realm, "ownKeys", Value::Object(keys))),
    ];
    let (proxy, _) = proxy(&mut realm, &target, traps);
    let found = invoke(
        &mut realm,
        "Object",
        "getPrototypeOf",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(ops::strict_equals(&found, &Value::Object(prototype)));
    invoke(
        &mut realm,
        "Object",
        "setPrototypeOf",
        &[proxy.clone(), Value::Null],
    )
    .unwrap();
    let extensible = invoke(
        &mut realm,
        "Object",
        "isExtensible",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(matches!(extensible, Value::Boolean(true)));
    let own = invoke(
        &mut realm,
        "Object",
        "getOwnPropertyDescriptor",
        &[proxy.clone(), Value::str("x")],
    )
    .unwrap();
    let value = ops::get(&mut realm, &own, &Key::name("value")).unwrap();
    assert_eq!(number(&value), 1.0);
    let writable = ops::get(&mut realm, &own, &Key::name("writable")).unwrap();
    assert!(matches!(writable, Value::Boolean(false)));
    let partial = realm.object();
    partial.define(Key::name("value"), Property::data(Value::Number(2.0)));
    partial.define(
        Key::name("enumerable"),
        Property::data(Value::Boolean(true)),
    );
    invoke(
        &mut realm,
        "Object",
        "defineProperty",
        &[proxy.clone(), Value::str("y"), Value::Object(partial)],
    )
    .unwrap();
    let received = defined.borrow().clone();
    let names = invoke(&mut realm, "Object", "keys", &[received]).unwrap();
    assert_eq!(text(&mut realm, &names), "value,enumerable");
    assert!(ops::has_property(&mut realm, &Key::name("z"), &proxy).unwrap());
    assert!(ops::delete(&mut realm, &proxy, &Key::name("z")).unwrap());
    let all = invoke(
        &mut realm,
        "Reflect",
        "ownKeys",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    let length = ops::get(&mut realm, &all, &Key::name("length")).unwrap();
    assert_eq!(number(&length), 3.0);
    // Object.keys reads each key's descriptor through the trap.
    let enumerable = invoke(&mut realm, "Object", "keys", std::slice::from_ref(&proxy)).unwrap();
    assert_eq!(text(&mut realm, &enumerable), "0,b");
    assert_eq!(
        calls.borrow().join(" "),
        "getPrototypeOf setPrototypeOf isExtensible getOwnPropertyDescriptor has deleteProperty ownKeys ownKeys getOwnPropertyDescriptor getOwnPropertyDescriptor"
    );
}

#[test]
fn trap_results_are_validated() {
    let mut realm = Realm::new();
    let target = realm.object();
    let cases: Vec<(&str, Value)> = vec![
        ("getPrototypeOf", Value::Number(1.0)),
        ("getOwnPropertyDescriptor", Value::Number(1.0)),
        ("isExtensible", Value::Boolean(false)),
        ("preventExtensions", Value::Boolean(true)),
        ("ownKeys", Value::Number(1.0)),
    ];
    for (name, result) in cases {
        let trap = returning(&realm, result);
        let (proxy, _) = proxy(&mut realm, &target, vec![(name, trap)]);
        let outcome = match name {
            "getPrototypeOf" => invoke(&mut realm, "Object", "getPrototypeOf", &[proxy]),
            "getOwnPropertyDescriptor" => invoke(
                &mut realm,
                "Reflect",
                "getOwnPropertyDescriptor",
                &[proxy, Value::str("x")],
            ),
            "isExtensible" => invoke(&mut realm, "Reflect", "isExtensible", &[proxy]),
            "preventExtensions" => invoke(&mut realm, "Reflect", "preventExtensions", &[proxy]),
            _ => invoke(&mut realm, "Reflect", "ownKeys", &[proxy]),
        };
        assert!(outcome.is_err(), "{name}");
    }
    for keys in [
        vec![Value::Number(1.0)],
        vec![Value::str("a"), Value::str("a")],
    ] {
        let keys = Value::Object(realm.array(keys));
        let trap = returning(&realm, keys);
        let (proxy, _) = proxy(&mut realm, &target, vec![("ownKeys", trap)]);
        assert!(invoke(&mut realm, "Reflect", "ownKeys", &[proxy]).is_err());
    }
    // Refusals reach the operations that report them.
    let refuse = returning(&realm, Value::Boolean(false));
    let traps = [
        "setPrototypeOf",
        "preventExtensions",
        "defineProperty",
        "deleteProperty",
        "set",
    ]
    .into_iter()
    .map(|name| (name, refuse.clone()))
    .collect();
    let (proxy, _) = proxy(&mut realm, &target, traps);
    assert!(
        invoke(
            &mut realm,
            "Object",
            "setPrototypeOf",
            &[proxy.clone(), Value::Null]
        )
        .is_err()
    );
    let reflected = invoke(
        &mut realm,
        "Reflect",
        "setPrototypeOf",
        &[proxy.clone(), Value::Null],
    )
    .unwrap();
    assert!(matches!(reflected, Value::Boolean(false)));
    assert!(
        invoke(
            &mut realm,
            "Object",
            "preventExtensions",
            std::slice::from_ref(&proxy)
        )
        .is_err()
    );
    assert!(invoke(&mut realm, "Object", "freeze", std::slice::from_ref(&proxy)).is_err());
    let empty = Value::Object(realm.object());
    assert!(
        invoke(
            &mut realm,
            "Object",
            "defineProperty",
            &[proxy.clone(), Value::str("x"), empty.clone()]
        )
        .is_err()
    );
    let reflected = invoke(
        &mut realm,
        "Reflect",
        "defineProperty",
        &[proxy.clone(), Value::str("x"), empty],
    )
    .unwrap();
    assert!(matches!(reflected, Value::Boolean(false)));
    assert!(ops::delete(&mut realm, &proxy, &Key::name("x")).is_err());
    assert!(ops::set(&mut realm, &proxy, &Key::name("x"), Value::Null).is_err());
    let reflected = invoke(
        &mut realm,
        "Reflect",
        "set",
        &[proxy.clone(), Value::str("x"), Value::Null],
    )
    .unwrap();
    assert!(matches!(reflected, Value::Boolean(false)));
}

#[test]
fn targets_answer_when_handlers_have_no_traps() {
    let mut realm = Realm::new();
    let target = realm.array(vec![Value::Number(1.0), Value::Number(2.0)]);
    let (proxy, _) = proxy(&mut realm, &target, Vec::new());
    let array = invoke(&mut realm, "Array", "isArray", std::slice::from_ref(&proxy)).unwrap();
    assert!(matches!(array, Value::Boolean(true)));
    let tag = crate::object_builtin::to_string(&mut realm, proxy.clone(), &[]).unwrap();
    assert_eq!(text(&mut realm, &tag), "[object Array]");
    let json = invoke(
        &mut realm,
        "JSON",
        "stringify",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert_eq!(text(&mut realm, &json), "[1,2]");
    let keys = invoke(&mut realm, "Object", "keys", std::slice::from_ref(&proxy)).unwrap();
    assert_eq!(text(&mut realm, &keys), "0,1");
    let keys: Vec<String> = crate::iter::for_in_keys(&mut realm, &proxy)
        .unwrap()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["0", "1"]);
    let copy = realm.object();
    crate::iter::copy_properties(&mut realm, &copy, &proxy, &[]).unwrap();
    let copied = ops::get(&mut realm, &Value::Object(copy), &Key::Index(1)).unwrap();
    assert_eq!(number(&copied), 2.0);
    let prototype = invoke(
        &mut realm,
        "Object",
        "getPrototypeOf",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(ops::strict_equals(
        &prototype,
        &Value::Object(realm.array_prototype.clone())
    ));
    ops::set(&mut realm, &proxy, &Key::Index(2), Value::Number(3.0)).unwrap();
    let length = ops::get(&mut realm, &proxy, &Key::name("length")).unwrap();
    assert_eq!(number(&length), 3.0);
    assert!(ops::has_property(&mut realm, &Key::Index(2), &proxy).unwrap());
    assert!(ops::delete(&mut realm, &proxy, &Key::Index(2)).unwrap());
    let descriptor = Value::Object(realm.object());
    invoke(
        &mut realm,
        "Object",
        "defineProperty",
        &[proxy.clone(), Value::str("extra"), descriptor],
    )
    .unwrap();
    assert!(target.own_property(&Key::name("extra")).is_some());
    let own = invoke(
        &mut realm,
        "Object",
        "getOwnPropertyDescriptors",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(matches!(own, Value::Object(_)));
    let frozen = invoke(
        &mut realm,
        "Object",
        "isFrozen",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(matches!(frozen, Value::Boolean(false)));
    invoke(&mut realm, "Object", "freeze", std::slice::from_ref(&proxy)).unwrap();
    let frozen = invoke(
        &mut realm,
        "Object",
        "isFrozen",
        std::slice::from_ref(&proxy),
    )
    .unwrap();
    assert!(matches!(frozen, Value::Boolean(true)));
    assert!(!target.is_extensible());
    let owned = invoke(
        &mut realm,
        "Object",
        "hasOwn",
        &[proxy.clone(), Value::str("0")],
    )
    .unwrap();
    assert!(matches!(owned, Value::Boolean(true)));
    invoke(
        &mut realm,
        "Object",
        "setPrototypeOf",
        &[proxy.clone(), Value::Null],
    )
    .unwrap_err();
}

#[test]
fn proxies_on_prototype_chains_answer_lookups_with_the_receiver() {
    let mut realm = Realm::new();
    let target = realm.object();
    target.define(Key::name("inherited"), Property::data(Value::Number(4.0)));
    let receivers = Rc::new(RefCell::new(Vec::<Value>::new()));
    let seen = receivers.clone();
    let get = realm.function("get", 3, false, move |realm, _, args| {
        seen.borrow_mut().push(args[2].clone());
        let key = ops::to_key(realm, &args[1])?;
        ops::get(realm, &args[0], &key)
    });
    let (proxy, _) = proxy(&mut realm, &target, vec![("get", get)]);
    let child = Object::new(proxy.as_object().cloned(), Kind::Ordinary);
    let child_value = Value::Object(child.clone());
    let value = ops::get(&mut realm, &child_value, &Key::name("inherited")).unwrap();
    assert_eq!(number(&value), 4.0);
    assert!(ops::strict_equals(&receivers.borrow()[0], &child_value));
    // Without set and has traps, lookups continue at the target.
    assert!(ops::has_property(&mut realm, &Key::name("inherited"), &child_value).unwrap());
    ops::set(
        &mut realm,
        &child_value,
        &Key::name("own"),
        Value::Number(5.0),
    )
    .unwrap();
    assert!(child.own_property(&Key::name("own")).is_some());
    let keys: Vec<String> = crate::iter::for_in_keys(&mut realm, &child_value)
        .unwrap()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["own", "inherited"]);
    let object_prototype = Value::Object(realm.object_prototype.clone());
    let is_prototype =
        ops::get(&mut realm, &object_prototype, &Key::name("isPrototypeOf")).unwrap();
    let found = ops::call(
        &mut realm,
        &is_prototype,
        Value::Object(target.clone()),
        std::slice::from_ref(&child_value),
    )
    .unwrap();
    assert!(matches!(found, Value::Boolean(false)));
    let found = ops::call(&mut realm, &is_prototype, proxy.clone(), &[child_value]).unwrap();
    assert!(matches!(found, Value::Boolean(true)));
}

#[test]
fn function_proxies_call_and_construct_through_traps() {
    let mut realm = Realm::new();
    let function = realm.function("double", 1, true, |realm, _, args| {
        let value = ops::to_number(realm, &args[0])?;
        Ok(Value::Number(value * 2.0))
    });
    let (plain, _) = proxy(&mut realm, &function, Vec::new());
    assert_eq!(ops::type_of(&plain), "function");
    let tag = crate::object_builtin::to_string(&mut realm, plain.clone(), &[]).unwrap();
    assert_eq!(text(&mut realm, &tag), "[object Function]");
    let result = ops::call(&mut realm, &plain, Value::Undefined, &[Value::Number(3.0)]).unwrap();
    assert_eq!(number(&result), 6.0);
    let made = ops::construct(&mut realm, &plain, &[Value::Number(1.0)]).unwrap();
    assert!(matches!(made, Value::Object(_)));
    let apply = realm.function("apply", 3, false, |realm, _, args| {
        let length = ops::get(realm, &args[2], &Key::name("length"))?;
        Ok(Value::Number(ops::to_number(realm, &length)? + 10.0))
    });
    let constructed = realm.object();
    let construct = returning(&realm, Value::Object(constructed.clone()));
    let (trapped, handler) = proxy(
        &mut realm,
        &function,
        vec![("apply", apply), ("construct", construct)],
    );
    let result = ops::call(
        &mut realm,
        &trapped,
        Value::Undefined,
        &[Value::Null, Value::Null],
    )
    .unwrap();
    assert_eq!(number(&result), 12.0);
    let made = ops::construct(&mut realm, &trapped, &[]).unwrap();
    assert!(ops::strict_equals(&made, &Value::Object(constructed)));
    let bad = returning(&realm, Value::Number(1.0));
    handler.define(Key::name("construct"), Property::data(Value::Object(bad)));
    assert!(ops::construct(&mut realm, &trapped, &[]).is_err());
    let callable = realm.function("callable", 0, false, |_, _, _| Ok(Value::Undefined));
    let (call_only, _) = proxy(&mut realm, &callable, Vec::new());
    assert!(ops::construct(&mut realm, &call_only, &[]).is_err());
    let debug = format!("{:?}", call_only.as_object().unwrap().data().kind);
    assert_eq!(debug, "Proxy");
    let Kind::Proxy(state) = &call_only.as_object().unwrap().data().kind else {
        panic!("a proxy");
    };
    assert_eq!(
        format!("{state:?}"),
        "Proxy(revoked: false, callable: true)"
    );
}

#[test]
fn revoked_proxies_throw_from_every_operation() {
    let mut realm = Realm::new();
    let target = realm.array(Vec::new());
    let handler = Value::Object(realm.object());
    let revocable = invoke(
        &mut realm,
        "Proxy",
        "revocable",
        &[Value::Object(target), handler],
    )
    .unwrap();
    let proxy = ops::get(&mut realm, &revocable, &Key::name("proxy")).unwrap();
    let revoke = ops::get(&mut realm, &revocable, &Key::name("revoke")).unwrap();
    assert!(ops::get(&mut realm, &proxy, &Key::Index(0)).is_ok());
    ops::call(&mut realm, &revoke, Value::Undefined, &[]).unwrap();
    let object = proxy.as_object().unwrap().clone();
    let error = ops::get(&mut realm, &proxy, &Key::name("x")).unwrap_err();
    let message = ops::get(&mut realm, &error.0, &Key::name("message")).unwrap();
    assert_eq!(
        text(&mut realm, &message),
        "Cannot perform 'get' on a proxy that has been revoked"
    );
    assert!(ops::set(&mut realm, &proxy, &Key::name("x"), Value::Null).is_err());
    assert!(ops::has_property(&mut realm, &Key::name("x"), &proxy).is_err());
    assert!(ops::delete(&mut realm, &proxy, &Key::name("x")).is_err());
    assert!(ops::own_property_keys(&mut realm, &object).is_err());
    assert!(ops::get_own_property(&mut realm, &object, &Key::name("x")).is_err());
    assert!(ops::get_prototype_of(&mut realm, &object).is_err());
    assert!(ops::set_prototype_of(&mut realm, &object, None).is_err());
    assert!(ops::is_extensible(&mut realm, &object).is_err());
    assert!(ops::prevent_extensions(&mut realm, &object).is_err());
    assert!(
        ops::try_define_property(
            &mut realm,
            &object,
            &Key::name("x"),
            crate::object::Descriptor::default()
        )
        .is_err()
    );
    assert!(invoke(&mut realm, "Array", "isArray", &[proxy]).is_err());
    let function = realm.function("f", 0, true, |_, _, _| Ok(Value::Undefined));
    let handler = Value::Object(realm.object());
    let revocable = invoke(
        &mut realm,
        "Proxy",
        "revocable",
        &[Value::Object(function), handler],
    )
    .unwrap();
    let proxy = ops::get(&mut realm, &revocable, &Key::name("proxy")).unwrap();
    let revoke = ops::get(&mut realm, &revocable, &Key::name("revoke")).unwrap();
    ops::call(&mut realm, &revoke, Value::Undefined, &[]).unwrap();
    assert!(ops::call(&mut realm, &proxy, Value::Undefined, &[]).is_err());
    assert!(ops::construct(&mut realm, &proxy, &[]).is_err());
    let constructor = global(&mut realm);
    assert!(ops::construct(&mut realm, &constructor, &[Value::Number(1.0), Value::Null]).is_err());
    assert!(
        constructor
            .as_object()
            .unwrap()
            .own_property(&Key::name("prototype"))
            .is_none()
    );
}

#[test]
fn reflect_set_and_get_honor_explicit_receivers() {
    let mut realm = Realm::new();
    let holder = realm.object();
    let fixed = |value: Value| Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: true,
        configurable: true,
    };
    holder.define(Key::name("fixed"), fixed(Value::Null));
    holder.define(
        Key::name("readonly"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let calls = Rc::new(RefCell::new(0));
    let counted = calls.clone();
    let setter = realm.function("set", 1, false, move |_, _, _| {
        *counted.borrow_mut() += 1;
        Ok(Value::Undefined)
    });
    holder.define(
        Key::name("accessor"),
        Property {
            slot: Slot::Accessor {
                get: None,
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
    let receiver = realm.object();
    receiver.define(Key::name("locked"), fixed(Value::Null));
    let set = |realm: &mut Realm, key: &str, receiver: Value| {
        ops::set_with_receiver(
            realm,
            &holder,
            &Key::name(key),
            Value::Number(1.0),
            &receiver,
        )
        .unwrap()
    };
    assert!(!set(&mut realm, "fixed", Value::Object(receiver.clone())));
    assert!(!set(
        &mut realm,
        "readonly",
        Value::Object(receiver.clone())
    ));
    assert!(set(&mut realm, "accessor", Value::Object(receiver.clone())));
    assert_eq!(*calls.borrow(), 1);
    assert!(!set(&mut realm, "fresh", Value::Number(1.0)));
    assert!(!set(&mut realm, "locked", Value::Object(receiver.clone())));
    assert!(set(&mut realm, "fresh", Value::Object(receiver.clone())));
    assert!(set(&mut realm, "fresh", Value::Object(receiver.clone())));
    let getter = realm.function("get", 0, false, |_, this, _| Ok(this));
    holder.define(
        Key::name("self"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let found = invoke(
        &mut realm,
        "Reflect",
        "get",
        &[
            Value::Object(holder.clone()),
            Value::str("self"),
            Value::Number(9.0),
        ],
    )
    .unwrap();
    assert_eq!(number(&found), 9.0);
    // The environment view cannot be made inextensible and deletes
    // variables through the environment.
    let environment = crate::environment_builtin::environment(&mut realm);
    let environment = environment.as_object().unwrap().clone();
    assert!(!ops::prevent_extensions(&mut realm, &environment).unwrap());
    assert!(
        ops::delete_property(
            &mut realm,
            &environment,
            &Key::name("NATIVEBOAT_UNSET_VARIABLE")
        )
        .unwrap()
    );
}
