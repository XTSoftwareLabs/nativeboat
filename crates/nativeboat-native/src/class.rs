//! Native class semantics: class constructors, derived-constructor `this`
//! initialization through `super()`, home-object `super` property access,
//! instance field initialization and private elements.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{ConstructorCode, Descriptor};
use crate::{
    Completion, ErrorKind, Key, Kind, Object, Property, Realm, Slot, Symbol, Thrown, Value, ops,
};
use alloc::rc::Rc;
use core::cell::RefCell;

/// The state of one derived-constructor activation: its `this` stays
/// uninitialized until `super()` returns.
#[derive(Debug)]
pub struct Frame {
    function: Object,
    new_target: Object,
    fields: Option<Object>,
    /// Whether the constructor initializes the fields itself after
    /// `super()`, once it has assigned its parameter properties, as
    /// TypeScript's `useDefineForClassFields: false` orders them.
    deferred: bool,
    this: RefCell<Option<Object>>,
}

/// A shared derived-constructor activation; arrow functions inside the
/// constructor share it.
pub type FrameRef = Rc<Frame>;

/// Starts a derived-constructor activation.
pub fn frame(function: Object, new_target: Object, fields: Option<Object>) -> FrameRef {
    Rc::new(Frame {
        function,
        new_target,
        fields,
        deferred: false,
        this: RefCell::new(None),
    })
}

/// Starts a derived-constructor activation whose `super()` leaves the
/// fields to [`initialize_deferred`].
pub fn frame_deferred(function: Object, new_target: Object, fields: Option<Object>) -> FrameRef {
    Rc::new(Frame {
        function,
        new_target,
        fields,
        deferred: true,
        this: RefCell::new(None),
    })
}

/// Initializes the fields of a deferring activation's `this`, after
/// `super()` and the constructor's parameter properties.
pub fn initialize_deferred(realm: &mut Realm, frame: &Frame, this: &Value) -> Result<(), Thrown> {
    initialize(realm, frame.fields.as_ref(), this)
}

/// Reads `this` in a derived constructor.
pub fn this_value(realm: &Realm, frame: &Frame) -> Completion {
    frame.this.borrow().clone().map(Value::Object).ok_or_else(|| {
        realm.throw(
            ErrorKind::Reference,
            "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
        )
    })
}

/// `super(...args)`: constructs the parent with the current new target,
/// binds `this` once and initializes the class's fields on it.
pub fn super_call(realm: &mut Realm, frame: &Frame, args: &[Value]) -> Completion {
    let parent = frame.function.data().prototype.clone();
    let parent = match parent {
        Some(parent) if parent.code().is_some_and(|(_, constructor)| constructor) => parent,
        _ => {
            return Err(realm.throw(ErrorKind::Type, "Super constructor is not a constructor"));
        }
    };
    let new_target = Value::Object(frame.new_target.clone());
    let result = ops::construct_target(realm, &Value::Object(parent), args, &new_target)?;
    if frame.this.borrow().is_some() {
        return Err(realm.throw(
            ErrorKind::Reference,
            "Super constructor may only be called once",
        ));
    }
    let object = result
        .as_object()
        .expect("construction returns an object")
        .clone();
    *frame.this.borrow_mut() = Some(object);
    if !frame.deferred {
        initialize(realm, frame.fields.as_ref(), &result)?;
    }
    Ok(result)
}

/// The end of a derived constructor: an explicitly returned object, or the
/// initialized `this`.
pub fn finish(realm: &Realm, frame: &Frame, returned: Value) -> Result<Object, Thrown> {
    match returned {
        Value::Object(object) => Ok(object),
        Value::Undefined => frame.this.borrow().clone().ok_or_else(|| {
            realm.throw(
                ErrorKind::Reference,
                "Must call super constructor in derived class before accessing 'this' or returning from derived constructor",
            )
        }),
        _ => Err(realm.throw(
            ErrorKind::Type,
            "Derived constructors may only return object or undefined",
        )),
    }
}

/// A member decorator of TypeScript's `experimentalDecorators`: one of the
/// member's own, or one of a method's parameters with the parameter's index.
#[derive(Debug, Clone)]
pub enum MemberDecorator {
    /// `@decorator` on the member.
    Member(Value),
    /// `@decorator` on parameter `index`, as `tsc`'s `__param(index, d)`.
    Parameter(u32, Value),
}

fn key_value(key: &Key) -> Value {
    match key {
        Key::Symbol(symbol) => Value::Symbol(symbol.clone()),
        other => Value::String(other.to_js()),
    }
}

