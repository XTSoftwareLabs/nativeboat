//! The `Reflect` namespace as ECMAScript 2024 specifies it: the object
//! internal methods as functions that report failure as `false` rather than
//! throwing, with receiver-aware `get` and `set`. Error messages follow V8.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{is_callable, is_constructor};
use crate::object::{Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn target(realm: &Realm, args: &[Value], method: &str) -> Result<Object, Thrown> {
    match args.first() {
        Some(Value::Object(object)) => Ok(object.clone()),
        _ => Err(realm.throw(
            ErrorKind::Type,
            &format!("Reflect.{method} called on non-object"),
        )),
    }
}

/// The arguments list of `apply` and `construct`.
fn list(realm: &mut Realm, value: &Value) -> Result<Vec<Value>, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Err(realm.throw(
            ErrorKind::Type,
            "CreateListFromArrayLike called on non-object",
        ));
    }
    crate::function::list_from_array_like(realm, value)
}

fn key_value(key: Key) -> Value {
    match key {
        Key::Symbol(symbol) => Value::Symbol(symbol),
        key => Value::String(key.to_js()),
    }
}

/// A native method body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

const METHODS: [(&str, u32, Native); 13] = [
    ("defineProperty", 3, |realm, _, args| {
        let object = target(realm, args, "defineProperty")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        let descriptor = crate::object_builtin::to_descriptor(realm, &arg(args, 2))?;
        Ok(Value::Boolean(ops::try_define_property(
            realm, &object, &key, descriptor,
        )?))
    }),
    ("deleteProperty", 2, |realm, _, args| {
        let object = target(realm, args, "deleteProperty")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        ops::delete_property(realm, &object, &key).map(Value::Boolean)
    }),
    ("apply", 3, |realm, _, args| {
        let function = arg(args, 0);
        if !is_callable(&function) {
            let text = ops::to_string(realm, &function)?.to_rust_lossy();
            let kind = ops::type_of(&function);
            let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
                "an"
            } else {
                "a"
            };
            return Err(realm.throw(
                ErrorKind::Type,
                &format!(
                    "Function.prototype.apply was called on {text}, which is {article} {kind} and not a function"
                ),
            ));
        }
        let values = list(realm, &arg(args, 2))?;
        ops::call(realm, &function, arg(args, 1), &values)
    }),
    ("construct", 2, |realm, _, args| {
        let function = arg(args, 0);
        let new_target = args.get(2).cloned().unwrap_or_else(|| function.clone());
        for value in [&function, &new_target] {
            if !is_constructor(value) {
                let text = ops::to_string(realm, value)?.to_rust_lossy();
                return Err(realm.throw(ErrorKind::Type, &format!("{text} is not a constructor")));
            }
        }
        let values = list(realm, &arg(args, 1))?;
        ops::construct_target(realm, &function, &values, &new_target)
    }),
    ("get", 2, |realm, _, args| {
        let object = target(realm, args, "get")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        let receiver = args
            .get(2)
            .cloned()
            .unwrap_or_else(|| Value::Object(object.clone()));
        ops::get_with_receiver(realm, &object, &key, &receiver)
    }),
    ("getOwnPropertyDescriptor", 2, |realm, _, args| {
        let object = target(realm, args, "getOwnPropertyDescriptor")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        Ok(ops::get_own_property(realm, &object, &key)?
            .map(|property| crate::object_builtin::from_descriptor(realm, &property))
            .unwrap_or_default())
    }),
    ("getPrototypeOf", 1, |realm, _, args| {
        let object = target(realm, args, "getPrototypeOf")?;
        let prototype = ops::get_prototype_of(realm, &object)?;
        Ok(prototype.map(Value::Object).unwrap_or(Value::Null))
    }),
    ("has", 2, |realm, _, args| {
        let object = target(realm, args, "has")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        ops::has_property(realm, &key, &Value::Object(object)).map(Value::Boolean)
    }),
    ("isExtensible", 1, |realm, _, args| {
        let object = target(realm, args, "isExtensible")?;
        ops::is_extensible(realm, &object).map(Value::Boolean)
    }),
    ("ownKeys", 1, |realm, _, args| {
        let object = target(realm, args, "ownKeys")?;
        let keys = ops::own_property_keys(realm, &object)?
            .into_iter()
            .map(key_value)
            .collect();
        Ok(Value::Object(realm.array(keys)))
    }),
    ("preventExtensions", 1, |realm, _, args| {
        let object = target(realm, args, "preventExtensions")?;
        ops::prevent_extensions(realm, &object).map(Value::Boolean)
    }),
    ("set", 3, |realm, _, args| {
        let object = target(realm, args, "set")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        let value = arg(args, 2);
        let receiver = args
            .get(3)
            .cloned()
            .unwrap_or_else(|| Value::Object(object.clone()));
        if matches!(object.data().kind, Kind::Environment) {
            ops::set(realm, &Value::Object(object), &key, value)?;
            return Ok(Value::Boolean(true));
        }
        ops::set_with_receiver(realm, &object, &key, value, &receiver).map(Value::Boolean)
    }),
    ("setPrototypeOf", 2, |realm, _, args| {
        let object = target(realm, args, "setPrototypeOf")?;
        let prototype = match arg(args, 1) {
            Value::Object(prototype) => Some(prototype),
            Value::Null => None,
            other => {
                let text = ops::to_string(realm, &other)?.to_rust_lossy();
                return Err(realm.throw(
                    ErrorKind::Type,
                    &format!("Object prototype may only be an Object or null: {text}"),
                ));
            }
        };
        ops::set_prototype_of(realm, &object, prototype).map(Value::Boolean)
    }),
];

pub(crate) fn install(realm: &mut Realm) {
    let reflect = realm.object();
    for (name, length, code) in METHODS {
        realm.method(&reflect, name, length, code);
    }
    reflect.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("Reflect"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm.set_global("Reflect", Value::Object(reflect));
}

#[cfg(test)]
mod tests;
