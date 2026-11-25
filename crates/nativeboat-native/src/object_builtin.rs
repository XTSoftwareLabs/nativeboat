//! The native `Object` constructor, its static functions and the
//! `Object.prototype` methods, over the native object model.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::iter::iterate;
use crate::object::{Descriptor, Kind, Wrapped, same_value};
use crate::{Completion, ErrorKind, JsStr, Key, Object, Property, Realm, Slot, Thrown, Value, ops};
use alloc::rc::Rc;
use core::cell::RefCell;

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn type_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Type, message)
}

/// The object argument many statics require.
fn require_object(realm: &Realm, value: &Value, function: &str) -> Result<Object, Thrown> {
    match value {
        Value::Object(object) => Ok(object.clone()),
        _ => Err(type_error(
            realm,
            &format!("Object.{function} called on non-object"),
        )),
    }
}

/// A prototype argument: an object or null.
fn prototype_argument(realm: &Realm, value: &Value) -> Result<Option<Object>, Thrown> {
    match value {
        Value::Object(object) => Ok(Some(object.clone())),
        Value::Null => Ok(None),
        _ => Err(type_error(
            realm,
            "Object prototype may only be an Object or null",
        )),
    }
}

/// `ToPropertyDescriptor`, reading fields in the language's order.
pub(crate) fn to_descriptor(realm: &mut Realm, value: &Value) -> Result<Descriptor, Thrown> {
    let Value::Object(object) = value else {
        return Err(type_error(realm, "Property description must be an object"));
    };
    let mut descriptor = Descriptor::default();
    let target = Value::Object(object.clone());
    let field = |realm: &mut Realm, name: &str| -> Result<Option<Value>, Thrown> {
        let key = Key::name(name);
        if ops::has_property(realm, &key, &target)? {
            Ok(Some(ops::get(realm, &target, &key)?))
        } else {
            Ok(None)
        }
    };
    descriptor.enumerable = field(realm, "enumerable")?.map(|value| ops::to_boolean(&value));
    descriptor.configurable = field(realm, "configurable")?.map(|value| ops::to_boolean(&value));
    descriptor.value = field(realm, "value")?;
    descriptor.writable = field(realm, "writable")?.map(|value| ops::to_boolean(&value));
    for (name, slot) in [("get", 0), ("set", 1)] {
        if let Some(function) = field(realm, name)? {
            let function = match function {
                Value::Undefined => None,
                Value::Object(object) if object.code().is_some() => Some(object),
                _ => {
                    return Err(type_error(
                        realm,
                        &format!("Property descriptor {name} must be a function"),
                    ));
                }
            };
            if slot == 0 {
                descriptor.get = Some(function);
            } else {
                descriptor.set = Some(function);
            }
        }
    }
    if descriptor.is_accessor() && descriptor.is_data() {
        return Err(type_error(
            realm,
            "Invalid property descriptor. Cannot both specify accessors and a value or writable attribute",
        ));
    }
    Ok(descriptor)
}

/// `FromPropertyDescriptor`.
pub(crate) fn from_descriptor(realm: &Realm, property: &Property) -> Value {
    let object = realm.object();
    let put = |name: &str, value: Value| {
        object.define(Key::name(name), Property::data(value));
    };
    let function = |value: &Option<Object>| value.clone().map(Value::Object).unwrap_or_default();
    match &property.slot {
        Slot::Data { value, writable } => {
            put("value", value.clone());
            put("writable", Value::Boolean(*writable));
        }
        Slot::Accessor { get, set } => {
            put("get", function(get));
            put("set", function(set));
        }
    }
    put("enumerable", Value::Boolean(property.enumerable));
    put("configurable", Value::Boolean(property.configurable));
    Value::Object(object)
}

