//! Collection behavior Node cannot show deterministically: entries, targets
//! and registrations end when nothing else keeps their keys alive.

use super::*;
use crate::Completion;
use std::sync::{Arc, Mutex};

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn construct(realm: &mut Realm, name: &str, args: &[Value]) -> Value {
    let constructor = global(realm, name);
    ops::construct(realm, &constructor, args).unwrap()
}

fn call(realm: &mut Realm, this: &Value, method: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, this, &Key::name(method)).unwrap();
    ops::call(realm, &function, this.clone(), args)
}

fn object(realm: &Realm) -> Value {
    Value::Object(realm.object())
}

fn show(realm: &mut Realm, value: &Value) -> String {
    crate::inspect::inspect(realm, value, 2).unwrap()
}

fn entries(collection: &Value) -> usize {
    match &collection.as_object().unwrap().data().kind {
        Kind::WeakCollection(collection) => collection.entries.borrow().map.len(),
        _ => unreachable!("a weak collection"),
    }
}

#[test]
fn entries_end_with_their_keys_and_are_pruned() {
    let mut realm = Realm::new();
    let map = construct(&mut realm, "WeakMap", &[]);
    let key = object(&realm);
    call(&mut realm, &map, "set", &[key.clone(), Value::Number(1.0)]).unwrap();
    let weak = key.as_object().unwrap().downgrade();
    drop(key);
    assert!(weak.upgrade().is_none());
    // A later key at the same address is a different key.
    let other = object(&realm);
    let found = call(&mut realm, &map, "get", std::slice::from_ref(&other)).unwrap();
    assert!(matches!(found, Value::Undefined));
    let present = call(&mut realm, &map, "has", std::slice::from_ref(&other)).unwrap();
    assert!(matches!(present, Value::Boolean(false)));
    let removed = call(&mut realm, &map, "delete", std::slice::from_ref(&other)).unwrap();
    assert!(matches!(removed, Value::Boolean(false)));
    // Entries of collected keys are dropped as the map grows.
    for _ in 0..200 {
        let key = object(&realm);
        call(&mut realm, &map, "set", &[key, Value::Undefined]).unwrap();
    }
    assert!(entries(&map) < 200, "{}", entries(&map));
    // Symbol keys end with their symbols too.
    let set = construct(&mut realm, "WeakSet", &[]);
    let symbol = Value::Symbol(crate::Symbol::new(None));
    call(&mut realm, &set, "add", std::slice::from_ref(&symbol)).unwrap();
    let weak_symbol = match &symbol {
        Value::Symbol(symbol) => symbol.downgrade(),
        _ => unreachable!(),
    };
    drop(symbol);
    assert!(weak_symbol.upgrade().is_none());
    let key = match &set.as_object().unwrap().data().kind {
        Kind::WeakCollection(collection) => collection
            .entries
            .borrow()
            .map
            .values()
            .next()
            .unwrap()
            .0
            .clone(),
        _ => unreachable!(),
    };
    assert!(!key.alive() && key.upgrade().is_none());
}

#[test]
fn weak_ref_targets_live_until_the_job_ends() {
    let mut realm = Realm::new();
    let target = object(&realm);
    let weak = target.as_object().unwrap().downgrade();
    let reference = construct(&mut realm, "WeakRef", std::slice::from_ref(&target));
    drop(target);
    // Kept during the job that created the reference.
    let kept = call(&mut realm, &reference, "deref", &[]).unwrap();
    assert!(matches!(kept, Value::Object(_)));
    drop(kept);
    realm.run_jobs().unwrap();
    assert!(weak.upgrade().is_none());
    let gone = call(&mut realm, &reference, "deref", &[]).unwrap();
    assert!(matches!(gone, Value::Undefined));
    assert_eq!(show(&mut realm, &reference), "WeakRef {}");
}

