//! Proxy exotic objects (ECMA-262 §10.5). Each internal method calls the
//! handler's trap, or forwards to the target when the handler has none.
//! Object operations reach these through `ops`, so a proxy behaves as one
//! anywhere it appears: as a receiver, on a prototype chain, as a function
//! and as a constructor. Revoked proxies throw from every internal method.

use crate::object::{ConstructorCode, Descriptor, NativeCode};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use core::cell::RefCell;

/// A proxy's target and handler, cleared when it is revoked, and the call
/// and construct behaviors it has when its target has them.
#[derive(Clone)]
pub struct Proxy {
    parts: Rc<RefCell<Option<(Object, Object)>>>,
    pub(crate) call: Option<NativeCode>,
    pub(crate) construct: Option<ConstructorCode>,
}

impl core::fmt::Debug for Proxy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let revoked = self.parts.borrow().is_none();
        write!(
            formatter,
            "Proxy(revoked: {revoked}, callable: {})",
            self.call.is_some()
        )
    }
}

/// The proxy behind an object, if it is one.
pub(crate) fn of(object: &Object) -> Option<Proxy> {
    match &object.data().kind {
        Kind::Proxy(proxy) => Some(proxy.clone()),
        _ => None,
    }
}

/// `ProxyCreate`.
fn create(realm: &Realm, target: &Value, handler: &Value) -> Result<Object, Thrown> {
    let (Some(target), Some(handler)) = (target.as_object(), handler.as_object()) else {
        return Err(realm.throw(
            ErrorKind::Type,
            "Cannot create proxy with a non-object as target or handler",
        ));
    };
    let parts = Rc::new(RefCell::new(Some((target.clone(), handler.clone()))));
    let (call, construct) = match target.code() {
        Some((_, constructor)) => {
            let called = parts.clone();
            let call: NativeCode = Rc::new(move |realm: &mut Realm, this, args: &[Value]| {
                apply(realm, &called, this, args)
            });
            let construct = constructor.then(|| {
                let constructed = parts.clone();
                let construct: ConstructorCode =
                    Rc::new(move |realm: &mut Realm, new_target, args: &[Value]| {
                        construct(realm, &constructed, new_target, args)
                    });
                construct
            });
            (Some(call), construct)
        }
        None => (None, None),
    };
    Ok(Object::new(
        None,
        Kind::Proxy(Proxy {
            parts,
            call,
            construct,
        }),
    ))
}

/// The target and handler of a live proxy, or a TypeError naming the
/// operation for a revoked one.
fn parts(
    realm: &Realm,
    parts: &Rc<RefCell<Option<(Object, Object)>>>,
    operation: &str,
) -> Result<(Object, Object), Thrown> {
    parts.borrow().clone().ok_or_else(|| {
        realm.throw(
            ErrorKind::Type,
            &format!("Cannot perform '{operation}' on a proxy that has been revoked"),
        )
    })
}

/// `GetMethod(handler, name)`: the trap, or `None` when it is undefined or
/// null.
/// A trap's name as a static string, whose lookups the handler's shape
/// caches: proxies run a trap lookup on every property access.
macro_rules! trap_name {
    ($text:literal) => {{
        static TEXT: crate::string::StaticStr<{ $text.len() }> =
            crate::string::StaticStr::new($text);
        &TEXT.header
    }};
}

fn trap_header(name: &str) -> Option<&'static crate::string::StaticHeader> {
    Some(match name {
        "get" => trap_name!("get"),
        "set" => trap_name!("set"),
        "has" => trap_name!("has"),
        "deleteProperty" => trap_name!("deleteProperty"),
        "ownKeys" => trap_name!("ownKeys"),
        "getOwnPropertyDescriptor" => trap_name!("getOwnPropertyDescriptor"),
        "defineProperty" => trap_name!("defineProperty"),
        "getPrototypeOf" => trap_name!("getPrototypeOf"),
        "setPrototypeOf" => trap_name!("setPrototypeOf"),
        "isExtensible" => trap_name!("isExtensible"),
        "preventExtensions" => trap_name!("preventExtensions"),
        _ => return None,
    })
}

fn trap(realm: &mut Realm, handler: &Object, name: &str) -> Result<Option<Value>, Thrown> {
    let handler_value = Value::Object(handler.clone());
    let trap = match trap_header(name) {
        Some(header) => ops::get_static(realm, &handler_value, header)?,
        None => ops::get(realm, &handler_value, &Key::name(name))?,
    };
    match trap {
        Value::Undefined | Value::Null => Ok(None),
        Value::Object(ref object) if object.code().is_some() => Ok(Some(trap)),
        _ => {
            let text = ops::to_string(realm, &trap)?.to_rust_lossy();
            Err(realm.throw(
                ErrorKind::Type,
                &format!(
                    "{text} returned for property '{name}' of object '#<Object>' is not a function"
                ),
            ))
        }
    }
}

fn key_value(key: &Key) -> Value {
    match key {
        Key::Symbol(symbol) => Value::Symbol(symbol.clone()),
        key => Value::String(key.to_js()),
    }
}