/// `FromPropertyDescriptor` of a partial descriptor: only its fields.
pub(crate) fn from_partial_descriptor(realm: &Realm, descriptor: &Descriptor) -> Value {
    let object = realm.object();
    let put = |name: &str, value: Value| {
        object.define(Key::name(name), Property::data(value));
    };
    let function = |value: &Option<Object>| value.clone().map(Value::Object).unwrap_or_default();
    if let Some(value) = &descriptor.value {
        put("value", value.clone());
    }
    if let Some(writable) = descriptor.writable {
        put("writable", Value::Boolean(writable));
    }
    if let Some(get) = &descriptor.get {
        put("get", function(get));
    }
    if let Some(set) = &descriptor.set {
        put("set", function(set));
    }
    if let Some(enumerable) = descriptor.enumerable {
        put("enumerable", Value::Boolean(enumerable));
    }
    if let Some(configurable) = descriptor.configurable {
        put("configurable", Value::Boolean(configurable));
    }
    Value::Object(object)
}

/// What `keys`, `values` and `entries` produce.
#[derive(Clone, Copy)]
enum Produce {
    Keys,
    Values,
    Entries,
}

/// `EnumerableOwnProperties`: string keys whose properties are still
/// enumerable when reached.
fn enumerable_own(realm: &mut Realm, value: &Value, produce: Produce) -> Completion {
    let object = ops::to_object(realm, value)?;
    let target = Value::Object(object.clone());
    let mut results = Vec::new();
    for key in ops::own_property_keys(realm, &object)? {
        if matches!(key, Key::Symbol(_))
            || !ops::get_own_property(realm, &object, &key)?
                .is_some_and(|property| property.enumerable)
        {
            continue;
        }
        let name = Value::String(key.to_js());
        match produce {
            Produce::Keys => results.push(name),
            Produce::Values => results.push(ops::get(realm, &target, &key)?),
            Produce::Entries => {
                let value = ops::get(realm, &target, &key)?;
                results.push(Value::Object(realm.array(vec![name, value])));
            }
        }
    }
    Ok(Value::Object(realm.array(results)))
}

fn assign(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let target = Value::Object(ops::to_object(realm, &arg(args, 0))?);
    for source in args.iter().skip(1) {
        if source.is_nullish() {
            continue;
        }
        let from = ops::to_object(realm, source)?;
        let source = Value::Object(from.clone());
        for key in ops::own_property_keys(realm, &from)? {
            if ops::get_own_property(realm, &from, &key)?
                .is_some_and(|property| property.enumerable)
            {
                let value = ops::get(realm, &source, &key)?;
                ops::set(realm, &target, &key, value)?;
            }
        }
    }
    Ok(target)
}

/// `ObjectDefineProperties`: every descriptor is read before any is applied.
fn define_all(realm: &mut Realm, object: &Object, properties: &Value) -> Result<(), Thrown> {
    let source = ops::to_object(realm, properties)?;
    let source_value = Value::Object(source.clone());
    let mut descriptors = Vec::new();
    for key in ops::own_property_keys(realm, &source)? {
        if ops::get_own_property(realm, &source, &key)?.is_some_and(|property| property.enumerable)
        {
            let value = ops::get(realm, &source_value, &key)?;
            descriptors.push((key, to_descriptor(realm, &value)?));
        }
    }
    for (key, descriptor) in descriptors {
        ops::define_property(realm, object, &key, descriptor)?;
    }
    Ok(())
}

fn create(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let prototype = prototype_argument(realm, &arg(args, 0))?;
    let object = Object::new(prototype, Kind::Ordinary);
    let properties = arg(args, 1);
    if !matches!(properties, Value::Undefined) {
        define_all(realm, &object, &properties)?;
    }
    Ok(Value::Object(object))
}

/// The integrity levels of `freeze`/`seal` and their tests.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Sealed,
    Frozen,
}

fn set_level(realm: &mut Realm, value: Value, level: Level) -> Completion {
    let Value::Object(object) = &value else {
        return Ok(value);
    };
    if matches!(object.data().kind, Kind::Environment) {
        return Err(type_error(
            realm,
            if level == Level::Frozen {
                "Cannot freeze"
            } else {
                "Cannot seal"
            },
        ));
    }
    if !ops::prevent_extensions(realm, object)? {
        return Err(type_error(
            realm,
            if level == Level::Frozen {
                "Cannot freeze"
            } else {
                "Cannot seal"
            },
        ));
    }
    for key in ops::own_property_keys(realm, object)? {
        let accessor = matches!(
            ops::get_own_property(realm, object, &key)?.map(|property| property.slot),
            Some(Slot::Accessor { .. })
        );
        let descriptor = Descriptor {
            configurable: Some(false),
            writable: (level == Level::Frozen && !accessor).then_some(false),
            ..Descriptor::default()
        };
        ops::define_property(realm, object, &key, descriptor)?;
    }
    Ok(value)
}

