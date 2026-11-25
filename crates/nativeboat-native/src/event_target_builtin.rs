//! Native `Event` and single-target `EventTarget` support for application code.

use crate::prelude::*;

use crate::{
    Completion, ErrorKind, Key, Kind, Object, Property, Realm, Slot, Symbol, Thrown, Value, ops,
};

#[derive(Clone)]
struct EventKeys {
    event_tag: Symbol,
    event_type: Symbol,
    event_cancelable: Symbol,
    event_default_prevented: Symbol,
    event_stopped: Symbol,
    event_dispatching: Symbol,
    listeners: Symbol,
}

pub(crate) fn install(realm: &mut Realm) {
    let keys = EventKeys {
        event_tag: Symbol::private("native.Event.tag"),
        event_type: Symbol::private("native.Event.type"),
        event_cancelable: Symbol::private("native.Event.cancelable"),
        event_default_prevented: Symbol::private("native.Event.defaultPrevented"),
        event_stopped: Symbol::private("native.Event.stopped"),
        event_dispatching: Symbol::private("native.Event.dispatching"),
        listeners: Symbol::private("native.EventTarget.listeners"),
    };
    install_event(realm, &keys);
    install_event_target(realm, &keys);
}

fn install_event(realm: &mut Realm, keys: &EventKeys) {
    let prototype = realm.object();
    let constructor_keys = keys.clone();
    let instance_prototype = prototype.clone();
    let constructor = realm.builtin_constructor(
        "Event",
        1,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Class constructor Event cannot be invoked without 'new'",
            ))
        },
        move |realm, new_target, args| {
            let prototype = prototype_from(realm, &new_target, &instance_prototype)?;
            let type_arg = arg(args, 0);
            let event_type = ops::to_string(realm, &type_arg)?;
            let options = arg(args, 1);
            let bubbles = option_bool(realm, &options, "bubbles")?;
            let cancelable = option_bool(realm, &options, "cancelable")?;
            let composed = option_bool(realm, &options, "composed")?;
            let event = Object::new(Some(prototype), Kind::Ordinary);
            event.set_private(
                &constructor_keys.event_tag,
                Property::data(Value::Boolean(true)),
            );
            event.set_private(
                &constructor_keys.event_type,
                Property::data(Value::String(event_type.clone())),
            );
            event.set_private(
                &constructor_keys.event_cancelable,
                Property::data(Value::Boolean(cancelable)),
            );
            event.set_private(
                &constructor_keys.event_default_prevented,
                Property::data(Value::Boolean(false)),
            );
            event.set_private(
                &constructor_keys.event_stopped,
                Property::data(Value::Boolean(false)),
            );
            event.set_private(
                &constructor_keys.event_dispatching,
                Property::data(Value::Boolean(false)),
            );
            for (name, value) in [
                ("type", Value::String(event_type)),
                ("bubbles", Value::Boolean(bubbles)),
                ("cancelable", Value::Boolean(cancelable)),
                ("composed", Value::Boolean(composed)),
                ("defaultPrevented", Value::Boolean(false)),
                ("target", Value::Null),
                ("currentTarget", Value::Null),
                ("eventPhase", Value::Number(0.0)),
                ("isTrusted", Value::Boolean(false)),
                ("timeStamp", Value::Number(0.0)),
            ] {
                event.define(Key::name(name), readonly(value));
            }
            Ok(event)
        },
    );
    for (name, value) in [
        ("NONE", 0.0),
        ("CAPTURING_PHASE", 1.0),
        ("AT_TARGET", 2.0),
        ("BUBBLING_PHASE", 3.0),
    ] {
        constructor.define(
            Key::name(name),
            Property {
                slot: Slot::Data {
                    value: Value::Number(value),
                    writable: false,
                },
                enumerable: true,
                configurable: false,
            },
        );
    }
    for (name, code) in [
        (
            "preventDefault",
            event_prevent_default as fn(&mut Realm, Value, &[Value], &EventKeys) -> Completion,
        ),
        ("stopPropagation", event_stop),
        ("stopImmediatePropagation", event_stop_immediate),
    ] {
        let method_keys = keys.clone();
        realm.method(&prototype, name, 0, move |realm, this, args| {
            code(realm, this, args, &method_keys)
        });
    }
    realm.set_global("Event", Value::Object(constructor));
}

