//! Engine-free AbortController/AbortSignal support for native host operations.

use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Symbol, Thrown, Value, ops};

#[derive(Clone)]
struct Keys {
    controller_signal: Symbol,
    signal_tag: Symbol,
    signal_aborted: Symbol,
    signal_reason: Symbol,
}

pub(crate) fn install(realm: &mut Realm) {
    let keys = Keys {
        controller_signal: Symbol::private("native.AbortController.signal"),
        signal_tag: Symbol::private("native.AbortSignal.tag"),
        signal_aborted: Symbol::private("native.AbortSignal.aborted"),
        signal_reason: Symbol::private("native.AbortSignal.reason"),
    };
    let target_constructor = get_global(realm, "EventTarget");
    let target_prototype = get(realm, &target_constructor, "prototype");
    let signal_prototype = realm.object();
    signal_prototype.set_prototype(target_prototype.as_object().cloned());

    let aborted_keys = keys.clone();
    let aborted = realm.function("get aborted", 0, false, move |realm, this, _| {
        let signal = signal_receiver(realm, &this, &aborted_keys, "aborted")?;
        Ok(Value::Boolean(private_bool(
            signal,
            &aborted_keys.signal_aborted,
        )))
    });
    signal_prototype.define(Key::name("aborted"), accessor(aborted));

    let reason_keys = keys.clone();
    let reason = realm.function("get reason", 0, false, move |realm, this, _| {
        let signal = signal_receiver(realm, &this, &reason_keys, "reason")?;
        Ok(private_value(signal, &reason_keys.signal_reason))
    });
    signal_prototype.define(Key::name("reason"), accessor(reason));

    let throw_keys = keys.clone();
    realm.method(
        &signal_prototype,
        "throwIfAborted",
        0,
        move |realm, this, _| {
            let signal = signal_receiver(realm, &this, &throw_keys, "throwIfAborted")?;
            if private_bool(signal, &throw_keys.signal_aborted) {
                Err(Thrown(private_value(signal, &throw_keys.signal_reason)))
            } else {
                Ok(Value::Undefined)
            }
        },
    );

    let signal_keys = keys.clone();
    let signal_prototype_for_constructor = signal_prototype.clone();
    let target_constructor_for_signal = target_constructor.clone();
    let signal_constructor = realm.builtin_constructor(
        "AbortSignal",
        0,
        &signal_prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "Illegal constructor")),
        move |realm, _, _| {
            let target = ops::construct(realm, &target_constructor_for_signal, &[])?;
            let signal = ops::to_object(realm, &target)?;
            signal.set_prototype(Some(signal_prototype_for_constructor.clone()));
            signal.set_private(
                &signal_keys.signal_tag,
                Property::data(Value::Boolean(true)),
            );
            Ok(signal)
        },
    );

    let static_keys = keys.clone();
    let static_constructor = signal_constructor.clone();
    let abort_static = realm.function("abort", 1, false, move |realm, _, args| {
        let signal = new_signal(realm, &static_constructor)?;
        let reason = abort_reason(realm, args);
        set_aborted(&signal, &static_keys, reason);
        Ok(Value::Object(signal))
    });
    signal_constructor.define(
        Key::name("abort"),
        Property::hidden(Value::Object(abort_static)),
    );
    // `AbortSignal` extends `EventTarget`, statics included.
    signal_constructor.set_prototype(target_constructor.as_object().cloned());
    realm.set_global("AbortSignal", Value::Object(signal_constructor.clone()));

    let controller_prototype = realm.object();
    let getter_keys = keys.clone();
    let signal_getter = realm.function("get signal", 0, false, move |realm, this, _| {
        let controller = controller_receiver(realm, &this, &getter_keys, "signal")?;
        Ok(private_value(controller, &getter_keys.controller_signal))
    });
    controller_prototype.define(Key::name("signal"), accessor(signal_getter));

    let abort_keys = keys.clone();
    realm.method(
        &controller_prototype,
        "abort",
        0,
        move |realm, this, args| {
            let controller = controller_receiver(realm, &this, &abort_keys, "abort")?;
            let signal = controller_signal(realm, controller, &abort_keys)?;
            if private_bool(&signal, &abort_keys.signal_aborted) {
                return Ok(Value::Undefined);
            }
            set_aborted(&signal, &abort_keys, abort_reason(realm, args));
            let event_constructor = get_global(realm, "Event");
            let event = ops::construct(realm, &event_constructor, &[Value::str("abort")])?;
            let dispatch = get(realm, &Value::Object(signal.clone()), "dispatchEvent");
            ops::call(realm, &dispatch, Value::Object(signal), &[event])
        },
    );

    let controller_keys = keys.clone();
    let signal_constructor_for_controller = signal_constructor;
    let controller_prototype_for_constructor = controller_prototype.clone();
    let constructor = realm.builtin_constructor(
        "AbortController",
        0,
        &controller_prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Class constructor AbortController cannot be invoked without 'new'",
            ))
        },
        move |realm, _, _| {
            let signal = new_signal(realm, &signal_constructor_for_controller)?;
            let object = Object::new(
                Some(controller_prototype_for_constructor.clone()),
                Kind::Ordinary,
            );
            object.set_private(
                &controller_keys.controller_signal,
                Property::data(Value::Object(signal)),
            );
            Ok(object)
        },
    );
    realm.set_global("AbortController", Value::Object(constructor));
}

