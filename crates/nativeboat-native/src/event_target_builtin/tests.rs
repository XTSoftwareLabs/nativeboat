use super::*;
use alloc::rc::Rc;
use core::cell::RefCell;

fn global(realm: &mut Realm, name: &str) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name(name),
    )
    .unwrap()
}

fn call(realm: &mut Realm, receiver: &Value, name: &str, args: &[Value]) -> Value {
    let method = ops::get(realm, receiver, &Key::name(name)).unwrap();
    ops::call(realm, &method, receiver.clone(), args).unwrap()
}

fn construct(realm: &mut Realm, constructor: &Value, args: &[Value]) -> Value {
    ops::construct(realm, constructor, args).unwrap()
}

#[test]
fn event_target_adds_deduplicates_removes_and_dispatches_events() {
    let mut realm = Realm::new();
    let event_target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &event_target_constructor, &[]);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let observed = calls.clone();
    let listener = realm.function("listener", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        let event_type = ops::get(realm, &event, &Key::name("type"))?;
        observed
            .borrow_mut()
            .push(ops::to_string(realm, &event_type)?.to_rust_lossy());
        Ok(Value::Undefined)
    });
    for _ in 0..2 {
        call(
            &mut realm,
            &target,
            "addEventListener",
            &[Value::str("change"), Value::Object(listener.clone())],
        );
    }
    let event = construct(&mut realm, &event_constructor, &[Value::str("change")]);
    assert!(matches!(
        call(
            &mut realm,
            &target,
            "dispatchEvent",
            core::slice::from_ref(&event),
        ),
        Value::Boolean(true)
    ));
    assert_eq!(*calls.borrow(), ["change"]);
    call(
        &mut realm,
        &target,
        "removeEventListener",
        &[Value::str("change"), Value::Object(listener)],
    );
    call(&mut realm, &target, "dispatchEvent", &[event]);
    assert_eq!(*calls.borrow(), ["change"]);
}

#[test]
fn event_once_prevent_default_and_immediate_stop_follow_target_dispatch() {
    let mut realm = Realm::new();
    let event_target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &event_target_constructor, &[]);
    let options = realm.object();
    options.define(Key::name("once"), Property::data(Value::Boolean(true)));
    options.define(Key::name("capture"), Property::data(Value::Boolean(true)));
    let calls = Rc::new(RefCell::new(0));
    let observed = calls.clone();
    let listener = realm.function("once", 1, false, move |realm, _, args| {
        *observed.borrow_mut() += 1;
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        call(realm, &event, "preventDefault", &[]);
        Ok(Value::Undefined)
    });
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[
            Value::str("change"),
            Value::Object(listener),
            Value::Object(options),
        ],
    );
    let event_options = realm.object();
    event_options.define(
        Key::name("cancelable"),
        Property::data(Value::Boolean(true)),
    );
    for index in 0..2 {
        let event = construct(
            &mut realm,
            &event_constructor,
            &[Value::str("change"), Value::Object(event_options.clone())],
        );
        let result = call(&mut realm, &target, "dispatchEvent", &[event]);
        if index == 0 {
            assert!(matches!(result, Value::Boolean(false)));
        } else {
            assert!(matches!(result, Value::Boolean(true)));
        }
    }
    assert_eq!(*calls.borrow(), 1);

    let immediate_target = construct(&mut realm, &event_target_constructor, &[]);
    let stopped = Rc::new(RefCell::new(0));
    let first_count = stopped.clone();
    let first = realm.function("first", 1, false, move |realm, _, args| {
        *first_count.borrow_mut() += 1;
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        call(realm, &event, "stopImmediatePropagation", &[]);
        Ok(Value::Undefined)
    });
    let next_count = stopped.clone();
    let next = realm.function("next", 1, false, move |_, _, _| {
        *next_count.borrow_mut() += 10;
        Ok(Value::Undefined)
    });
    for listener in [first, next] {
        call(
            &mut realm,
            &immediate_target,
            "addEventListener",
            &[Value::str("stop"), Value::Object(listener)],
        );
    }
    let event = construct(&mut realm, &event_constructor, &[Value::str("stop")]);
    call(&mut realm, &immediate_target, "dispatchEvent", &[event]);
    assert_eq!(*stopped.borrow(), 1);
}