fn install_event_target(realm: &mut Realm, keys: &EventKeys) {
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    let constructor = realm.builtin_constructor(
        "EventTarget",
        0,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Class constructor EventTarget cannot be invoked without 'new'",
            ))
        },
        move |realm, new_target, _| {
            let prototype = prototype_from(realm, &new_target, &instance_prototype)?;
            Ok(Object::new(Some(prototype), Kind::Ordinary))
        },
    );
    let add_keys = keys.clone();
    realm.method(
        &prototype,
        "addEventListener",
        2,
        move |realm, this, args| add_listener(realm, &this, args, &add_keys),
    );
    let remove_keys = keys.clone();
    realm.method(
        &prototype,
        "removeEventListener",
        2,
        move |realm, this, args| remove_listener(realm, &this, args, &remove_keys),
    );
    let dispatch_keys = keys.clone();
    realm.method(&prototype, "dispatchEvent", 1, move |realm, this, args| {
        dispatch_event(realm, &this, args, &dispatch_keys)
    });
    realm.set_global("EventTarget", Value::Object(constructor));
}

/// `GetPrototypeFromConstructor`: the prototype of the class a script
/// extends `Event` or `EventTarget` with, else the built-in one.
fn prototype_from(
    realm: &mut Realm,
    new_target: &Object,
    fallback: &Object,
) -> Result<Object, Thrown> {
    match ops::get_named(realm, &Value::Object(new_target.clone()), "prototype")? {
        Value::Object(prototype) => Ok(prototype),
        _ => Ok(fallback.clone()),
    }
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn option_bool(realm: &mut Realm, options: &Value, name: &str) -> Result<bool, Thrown> {
    if matches!(options, Value::Undefined | Value::Null) {
        return Ok(false);
    }
    Ok(ops::to_boolean(&ops::get(
        realm,
        options,
        &Key::name(name),
    )?))
}

fn readonly(value: Value) -> Property {
    Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: true,
        configurable: true,
    }
}

fn event_prevent_default(
    realm: &mut Realm,
    this: Value,
    _args: &[Value],
    keys: &EventKeys,
) -> Completion {
    let event = event_receiver(realm, &this, keys)?;
    if private_bool(&event, &keys.event_cancelable) {
        event.set_private(
            &keys.event_default_prevented,
            Property::data(Value::Boolean(true)),
        );
        event.define(
            Key::name("defaultPrevented"),
            readonly(Value::Boolean(true)),
        );
    }
    Ok(Value::Undefined)
}

fn event_stop(realm: &mut Realm, this: Value, _args: &[Value], keys: &EventKeys) -> Completion {
    event_receiver(realm, &this, keys)?;
    Ok(Value::Undefined)
}

fn event_stop_immediate(
    realm: &mut Realm,
    this: Value,
    _args: &[Value],
    keys: &EventKeys,
) -> Completion {
    let event = event_receiver(realm, &this, keys)?;
    event.set_private(&keys.event_stopped, Property::data(Value::Boolean(true)));
    Ok(Value::Undefined)
}

fn event_receiver(realm: &Realm, value: &Value, keys: &EventKeys) -> Result<Object, Thrown> {
    value
        .as_object()
        .filter(|event| private_bool(event, &keys.event_tag))
        .cloned()
        .ok_or_else(|| {
            realm.throw(
                ErrorKind::Type,
                "Event method called on an invalid receiver",
            )
        })
}

fn private_bool(object: &Object, key: &Symbol) -> bool {
    object.private_property(key).is_some_and(|property| {
        matches!(
            property.slot,
            Slot::Data {
                value: Value::Boolean(true),
                ..
            }
        )
    })
}