/// Applies the decorators of a class member as `tsc`'s `__decorate` does
/// with TypeScript's `experimentalDecorators`: last first, each called with
/// the target (the prototype, or the class for static members), the key and
/// the descriptor so far, a returned value replacing the descriptor; then
/// the descriptor is defined. Methods and accessors start from their own
/// property's descriptor, fields from none, so a field is only defined when
/// a decorator returns a descriptor.
pub fn decorate_member(
    realm: &mut Realm,
    target: &Value,
    key: Key,
    decorators: &[MemberDecorator],
    method: bool,
) -> Result<(), Thrown> {
    let object = target
        .as_object()
        .expect("decorated members belong to objects")
        .clone();
    let name = key_value(&key);
    let mut descriptor = if method {
        ops::get_own_property(realm, &object, &key)?
            .map(|property| crate::object_builtin::from_descriptor(realm, &property))
            .unwrap_or_default()
    } else {
        Value::Undefined
    };
    for decorator in decorators.iter().rev() {
        match decorator {
            MemberDecorator::Member(decorator) => {
                let result = ops::call(
                    realm,
                    decorator,
                    Value::Undefined,
                    &[target.clone(), name.clone(), descriptor.clone()],
                )?;
                if ops::to_boolean(&result) {
                    descriptor = result;
                }
            }
            MemberDecorator::Parameter(index, decorator) => {
                ops::call(
                    realm,
                    decorator,
                    Value::Undefined,
                    &[
                        target.clone(),
                        name.clone(),
                        Value::Number(f64::from(*index)),
                    ],
                )?;
            }
        }
    }
    if ops::to_boolean(&descriptor) {
        let descriptor = crate::object_builtin::to_descriptor(realm, &descriptor)?;
        ops::define_property(realm, &object, &key, descriptor)?;
    }
    Ok(())
}

/// Applies the decorators of a class's constructor parameters, last first,
/// each called with the class, `undefined` and the parameter's index, as
/// `tsc` applies them before the class's own decorators.
pub fn decorate_parameters(
    realm: &mut Realm,
    class: &Value,
    decorators: &[(u32, Value)],
) -> Result<(), Thrown> {
    for (index, decorator) in decorators.iter().rev() {
        ops::call(
            realm,
            decorator,
            Value::Undefined,
            &[
                class.clone(),
                Value::Undefined,
                Value::Number(f64::from(*index)),
            ],
        )?;
    }
    Ok(())
}

/// Runs a class's instance field initializer on a new instance.
pub fn initialize(realm: &mut Realm, fields: Option<&Object>, this: &Value) -> Result<(), Thrown> {
    if let Some(fields) = fields {
        ops::call(realm, &Value::Object(fields.clone()), this.clone(), &[])?;
    }
    Ok(())
}

/// The prototypes a class inherits: its prototype object's parent and its
/// constructor's parent (`ClassDefinitionEvaluation` heritage steps).
pub fn heritage(
    realm: &mut Realm,
    superclass: Option<&Value>,
) -> Result<(Option<Object>, Object), Thrown> {
    let Some(superclass) = superclass else {
        return Ok((
            Some(realm.object_prototype.clone()),
            realm.function_prototype.clone(),
        ));
    };
    match superclass {
        Value::Null => Ok((None, realm.function_prototype.clone())),
        Value::Object(parent) if parent.code().is_some_and(|(_, constructor)| constructor) => {
            match ops::get_named(realm, superclass, "prototype")? {
                Value::Object(prototype) => Ok((Some(prototype), parent.clone())),
                Value::Null => Ok((None, parent.clone())),
                _ => Err(realm.throw(
                    ErrorKind::Type,
                    "Class extends value does not have valid prototype property",
                )),
            }
        }
        _ => Err(realm.throw(
            ErrorKind::Type,
            "Class extends value is not a constructor or null",
        )),
    }
}