#[test]
fn registries_run_cleanup_for_collected_targets() {
    let mut realm = Realm::new();
    let log = Arc::new(Mutex::new(Vec::new()));
    let record = log.clone();
    let cleanup = realm.function("cleanup", 1, false, move |realm, this, args| {
        let held = ops::to_string(realm, &args[0])?.to_rust_lossy();
        record
            .lock()
            .unwrap()
            .push(format!("{held} {}", matches!(this, Value::Undefined)));
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    let (kept, collected, token) = (object(&realm), object(&realm), object(&realm));
    for (target, held) in [(&kept, "kept"), (&collected, "collected")] {
        let args = [target.clone(), Value::str(held), token.clone()];
        call(&mut realm, &registry, "register", &args).unwrap();
    }
    drop(collected);
    realm.finalize().unwrap();
    assert_eq!(*log.lock().unwrap(), ["collected true"]);
    // A collected token unregisters nothing.
    let weak_token = token.as_object().unwrap().downgrade();
    drop(token);
    assert!(weak_token.upgrade().is_none());
    let other = object(&realm);
    let removed = call(&mut realm, &registry, "unregister", &[other]).unwrap();
    assert!(matches!(removed, Value::Boolean(false)));
    drop(kept);
    // Cleanup errors are uncaught exceptions from the event loop.
    let throwing = realm.function("cleanup", 1, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "cleanup"))
    });
    let failing = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(throwing)],
    );
    let target = object(&realm);
    call(
        &mut realm,
        &failing,
        "register",
        &[target.clone(), Value::Null],
    )
    .unwrap();
    drop(target);
    let thrown = realm.run_event_loop().unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "[RangeError: cleanup]");
    assert_eq!(*log.lock().unwrap(), ["collected true", "kept true"]);
    // Registries nothing references are forgotten.
    drop((registry, failing));
    realm.finalize().unwrap();
    assert!(realm.registries.is_empty());
}

#[test]
fn receivers_are_described_as_v8_does() {
    let mut realm = Realm::new();
    let null = Value::Object(Object::new(None, Kind::Ordinary));
    let unnamed = realm.object();
    let anonymous = realm.function("", 0, false, |_, _, _| Ok(Value::Undefined));
    unnamed.define(
        Key::name("constructor"),
        Property::data(Value::Object(anonymous)),
    );
    let numbered = realm.object();
    numbered.define(Key::name("constructor"), Property::data(Value::Number(5.0)));
    let array = Value::Object(realm.array(Vec::new()));
    let function = global(&mut realm, "WeakMap");
    for (value, expected) in [
        (Value::Number(1.0), "1"),
        (
            Value::Symbol(crate::Symbol::new(Some(crate::JsStr::from("s")))),
            "Symbol(s)",
        ),
        (
            Value::BigInt(alloc::rc::Rc::new(crate::bigint::BigInt::from_i64(2))),
            "2",
        ),
        (array, "[object Array]"),
        (null, "[object Object]"),
        (Value::Object(unnamed), "[object Object]"),
        (Value::Object(numbered), "[object Object]"),
        (object(&realm), "#<Object>"),
    ] {
        assert_eq!(receiver_text(&value), expected);
    }
    assert_eq!(
        receiver_text(&function),
        "function WeakMap() { [native code] }"
    );
    let unnamed = Value::Object(realm.function("", 0, false, |_, _, _| Ok(Value::Undefined)));
    unnamed
        .as_object()
        .unwrap()
        .define(Key::name("name"), Property::data(Value::Number(1.0)));
    assert_eq!(receiver_text(&unnamed), "function () { [native code] }");
    for (value, expected) in [
        (Value::str("a"), "a"),
        (Value::Boolean(true), "true"),
        (Value::Null, "null"),
        (Value::Undefined, "undefined"),
    ] {
        assert_eq!(receiver_text(&value), expected);
    }
}

#[test]
fn weak_collections_are_inspected_and_typed_like_node() {
    let mut realm = Realm::new();
    let key = object(&realm);
    let pair = Value::Object(realm.array(vec![key.clone(), Value::Number(1.0)]));
    let source = Value::Object(realm.array(vec![pair]));
    let map = construct(&mut realm, "WeakMap", &[source]);
    let set = construct(&mut realm, "WeakSet", &[]);
    assert_eq!(show(&mut realm, &map), "WeakMap { <items unknown> }");
    ops::set(&mut realm, &set, &Key::name("extra"), Value::Number(2.0)).unwrap();
    assert_eq!(
        show(&mut realm, &set),
        "WeakSet { <items unknown>, extra: 2 }"
    );
    let types = crate::modules::load(&mut realm, "binding/types").unwrap();
    for (name, value, expected) in [
        ("isWeakMap", &map, true),
        ("isWeakMap", &set, false),
        ("isWeakSet", &set, true),
    ] {
        let result = call(&mut realm, &types, name, std::slice::from_ref(value)).unwrap();
        assert!(
            matches!(result, Value::Boolean(found) if found == expected),
            "{name}"
        );
    }
    drop(key);
}