fn listener_map(target: &Object, key: &Symbol) -> Option<Object> {
    target
        .private_property(key)
        .and_then(|property| match property.slot {
            Slot::Data {
                value: Value::Object(map),
                ..
            } => Some(map),
            _ => None,
        })
}

fn entries(target: &Object, event_type: &str, keys: &EventKeys) -> Vec<Object> {
    let Some(map) = listener_map(target, &keys.listeners) else {
        return Vec::new();
    };
    // Listener records are native state. Never consult Object.prototype or
    // invoke source getters while inspecting the dispatch snapshot.
    let list = entry_field(&map, event_type);
    let Some(list) = list.as_object() else {
        return Vec::new();
    };
    let length = match &list.data().kind {
        Kind::Array(array) => array.length(),
        _ => return Vec::new(),
    };
    let mut result = Vec::new();
    for index in 0..length {
        if let Some(Property {
            slot:
                Slot::Data {
                    value: Value::Object(entry),
                    ..
                },
            ..
        }) = list.own_property(&Key::Index(index))
        {
            result.push(entry);
        }
    }
    result
}

fn entry_field(entry: &Object, name: &str) -> Value {
    entry
        .own_property(&Key::name(name))
        .and_then(|property| match property.slot {
            Slot::Data { value, .. } => Some(value),
            _ => None,
        })
        .unwrap_or_default()
}

fn entry_capture(entry: &Object) -> bool {
    ops::to_boolean(&entry_field(entry, "capture"))
}

fn listener_options(realm: &mut Realm, value: &Value, name: &str) -> Result<bool, Thrown> {
    if let Value::Boolean(capture) = value
        && name == "capture"
    {
        return Ok(*capture);
    }
    option_bool(realm, value, name)
}

fn add_listener(realm: &mut Realm, this: &Value, args: &[Value], keys: &EventKeys) -> Completion {
    let Some(target) = this.as_object().cloned() else {
        return Err(realm.throw(
            ErrorKind::Type,
            "EventTarget.addEventListener called on an invalid receiver",
        ));
    };
    let event_type = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
    let callback = arg(args, 1);
    let Some(callback_object) = callback.as_object() else {
        return Ok(Value::Undefined);
    };
    if callback_object.code().is_none()
        && ops::get_named(realm, &callback, "handleEvent")?.is_nullish()
    {
        return Ok(Value::Undefined);
    }
    let options = arg(args, 2);
    let capture = listener_options(realm, &options, "capture")?;
    let once = listener_options(realm, &options, "once")?;
    for entry in entries(&target, &event_type, keys) {
        if ops::strict_equals(&entry_field(&entry, "callback"), &callback)
            && entry_capture(&entry) == capture
        {
            return Ok(Value::Undefined);
        }
    }
    let entry = realm.object();
    entry.define(Key::name("callback"), Property::hidden(callback));
    entry.define(
        Key::name("capture"),
        Property::hidden(Value::Boolean(capture)),
    );
    entry.define(Key::name("once"), Property::hidden(Value::Boolean(once)));
    let mut listeners = entries(&target, &event_type, keys)
        .into_iter()
        .map(Value::Object)
        .collect::<Vec<_>>();
    listeners.push(Value::Object(entry));
    let list = Value::Object(realm.array(listeners));
    let map = listener_map(&target, &keys.listeners).unwrap_or_else(|| {
        let map = Object::new(None, Kind::Ordinary);
        target.set_private(&keys.listeners, Property::data(Value::Object(map.clone())));
        map
    });
    map.define(Key::name(&event_type), Property::data(list));
    Ok(Value::Undefined)
}

