//! Electron's context bridge between a preload and its page. Values the
//! preload exposes with `contextBridge.exposeInMainWorld` reach the page as
//! copies, as Electron passes them between worlds:
//!
//! - primitives, plain objects and arrays are copied;
//! - functions become proxies that run in the world that made them, with
//!   their arguments, results and exceptions copied across;
//! - promises become promises of the other world that settle with copies;
//! - errors keep their message, dates their time, and byte views their bytes;
//! - maps and sets are copied with copies of their entries, and regular
//!   expressions and wrapped primitives are cloned.
//!
//! A function crosses as the same proxy each time, so a callback given to a
//! listener API can be removed again. Symbols cannot cross.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Thrown, Value, ops};

/// Exposes a preload's `exposeInMainWorld` values as globals of its page.
///
/// # Safety
///
/// Both realms must stay at the same addresses for as long as the page can
/// call the values, and neither may be borrowed elsewhere while a bridged
/// call runs.
pub unsafe fn expose(preload: *mut Realm, page: *mut Realm) {
    // SAFETY: the caller keeps both realms alive and unaliased.
    let exports = unsafe { (*preload).electron_bridge_exports.clone() };
    for (name, value) in exports {
        // SAFETY: as above.
        match unsafe { copy(preload, page, &value, &mut Vec::new()) } {
            // SAFETY: as above.
            Ok(value) => unsafe { (*page).set_global(&name, value) },
            Err(_) => {
                crate::platform::write_stderr(&format!(
                    "nativeboat: contextBridge could not expose '{name}' to the page\n"
                ));
            }
        }
    }
}

/// A value of `from` as `to` sees it.
///
/// # Safety
///
/// As [`expose`].
unsafe fn copy(
    from: *mut Realm,
    to: *mut Realm,
    value: &Value,
    seen: &mut Vec<(Object, Object)>,
) -> Result<Value, Thrown> {
    let Value::Object(source) = value else {
        if matches!(value, Value::Symbol(_)) {
            // SAFETY: as above.
            return Err(unsafe { &*to }.throw(
                ErrorKind::Type,
                "A symbol cannot be passed across the context bridge",
            ));
        }
        return Ok(value.clone());
    };
    if let Some((_, copied)) = seen.iter().find(|(known, _)| known == source) {
        return Ok(Value::Object(copied.clone()));
    }
    if source.code().is_some() {
        // SAFETY: as above.
        return Ok(Value::Object(unsafe { function(from, to, source) }));
    }
    let kind = match &source.data().kind {
        Kind::Promise(_) => 0,
        Kind::Error => 1,
        Kind::Date(_) => 2,
        Kind::TypedArray(_) | Kind::ArrayBuffer(_) | Kind::DataView(_) => 3,
        Kind::Array(_) => 4,
        Kind::Collection(_) => 6,
        Kind::RegExp(_) | Kind::Primitive(_) => 7,
        _ => 5,
    };
    // SAFETY: as above.
    let (source_realm, target_realm) = unsafe { (&mut *from, &mut *to) };
    match kind {
        // SAFETY: as above.
        0 => unsafe { promise(from, to, source) },
        1 => {
            let message = ops::get(source_realm, value, &Key::name("message"))
                .and_then(|message| ops::to_string(source_realm, &message))
                .map(|message| message.to_rust_lossy())
                .unwrap_or_default();
            Ok(Value::Object(
                target_realm.error(ErrorKind::Error, &message),
            ))
        }
        2 => {
            let Kind::Date(time) = source.data().kind else {
                unreachable!("checked above")
            };
            Ok(Value::Object(Object::new(
                Some(target_realm.date_prototype.clone()),
                Kind::Date(time),
            )))
        }
        6 => {
            let (set, entries) = match &source.data().kind {
                Kind::Collection(collection) => (collection.set, collection.snapshot(usize::MAX)),
                _ => unreachable!("checked above"),
            };
            let global = Value::Object(target_realm.global_object());
            let name = Key::name(if set { "Set" } else { "Map" });
            let constructor = ops::get(target_realm, &global, &name)?;
            let Value::Object(target) = ops::construct(target_realm, &constructor, &[])? else {
                return Err(target_realm.throw(ErrorKind::Type, "A collection could not be copied"));
            };
            seen.push((source.clone(), target.clone()));
            let add = Key::name(if set { "add" } else { "set" });
            let add = ops::get(target_realm, &Value::Object(target.clone()), &add)?;
            for (key, entry) in entries {
                // SAFETY: as above.
                let key = unsafe { copy(from, to, &key, seen) }?;
                let entry = if set {
                    Value::Undefined
                } else {
                    // SAFETY: as above.
                    unsafe { copy(from, to, &entry, seen) }?
                };
                // SAFETY: as above.
                let target_realm = unsafe { &mut *to };
                ops::call(
                    target_realm,
                    &add,
                    Value::Object(target.clone()),
                    &[key, entry],
                )?;
            }
            Ok(Value::Object(target))
        }
        7 => crate::structured_clone_builtin::clone_between(source_realm, target_realm, value),
        3 => {
            let bytes = crate::typed_array_builtin::view_bytes(value).unwrap_or_default();
            Ok(Value::Object(crate::typed_array_builtin::new_bytes(
                target_realm,
                bytes,
                None,
            )))
        }
        _ => {
            let target = if kind == 4 {
                target_realm.array(Vec::new())
            } else {
                target_realm.object()
            };
            seen.push((source.clone(), target.clone()));
            let keys = ops::own_property_keys(source_realm, source)
                .map_err(|thrown| rethrow(from, to, thrown))?;
            for key in keys {
                if matches!(key, Key::Symbol(_)) {
                    continue;
                }
                let enumerable = ops::get_own_property(source_realm, source, &key)
                    .map_err(|thrown| rethrow(from, to, thrown))?
                    .is_some_and(|property| property.enumerable);
                if !enumerable {
                    continue;
                }
                let property = ops::get(source_realm, value, &key)
                    .map_err(|thrown| rethrow(from, to, thrown))?;
                // SAFETY: as above.
                let property = unsafe { copy(from, to, &property, seen) }?;
                target.define(key, Property::data(property));
            }
            Ok(Value::Object(target))
        }
    }
}