#[test]
fn event_rejects_invalid_construction_and_dispatch_values() {
    let mut realm = Realm::new();
    let event_constructor = global(&mut realm, "Event");
    let event_target_constructor = global(&mut realm, "EventTarget");
    assert!(ops::call(&mut realm, &event_constructor, Value::Undefined, &[]).is_err());
    assert!(ops::call(&mut realm, &event_target_constructor, Value::Undefined, &[]).is_err());
    let target = construct(&mut realm, &event_target_constructor, &[]);
    let dispatch = ops::get(&mut realm, &target, &Key::name("dispatchEvent")).unwrap();
    assert!(ops::call(&mut realm, &dispatch, target.clone(), &[Value::Null]).is_err());
    let plain_event = realm.object();
    assert!(ops::call(&mut realm, &dispatch, target, &[Value::Object(plain_event)],).is_err());
}

#[test]
fn event_listener_objects_and_listener_errors_leave_event_redispatchable() {
    let mut realm = Realm::new();
    let target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &target_constructor, &[]);
    let event = construct(&mut realm, &event_constructor, &[Value::str("change")]);
    let calls = Rc::new(RefCell::new(0));
    let observed = calls.clone();
    let handle_event = realm.function("handleEvent", 1, false, move |realm, _, _| {
        *observed.borrow_mut() += 1;
        if *observed.borrow() == 1 {
            Err(realm.throw(ErrorKind::Type, "listener failed"))
        } else {
            Ok(Value::Undefined)
        }
    });
    let listener = realm.object();
    listener.define(
        Key::name("handleEvent"),
        Property::data(Value::Object(handle_event)),
    );
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[Value::str("change"), Value::Object(listener)],
    );
    let dispatch = ops::get(&mut realm, &target, &Key::name("dispatchEvent")).unwrap();
    assert!(
        ops::call(
            &mut realm,
            &dispatch,
            target.clone(),
            core::slice::from_ref(&event)
        )
        .is_err()
    );
    assert!(matches!(
        ops::get(&mut realm, &event, &Key::name("currentTarget")).unwrap(),
        Value::Null
    ));
    assert!(matches!(
        ops::get(&mut realm, &event, &Key::name("eventPhase")).unwrap(),
        Value::Number(0.0)
    ));
    assert!(matches!(
        ops::call(&mut realm, &dispatch, target, &[event]),
        Ok(Value::Boolean(true))
    ));
    assert_eq!(*calls.borrow(), 2);
}

#[test]
fn immediate_propagation_stopping_is_cleared_before_redispatch() {
    let mut realm = Realm::new();
    let event_target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &event_target_constructor, &[]);
    let event = construct(&mut realm, &event_constructor, &[Value::str("change")]);
    let calls = Rc::new(RefCell::new(0));
    let observed = calls.clone();
    let first = realm.function("first", 1, false, move |realm, _, args| {
        let mut count = observed.borrow_mut();
        *count += 1;
        let should_stop = *count == 1;
        drop(count);
        if should_stop {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            call(realm, &event, "stopImmediatePropagation", &[]);
        }
        Ok(Value::Undefined)
    });
    let next_count = calls.clone();
    let next = realm.function("next", 1, false, move |_, _, _| {
        *next_count.borrow_mut() += 10;
        Ok(Value::Undefined)
    });
    for listener in [first, next] {
        call(
            &mut realm,
            &target,
            "addEventListener",
            &[Value::str("change"), Value::Object(listener)],
        );
    }
    let dispatch = ops::get(&mut realm, &target, &Key::name("dispatchEvent")).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            ops::call(
                &mut realm,
                &dispatch,
                target.clone(),
                core::slice::from_ref(&event)
            ),
            Ok(Value::Boolean(true))
        ));
    }
    assert_eq!(*calls.borrow(), 12);
}

