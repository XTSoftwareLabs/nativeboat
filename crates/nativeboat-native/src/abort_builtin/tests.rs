use super::*;
use crate::{Key, Realm, ops};
use alloc::rc::Rc;
use std::cell::RefCell;

fn global(realm: &mut Realm, name: &str) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name(name),
    )
    .unwrap()
}

fn member(realm: &mut Realm, object: &Value, name: &str) -> Value {
    ops::get(realm, object, &Key::name(name)).unwrap()
}

fn construct(realm: &mut Realm, constructor: &Value) -> Value {
    ops::construct(realm, constructor, &[]).unwrap()
}

#[test]
fn controller_dispatches_abort_once_and_preserves_the_first_reason() {
    let mut realm = Realm::new();
    let controller_constructor = global(&mut realm, "AbortController");
    let controller = construct(&mut realm, &controller_constructor);
    let signal = member(&mut realm, &controller, "signal");
    assert!(matches!(
        member(&mut realm, &signal, "aborted"),
        Value::Boolean(false)
    ));
    assert!(matches!(
        member(&mut realm, &signal, "reason"),
        Value::Undefined
    ));
    let throw = member(&mut realm, &signal, "throwIfAborted");
    assert!(matches!(
        ops::call(&mut realm, &throw, signal.clone(), &[]).unwrap(),
        Value::Undefined
    ));

    let seen = Rc::new(RefCell::new(Vec::new()));
    let callback_seen = seen.clone();
    let callback = realm.function("abort listener", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or_default();
        callback_seen
            .borrow_mut()
            .push(ops::get(realm, &event, &Key::name("type"))?);
        Ok(Value::Undefined)
    });
    let add = member(&mut realm, &signal, "addEventListener");
    ops::call(
        &mut realm,
        &add,
        signal.clone(),
        &[Value::str("abort"), callback.into()],
    )
    .unwrap();

    let abort = member(&mut realm, &controller, "abort");
    ops::call(
        &mut realm,
        &abort,
        controller.clone(),
        &[Value::str("first reason")],
    )
    .unwrap();
    ops::call(
        &mut realm,
        &abort,
        controller,
        &[Value::str("second reason")],
    )
    .unwrap();

    assert!(matches!(
        member(&mut realm, &signal, "aborted"),
        Value::Boolean(true)
    ));
    assert!(ops::strict_equals(
        &member(&mut realm, &signal, "reason"),
        &Value::str("first reason")
    ));
    assert_eq!(seen.borrow().len(), 1);
    assert!(ops::strict_equals(&seen.borrow()[0], &Value::str("abort")));
    let throw = member(&mut realm, &signal, "throwIfAborted");
    let error = ops::call(&mut realm, &throw, signal, &[]).unwrap_err();
    assert!(ops::strict_equals(&error.0, &Value::str("first reason")));
}

#[test]
fn abort_signal_static_abort_and_default_abort_error_match_the_host_contract() {
    let mut realm = Realm::new();
    let signal_constructor = global(&mut realm, "AbortSignal");
    let abort = member(&mut realm, &signal_constructor, "abort");
    let signal = ops::call(
        &mut realm,
        &abort,
        signal_constructor.clone(),
        &[Value::Number(17.0)],
    )
    .unwrap();
    assert!(matches!(
        member(&mut realm, &signal, "aborted"),
        Value::Boolean(true)
    ));
    assert!(ops::strict_equals(
        &member(&mut realm, &signal, "reason"),
        &Value::Number(17.0)
    ));

    let controller_constructor = global(&mut realm, "AbortController");
    let controller = construct(&mut realm, &controller_constructor);
    let controller_signal = member(&mut realm, &controller, "signal");
    let abort = member(&mut realm, &controller, "abort");
    ops::call(&mut realm, &abort, controller, &[]).unwrap();
    let reason = member(&mut realm, &controller_signal, "reason");
    assert!(matches!(reason, Value::Object(_)));
    assert!(ops::strict_equals(
        &member(&mut realm, &reason, "name"),
        &Value::str("AbortError")
    ));
    assert!(ops::strict_equals(
        &member(&mut realm, &reason, "code"),
        &Value::Number(20.0)
    ));
}

#[test]
fn constructors_and_brand_checked_methods_reject_invalid_receivers() {
    let mut realm = Realm::new();
    let controller_constructor = global(&mut realm, "AbortController");
    assert!(ops::call(&mut realm, &controller_constructor, Value::Undefined, &[]).is_err());
    let controller = construct(&mut realm, &controller_constructor);
    let signal = member(&mut realm, &controller, "signal");
    let prototype = controller
        .as_object()
        .unwrap()
        .data()
        .prototype
        .clone()
        .unwrap();
    let getter = prototype.own_property(&Key::name("signal")).unwrap();
    let Slot::Accessor {
        get: Some(getter), ..
    } = getter.slot
    else {
        panic!("signal must be an accessor");
    };
    assert!(ops::call(&mut realm, &Value::Object(getter), Value::Undefined, &[]).is_err());
    let throw = member(&mut realm, &signal, "throwIfAborted");
    assert!(ops::call(&mut realm, &throw, Value::Undefined, &[]).is_err());
    let abort = member(&mut realm, &controller, "abort");
    assert!(ops::call(&mut realm, &abort, Value::Undefined, &[]).is_err());
    let signal_constructor = global(&mut realm, "AbortSignal");
    assert!(ops::call(&mut realm, &signal_constructor, Value::Undefined, &[]).is_err());
}

#[test]
fn corrupted_controller_signal_state_is_rejected() {
    let mut realm = Realm::new();
    let keys = Keys {
        controller_signal: Symbol::private("test.AbortController.signal"),
        signal_tag: Symbol::private("test.AbortSignal.tag"),
        signal_aborted: Symbol::private("test.AbortSignal.aborted"),
        signal_reason: Symbol::private("test.AbortSignal.reason"),
    };
    let controller = realm.object();
    controller.set_private(&keys.controller_signal, Property::data(Value::Undefined));
    assert!(controller_signal(&mut realm, &controller, &keys).is_err());
    let signal = realm.object();
    controller.set_private(
        &keys.controller_signal,
        Property::data(Value::Object(signal.clone())),
    );
    assert!(controller_signal(&mut realm, &controller, &keys).is_ok());
    signal.set_private(
        &keys.signal_reason,
        Property {
            slot: Slot::Accessor {
                get: None,
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    assert!(matches!(
        private_value(&signal, &keys.signal_reason),
        Value::Undefined
    ));
}
