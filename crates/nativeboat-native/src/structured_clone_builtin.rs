//! The HTML structured clone algorithm, for `structuredClone` and for the
//! values Electron IPC and the preload bridge pass between contexts:
//! primitives, primitive wrappers, dates, regular expressions, maps, sets,
//! errors, array buffers and typed arrays, arrays and plain objects, with
//! shared references and cycles kept.

use crate::prelude::*;

use crate::{ArrayData, ErrorKind, Key, Kind, Object, Realm, Thrown, Value, ops};

pub(crate) fn install(realm: &mut Realm) {
    let function = realm.function("structuredClone", 1, false, |realm, _, args| {
        if args
            .get(1)
            .is_some_and(|options| !matches!(options, Value::Undefined))
        {
            return Err(realm.throw(
                ErrorKind::Type,
                "structuredClone options are not supported by the native target",
            ));
        }
        let value = args.first().cloned().unwrap_or(Value::Undefined);
        let mut cloner = Cloner {
            source: None,
            seen: Vec::new(),
        };
        cloner.clone(realm, &value)
    });
    realm.set_global("structuredClone", Value::Object(function));
}

/// Structured-clones a value from one native execution context into
/// another. This is shared by Electron IPC and the isolated preload bridge.
pub(crate) fn clone_between(
    source_realm: &mut Realm,
    target_realm: &mut Realm,
    value: &Value,
) -> Result<Value, Thrown> {
    let mut cloner = Cloner {
        source: Some(source_realm),
        seen: Vec::new(),
    };
    cloner.clone(target_realm, value)
}

/// Structured-clones a value within one context, as `structuredClone` does.
pub(crate) fn clone_within(realm: &mut Realm, value: &Value) -> Result<Value, Thrown> {
    let mut cloner = Cloner {
        source: None,
        seen: Vec::new(),
    };
    cloner.clone(realm, value)
}

/// One cloning: where values come from, and the objects already cloned.
struct Cloner<'s> {
    /// The context the value comes from; `None` when it is the target's.
    source: Option<&'s mut Realm>,
    seen: Vec<(Object, Object)>,
}

/// What a source object is, read before any of its properties.
enum Shape {
    Wrapper(Value),
    Date(f64),
    RegExp(Vec<u16>, String),
    Collection(bool, Vec<(Value, Value)>),
    Buffer(Vec<u8>),
    View(&'static str, Vec<u8>),
    Error,
    Array(u32),
    Plain,
}

impl Cloner<'_> {
    fn unclonable(target: &Realm, what: &str) -> Thrown {
        target.throw(ErrorKind::Type, &format!("{what} could not be cloned."))
    }

    /// `object[key]` in the source context.
    fn read(&mut self, target: &mut Realm, object: &Object, key: &Key) -> Result<Value, Thrown> {
        let base = Value::Object(object.clone());
        match &mut self.source {
            Some(source) => ops::get(source, &base, key).map_err(|_| {
                target.throw(ErrorKind::Type, "value cannot be cloned between contexts")
            }),
            None => ops::get(target, &base, key),
        }
    }

    /// A target-context global constructor called with arguments.
    fn construct(target: &mut Realm, name: &str, args: &[Value]) -> Result<Object, Thrown> {
        let global = Value::Object(target.global_object());
        let constructor = ops::get(target, &global, &Key::name(name))?;
        match ops::construct(target, &constructor, args)? {
            Value::Object(object) => Ok(object),
            _ => Err(Self::unclonable(target, name)),
        }
    }