#[test]
fn dispatch_skips_removed_listeners_and_rejects_recursive_dispatch() {
    let mut realm = Realm::new();
    let target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &target_constructor, &[]);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let observed = calls.clone();
    let second = Value::Object(realm.function("second", 1, false, move |_, _, _| {
        observed.borrow_mut().push("second");
        Ok(Value::Undefined)
    }));
    let first_target = target.clone();
    let removed = second.clone();
    let observed = calls.clone();
    let first = Value::Object(realm.function("first", 1, false, move |realm, _, args| {
        observed.borrow_mut().push("first");
        call(
            realm,
            &first_target,
            "removeEventListener",
            &[Value::str("update"), removed.clone(), Value::Boolean(true)],
        );
        let dispatch = ops::get(realm, &first_target, &Key::name("dispatchEvent"))?;
        assert!(ops::call(realm, &dispatch, first_target.clone(), args).is_err());
        call(realm, &args[0], "stopPropagation", &[]);
        call(realm, &args[0], "preventDefault", &[]);
        Ok(Value::Undefined)
    }));
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[Value::str("update"), first],
    );
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[Value::str("update"), second, Value::Boolean(true)],
    );
    let event = construct(&mut realm, &event_constructor, &[Value::str("update")]);
    call(&mut realm, &target, "dispatchEvent", &[event]);
    assert_eq!(*calls.borrow(), ["first"]);
}

#[test]
fn event_target_rejects_invalid_receivers_and_propagates_option_getters() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "EventTarget");
    let target = construct(&mut realm, &constructor, &[]);
    for method in ["addEventListener", "removeEventListener", "dispatchEvent"] {
        let function = ops::get(&mut realm, &target, &Key::name(method)).unwrap();
        assert!(ops::call(&mut realm, &function, Value::Null, &[]).is_err());
    }
    for callback in [Value::Null, Value::Object(realm.object())] {
        call(
            &mut realm,
            &target,
            "addEventListener",
            &[Value::str("update"), callback],
        );
    }
    let listener =
        Value::Object(realm.function("listener", 0, false, |_, _, _| Ok(Value::Undefined)));
    let options = realm.object();
    let getter = realm.function("get capture", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "options"))
    });
    options.define(
        Key::name("capture"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let add = ops::get(&mut realm, &target, &Key::name("addEventListener")).unwrap();
    assert!(
        ops::call(
            &mut realm,
            &add,
            target,
            &[Value::str("update"), listener, Value::Object(options)]
        )
        .is_err()
    );
}

#[test]
fn once_listener_errors_remove_the_registration_and_clear_dispatch_state() {
    let mut realm = Realm::new();
    let target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &target_constructor, &[]);
    let event = construct(&mut realm, &event_constructor, &[Value::str("change")]);
    for method in [
        "preventDefault",
        "stopPropagation",
        "stopImmediatePropagation",
    ] {
        let function = ops::get(&mut realm, &event, &Key::name(method)).unwrap();
        for receiver in [Value::Null, Value::Object(realm.object())] {
            assert!(ops::call(&mut realm, &function, receiver, &[]).is_err());
        }
    }
    let options = realm.object();
    options.define(Key::name("once"), Property::data(Value::Boolean(true)));
    options.define(Key::name("capture"), Property::data(Value::Boolean(true)));
    let listener = realm.function("fail", 1, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "listener"))
    });
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[
            Value::str("change"),
            Value::Object(listener),
            Value::Object(options),
        ],
    );
    let dispatch = ops::get(&mut realm, &target, &Key::name("dispatchEvent")).unwrap();
    assert!(
        ops::call(
            &mut realm,
            &dispatch,
            target.clone(),
            core::slice::from_ref(&event)
        )
        .is_err()
    );
    assert!(matches!(
        ops::get(&mut realm, &event, &Key::name("currentTarget")).unwrap(),
        Value::Null
    ));
    assert!(matches!(
        ops::call(&mut realm, &dispatch, target, &[event]).unwrap(),
        Value::Boolean(true)
    ));
}

#[test]
fn listener_snapshots_ignore_object_prototype_pollution() {
    let mut realm = Realm::new();
    let target_constructor = global(&mut realm, "EventTarget");
    let event_constructor = global(&mut realm, "Event");
    let target = construct(&mut realm, &target_constructor, &[]);
    let listener = Value::Object(realm.function("phantom", 0, false, |_, _, _| {
        panic!("an unregistered listener must not run")
    }));
    let entry = realm.object();
    entry.define(Key::name("callback"), Property::data(listener.clone()));
    realm.object_prototype.define(
        Key::name("poisoned"),
        Property::data(Value::Object(realm.array(vec![Value::Object(entry)]))),
    );
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[Value::str("other"), listener],
    );
    let event = construct(&mut realm, &event_constructor, &[Value::str("poisoned")]);
    assert!(matches!(
        call(&mut realm, &target, "dispatchEvent", &[event]),
        Value::Boolean(true)
    ));
}

