//! Native Symbol construction, methods and agent-local registry. Conversions
//! run before registry borrowing, so reentrant user callbacks remain valid.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::table::HashMap;
use crate::{
    Completion, ErrorKind, JsStr, Key, Kind, Property, Realm, Slot, Symbol, Thrown, Value, ops,
};
/// The agent's `Symbol.for` registry.
#[derive(Debug, Default)]
pub(crate) struct Registry {
    symbols: HashMap<JsStr, Symbol>,
    keys: HashMap<Symbol, JsStr>,
    private: HashMap<JsStr, Symbol>,
}

fn fixed(value: Value, configurable: bool) -> Property {
    Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: false,
        configurable,
    }
}

pub(crate) fn install(realm: &Realm) {
    let function = realm.function("Symbol", 0, false, create);
    let prototype = &realm.symbol_prototype;
    function.define(
        Key::name("prototype"),
        fixed(Value::Object(prototype.clone()), false),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(function.clone())),
    );
    for (name, symbol) in realm.symbols.entries() {
        function.define(Key::name(name), fixed(Value::Symbol(symbol.clone()), false));
    }
    realm.method(&function, "for", 1, registered);
    realm.method(&function, "keyFor", 1, key_for);
    realm.method(prototype, "toString", 0, |realm, this, _| {
        Ok(Value::String(symbol_data(realm, &this)?.descriptive()))
    });
    realm.method(prototype, "valueOf", 0, primitive);
    let description = realm.function("get description", 0, false, |realm, this, _| {
        Ok(symbol_data(realm, &this)?
            .description()
            .cloned()
            .map(Value::String)
            .unwrap_or_default())
    });
    prototype.define(
        Key::name("description"),
        Property {
            slot: Slot::Accessor {
                get: Some(description),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let to_primitive = realm.function("[Symbol.toPrimitive]", 1, false, primitive);
    prototype.define(
        Key::Symbol(realm.symbols.to_primitive.clone()),
        fixed(Value::Object(to_primitive), true),
    );
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        fixed(Value::str("Symbol"), true),
    );
    realm.set_global("Symbol", Value::Object(function));
}

fn symbol_data(realm: &Realm, value: &Value) -> Result<Symbol, Thrown> {
    if let Value::Symbol(symbol) = value {
        return Ok(symbol.clone());
    }
    if let Value::Object(object) = value
        && let Kind::SymbolObject(symbol) = &object.data().kind
    {
        return Ok(symbol.clone());
    }
    Err(realm.throw(
        ErrorKind::Type,
        "Symbol method called on an incompatible receiver",
    ))
}

fn primitive(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    symbol_data(realm, &this).map(Value::Symbol)
}

fn create(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let description = match args.first() {
        None | Some(Value::Undefined) => None,
        Some(value) => Some(ops::to_string(realm, value)?),
    };
    Ok(Value::Symbol(Symbol::new(description)))
}

fn registered(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let key = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;
    Ok(Value::Symbol(intern(realm, key)))
}

/// Shared registry identity for native bindings and compiled Symbol.for calls.
pub(crate) fn intern(realm: &Realm, key: JsStr) -> Symbol {
    let mut registry = realm.registry.borrow_mut();
    if let Some(symbol) = registry.symbols.get(&key) {
        return symbol.clone();
    }
    let symbol = Symbol::new(Some(key.clone()));
    registry.symbols.insert(key.clone(), symbol.clone());
    registry.keys.insert(symbol.clone(), key);
    symbol
}

/// Node's private symbols share agent identity without entering Symbol.for.
pub(crate) fn private_intern(realm: &Realm, description: &str) -> Symbol {
    let key = JsStr::from(description);
    let mut registry = realm.registry.borrow_mut();
    if let Some(symbol) = registry.private.get(&key) {
        return symbol.clone();
    }
    let symbol = Symbol::private(description);
    registry.private.insert(key, symbol.clone());
    symbol
}

/// Whether `Symbol.for` created the symbol; such symbols cannot be held weakly.
pub(crate) fn is_registered(realm: &Realm, symbol: &Symbol) -> bool {
    realm.registry.borrow().keys.get(symbol).is_some()
}

fn key_for(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let Some(Value::Symbol(symbol)) = args.first() else {
        return Err(realm.throw(ErrorKind::Type, "Symbol.keyFor requires a symbol"));
    };
    Ok(realm
        .registry
        .borrow()
        .keys
        .get(symbol)
        .cloned()
        .map(Value::String)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests;