    fn clone(&mut self, target: &mut Realm, value: &Value) -> Result<Value, Thrown> {
        let Some(source) = value.as_object() else {
            return match value {
                Value::Symbol(_) => Err(Self::unclonable(target, "Symbol()")),
                _ => Ok(value.clone()),
            };
        };
        if let Some((_, clone)) = self.seen.iter().find(|(known, _)| known == source) {
            return Ok(Value::Object(clone.clone()));
        }
        // Read the source's internal state before any accessor runs:
        // getters may change the object being cloned.
        let shape = {
            let data = source.data();
            match &data.kind {
                Kind::Primitive(wrapped) => Shape::Wrapper(match wrapped {
                    crate::object::Wrapped::Boolean(value) => Value::Boolean(*value),
                    crate::object::Wrapped::Number(value) => Value::Number(*value),
                    crate::object::Wrapped::String(value) => Value::String(value.clone()),
                    crate::object::Wrapped::BigInt(value) => Value::BigInt(value.clone()),
                }),
                Kind::Date(time) => Shape::Date(*time),
                Kind::RegExp(regexp) => {
                    Shape::RegExp(regexp.source.units().to_vec(), regexp.flags.to_rust_lossy())
                }
                Kind::Collection(collection) => {
                    Shape::Collection(collection.set, collection.snapshot(usize::MAX))
                }
                Kind::ArrayBuffer(bytes) => Shape::Buffer(bytes.borrow().clone()),
                Kind::TypedArray(view) => {
                    let bytes = view.bytes.borrow();
                    let width = view.element.size();
                    let end = (view.offset + view.length * width).min(bytes.len());
                    Shape::View(
                        view.element.name(),
                        bytes[view.offset.min(end)..end].to_vec(),
                    )
                }
                Kind::Error => Shape::Error,
                Kind::Array(array) => Shape::Array(array.length()),
                Kind::Ordinary => Shape::Plain,
                Kind::Function { .. }
                | Kind::BuiltinConstructor { .. }
                | Kind::BoundFunction { .. } => {
                    return Err(Self::unclonable(target, "function"));
                }
                _ => {
                    return Err(target.throw(
                        ErrorKind::Type,
                        "This object type cannot be cloned by the native target",
                    ));
                }
            }
        };
        let clone = match shape {
            Shape::Wrapper(primitive) => {
                let clone = ops::to_object(target, &primitive)?;
                self.seen.push((source.clone(), clone.clone()));
                return Ok(Value::Object(clone));
            }
            Shape::Date(time) => Object::new(Some(target.date_prototype.clone()), Kind::Date(time)),
            Shape::RegExp(pattern, flags) => {
                match crate::regexp_builtin::literal(target, &pattern, &flags)? {
                    Value::Object(object) => object,
                    _ => return Err(Self::unclonable(target, "RegExp")),
                }
            }
            Shape::Collection(set, entries) => {
                let clone = Self::construct(target, if set { "Set" } else { "Map" }, &[])?;
                self.seen.push((source.clone(), clone.clone()));
                let method = Key::name(if set { "add" } else { "set" });
                let method = ops::get(target, &Value::Object(clone.clone()), &method)?;
                for (key, value) in entries {
                    let key = self.clone(target, &key)?;
                    let value = if set {
                        Value::Undefined
                    } else {
                        self.clone(target, &value)?
                    };
                    ops::call(target, &method, Value::Object(clone.clone()), &[key, value])?;
                }
                return Ok(Value::Object(clone));
            }
            Shape::Buffer(bytes) => {
                let clone =
                    Self::construct(target, "ArrayBuffer", &[Value::Number(bytes.len() as f64)])?;
                if let Kind::ArrayBuffer(storage) = &clone.data().kind {
                    *storage.borrow_mut() = bytes;
                }
                clone
            }
            Shape::View(name, bytes) => {
                let buffer =
                    Self::construct(target, "ArrayBuffer", &[Value::Number(bytes.len() as f64)])?;
                if let Kind::ArrayBuffer(storage) = &buffer.data().kind {
                    *storage.borrow_mut() = bytes;
                }
                Self::construct(target, name, &[Value::Object(buffer)])?
            }
            Shape::Error => {
                // The clone is an error of the same standard kind.
                let name = self.read(target, source, &Key::name("name"))?;
                let name = match &name {
                    Value::String(name) => name.to_rust_lossy(),
                    _ => String::new(),
                };
                let kind = match name.as_str() {
                    "EvalError" | "RangeError" | "ReferenceError" | "SyntaxError" | "TypeError"
                    | "URIError" => name.as_str(),
                    _ => "Error",
                };
                let message = self.read(target, source, &Key::name("message"))?;
                let message = match message {
                    Value::Undefined => Vec::new(),
                    message => vec![Value::String(ops::to_string(target, &message)?)],
                };
                let clone = Self::construct(target, kind, &message)?;
                self.seen.push((source.clone(), clone.clone()));
                for key in ["stack", "cause"] {
                    let key = Key::name(key);
                    if source.own_property(&key).is_some() {
                        let value = self.read(target, source, &key)?;
                        let value = self.clone(target, &value)?;
                        clone.define(key, crate::Property::hidden(value));
                    }
                }
                return Ok(Value::Object(clone));
            }
            Shape::Array(length) => {
                let clone = Object::new(
                    Some(target.array_prototype.clone()),
                    Kind::Array(ArrayData::new(Vec::new())),
                );
                clone.define_own_property(
                    &Key::name("length"),
                    &crate::Descriptor {
                        value: Some(Value::Number(f64::from(length))),
                        ..crate::Descriptor::default()
                    },
                );
                self.seen.push((source.clone(), clone.clone()));
                self.properties(target, source, &clone)?;
                return Ok(Value::Object(clone));
            }
            Shape::Plain => {
                let clone = Object::new(Some(target.object_prototype.clone()), Kind::Ordinary);
                self.seen.push((source.clone(), clone.clone()));
                self.properties(target, source, &clone)?;
                return Ok(Value::Object(clone));
            }
        };
        self.seen.push((source.clone(), clone.clone()));
        Ok(Value::Object(clone))
    }

    /// Copies the source's own enumerable string-keyed properties.
    fn properties(
        &mut self,
        target: &mut Realm,
        source: &Object,
        clone: &Object,
    ) -> Result<(), Thrown> {
        for key in source.own_keys() {
            if matches!(key, Key::Symbol(_))
                || !source
                    .own_property(&key)
                    .is_some_and(|property| property.enumerable)
            {
                continue;
            }
            let value = self.read(target, source, &key)?;
            let value = self.clone(target, &value)?;
            clone.define(key, crate::Property::data(value));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