fn remove_listener(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    keys: &EventKeys,
) -> Completion {
    let Some(target) = this.as_object().cloned() else {
        return Err(realm.throw(
            ErrorKind::Type,
            "EventTarget.removeEventListener called on an invalid receiver",
        ));
    };
    let event_type = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
    let callback = arg(args, 1);
    let options = arg(args, 2);
    let capture = listener_options(realm, &options, "capture")?;
    let mut listeners = Vec::new();
    for entry in entries(&target, &event_type, keys) {
        if ops::strict_equals(&entry_field(&entry, "callback"), &callback)
            && entry_capture(&entry) == capture
        {
            continue;
        }
        listeners.push(Value::Object(entry));
    }
    if let Some(map) = listener_map(&target, &keys.listeners) {
        map.define(
            Key::name(&event_type),
            Property::data(Value::Object(realm.array(listeners))),
        );
    }
    Ok(Value::Undefined)
}

fn dispatch_event(realm: &mut Realm, this: &Value, args: &[Value], keys: &EventKeys) -> Completion {
    let Some(target) = this.as_object().cloned() else {
        return Err(realm.throw(
            ErrorKind::Type,
            "EventTarget.dispatchEvent called on an invalid receiver",
        ));
    };
    let event_value = arg(args, 0);
    let Some(event) = event_value.as_object().cloned() else {
        return Err(realm.throw(ErrorKind::Type, "dispatchEvent requires an Event"));
    };
    if !private_bool(&event, &keys.event_tag) {
        return Err(realm.throw(ErrorKind::Type, "dispatchEvent requires an Event"));
    }
    if private_bool(&event, &keys.event_dispatching) {
        return Err(realm.throw(ErrorKind::Type, "The event is already being dispatched"));
    }
    let event_type = match event
        .private_property(&keys.event_type)
        .map(|property| property.slot)
    {
        Some(Slot::Data {
            value: Value::String(value),
            ..
        }) => value.to_rust_lossy(),
        _ => return Err(realm.throw(ErrorKind::Type, "dispatchEvent requires an Event")),
    };
    event.set_private(
        &keys.event_dispatching,
        Property::data(Value::Boolean(true)),
    );
    event.define(Key::name("target"), readonly(Value::Object(target.clone())));
    event.define(
        Key::name("currentTarget"),
        readonly(Value::Object(target.clone())),
    );
    event.define(Key::name("eventPhase"), readonly(Value::Number(2.0)));
    event.set_private(&keys.event_stopped, Property::data(Value::Boolean(false)));
    let dispatch_result = (|| {
        let snapshot = entries(&target, &event_type, keys);
        for entry in snapshot {
            if !entries(&target, &event_type, keys)
                .iter()
                .any(|current| current == &entry)
            {
                continue;
            }
            let once = ops::to_boolean(&entry_field(&entry, "once"));
            if once {
                let listeners = entries(&target, &event_type, keys)
                    .into_iter()
                    .filter(|current| current != &entry)
                    .map(Value::Object)
                    .collect();
                // The record came from this map, so remove that exact record
                // before invoking the callback (including capture listeners).
                if let Some(map) = listener_map(&target, &keys.listeners) {
                    map.define(
                        Key::name(&event_type),
                        Property::data(Value::Object(realm.array(listeners))),
                    );
                }
            }
            let callback = entry_field(&entry, "callback");
            if callback
                .as_object()
                .is_some_and(|object| object.code().is_some())
            {
                ops::call(
                    realm,
                    &callback,
                    Value::Object(target.clone()),
                    core::slice::from_ref(&event_value),
                )?;
            } else {
                let method = ops::get_named(realm, &callback, "handleEvent")?;
                ops::call(
                    realm,
                    &method,
                    callback,
                    core::slice::from_ref(&event_value),
                )?;
            }
            if private_bool(&event, &keys.event_stopped) {
                break;
            }
        }
        Ok(Value::Boolean(!private_bool(
            &event,
            &keys.event_default_prevented,
        )))
    })();
    event.set_private(
        &keys.event_dispatching,
        Property::data(Value::Boolean(false)),
    );
    event.set_private(&keys.event_stopped, Property::data(Value::Boolean(false)));
    event.define(Key::name("currentTarget"), readonly(Value::Null));
    event.define(Key::name("eventPhase"), readonly(Value::Number(0.0)));
    dispatch_result
}

#[cfg(test)]
mod tests;
