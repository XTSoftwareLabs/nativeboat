//! The built-in methods that the language's own conversions depend on.
//! Library coverage grows with the compiled feature inventory; a missing
//! built-in is a compile-time blocker, never a silent no-op.

#[allow(unused_imports)]
use crate::prelude::*;

use alloc::rc::Rc;
use core::cell::RefCell;

use crate::iter;
use crate::object::{Key, Object};
use crate::ops::{self, to_string};
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Value};

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn this_object(realm: &Realm, this: &Value, method: &str) -> Result<Object, crate::Thrown> {
    match this {
        Value::Object(object) => Ok(object.clone()),
        _ => Err(realm.throw(ErrorKind::Type, &format!("{method} called on a non-object"))),
    }
}

fn function_call(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let receiver = arg(args, 0);
    ops::call(realm, &this, receiver, args.get(1..).unwrap_or(&[]))
}

pub(crate) fn error_to_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    this_object(realm, &this, "Error.prototype.toString")?;
    let field = |realm: &mut Realm, name: &str, fallback: &str| -> Result<JsStr, crate::Thrown> {
        match ops::get(realm, &this, &Key::name(name))? {
            Value::Undefined => Ok(JsStr::from(fallback)),
            value => to_string(realm, &value),
        }
    };
    let name = field(realm, "name", "Error")?;
    let message = field(realm, "message", "")?;
    Ok(Value::String(if name.is_empty() {
        message
    } else if message.is_empty() {
        name
    } else {
        name.concat(&JsStr::from(": ")).concat(&message)
    }))
}

/// Defines a method under a symbol key.
fn symbol_method(
    realm: &Realm,
    target: &Object,
    key: crate::Symbol,
    name: &str,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Object {
    let function = realm.function(name, 0, false, code);
    target.define(
        Key::Symbol(key),
        crate::Property::hidden(Value::Object(function.clone())),
    );
    function
}

fn install_iterators(realm: &mut Realm) {
    let iterator = realm.symbols.iterator.clone();
    let tag = realm.symbols.to_string_tag.clone();
    let iterators = realm.iterator_prototype.clone();
    // Node 22's shared Iterator base metadata, also inherited by compiled
    // URLSearchParams iterators. Lazy helper methods remain separate work.
    let self_reference = Rc::new(RefCell::new(None::<crate::object::WeakObject>));
    let called = self_reference.clone();
    let fallback = iterators.clone();
    let constructor = realm.builtin_constructor(
        "Iterator",
        0,
        &iterators,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "Constructor Iterator requires 'new'")),
        move |realm, target, _| {
            if Some(&target)
                == called
                    .borrow()
                    .as_ref()
                    .and_then(crate::object::WeakObject::upgrade)
                    .as_ref()
            {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "Abstract class Iterator not directly constructable",
                ));
            }
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => fallback.clone(),
            };
            Ok(Object::new(Some(prototype), crate::Kind::Ordinary))
        },
    );
    *self_reference.borrow_mut() = Some(constructor.downgrade());
    for (key, name, value) in [
        (
            Key::name("constructor"),
            "constructor",
            Value::Object(constructor),
        ),
        (
            Key::Symbol(tag.clone()),
            "[Symbol.toStringTag]",
            Value::str("Iterator"),
        ),
    ] {
        let getter = realm.function(&format!("get {name}"), 0, false, move |_, _, _| {
            Ok(value.clone())
        });
        let prototype = iterators.downgrade();
        let set_key = key.clone();
        let setter = realm.function(
            &format!("set {name}"),
            1,
            false,
            move |realm, this, args| {
                let Value::Object(receiver) = this else {
                    let member = if name == "constructor" {
                        ".constructor"
                    } else {
                        name
                    };
                    return Err(realm.throw(
                        ErrorKind::Type,
                        &format!("set Iterator.prototype{member} called on non-object"),
                    ));
                };
                if prototype.upgrade().as_ref() == Some(&receiver) {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        &format!(
                            "Cannot assign to read only property '{}' of object '[object Object]'",
                            set_key.to_js()
                        ),
                    ));
                }
                if !receiver.define_own_property(
                    &set_key,
                    &crate::object::Descriptor::of(&crate::Property::data(arg(args, 0))),
                ) {
                    let message =
                        if receiver.own_property(&set_key).is_none() && !receiver.is_extensible() {
                            format!(
                                "Cannot define property {}, object is not extensible",
                                set_key.to_js()
                            )
                        } else {
                            format!("Cannot redefine property: {}", set_key.to_js())
                        };
                    return Err(realm.throw(ErrorKind::Type, &message));
                }
                Ok(Value::Undefined)
            },
        );
        iterators.define(
            key,
            crate::Property {
                slot: crate::Slot::Accessor {
                    get: Some(getter),
                    set: Some(setter),
                },
                enumerable: false,
                configurable: true,
            },
        );
    }

    symbol_method(
        realm,
        &iterators,
        iterator.clone(),
        "[Symbol.iterator]",
        |_, this, _| Ok(this),
    );
    for (prototype, string, name) in [
        (
            realm.array_iterator_prototype.clone(),
            false,
            "Array Iterator",
        ),
        (
            realm.string_iterator_prototype.clone(),
            true,
            "String Iterator",
        ),
    ] {
        let next = realm.function("next", 0, false, move |realm, this, _| {
            iter::next(realm, this, string)
        });
        prototype.define(
            Key::name("next"),
            crate::Property::hidden(Value::Object(next.clone())),
        );
        realm.iterator_next.push(next);
        let fixed = crate::Property {
            slot: crate::Slot::Data {
                value: Value::str(name),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        };
        prototype.define(Key::Symbol(tag.clone()), fixed);
    }
    let array = realm.array_prototype.clone();
    for (name, yields) in [
        ("keys", iter::Yield::Keys),
        ("entries", iter::Yield::Entries),
    ] {
        realm.method(&array, name, 0, move |realm, this, _| {
            iter::array_iterator(realm, this, yields)
        });
    }
    let values = realm.function("values", 0, false, |realm, this, _| {
        iter::array_iterator(realm, this, iter::Yield::Values)
    });
    realm.array_values = values.clone();
    array.define(
        Key::name("values"),
        crate::Property::hidden(Value::Object(values.clone())),
    );
    array.define(
        Key::Symbol(iterator.clone()),
        crate::Property::hidden(Value::Object(values)),
    );
    let strings = realm.string_prototype.clone();
    symbol_method(
        realm,
        &strings,
        iterator,
        "[Symbol.iterator]",
        |realm, this, _| {
            let text = ops::to_string(realm, &this)?;
            Ok(iter::string_iterator(realm, text))
        },
    );
}

