//! Shared bindings: variables that closures capture or modules export.
//! Bindings that are never captured stay ordinary Rust locals. A binding
//! holds `None` until its declaration runs, which is the temporal dead zone
//! of `let`, `const` and `class` declarations.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Object, Property, Slot};
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use alloc::rc::Rc;
use core::cell::RefCell;

/// A shared, mutable binding.
pub type Binding = Rc<RefCell<Option<Value>>>;

/// A binding, initialized (`var`, parameters) or not yet (`let`, `const`).
pub fn new(initial: Option<Value>) -> Binding {
    Rc::new(RefCell::new(initial))
}

/// Reads a binding; before initialization this is a ReferenceError.
#[inline]
pub fn read(realm: &Realm, binding: &Binding, name: &str) -> Completion {
    binding
        .borrow()
        .clone()
        .ok_or_else(|| uninitialized(realm, name))
}

/// Assigns an initialized binding; before initialization this is a
/// ReferenceError.
#[inline]
pub fn write(realm: &Realm, binding: &Binding, name: &str, value: Value) -> Result<(), Thrown> {
    let mut slot = binding.borrow_mut();
    if slot.is_none() {
        return Err(uninitialized(realm, name));
    }
    *slot = Some(value);
    Ok(())
}

/// Runs a declaration: the binding leaves its temporal dead zone.
#[inline]
pub fn initialize(binding: &Binding, value: Value) {
    *binding.borrow_mut() = Some(value);
}

#[cold]
fn uninitialized(realm: &Realm, name: &str) -> Thrown {
    realm.throw(
        ErrorKind::Reference,
        &format!("Cannot access '{name}' before initialization"),
    )
}

/// Exposes a module binding as a live, read-only export.
pub fn export(realm: &Realm, exports: &Object, name: &str, binding: &Binding) {
    let binding = binding.clone();
    let local = name.to_owned();
    let getter = realm.function(name, 0, false, move |realm, _, _| {
        read(realm, &binding, &local)
    });
    exports.define(
        Key::name(name),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: false,
        },
    );
}

/// A module namespace over a graph's cells: `entries` name an export and
/// its cell, as a static table the linker emits.
pub fn namespace_of(realm: &Realm, cells: &[Binding], entries: &[(&str, u32)]) -> Object {
    let entries: Vec<(&str, &Binding)> = entries
        .iter()
        .map(|(name, cell)| (*name, &cells[*cell as usize]))
        .collect();
    namespace(realm, &entries)
}

/// A module namespace object: no prototype, one live read-only export per
/// entry (the linker passes them in code unit order) and not extensible.
pub fn namespace(realm: &Realm, entries: &[(&str, &Binding)]) -> Object {
    let namespace = Object::new(None, crate::Kind::Ordinary);
    for (name, binding) in entries {
        export(realm, &namespace, name, binding);
    }
    namespace.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("Module"),
                writable: false,
            },
            enumerable: false,
            configurable: false,
        },
    );
    namespace.data_mut().extensible = false;
    namespace
}

/// The namespace of an evaluated CommonJS module: `default` is
/// `module.exports`, and each own enumerable string-keyed property of an
/// exports object is a named export holding its value now, as Node takes
/// the snapshot once the module has run.
pub fn commonjs_namespace(realm: &mut Realm, exports: &Value) -> Result<Object, Thrown> {
    let mut entries = alloc::collections::BTreeMap::new();
    if let Value::Object(object) = exports {
        for key in object.own_keys() {
            if !matches!(key, Key::Symbol(_))
                && object
                    .own_property(&key)
                    .is_some_and(|property| property.enumerable)
            {
                let value = crate::ops::get(realm, exports, &key)?;
                let name = key.to_js();
                entries.insert(name.units().to_vec(), (name.to_rust_lossy(), value));
            }
        }
    }
    let default: Vec<u16> = "default".encode_utf16().collect();
    entries.insert(default, ("default".to_owned(), exports.clone()));
    let bindings: Vec<(String, Binding)> = entries
        .into_values()
        .map(|(name, value)| (name, new(Some(value))))
        .collect();
    let entries: Vec<(&str, &Binding)> = bindings
        .iter()
        .map(|(name, binding)| (name.as_str(), binding))
        .collect();
    Ok(namespace(realm, &entries))
}

/// Outside `try` blocks, evaluates a fallible expression, returning a thrown
/// value from the enclosing function. Unlike `?`, it calls no conversion
/// functions, which unoptimized generated code would keep at every use.
#[macro_export]
macro_rules! check {
    ($operation:expr) => {
        match $operation {
            Ok(value) => value,
            Err(thrown) => return Err(thrown),
        }
    };
}

/// Inside a `try` block, evaluates a fallible expression; a thrown value is
/// stored in the block's slot and control leaves the labeled block. Outside
/// `try`, generated code uses [`check!`] instead.
#[macro_export]
macro_rules! attempt {
    ($operation:expr, $slot:ident, $label:lifetime) => {
        match $operation {
            Ok(value) => value,
            Err(thrown) => {
                $slot = Some(thrown);
                break $label;
            }
        }
    };
}

#[cfg(test)]
mod tests;