/// An exception thrown in `from`, as `to` sees it.
fn rethrow(from: *mut Realm, to: *mut Realm, thrown: Thrown) -> Thrown {
    // SAFETY: the bridge's callers keep both realms alive and unaliased.
    unsafe { copy(from, to, &thrown.0, &mut Vec::new()) }.map_or_else(|error| error, Thrown)
}

/// The proxy in `to` of a function of `from`, made once per function.
///
/// # Safety
///
/// As [`expose`].
unsafe fn function(from: *mut Realm, to: *mut Realm, original: &Object) -> Object {
    // SAFETY: as above.
    let target = unsafe { &mut *to };
    if let Some((_, proxy)) = target
        .bridged_functions
        .iter()
        .find(|(known, _)| known == original)
    {
        return proxy.clone();
    }
    let callee = Value::Object(original.clone());
    let proxy = target.function("", 0, false, move |_, _, args| {
        let mut converted = Vec::with_capacity(args.len());
        for arg in args {
            // SAFETY: as above; the proxy runs only while both realms live.
            let arg = unsafe { copy(to, from, arg, &mut Vec::new()) }
                .map_err(|thrown| rethrow(from, to, thrown))?;
            converted.push(arg);
        }
        // SAFETY: as above.
        let result = ops::call(unsafe { &mut *from }, &callee, Value::Undefined, &converted)
            .map_err(|thrown| rethrow(from, to, thrown))?;
        // SAFETY: as above.
        unsafe { copy(from, to, &result, &mut Vec::new()) }
    });
    target
        .bridged_functions
        .push((original.clone(), proxy.clone()));
    proxy
}

/// A promise of `to` that settles as a promise of `from` does.
///
/// # Safety
///
/// As [`expose`].
unsafe fn promise(from: *mut Realm, to: *mut Realm, source: &Object) -> Result<Value, Thrown> {
    // SAFETY: as above.
    let (promise, resolve, reject) = crate::promise_builtin::capability(unsafe { &*to });
    let settle = move |settle: Value, reject: Value| {
        move |_: &mut Realm, _: Value, args: &[Value]| {
            let value = args.first().cloned().unwrap_or_default();
            // SAFETY: as above; reactions run only while both realms live.
            let (function, value) = match unsafe { copy(from, to, &value, &mut Vec::new()) } {
                Ok(value) => (settle.clone(), value),
                Err(thrown) => (reject.clone(), thrown.0),
            };
            // SAFETY: as above.
            ops::call(unsafe { &mut *to }, &function, Value::Undefined, &[value])?;
            Ok(Value::Undefined)
        }
    };
    // SAFETY: as above.
    let source_realm = unsafe { &mut *from };
    let fulfilled = source_realm.function("", 1, false, settle(resolve, reject.clone()));
    let rejected = source_realm.function("", 1, false, settle(reject.clone(), reject));
    let receiver = Value::Object(source.clone());
    let then = ops::get(source_realm, &receiver, &Key::name("then"))
        .map_err(|thrown| rethrow(from, to, thrown))?;
    ops::call(
        source_realm,
        &then,
        receiver,
        &[Value::Object(fulfilled), Value::Object(rejected)],
    )
    .map_err(|thrown| rethrow(from, to, thrown))?;
    Ok(promise)
}

#[cfg(test)]
mod tests;