#[test]
fn cleanup_can_unregister_other_collected_cells() {
    let mut realm = Realm::new();
    let owner = Rc::new(RefCell::new(None::<Value>));
    let callback_owner = owner.clone();
    let unregister = object(&realm);
    let callback_token = unregister.clone();
    let log = Rc::new(RefCell::new(Vec::new()));
    let record = log.clone();
    let cleanup = realm.function("cleanup", 1, false, move |realm, _, args| {
        record.borrow_mut().push(args[0].clone());
        let registry = callback_owner.borrow().as_ref().unwrap().clone();
        let removed = call(
            realm,
            &registry,
            "unregister",
            std::slice::from_ref(&callback_token),
        )?;
        assert!(matches!(removed, Value::Boolean(true)));
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    *owner.borrow_mut() = Some(registry.clone());
    for held in ["first", "second"] {
        let target = object(&realm);
        call(
            &mut realm,
            &registry,
            "register",
            &[target, Value::str(held), unregister.clone()],
        )
        .unwrap();
    }
    realm.finalize().unwrap();
    assert_eq!(log.borrow().len(), 1);
    owner.borrow_mut().take();
}

#[test]
fn cleanup_failure_preserves_unprocessed_cells() {
    let mut realm = Realm::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let record = log.clone();
    let cleanup = realm.function("cleanup", 1, false, move |realm, _, args| {
        let mut log = record.borrow_mut();
        log.push(args[0].clone());
        if log.len() == 1 {
            return Err(realm.throw(ErrorKind::Range, "first cleanup"));
        }
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    for held in ["first", "second"] {
        let target = object(&realm);
        call(
            &mut realm,
            &registry,
            "register",
            &[target, Value::str(held)],
        )
        .unwrap();
    }
    assert!(realm.finalize().is_err());
    realm.finalize().unwrap();
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn one_registry_cleanup_job_drains_microtasks_after_its_callbacks() {
    let mut realm = Realm::new();
    let log = Arc::new(Mutex::new(Vec::new()));
    let record = log.clone();
    let cleanup = realm.function("cleanup", 1, false, move |realm, _, _| {
        record.lock().unwrap().push("cleanup");
        let later = record.clone();
        realm.enqueue(Box::new(move |_| {
            later.lock().unwrap().push("job");
            Ok(())
        }));
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    for _ in 0..2 {
        let target = object(&realm);
        call(&mut realm, &registry, "register", &[target, Value::Null]).unwrap();
    }
    realm.finalize().unwrap();
    assert_eq!(*log.lock().unwrap(), ["cleanup", "cleanup", "job", "job"]);
}

fn error_message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    let message = ops::get(realm, &thrown.0, &Key::name("message")).unwrap();
    ops::to_string(realm, &message).unwrap().to_rust_lossy()
}

#[test]
fn constructors_and_methods_validate_targets_tokens_and_receiver_brands() {
    let mut realm = Realm::new();
    let map = construct(&mut realm, "WeakMap", &[]);
    let set = construct(&mut realm, "WeakSet", &[]);
    let target = object(&realm);
    let reference = construct(&mut realm, "WeakRef", std::slice::from_ref(&target));
    let cleanup =
        Value::Object(realm.function("cleanup", 1, false, |_, _, _| Ok(Value::Undefined)));
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        std::slice::from_ref(&cleanup),
    );
    for name in ["WeakMap", "WeakSet", "WeakRef", "FinalizationRegistry"] {
        let constructor = global(&mut realm, name);
        let result = ops::call(&mut realm, &constructor, Value::Undefined, &[]);
        assert_eq!(
            error_message(&mut realm, result),
            format!("Constructor {name} requires 'new'")
        );
    }
    for (owner, name) in [
        (&map, "get"),
        (&map, "set"),
        (&map, "has"),
        (&map, "delete"),
        (&set, "add"),
        (&set, "has"),
        (&set, "delete"),
        (&reference, "deref"),
        (&registry, "register"),
        (&registry, "unregister"),
    ] {
        let function = ops::get(&mut realm, owner, &Key::name(name)).unwrap();
        for receiver in [Value::Null, object(&realm)] {
            let result = ops::call(&mut realm, &function, receiver, &[]);
            assert!(error_message(&mut realm, result).contains("incompatible receiver"));
        }
    }
    let has = ops::get(&mut realm, &set, &Key::name("has")).unwrap();
    let result = ops::call(&mut realm, &has, map.clone(), &[]);
    assert!(error_message(&mut realm, result).contains("WeakSet.prototype.has"));
    let symbol_constructor = global(&mut realm, "Symbol");
    let registered = call(
        &mut realm,
        &symbol_constructor,
        "for",
        &[Value::str("registered")],
    )
    .unwrap();
    for key in [
        Value::Undefined,
        Value::Null,
        Value::Number(1.0),
        registered,
    ] {
        assert!(matches!(
            call(&mut realm, &map, "get", std::slice::from_ref(&key)).unwrap(),
            Value::Undefined
        ));
        for collection in [&map, &set] {
            for name in ["has", "delete"] {
                assert!(matches!(
                    call(&mut realm, collection, name, std::slice::from_ref(&key)).unwrap(),
                    Value::Boolean(false)
                ));
            }
        }
        let result = call(&mut realm, &map, "set", &[key.clone(), Value::Null]);
        assert_eq!(
            error_message(&mut realm, result),
            "Invalid value used as weak map key"
        );
        let result = call(&mut realm, &set, "add", std::slice::from_ref(&key));
        assert_eq!(
            error_message(&mut realm, result),
            "Invalid value used in weak set"
        );
        let constructor = global(&mut realm, "WeakRef");
        let result = ops::construct(&mut realm, &constructor, std::slice::from_ref(&key));
        assert_eq!(error_message(&mut realm, result), "WeakRef: invalid target");
        let result = call(
            &mut realm,
            &registry,
            "register",
            &[key.clone(), Value::Null],
        );
        assert!(error_message(&mut realm, result).contains("invalid target"));
        let result = call(
            &mut realm,
            &registry,
            "unregister",
            std::slice::from_ref(&key),
        );
        assert!(error_message(&mut realm, result).contains("Invalid unregisterToken"));
    }
    let result = call(
        &mut realm,
        &registry,
        "register",
        &[target.clone(), target.clone()],
    );
    assert!(error_message(&mut realm, result).contains("target and holdings must not be same"));
    let result = call(
        &mut realm,
        &registry,
        "register",
        &[target.clone(), Value::Null, Value::Null],
    );
    assert!(error_message(&mut realm, result).contains("Invalid unregisterToken"));
    let constructor = global(&mut realm, "FinalizationRegistry");
    let result = ops::construct(&mut realm, &constructor, &[Value::Number(1.0)]);
    assert_eq!(
        error_message(&mut realm, result),
        "FinalizationRegistry: cleanup must be callable"
    );
    call(&mut realm, &map, "set", &[target.clone(), Value::Undefined]).unwrap();
    assert!(matches!(
        call(&mut realm, &map, "delete", &[target]).unwrap(),
        Value::Boolean(true)
    ));
    for (value, expected) in [
        (&map, "WeakCollection(false)"),
        (&set, "WeakCollection(true)"),
        (&reference, "WeakRef"),
        (&registry, "FinalizationRegistry"),
    ] {
        assert_eq!(
            format!("{:?}", value.as_object().unwrap().data().kind),
            expected
        );
    }
}

#[test]
fn subclass_prototypes_and_iterable_adders_follow_constructor_order() {
    let mut realm = Realm::new();
    let map = global(&mut realm, "WeakMap");
    let intrinsic = ops::get(&mut realm, &map, &Key::name("prototype")).unwrap();
    let target = realm.function("Sub", 0, true, |_, _, _| Ok(Value::Undefined));
    target.define(Key::name("prototype"), Property::data(Value::Number(1.0)));
    let created =
        ops::construct_target(&mut realm, &map, &[], &Value::Object(target.clone())).unwrap();
    assert_eq!(
        created.as_object().unwrap().data().prototype.as_ref(),
        intrinsic.as_object()
    );
    let prototype = realm.object();
    prototype.define(Key::name("set"), Property::data(Value::Number(5.0)));
    target.define(
        Key::name("prototype"),
        Property::data(Value::Object(prototype.clone())),
    );
    let empty = Value::Object(realm.array(Vec::new()));
    let result = ops::construct_target(&mut realm, &map, &[empty], &Value::Object(target.clone()));
    assert!(error_message(&mut realm, result).contains("is not a function"));
    let created =
        ops::construct_target(&mut realm, &map, &[], &Value::Object(target.clone())).unwrap();
    assert_eq!(
        created.as_object().unwrap().data().prototype.as_ref(),
        Some(&prototype)
    );
    let getter = realm.function("get prototype", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "prototype getter"))
    });
    let bound_target = call(
        &mut realm,
        &Value::Object(target),
        "bind",
        &[Value::Undefined],
    )
    .unwrap()
    .as_object()
    .unwrap()
    .clone();
    bound_target.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let result = ops::construct_target(&mut realm, &map, &[], &Value::Object(bound_target));
    assert_eq!(error_message(&mut realm, result), "prototype getter");
    let key = object(&realm);
    let source = Value::Object(realm.array(vec![key.clone()]));
    let set = construct(&mut realm, "WeakSet", &[source]);
    assert!(matches!(
        call(&mut realm, &set, "has", &[key]).unwrap(),
        Value::Boolean(true)
    ));
    for (name, source, expected) in [
        ("WeakMap", Value::Number(1.0), "is not an entry object"),
        (
            "WeakMap",
            Value::Object(realm.array(vec![Value::Number(1.0), Value::Null])),
            "weak map key",
        ),
        ("WeakSet", Value::Number(1.0), "weak set"),
    ] {
        let source = Value::Object(realm.array(vec![source]));
        let constructor = global(&mut realm, name);
        let result = ops::construct(&mut realm, &constructor, &[source]);
        assert!(error_message(&mut realm, result).contains(expected));
    }
}

#[test]
fn cleanup_registrations_created_by_a_callback_wait_for_a_later_job() {
    let mut realm = Realm::new();
    let owner = Rc::new(RefCell::new(None::<Value>));
    let callback_owner = owner.clone();
    let count = Arc::new(Mutex::new(0));
    let callbacks = count.clone();
    let cleanup = realm.function("cleanup", 1, false, move |realm, _, _| {
        let mut count = callbacks.lock().unwrap();
        *count += 1;
        if *count == 1 {
            let registry = callback_owner.borrow().as_ref().unwrap().clone();
            let target = object(realm);
            call(realm, &registry, "register", &[target, Value::Null])?;
        }
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    *owner.borrow_mut() = Some(registry.clone());
    let target = object(&realm);
    call(&mut realm, &registry, "register", &[target, Value::Null]).unwrap();
    realm.finalize().unwrap();
    assert_eq!(*count.lock().unwrap(), 1);
    realm.finalize().unwrap();
    assert_eq!(*count.lock().unwrap(), 2);
    owner.borrow_mut().take();
}

#[test]
fn symbol_references_keep_targets_until_the_checkpoint_and_cleanup_job_errors_propagate() {
    let mut realm = Realm::new();
    let symbol = Value::Symbol(crate::Symbol::new(Some(crate::JsStr::from("target"))));
    let reference = construct(&mut realm, "WeakRef", std::slice::from_ref(&symbol));
    drop(symbol);
    let value = call(&mut realm, &reference, "deref", &[]).unwrap();
    assert!(matches!(value, Value::Symbol(_)));
    drop(value);
    realm.run_jobs().unwrap();
    assert!(matches!(
        call(&mut realm, &reference, "deref", &[]).unwrap(),
        Value::Undefined
    ));
    let cleanup = realm.function("cleanup", 1, false, |realm, _, _| {
        realm.enqueue_tick(Box::new(|realm| {
            Err(realm.throw(ErrorKind::Range, "cleanup next tick"))
        }));
        Ok(Value::Undefined)
    });
    let registry = construct(
        &mut realm,
        "FinalizationRegistry",
        &[Value::Object(cleanup)],
    );
    let target = object(&realm);
    call(&mut realm, &registry, "register", &[target, Value::Null]).unwrap();
    let result = realm.finalize().map(|_| Value::Undefined);
    assert_eq!(error_message(&mut realm, result), "cleanup next tick");
}