/// The live parts and trap of an internal method.
fn prepare(
    realm: &mut Realm,
    proxy: &Proxy,
    operation: &str,
) -> Result<(Object, Object, Option<Value>), Thrown> {
    let (target, handler) = parts(realm, &proxy.parts, operation)?;
    let trap = trap(realm, &handler, operation)?;
    Ok((target, handler, trap))
}

/// `[[GetPrototypeOf]]`.
pub(crate) fn get_prototype_of(realm: &mut Realm, proxy: &Proxy) -> Result<Option<Object>, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "getPrototypeOf")?;
    let Some(trap) = trap else {
        return ops::get_prototype_of(realm, &target);
    };
    match ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target)],
    )? {
        Value::Object(prototype) => Ok(Some(prototype)),
        Value::Null => Ok(None),
        _ => Err(realm.throw(
            ErrorKind::Type,
            "'getPrototypeOf' on proxy: trap returned neither object nor null",
        )),
    }
}

/// `[[SetPrototypeOf]]`.
pub(crate) fn set_prototype_of(
    realm: &mut Realm,
    proxy: &Proxy,
    prototype: Option<Object>,
) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "setPrototypeOf")?;
    let Some(trap) = trap else {
        return ops::set_prototype_of(realm, &target, prototype);
    };
    let prototype = prototype.map_or(Value::Null, Value::Object);
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), prototype],
    )?;
    Ok(ops::to_boolean(&result))
}

/// `[[IsExtensible]]`: the trap must agree with the target.
pub(crate) fn is_extensible(realm: &mut Realm, proxy: &Proxy) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "isExtensible")?;
    let Some(trap) = trap else {
        return ops::is_extensible(realm, &target);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target.clone())],
    )?;
    let result = ops::to_boolean(&result);
    if result != ops::is_extensible(realm, &target)? {
        return Err(realm.throw(
            ErrorKind::Type,
            "'isExtensible' on proxy: trap result does not reflect extensibility of proxy target",
        ));
    }
    Ok(result)
}

/// `[[PreventExtensions]]`.
pub(crate) fn prevent_extensions(realm: &mut Realm, proxy: &Proxy) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "preventExtensions")?;
    let Some(trap) = trap else {
        return ops::prevent_extensions(realm, &target);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target.clone())],
    )?;
    let result = ops::to_boolean(&result);
    if result && ops::is_extensible(realm, &target)? {
        return Err(realm.throw(
            ErrorKind::Type,
            "'preventExtensions' on proxy: trap returned truish but the proxy target is extensible",
        ));
    }
    Ok(result)
}

/// `[[GetOwnProperty]]`: the trap's descriptor, completed.
pub(crate) fn get_own_property(
    realm: &mut Realm,
    proxy: &Proxy,
    key: &Key,
) -> Result<Option<Property>, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "getOwnPropertyDescriptor")?;
    let Some(trap) = trap else {
        return ops::get_own_property(realm, &target, key);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), key_value(key)],
    )?;
    match result {
        Value::Undefined => Ok(None),
        Value::Object(_) => {
            let descriptor = crate::object_builtin::to_descriptor(realm, &result)?;
            Ok(Some(descriptor.complete()))
        }
        _ => Err(realm.throw(
            ErrorKind::Type,
            &format!(
                "'getOwnPropertyDescriptor' on proxy: trap returned neither object nor undefined for property '{}'",
                key.to_js()
            ),
        )),
    }
}

/// `[[DefineOwnProperty]]`.
pub(crate) fn define_own_property(
    realm: &mut Realm,
    proxy: &Proxy,
    key: &Key,
    descriptor: Descriptor,
) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "defineProperty")?;
    let Some(trap) = trap else {
        return ops::try_define_property(realm, &target, key, descriptor);
    };
    let descriptor = crate::object_builtin::from_partial_descriptor(realm, &descriptor);
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), key_value(key), descriptor],
    )?;
    Ok(ops::to_boolean(&result))
}

/// `[[HasProperty]]`.
pub(crate) fn has(realm: &mut Realm, proxy: &Proxy, key: &Key) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "has")?;
    let Some(trap) = trap else {
        return ops::has_property(realm, key, &Value::Object(target));
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), key_value(key)],
    )?;
    Ok(ops::to_boolean(&result))
}

/// `[[Get]]` with the original receiver.
pub(crate) fn get(
    realm: &mut Realm,
    proxy: &Proxy,
    key: &Key,
    receiver: &Value,
) -> crate::Completion {
    let (target, handler, trap) = prepare(realm, proxy, "get")?;
    let Some(trap) = trap else {
        return ops::get_with_receiver(realm, &target, key, receiver);
    };
    ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), key_value(key), receiver.clone()],
    )
}

/// `[[Set]]` with the original receiver: whether the assignment happened.
pub(crate) fn set(
    realm: &mut Realm,
    proxy: &Proxy,
    key: &Key,
    value: Value,
    receiver: &Value,
) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "set")?;
    let Some(trap) = trap else {
        return ops::set_with_receiver(realm, &target, key, value, receiver);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[
            Value::Object(target),
            key_value(key),
            value,
            receiver.clone(),
        ],
    )?;
    Ok(ops::to_boolean(&result))
}