#[test]
fn malformed_native_listener_records_are_ignored_without_invoking_getters() {
    let mut realm = Realm::new();
    let keys = EventKeys {
        event_tag: Symbol::private("tag"),
        event_type: Symbol::private("type"),
        event_cancelable: Symbol::private("cancelable"),
        event_default_prevented: Symbol::private("prevented"),
        event_stopped: Symbol::private("stopped"),
        event_dispatching: Symbol::private("dispatching"),
        listeners: Symbol::private("listeners"),
    };
    let target = realm.object();
    let property = Property {
        slot: Slot::Accessor {
            get: None,
            set: None,
        },
        enumerable: false,
        configurable: true,
    };
    target.set_private(&keys.listeners, property.clone());
    assert!(listener_map(&target, &keys.listeners).is_none());
    let map = realm.object();
    target.set_private(&keys.listeners, Property::data(Value::Object(map.clone())));
    for property in [property, Property::data(Value::Object(realm.object()))] {
        map.define(Key::name("change"), property);
        assert!(entries(&target, "change", &keys).is_empty());
    }
    let event = realm.object();
    event.set_private(&keys.event_tag, Property::data(Value::Boolean(true)));
    assert!(
        dispatch_event(
            &mut realm,
            &Value::Object(target),
            &[Value::Object(event)],
            &keys
        )
        .is_err()
    );
}

#[test]
fn subclasses_construct_events_and_targets_with_their_own_prototypes() {
    let mut realm = Realm::new();
    for name in ["Event", "EventTarget"] {
        let base = global(&mut realm, name);
        let prototype = realm.object();
        let base_prototype = ops::get(&mut realm, &base, &Key::name("prototype")).unwrap();
        assert!(prototype.set_prototype(base_prototype.as_object().cloned()));
        let subclass = realm.function("Subclass", 0, true, |_, _, _| Ok(Value::Undefined));
        subclass.define(
            Key::name("prototype"),
            Property::data(Value::Object(prototype.clone())),
        );
        let instance = ops::construct_target(
            &mut realm,
            &base,
            &[Value::str("change")],
            &Value::Object(subclass),
        )
        .unwrap();
        assert_eq!(
            instance.as_object().unwrap().data().prototype.clone(),
            Some(prototype)
        );
    }
    let event_target = global(&mut realm, "EventTarget");
    let target = construct(&mut realm, &event_target, &[]);
    let expected = ops::get(&mut realm, &event_target, &Key::name("prototype")).unwrap();
    assert_eq!(
        target.as_object().unwrap().data().prototype.clone(),
        expected.as_object().cloned()
    );
}

#[test]
fn node_knows_event_targets_and_reads_their_listeners() {
    let mut realm = Realm::new();
    let target_constructor = global(&mut realm, "EventTarget");
    let signal_constructor = global(&mut realm, "AbortSignal");
    let mark = Key::Symbol(crate::symbol_builtin::intern(
        &realm,
        crate::JsStr::from("nodejs.event_target"),
    ));
    for constructor in [&target_constructor, &signal_constructor] {
        assert!(matches!(
            ops::get(&mut realm, constructor, &mark).unwrap(),
            Value::Boolean(true)
        ));
    }
    let target = construct(&mut realm, &target_constructor, &[]);
    let listener = realm.function("listener", 1, false, |_, _, _| Ok(Value::Undefined));
    call(
        &mut realm,
        &target,
        "addEventListener",
        &[Value::str("abort"), Value::Object(listener.clone())],
    );
    let events = Key::Symbol(crate::symbol_builtin::intern(
        &realm,
        crate::JsStr::from("nativeboat.event_target.events"),
    ));
    let view = ops::get(&mut realm, &target, &events).unwrap();
    let head = call(&mut realm, &view, "get", &[Value::str("abort")]);
    let first = ops::get(&mut realm, &head, &Key::name("next")).unwrap();
    let found = ops::get(&mut realm, &first, &Key::name("listener")).unwrap();
    assert!(matches!(found, Value::Object(object) if object == listener));
    let rest = ops::get(&mut realm, &first, &Key::name("next")).unwrap();
    assert!(matches!(rest, Value::Undefined));
    let none = call(&mut realm, &view, "get", &[Value::str("other")]);
    assert!(matches!(
        ops::get(&mut realm, &none, &Key::name("next")).unwrap(),
        Value::Undefined
    ));
}