pub(crate) fn install(realm: &mut Realm) {
    let function = realm.function_prototype.clone();
    realm.method(&function, "call", 1, function_call);
    crate::function::install(realm);
    crate::array_builtin::install(realm);
    crate::string_builtin::install(realm);
    let error = realm.error_prototype.clone();
    realm.method(&error, "toString", 0, error_to_string);
    install_iterators(realm);
    crate::math::install(realm);
    crate::global_number::install(realm);
    crate::number_builtin::install(realm);
    crate::collection_builtin::install(realm);
    crate::weak_builtin::install(realm);
    crate::json_builtin::install(realm);
    crate::console_builtin::install(realm);
    crate::regexp_builtin::install(realm);
    crate::promise_builtin::install(realm);
    crate::event_loop::install(realm);
    crate::event_target_builtin::install(realm);
    crate::abort_builtin::install(realm);
    crate::coroutine::install(realm);
    crate::reflect_builtin::install(realm);
    crate::global_uri::install(realm);
    crate::date_builtin::install(realm);
    crate::typed_array_builtin::install(realm);
    crate::process_builtin::install(realm);
    crate::proxy_builtin::install(realm);
    crate::bigint_builtin::install(realm);
    crate::text_encoding_builtin::install(realm);
    crate::web_crypto_builtin::install(realm);
    crate::structured_clone_builtin::install(realm);
    crate::performance_builtin::install(realm);
    crate::intl_builtin::install(realm);
    crate::modules::install(realm);
    crate::errors::install(realm);
    crate::symbol_builtin::install(realm);
    crate::object_builtin::install(realm);
}

#[cfg(test)]
mod tests;