/// `[[Delete]]`.
pub(crate) fn delete(realm: &mut Realm, proxy: &Proxy, key: &Key) -> Result<bool, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "deleteProperty")?;
    let Some(trap) = trap else {
        return ops::delete_property(realm, &target, key);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), key_value(key)],
    )?;
    Ok(ops::to_boolean(&result))
}

/// `[[OwnPropertyKeys]]`: the trap's strings and symbols, without repeats.
pub(crate) fn own_keys(realm: &mut Realm, proxy: &Proxy) -> Result<Vec<Key>, Thrown> {
    let (target, handler, trap) = prepare(realm, proxy, "ownKeys")?;
    let Some(trap) = trap else {
        return ops::own_property_keys(realm, &target);
    };
    let result = ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target)],
    )?;
    if !matches!(result, Value::Object(_)) {
        return Err(realm.throw(
            ErrorKind::Type,
            "CreateListFromArrayLike called on non-object",
        ));
    }
    let mut keys: Vec<Key> = Vec::new();
    for value in crate::function::list_from_array_like(realm, &result)? {
        let key = match value {
            Value::String(text) => Key::from_js(&text),
            Value::Symbol(symbol) => Key::Symbol(symbol),
            other => {
                let text = ops::to_string(realm, &other)?.to_rust_lossy();
                return Err(realm.throw(
                    ErrorKind::Type,
                    &format!("{text} is not a valid property name"),
                ));
            }
        };
        if keys.contains(&key) {
            return Err(realm.throw(
                ErrorKind::Type,
                "'ownKeys' on proxy: trap returned duplicate entries",
            ));
        }
        keys.push(key);
    }
    Ok(keys)
}

/// `[[Call]]`.
fn apply(
    realm: &mut Realm,
    parts_cell: &Rc<RefCell<Option<(Object, Object)>>>,
    this: Value,
    args: &[Value],
) -> crate::Completion {
    let (target, handler) = parts(realm, parts_cell, "apply")?;
    let Some(trap) = trap(realm, &handler, "apply")? else {
        return ops::call(realm, &Value::Object(target), this, args);
    };
    let arguments = Value::Object(realm.array(args.to_vec()));
    ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), this, arguments],
    )
}

/// `[[Construct]]`: the trap must return an object.
fn construct(
    realm: &mut Realm,
    parts_cell: &Rc<RefCell<Option<(Object, Object)>>>,
    new_target: Object,
    args: &[Value],
) -> Result<Object, Thrown> {
    let (target, handler) = parts(realm, parts_cell, "construct")?;
    let Some(trap) = trap(realm, &handler, "construct")? else {
        let result = ops::construct_target(
            realm,
            &Value::Object(target),
            args,
            &Value::Object(new_target),
        )?;
        return Ok(result
            .as_object()
            .cloned()
            .expect("construction returns an object"));
    };
    let arguments = Value::Object(realm.array(args.to_vec()));
    match ops::call(
        realm,
        &trap,
        Value::Object(handler),
        &[Value::Object(target), arguments, Value::Object(new_target)],
    )? {
        Value::Object(object) => Ok(object),
        _ => Err(realm.throw(ErrorKind::Type, "proxy [[Construct]] must return an object")),
    }
}

/// `IsArray`: an array, or a proxy whose target is one.
pub(crate) fn is_array(realm: &Realm, object: &Object) -> Result<bool, Thrown> {
    match of(object) {
        Some(proxy) => {
            let (target, _) = parts(realm, &proxy.parts, "IsArray")?;
            is_array(realm, &target)
        }
        None => Ok(object.is_array()),
    }
}

pub(crate) fn install(realm: &mut Realm) {
    // A constructor without a `prototype` property: proxies have none.
    let constructor = realm.function("Proxy", 2, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Type, "Constructor Proxy requires 'new'"))
    });
    let (code, _) = constructor.code().expect("new function has native code");
    let construct: ConstructorCode =
        Rc::new(|realm: &mut Realm, _, args: &[Value]| create(realm, &arg(args, 0), &arg(args, 1)));
    constructor.data_mut().kind = Kind::BuiltinConstructor { code, construct };
    realm.method(&constructor, "revocable", 2, |realm, _, args| {
        let proxy = create(realm, &arg(args, 0), &arg(args, 1))?;
        let revoked = of(&proxy).expect("a created proxy").parts;
        let revoke = realm.function("", 0, false, move |_, _, _| {
            revoked.borrow_mut().take();
            Ok(Value::Undefined)
        });
        let result = realm.object();
        result.define(Key::name("proxy"), Property::data(Value::Object(proxy)));
        result.define(Key::name("revoke"), Property::data(Value::Object(revoke)));
        Ok(Value::Object(result))
    });
    realm.set_global("Proxy", Value::Object(constructor));
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

#[cfg(test)]
mod tests;