fn test_level(realm: &mut Realm, value: &Value, level: Level) -> Result<bool, Thrown> {
    let Value::Object(object) = value else {
        return Ok(true);
    };
    if ops::is_extensible(realm, object)? {
        return Ok(false);
    }
    for key in ops::own_property_keys(realm, object)? {
        if ops::get_own_property(realm, object, &key)?.is_some_and(|property| {
            property.configurable
                || (level == Level::Frozen
                    && matches!(property.slot, Slot::Data { writable: true, .. }))
        }) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn from_entries(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let iterable = arg(args, 0);
    if iterable.is_nullish() {
        return Err(type_error(realm, "Object.fromEntries requires an iterable"));
    }
    let object = realm.object();
    let mut iteration = iterate(realm, &iterable)?;
    while let Some(entry) = iteration.step(realm)? {
        let added = (|| -> Result<(), Thrown> {
            if !matches!(entry, Value::Object(_)) {
                return Err(type_error(realm, "Iterator value is not an entry object"));
            }
            let key = ops::get(realm, &entry, &Key::Index(0))?;
            let value = ops::get(realm, &entry, &Key::Index(1))?;
            let key = ops::to_key(realm, &key)?;
            object.define(key, Property::data(value));
            Ok(())
        })();
        if let Err(thrown) = added {
            return Err(iteration.close_thrown(realm, thrown));
        }
    }
    Ok(Value::Object(object))
}

fn group_by(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let items = arg(args, 0);
    let callback = arg(args, 1);
    if items.is_nullish() {
        return Err(type_error(realm, "Object.groupBy requires an iterable"));
    }
    if !callback
        .as_object()
        .is_some_and(|object| object.code().is_some())
    {
        return Err(type_error(
            realm,
            "Object.groupBy callback is not a function",
        ));
    }
    let groups = Object::new(None, Kind::Ordinary);
    let mut iteration = iterate(realm, &items)?;
    let mut index = 0u32;
    while let Some(value) = iteration.step(realm)? {
        let grouped = (|| -> Result<(), Thrown> {
            let arguments = [value.clone(), Value::Number(f64::from(index))];
            let key = ops::call(realm, &callback, Value::Undefined, &arguments)?;
            let key = ops::to_key(realm, &key)?;
            match groups.own_property(&key) {
                Some(Property {
                    slot: Slot::Data { value: list, .. },
                    ..
                }) => {
                    let list = list.as_object().expect("groups are arrays").clone();
                    let length = list.own_keys().len() as u32 - 1;
                    list.define(Key::Index(length), Property::data(value.clone()));
                }
                _ => {
                    let list = realm.array(vec![value.clone()]);
                    groups.define(key, Property::data(Value::Object(list)));
                }
            }
            Ok(())
        })();
        if let Err(thrown) = grouped {
            return Err(iteration.close_thrown(realm, thrown));
        }
        index += 1;
    }
    Ok(Value::Object(groups))
}

fn set_prototype_of(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let target = arg(args, 0);
    if target.is_nullish() {
        return Err(type_error(
            realm,
            "Object.setPrototypeOf called on null or undefined",
        ));
    }
    let prototype = prototype_argument(realm, &arg(args, 1))?;
    if let Value::Object(object) = &target
        && !ops::set_prototype_of(realm, object, prototype)?
    {
        return Err(type_error(realm, "Cannot set the prototype of this object"));
    }
    Ok(target)
}

fn keys_of(realm: &mut Realm, value: &Value, symbols: bool) -> Completion {
    let object = ops::to_object(realm, value)?;
    let keys = ops::own_property_keys(realm, &object)?
        .into_iter()
        .filter(|key| matches!(key, Key::Symbol(_)) == symbols)
        .map(|key| match key {
            Key::Symbol(symbol) => Value::Symbol(symbol),
            key => Value::String(key.to_js()),
        })
        .collect();
    Ok(Value::Object(realm.array(keys)))
}

fn install_statics(realm: &Realm, constructor: &Object) {
    realm.method(constructor, "assign", 2, assign);
    realm.method(constructor, "create", 2, create);
    realm.method(constructor, "defineProperties", 2, |realm, _, args| {
        let object = require_object(realm, &arg(args, 0), "defineProperties")?;
        define_all(realm, &object, &arg(args, 1))?;
        Ok(Value::Object(object))
    });
    realm.method(constructor, "defineProperty", 3, |realm, _, args| {
        let object = require_object(realm, &arg(args, 0), "defineProperty")?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        let descriptor = to_descriptor(realm, &arg(args, 2))?;
        ops::define_property(realm, &object, &key, descriptor)?;
        Ok(Value::Object(object))
    });
    realm.method(constructor, "entries", 1, |realm, _, args| {
        enumerable_own(realm, &arg(args, 0), Produce::Entries)
    });
    realm.method(constructor, "keys", 1, |realm, _, args| {
        enumerable_own(realm, &arg(args, 0), Produce::Keys)
    });
    realm.method(constructor, "values", 1, |realm, _, args| {
        enumerable_own(realm, &arg(args, 0), Produce::Values)
    });
    realm.method(constructor, "freeze", 1, |realm, _, args| {
        set_level(realm, arg(args, 0), Level::Frozen)
    });
    realm.method(constructor, "seal", 1, |realm, _, args| {
        set_level(realm, arg(args, 0), Level::Sealed)
    });
    realm.method(constructor, "isFrozen", 1, |realm, _, args| {
        test_level(realm, &arg(args, 0), Level::Frozen).map(Value::Boolean)
    });
    realm.method(constructor, "isSealed", 1, |realm, _, args| {
        test_level(realm, &arg(args, 0), Level::Sealed).map(Value::Boolean)
    });
    realm.method(constructor, "isExtensible", 1, |realm, _, args| {
        match arg(args, 0) {
            Value::Object(object) => ops::is_extensible(realm, &object).map(Value::Boolean),
            _ => Ok(Value::Boolean(false)),
        }
    });
    realm.method(constructor, "preventExtensions", 1, |realm, _, args| {
        let value = arg(args, 0);
        if let Value::Object(object) = &value
            && !ops::prevent_extensions(realm, object)?
        {
            return Err(type_error(realm, "Cannot prevent extensions"));
        }
        Ok(value)
    });
    realm.method(constructor, "fromEntries", 1, from_entries);
    realm.method(constructor, "groupBy", 2, group_by);
    realm.method(
        constructor,
        "getOwnPropertyDescriptor",
        2,
        |realm, _, args| {
            let object = ops::to_object(realm, &arg(args, 0))?;
            let key = ops::to_key(realm, &arg(args, 1))?;
            Ok(ops::get_own_property(realm, &object, &key)?
                .map(|property| from_descriptor(realm, &property))
                .unwrap_or_default())
        },
    );
    realm.method(
        constructor,
        "getOwnPropertyDescriptors",
        1,
        |realm, _, args| {
            let object = ops::to_object(realm, &arg(args, 0))?;
            let result = realm.object();
            for key in ops::own_property_keys(realm, &object)? {
                if let Some(property) = ops::get_own_property(realm, &object, &key)? {
                    result.define(key, Property::data(from_descriptor(realm, &property)));
                }
            }
            Ok(Value::Object(result))
        },
    );
    realm.method(constructor, "getOwnPropertyNames", 1, |realm, _, args| {
        keys_of(realm, &arg(args, 0), false)
    });
    realm.method(constructor, "getOwnPropertySymbols", 1, |realm, _, args| {
        keys_of(realm, &arg(args, 0), true)
    });
    realm.method(constructor, "getPrototypeOf", 1, |realm, _, args| {
        let object = ops::to_object(realm, &arg(args, 0))?;
        let prototype = ops::get_prototype_of(realm, &object)?;
        Ok(prototype.map(Value::Object).unwrap_or(Value::Null))
    });
    realm.method(constructor, "setPrototypeOf", 2, set_prototype_of);
    realm.method(constructor, "hasOwn", 2, |realm, _, args| {
        let object = ops::to_object(realm, &arg(args, 0))?;
        let key = ops::to_key(realm, &arg(args, 1))?;
        Ok(Value::Boolean(
            ops::get_own_property(realm, &object, &key)?.is_some(),
        ))
    });
    realm.method(constructor, "is", 2, |_, _, args| {
        Ok(Value::Boolean(same_value(&arg(args, 0), &arg(args, 1))))
    });
}

/// `Object.prototype.toString`.
pub(crate) fn to_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let builtin = match &this {
        Value::Undefined => return Ok(Value::str("[object Undefined]")),
        Value::Null => return Ok(Value::str("[object Null]")),
        _ => {
            let object = ops::to_object(realm, &this)?;
            let array = crate::proxy_builtin::is_array(realm, &object)?;
            let tag = match &object.data().kind {
                _ if array => "Array",
                Kind::Function { .. }
                | Kind::BuiltinConstructor { .. }
                | Kind::BoundFunction { .. } => "Function",
                Kind::Proxy(_) if object.code().is_some() => "Function",
                Kind::Error => "Error",
                Kind::RegExp(_) => "RegExp",
                Kind::Date(_) => "Date",
                Kind::Arguments => "Arguments",
                Kind::Primitive(Wrapped::Boolean(_)) => "Boolean",
                Kind::Primitive(Wrapped::Number(_)) => "Number",
                Kind::Primitive(Wrapped::String(_)) => "String",
                _ => "Object",
            };
            (object, tag)
        }
    };
    let (object, tag) = builtin;
    let key = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = match ops::get(realm, &Value::Object(object), &key)? {
        Value::String(tag) => tag,
        _ => JsStr::from(tag),
    };
    Ok(Value::String(
        JsStr::from("[object ")
            .concat(&tag)
            .concat(&JsStr::from("]")),
    ))
}

/// Annex B `__defineGetter__`/`__defineSetter__`.
fn define_accessor_method(getter: bool) -> impl Fn(&mut Realm, Value, &[Value]) -> Completion {
    move |realm, this, args| {
        let object = ops::to_object(realm, &this)?;
        let function = arg(args, 1);
        let Some(function) = function
            .as_object()
            .filter(|object| object.code().is_some())
        else {
            return Err(type_error(realm, "Accessor must be a function"));
        };
        let descriptor = Descriptor {
            get: getter.then(|| Some(function.clone())),
            set: (!getter).then(|| Some(function.clone())),
            enumerable: Some(true),
            configurable: Some(true),
            ..Descriptor::default()
        };
        let key = ops::to_key(realm, &arg(args, 0))?;
        ops::define_property(realm, &object, &key, descriptor)?;
        Ok(Value::Undefined)
    }
}

/// Annex B `__lookupGetter__`/`__lookupSetter__`.
fn lookup_accessor_method(getter: bool) -> impl Fn(&mut Realm, Value, &[Value]) -> Completion {
    move |realm, this, args| {
        let object = ops::to_object(realm, &this)?;
        let key = ops::to_key(realm, &arg(args, 0))?;
        let mut current = Some(object);
        while let Some(object) = current {
            if let Some(property) = ops::get_own_property(realm, &object, &key)? {
                return Ok(match property.slot {
                    Slot::Accessor { get, set } => {
                        let function = if getter { get } else { set };
                        function.map(Value::Object).unwrap_or_default()
                    }
                    Slot::Data { .. } => Value::Undefined,
                });
            }
            current = ops::get_prototype_of(realm, &object)?;
        }
        Ok(Value::Undefined)
    }
}

fn install_prototype(realm: &Realm) {
    let prototype = realm.object_prototype.clone();
    realm.method(&prototype, "toString", 0, to_string);
    realm.method(&prototype, "toLocaleString", 0, |realm, this, _| {
        let method = ops::get_named(realm, &this, "toString")?;
        ops::call(realm, &method, this, &[])
    });
    realm.method(&prototype, "valueOf", 0, |realm, this, _| {
        Ok(Value::Object(ops::to_object(realm, &this)?))
    });
    realm.method(&prototype, "hasOwnProperty", 1, |realm, this, args| {
        let key = ops::to_key(realm, &arg(args, 0))?;
        let object = ops::to_object(realm, &this)?;
        Ok(Value::Boolean(
            ops::get_own_property(realm, &object, &key)?.is_some(),
        ))
    });
    realm.method(&prototype, "isPrototypeOf", 1, |realm, this, args| {
        let Value::Object(value) = arg(args, 0) else {
            return Ok(Value::Boolean(false));
        };
        let object = ops::to_object(realm, &this)?;
        let mut current = ops::get_prototype_of(realm, &value)?;
        while let Some(candidate) = current {
            if candidate == object {
                return Ok(Value::Boolean(true));
            }
            current = ops::get_prototype_of(realm, &candidate)?;
        }
        Ok(Value::Boolean(false))
    });
    realm.method(
        &prototype,
        "propertyIsEnumerable",
        1,
        |realm, this, args| {
            let key = ops::to_key(realm, &arg(args, 0))?;
            let object = ops::to_object(realm, &this)?;
            Ok(Value::Boolean(
                ops::get_own_property(realm, &object, &key)?
                    .is_some_and(|property| property.enumerable),
            ))
        },
    );
    realm.method(
        &prototype,
        "__defineGetter__",
        2,
        define_accessor_method(true),
    );
    realm.method(
        &prototype,
        "__defineSetter__",
        2,
        define_accessor_method(false),
    );
    realm.method(
        &prototype,
        "__lookupGetter__",
        1,
        lookup_accessor_method(true),
    );
    realm.method(
        &prototype,
        "__lookupSetter__",
        1,
        lookup_accessor_method(false),
    );
    let get = realm.function("get __proto__", 0, false, |realm, this, _| {
        let object = ops::to_object(realm, &this)?;
        let prototype = ops::get_prototype_of(realm, &object)?;
        Ok(prototype.map(Value::Object).unwrap_or(Value::Null))
    });
    let set = realm.function("set __proto__", 1, false, |realm, this, args| {
        if this.is_nullish() {
            return Err(type_error(
                realm,
                "Object.prototype.__proto__ called on null or undefined",
            ));
        }
        let prototype = match arg(args, 0) {
            Value::Object(object) => Some(object),
            Value::Null => None,
            _ => return Ok(Value::Undefined),
        };
        if let Value::Object(object) = &this
            && !ops::set_prototype_of(realm, object, prototype)?
        {
            return Err(type_error(realm, "Cannot set the prototype of this object"));
        }
        Ok(Value::Undefined)
    });
    prototype.define(
        Key::name("__proto__"),
        Property {
            slot: Slot::Accessor {
                get: Some(get),
                set: Some(set),
            },
            enumerable: false,
            configurable: true,
        },
    );
}

pub(crate) fn install(realm: &Realm) {
    let prototype = realm.object_prototype.clone();
    // Construction compares the new target with this constructor, which is
    // recorded once it exists. Intrinsics live as long as their realm.
    let itself: Rc<RefCell<Option<Object>>> = Rc::default();
    let target = itself.clone();
    let constructor = realm.builtin_constructor(
        "Object",
        1,
        &prototype,
        |realm, _, args| Ok(Value::Object(object_call(realm, &arg(args, 0))?)),
        move |realm, new_target, args| {
            if target.borrow().as_ref() == Some(&new_target) {
                return object_call(realm, &arg(args, 0));
            }
            let prototype = match ops::get_named(realm, &Value::Object(new_target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => realm.object_prototype.clone(),
            };
            Ok(Object::new(Some(prototype), Kind::Ordinary))
        },
    );
    *itself.borrow_mut() = Some(constructor.clone());
    install_statics(realm, &constructor);
    install_prototype(realm);
    realm.set_global("Object", Value::Object(constructor));
}

/// `Object(value)`: a new object for null/undefined, otherwise ToObject.
fn object_call(realm: &mut Realm, value: &Value) -> Result<Object, Thrown> {
    if value.is_nullish() {
        return Ok(realm.object());
    }
    ops::to_object(realm, value)
}

#[cfg(test)]
mod tests;