/// Creates a class constructor: calling it throws, construction runs
/// `construct` with the constructor itself and the new target, and it
/// inherits from `parent`.
pub fn create(
    realm: &Realm,
    name: &str,
    length: u32,
    prototype: &Object,
    parent: Object,
    construct: impl Fn(&mut Realm, &Object, Object, &[Value]) -> Result<Object, Thrown> + 'static,
) -> Object {
    let message = format!("Class constructor {name} cannot be invoked without 'new'");
    let function = realm.function(name, length, false, move |realm, _, _| {
        Err(realm.throw(ErrorKind::Type, &message))
    });
    let (code, _) = function.code().expect("new function has native code");
    // The construction code refers to its function without owning it.
    let weak = function.downgrade();
    let construct: ConstructorCode = Rc::new(move |realm, new_target, args| {
        let function = weak.upgrade().expect("a constructor being run is alive");
        construct(realm, &function, new_target, args)
    });
    {
        let mut data = function.data_mut();
        data.kind = Kind::BuiltinConstructor { code, construct };
        data.prototype = Some(parent);
    }
    function.define(
        crate::string::static_key!("prototype"),
        Property {
            slot: Slot::Data {
                value: Value::Object(prototype.clone()),
                writable: false,
            },
            enumerable: false,
            configurable: false,
        },
    );
    prototype.define(
        crate::string::static_key!("constructor"),
        Property::hidden(Value::Object(function.clone())),
    );
    function
}

/// `OrdinaryCreateFromConstructor(newTarget, "%Object.prototype%")`.
pub fn create_this(realm: &mut Realm, new_target: &Object) -> Result<Object, Thrown> {
    let prototype = match ops::get(
        realm,
        &Value::Object(new_target.clone()),
        &crate::string::static_key!("prototype"),
    )? {
        Value::Object(prototype) => prototype,
        _ => realm.object_prototype.clone(),
    };
    Ok(Object::new(Some(prototype), Kind::Ordinary))
}

/// The end of a base constructor: an explicitly returned object, or `this`.
pub fn finish_base(this: &Value, returned: Value) -> Object {
    match returned {
        Value::Object(object) => object,
        _ => this
            .as_object()
            .expect("base constructors bind an object")
            .clone(),
    }
}

/// A base class's implicit constructor: `constructor() {}`.
pub fn default_base(
    realm: &mut Realm,
    new_target: &Object,
    fields: Option<&Object>,
) -> Result<Object, Thrown> {
    let this = create_this(realm, new_target)?;
    initialize(realm, fields, &Value::Object(this.clone()))?;
    Ok(this)
}

/// A derived class's implicit constructor:
/// `constructor(...args) { super(...args); }`.
pub fn default_derived(realm: &mut Realm, frame: &Frame, args: &[Value]) -> Result<Object, Thrown> {
    super_call(realm, frame, args)?;
    finish(realm, frame, Value::Undefined)
}

/// Defines a class method: writable, configurable and not enumerable.
pub fn define_method(target: &Object, key: Key, function: Value) {
    target.define(key, Property::hidden(function));
}

/// Defines a class getter or setter, merging with the other half.
pub fn define_accessor(target: &Object, key: Key, get: Option<Value>, set: Option<Value>) {
    let get = get.and_then(|get| get.as_object().cloned());
    let set = set.and_then(|set| set.as_object().cloned());
    let (get, set) = match target.own_property(&key) {
        Some(Property {
            slot:
                Slot::Accessor {
                    get: old_get,
                    set: old_set,
                },
            ..
        }) => (get.or(old_get), set.or(old_set)),
        _ => (get, set),
    };
    target.define(
        key,
        Property {
            slot: Slot::Accessor { get, set },
            enumerable: false,
            configurable: true,
        },
    );
}

/// A private method element: a read-only function.
pub fn private_method(function: &Value) -> Property {
    Property {
        slot: Slot::Data {
            value: function.clone(),
            writable: false,
        },
        enumerable: false,
        configurable: false,
    }
}

/// A private accessor element.
pub fn private_accessor(get: Option<&Value>, set: Option<&Value>) -> Property {
    Property {
        slot: Slot::Accessor {
            get: get.and_then(Value::as_object).cloned(),
            set: set.and_then(Value::as_object).cloned(),
        },
        enumerable: false,
        configurable: false,
    }
}

/// A class field: `CreateDataPropertyOrThrow` on the instance.
pub fn define_field(realm: &Realm, target: &Value, key: Key, value: Value) -> Result<(), Thrown> {
    let object = target.as_object().expect("fields initialize objects");
    let descriptor = Descriptor {
        value: Some(value),
        writable: Some(true),
        enumerable: Some(true),
        configurable: Some(true),
        ..Descriptor::default()
    };
    if object.define_own_property(&key, &descriptor) {
        Ok(())
    } else {
        Err(realm.throw(
            ErrorKind::Type,
            &format!("Cannot define property {}", key.to_js()),
        ))
    }
}

