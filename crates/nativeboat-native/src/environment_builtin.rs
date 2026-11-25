//! Native Windows environment operations on a stable data handle selected by
//! the compiler. No process facade, module loader or engine is installed.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    Descriptor, ErrorKind, Key, Kind, Object, Property, Realm, Thrown, Value, ops, platform,
};

/// A stable environment data handle. Its own properties are read from Windows,
/// so aliases, dynamic keys and snapshot copies all observe live native state.
pub fn environment(realm: &mut Realm) -> Value {
    if let Some(environment) = &realm.process_environment {
        return Value::Object(environment.clone());
    }
    let constructor = realm.function("", 0, true, |realm, receiver, _| {
        if receiver.is_nullish() {
            Ok(Value::Object(realm.global.clone()))
        } else {
            ops::to_object(realm, &receiver).map(Value::Object)
        }
    });
    let prototype = ops::get_named(realm, &Value::Object(constructor), "prototype")
        .expect("new constructor has a prototype")
        .as_object()
        .expect("constructor prototype is an object")
        .clone();
    let environment = Object::new(Some(prototype), Kind::Environment);
    realm.process_environment = Some(environment.clone());
    Value::Object(environment)
}

pub(crate) fn property(key: &Key) -> Option<Property> {
    if matches!(key, Key::Symbol(_)) {
        return None;
    }
    platform::environment_value(&key.to_js()).map(|value| Property::data(Value::String(value)))
}

pub(crate) fn keys() -> Vec<Key> {
    let mut indexes = Vec::new();
    let mut strings = Vec::new();
    for name in platform::environment_keys() {
        match Key::from_js(&name) {
            Key::Index(index) => indexes.push(index),
            name => strings.push(name),
        }
    }
    indexes.sort_unstable();
    indexes.into_iter().map(Key::Index).chain(strings).collect()
}

pub(crate) fn set(realm: &mut Realm, key: &Key, value: &Value) -> Result<(), Thrown> {
    if matches!(key, Key::Symbol(_)) {
        return Err(realm.throw(ErrorKind::Type, "Cannot convert a Symbol value to a string"));
    }
    let value = ops::to_string(realm, value)?;
    platform::set_environment(&key.to_js(), Some(&value));
    Ok(())
}

pub(crate) fn delete(key: &Key) {
    if !matches!(key, Key::Symbol(_)) {
        platform::set_environment(&key.to_js(), None);
    }
}

pub(crate) fn define(realm: &mut Realm, key: &Key, descriptor: &Descriptor) -> Result<(), Thrown> {
    if !descriptor.is_accessor()
        && descriptor.writable == Some(true)
        && descriptor.enumerable == Some(true)
        && descriptor.configurable == Some(true)
        && let Some(value) = &descriptor.value
    {
        return set(realm, key, value);
    }
    let message = if descriptor.is_accessor() {
        "'process.env' does not accept an accessor(getter/setter) descriptor"
    } else {
        "'process.env' only accepts a configurable, writable, and enumerable data descriptor"
    };
    let error = realm.error(ErrorKind::Type, message);
    error.define(
        Key::name("code"),
        Property::data(Value::str("ERR_INVALID_OBJECT_DEFINE_PROPERTY")),
    );
    Err(Thrown(Value::Object(error)))
}

#[cfg(test)]
mod tests;