fn new_signal(realm: &mut Realm, constructor: &Object) -> Result<Object, Thrown> {
    let signal = ops::construct(realm, &Value::Object(constructor.clone()), &[])?;
    ops::to_object(realm, &signal)
}

fn abort_reason(realm: &Realm, args: &[Value]) -> Value {
    args.first()
        .cloned()
        .filter(|value| !matches!(value, Value::Undefined))
        .unwrap_or_else(|| {
            let error = realm.error(ErrorKind::Error, "This operation was aborted");
            error.define(Key::name("name"), Property::data(Value::str("AbortError")));
            error.define(Key::name("code"), Property::data(Value::Number(20.0)));
            Value::Object(error)
        })
}

fn set_aborted(signal: &Object, keys: &Keys, reason: Value) {
    signal.set_private(&keys.signal_aborted, Property::data(Value::Boolean(true)));
    signal.set_private(&keys.signal_reason, Property::data(reason));
}

fn signal_receiver<'a>(
    realm: &mut Realm,
    this: &'a Value,
    keys: &Keys,
    method: &str,
) -> Result<&'a Object, Thrown> {
    this.as_object()
        .filter(|signal| private_bool(signal, &keys.signal_tag))
        .ok_or_else(|| {
            realm.throw(
                ErrorKind::Type,
                &alloc::format!("AbortSignal.{method} called on an invalid receiver"),
            )
        })
}

fn controller_receiver<'a>(
    realm: &mut Realm,
    this: &'a Value,
    keys: &Keys,
    method: &str,
) -> Result<&'a Object, Thrown> {
    this.as_object()
        .filter(|controller| {
            controller
                .private_property(&keys.controller_signal)
                .is_some()
        })
        .ok_or_else(|| {
            realm.throw(
                ErrorKind::Type,
                &alloc::format!("AbortController.{method} called on an invalid receiver"),
            )
        })
}

fn controller_signal(
    realm: &mut Realm,
    controller: &Object,
    keys: &Keys,
) -> Result<Object, Thrown> {
    match private_value(controller, &keys.controller_signal) {
        Value::Object(signal) => Ok(signal),
        _ => Err(realm.throw(ErrorKind::Type, "Illegal invocation")),
    }
}

fn private_bool(object: &Object, key: &Symbol) -> bool {
    matches!(
        object.private_property(key).map(|property| property.slot),
        Some(Slot::Data {
            value: Value::Boolean(true),
            ..
        })
    )
}

fn private_value(object: &Object, key: &Symbol) -> Value {
    object
        .private_property(key)
        .and_then(|property| match property.slot {
            Slot::Data { value, .. } => Some(value),
            Slot::Accessor { .. } => None,
        })
        .unwrap_or(Value::Undefined)
}

fn accessor(get: Object) -> Property {
    Property {
        slot: Slot::Accessor {
            get: Some(get),
            set: None,
        },
        enumerable: true,
        configurable: true,
    }
}

fn get_global(realm: &mut Realm, name: &str) -> Value {
    ops::get(
        realm,
        &Value::Object(realm.global.clone()),
        &Key::name(name),
    )
    .unwrap_or(Value::Undefined)
}

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap_or(Value::Undefined)
}

#[cfg(test)]
mod tests;