/// `super[key]`: looks the key up from the home object's prototype with
/// `this` as the receiver.
pub fn super_get(realm: &mut Realm, home: &Object, this: &Value, key: &Key) -> Completion {
    let mut current = home.data().prototype.clone();
    while let Some(object) = current {
        if let Some(property) = object.own_property(key) {
            return match property.slot {
                Slot::Data { value, .. } => Ok(value),
                Slot::Accessor { get: Some(get), .. } => {
                    ops::call(realm, &Value::Object(get), this.clone(), &[])
                }
                Slot::Accessor { get: None, .. } => Ok(Value::Undefined),
            };
        }
        current = object.data().prototype.clone();
    }
    Ok(Value::Undefined)
}

/// `super[key] = value`: sets the key from the home object's prototype with
/// `this` as the receiver, so a setter there runs on `this` and a data
/// property lands on `this`. Class code is strict, so a refused set throws.
pub fn super_set(
    realm: &mut Realm,
    home: &Object,
    this: &Value,
    key: &Key,
    value: Value,
) -> Result<(), Thrown> {
    let Some(base) = home.data().prototype.clone() else {
        return Err(realm.throw(
            ErrorKind::Type,
            &format!("Cannot set property {} of null", key.to_js()),
        ));
    };
    if ops::set_with_receiver(realm, &base, key, value, this)? {
        Ok(())
    } else {
        Err(realm.throw(
            ErrorKind::Type,
            &format!("Cannot assign to read only property '{}'", key.to_js()),
        ))
    }
}

fn private_error(realm: &Realm, action: &str, name: &Symbol) -> Thrown {
    if let Some(owner) = name.member_owner() {
        return realm.throw(
            ErrorKind::Type,
            &format!("Receiver must be an instance of class {owner}"),
        );
    }
    realm.throw(
        ErrorKind::Type,
        &format!(
            "Cannot {action} private member {} from an object whose class did not declare it",
            name.description().cloned().unwrap_or_default()
        ),
    )
}

fn private_element(value: &Value, name: &Symbol) -> Option<(Object, Property)> {
    let object = value.as_object()?;
    let property = object.private_property(name)?;
    Some((object.clone(), property))
}

/// Reads a private field, method or accessor.
pub fn private_get(realm: &mut Realm, value: &Value, name: &Symbol) -> Completion {
    let Some((_, property)) = private_element(value, name) else {
        return Err(private_error(realm, "read", name));
    };
    match property.slot {
        Slot::Data { value, .. } => Ok(value),
        Slot::Accessor { get: Some(get), .. } => {
            ops::call(realm, &Value::Object(get), value.clone(), &[])
        }
        Slot::Accessor { get: None, .. } => Err(realm.throw(
            ErrorKind::Type,
            "Private accessor was defined without a getter",
        )),
    }
}

/// Writes a private field or calls a private setter.
pub fn private_set(
    realm: &mut Realm,
    target: &Value,
    name: &Symbol,
    value: Value,
) -> Result<(), Thrown> {
    let Some((object, property)) = private_element(target, name) else {
        return Err(private_error(realm, "write", name));
    };
    match property.slot {
        Slot::Data { writable: true, .. } => {
            object.set_private(name, Property::data(value));
            Ok(())
        }
        Slot::Data {
            writable: false, ..
        } => Err(realm.throw(ErrorKind::Type, "Private method is not writable")),
        Slot::Accessor { set: Some(set), .. } => {
            ops::call(realm, &Value::Object(set), target.clone(), &[value])?;
            Ok(())
        }
        Slot::Accessor { set: None, .. } => Err(realm.throw(
            ErrorKind::Type,
            "Private accessor was defined without a setter",
        )),
    }
}

/// Adds a private field, method or accessor to a new instance.
pub fn private_define(
    realm: &Realm,
    target: &Value,
    name: &Symbol,
    property: Property,
) -> Result<(), Thrown> {
    let object = target
        .as_object()
        .expect("private elements belong to objects");
    if object.private_property(name).is_some() {
        return Err(realm.throw(
            ErrorKind::Type,
            "Cannot initialize a private member twice on the same object",
        ));
    }
    object.set_private(name, property);
    Ok(())
}

/// `#name in value`.
pub fn private_in(realm: &Realm, name: &Symbol, value: &Value) -> Result<bool, Thrown> {
    match value {
        Value::Object(object) => Ok(object.private_property(name).is_some()),
        _ => Err(realm.throw(
            ErrorKind::Type,
            "Cannot use 'in' operator to search for a private member in a primitive",
        )),
    }
}

#[cfg(test)]
mod tests;
