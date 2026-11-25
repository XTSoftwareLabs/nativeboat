//! Native `WeakMap`, `WeakSet`, `WeakRef` and `FinalizationRegistry`. Keys
//! and targets are objects, or symbols not created by `Symbol.for`, held
//! through weak references: an entry ends when nothing else keeps its key
//! alive. `WeakRef` targets stay alive until the current job ends, as the
//! specification's `KeepDuringJob` requires. Registries are checked as the
//! event loop turns, and their cleanup callbacks run as tasks; as in Node,
//! they do not keep the program running.

use crate::object::WeakObject;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::symbol::WeakSymbol;
use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, iter, ops};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::RefCell;

/// A key or target held without keeping it alive.
#[derive(Clone, Debug)]
pub enum WeakKey {
    /// An object.
    Object(WeakObject),
    /// A symbol not created by `Symbol.for`.
    Symbol(WeakSymbol),
}

impl WeakKey {
    /// The key's identity; another key gets it only after this one is gone.
    fn address(&self) -> usize {
        match self {
            WeakKey::Object(object) => object.address(),
            WeakKey::Symbol(symbol) => symbol.address(),
        }
    }

    /// The key as a value, while it is alive.
    pub(crate) fn upgrade(&self) -> Option<Value> {
        match self {
            WeakKey::Object(object) => object.upgrade().map(Value::Object),
            WeakKey::Symbol(symbol) => symbol.upgrade().map(Value::Symbol),
        }
    }

    fn alive(&self) -> bool {
        match self {
            WeakKey::Object(object) => object.alive(),
            WeakKey::Symbol(symbol) => symbol.upgrade().is_some(),
        }
    }
}

/// `CanBeHeldWeakly`: objects and unregistered symbols.
fn weak_key(realm: &Realm, value: &Value) -> Option<WeakKey> {
    match value {
        Value::Object(object) => Some(WeakKey::Object(object.downgrade())),
        Value::Symbol(symbol) if !crate::symbol_builtin::is_registered(realm, symbol) => {
            Some(WeakKey::Symbol(symbol.downgrade()))
        }
        _ => None,
    }
}

/// The live entries of a WeakMap or WeakSet, by key identity.
#[derive(Debug, Default)]
struct Entries {
    map: BTreeMap<usize, (WeakKey, Value)>,
    /// The entry count at which entries of collected keys are dropped.
    threshold: usize,
}

impl Entries {
    fn get(&self, key: &WeakKey) -> Option<&Value> {
        self.map
            .get(&key.address())
            .filter(|(known, _)| known.alive())
            .map(|(_, value)| value)
    }

    fn insert(&mut self, key: WeakKey, value: Value) {
        if self.map.len() >= self.threshold {
            self.map.retain(|_, (known, _)| known.alive());
            self.threshold = (self.map.len() * 2).max(64);
        }
        self.map.insert(key.address(), (key, value));
    }

    fn remove(&mut self, key: &WeakKey) -> bool {
        let present = self.get(key).is_some();
        if present {
            self.map.remove(&key.address());
        }
        present
    }
}

/// A WeakMap's or WeakSet's entries.
#[derive(Clone, Debug)]
pub struct WeakCollection {
    /// Whether this is a WeakSet rather than a WeakMap.
    pub set: bool,
    entries: Rc<RefCell<Entries>>,
}

/// One target registered with a FinalizationRegistry.
#[derive(Debug)]
struct Cell {
    target: WeakKey,
    held: Value,
    token: Option<WeakKey>,
}

/// A FinalizationRegistry's cleanup callback and registrations.
#[derive(Debug)]
pub struct Registry {
    cleanup: Value,
    cells: RefCell<Vec<Cell>>,
}

/// V8's text for a receiver in an error message. Functions have no source
/// text here, so they read as V8 shows native functions.
pub(crate) fn receiver_text(value: &Value) -> String {
    let Value::Object(object) = value else {
        return match value {
            Value::Symbol(symbol) => symbol.descriptive().to_rust_lossy(),
            Value::BigInt(value) => value.to_radix(10),
            Value::String(text) => text.to_rust_lossy(),
            Value::Number(number) => crate::inspect::number(*number),
            Value::Boolean(value) => value.to_string(),
            Value::Null => "null".to_owned(),
            _ => "undefined".to_owned(),
        };
    };
    if object.is_array() {
        return "[object Array]".to_owned();
    }
    if object.code().is_some() {
        let name = match object.own_property(&Key::name("name")) {
            Some(Property {
                slot:
                    Slot::Data {
                        value: Value::String(name),
                        ..
                    },
                ..
            }) => name.to_rust_lossy(),
            _ => String::new(),
        };
        return format!("function {name}() {{ [native code] }}");
    }
    // The class name: the first `constructor` data property on the chain.
    let mut current = Some(object.clone());
    while let Some(holder) = current {
        if let Some(Property { slot, .. }) = holder.own_property(&Key::name("constructor")) {
            if let Slot::Data {
                value: Value::Object(constructor),
                ..
            } = slot
                && let Some(Property {
                    slot:
                        Slot::Data {
                            value: Value::String(name),
                            ..
                        },
                    ..
                }) = constructor.own_property(&Key::name("name"))
                && !name.is_empty()
            {
                return format!("#<{}>", name.to_rust_lossy());
            }
            break;
        }
        current = holder.data().prototype.clone();
    }
    "[object Object]".to_owned()
}

fn incompatible(realm: &mut Realm, class: &str, method: &str, this: &Value) -> Thrown {
    let text = receiver_text(this);
    realm.throw(
        ErrorKind::Type,
        &format!("Method {class}.prototype.{method} called on incompatible receiver {text}"),
    )
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn collection(
    realm: &mut Realm,
    this: &Value,
    set: bool,
    method: &str,
) -> Result<WeakCollection, Thrown> {
    if let Value::Object(object) = this
        && let Kind::WeakCollection(collection) = &object.data().kind
        && collection.set == set
    {
        return Ok(collection.clone());
    }
    let class = if set { "WeakSet" } else { "WeakMap" };
    Err(incompatible(realm, class, method, this))
}

/// The prototype for `new target`, or the intrinsic one.
fn prototype_for(realm: &mut Realm, target: Object, fallback: &Object) -> Result<Object, Thrown> {
    Ok(
        match ops::get_named(realm, &Value::Object(target), "prototype")? {
            Value::Object(prototype) => prototype,
            _ => fallback.clone(),
        },
    )
}

fn construct_collection(
    realm: &mut Realm,
    target: Object,
    args: &[Value],
    set: bool,
    fallback: &Object,
) -> Result<Object, Thrown> {
    let prototype = prototype_for(realm, target, fallback)?;
    let object = Object::new(
        Some(prototype),
        Kind::WeakCollection(WeakCollection {
            set,
            entries: Rc::default(),
        }),
    );
    let source = arg(args, 0);
    if source.is_nullish() {
        return Ok(object);
    }
    let this = Value::Object(object.clone());
    let property = if set { "add" } else { "set" };
    let adder = ops::get(realm, &this, &Key::name(property))?;
    if adder.as_object().and_then(Object::code).is_none() {
        let (text, receiver) = (receiver_text(&adder), receiver_text(&this));
        return Err(realm.throw(
            ErrorKind::Type,
            &format!("'{text}' returned for property '{property}' of object '{receiver}' is not a function"),
        ));
    }
    let mut iterator = iter::iterate(realm, &source)?;
    while let Some(item) = iterator.step(realm)? {
        let added = (|| -> Result<(), Thrown> {
            let arguments = if set {
                vec![item]
            } else {
                if !matches!(item, Value::Object(_)) {
                    let text = receiver_text(&item);
                    return Err(realm.throw(
                        ErrorKind::Type,
                        &format!("Iterator value {text} is not an entry object"),
                    ));
                }
                let key = ops::get(realm, &item, &Key::Index(0))?;
                let value = ops::get(realm, &item, &Key::Index(1))?;
                vec![key, value]
            };
            ops::call(realm, &adder, this.clone(), &arguments)?;
            Ok(())
        })();
        if let Err(thrown) = added {
            return Err(iterator.close_thrown(realm, thrown));
        }
    }
    Ok(object)
}

fn install_collection(realm: &Realm, set: bool) {
    let class = if set { "WeakSet" } else { "WeakMap" };
    let prototype = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        class,
        0,
        &prototype,
        move |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                &format!("Constructor {class} requires 'new'"),
            ))
        },
        move |realm, target, args| construct_collection(realm, target, args, set, &fallback),
    );
    realm.method(&prototype, "delete", 1, move |realm, this, args| {
        let collection = collection(realm, &this, set, "delete")?;
        let removed = weak_key(realm, &arg(args, 0))
            .is_some_and(|key| collection.entries.borrow_mut().remove(&key));
        Ok(Value::Boolean(removed))
    });
    if !set {
        realm.method(&prototype, "get", 1, |realm, this, args| {
            let collection = collection(realm, &this, false, "get")?;
            let Some(key) = weak_key(realm, &arg(args, 0)) else {
                return Ok(Value::Undefined);
            };
            Ok(collection
                .entries
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or_default())
        });
        realm.method(&prototype, "set", 2, |realm, this, args| {
            let collection = collection(realm, &this, false, "set")?;
            let Some(key) = weak_key(realm, &arg(args, 0)) else {
                return Err(realm.throw(ErrorKind::Type, "Invalid value used as weak map key"));
            };
            collection.entries.borrow_mut().insert(key, arg(args, 1));
            Ok(this)
        });
    }
    realm.method(&prototype, "has", 1, move |realm, this, args| {
        let collection = collection(realm, &this, set, "has")?;
        let present = weak_key(realm, &arg(args, 0))
            .is_some_and(|key| collection.entries.borrow().get(&key).is_some());
        Ok(Value::Boolean(present))
    });
    if set {
        realm.method(&prototype, "add", 1, |realm, this, args| {
            let collection = collection(realm, &this, true, "add")?;
            let Some(key) = weak_key(realm, &arg(args, 0)) else {
                return Err(realm.throw(ErrorKind::Type, "Invalid value used in weak set"));
            };
            collection
                .entries
                .borrow_mut()
                .insert(key, Value::Undefined);
            Ok(this)
        });
    }
    tag(realm, &prototype, class);
    realm.set_global(class, Value::Object(constructor));
}

fn tag(realm: &Realm, target: &Object, text: &str) {
    target.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str(text),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

/// `AddToKeptObjects`.
fn keep(realm: &mut Realm, value: Value) {
    realm.kept.push(value);
}

fn install_weak_ref(realm: &Realm) {
    let prototype = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        "WeakRef",
        1,
        &prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "Constructor WeakRef requires 'new'")),
        move |realm, target, args| {
            let value = arg(args, 0);
            let Some(key) = weak_key(realm, &value) else {
                return Err(realm.throw(ErrorKind::Type, "WeakRef: invalid target"));
            };
            let prototype = prototype_for(realm, target, &fallback)?;
            keep(realm, value);
            Ok(Object::new(Some(prototype), Kind::WeakRef(key)))
        },
    );
    realm.method(&prototype, "deref", 0, |realm, this, _| {
        let target = match &this {
            Value::Object(object) => match &object.data().kind {
                Kind::WeakRef(key) => Some(key.upgrade()),
                _ => None,
            },
            _ => None,
        };
        let Some(target) = target else {
            return Err(incompatible(realm, "WeakRef", "deref", &this));
        };
        Ok(match target {
            Some(value) => {
                keep(realm, value.clone());
                value
            }
            None => Value::Undefined,
        })
    });
    tag(realm, &prototype, "WeakRef");
    realm.set_global("WeakRef", Value::Object(constructor));
}

fn registry(realm: &mut Realm, this: &Value, method: &str) -> Result<Rc<Registry>, Thrown> {
    if let Value::Object(object) = this
        && let Kind::FinalizationRegistry(registry) = &object.data().kind
    {
        return Ok(registry.clone());
    }
    Err(incompatible(realm, "FinalizationRegistry", method, this))
}

/// An unregister token, which must be able to be held weakly.
fn token(realm: &mut Realm, value: &Value) -> Result<WeakKey, Thrown> {
    if let Some(key) = weak_key(realm, value) {
        return Ok(key);
    }
    let text = receiver_text(value);
    Err(realm.throw(
        ErrorKind::Type,
        &format!("Invalid unregisterToken ('{text}')"),
    ))
}

fn install_registry(realm: &Realm) {
    let prototype = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        "FinalizationRegistry",
        1,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Constructor FinalizationRegistry requires 'new'",
            ))
        },
        move |realm, target, args| {
            let cleanup = arg(args, 0);
            if cleanup.as_object().and_then(Object::code).is_none() {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "FinalizationRegistry: cleanup must be callable",
                ));
            }
            let prototype = prototype_for(realm, target, &fallback)?;
            let registry = Rc::new(Registry {
                cleanup,
                cells: RefCell::default(),
            });
            realm.registries.push(Rc::downgrade(&registry));
            Ok(Object::new(
                Some(prototype),
                Kind::FinalizationRegistry(registry),
            ))
        },
    );
    realm.method(&prototype, "register", 2, |realm, this, args| {
        let registry = registry(realm, &this, "register")?;
        let (target, held) = (arg(args, 0), arg(args, 1));
        let Some(key) = weak_key(realm, &target) else {
            return Err(realm.throw(
                ErrorKind::Type,
                "FinalizationRegistry.prototype.register: invalid target",
            ));
        };
        if crate::same_value(&target, &held) {
            return Err(realm.throw(
                ErrorKind::Type,
                "FinalizationRegistry.prototype.register: target and holdings must not be same",
            ));
        }
        let unregister = match arg(args, 2) {
            Value::Undefined => None,
            value => Some(token(realm, &value)?),
        };
        registry.cells.borrow_mut().push(Cell {
            target: key,
            held,
            token: unregister,
        });
        Ok(Value::Undefined)
    });
    realm.method(&prototype, "unregister", 1, |realm, this, args| {
        let registry = registry(realm, &this, "unregister")?;
        let key = token(realm, &arg(args, 0))?;
        let mut cells = registry.cells.borrow_mut();
        let before = cells.len();
        cells.retain(|cell| {
            !cell
                .token
                .as_ref()
                .is_some_and(|token| token.alive() && token.address() == key.address())
        });
        Ok(Value::Boolean(cells.len() != before))
    });
    tag(realm, &prototype, "FinalizationRegistry");
    realm.set_global("FinalizationRegistry", Value::Object(constructor));
}

pub(crate) fn install(realm: &Realm) {
    install_collection(realm, false);
    install_collection(realm, true);
    install_weak_ref(realm);
    install_registry(realm);
}

impl Realm {
    /// `ClearKeptObjects`, when a job ends.
    pub(crate) fn clear_kept(&mut self) {
        self.kept.clear();
    }

    /// Runs a cleanup job for each registry with collected targets, followed
    /// by its microtasks. A callback can unregister cells not yet processed.
    /// Registries no longer referenced are forgotten.
    pub(crate) fn finalize(&mut self) -> Result<(), Thrown> {
        self.registries
            .retain(|registry| registry.strong_count() > 0);
        let mut pending = Vec::new();
        for registry in self.registries.iter().filter_map(alloc::rc::Weak::upgrade) {
            // Snapshot only weak identities, leaving held values and tokens
            // in the registry until their individual callbacks begin. This
            // also defers registrations made by a cleanup callback.
            let targets: Vec<WeakKey> = registry
                .cells
                .borrow()
                .iter()
                .filter(|cell| !cell.target.alive())
                .map(|cell| cell.target.clone())
                .collect();
            if !targets.is_empty() {
                pending.push((registry, targets));
            }
        }
        for (registry, targets) in pending {
            for target in targets {
                let held = {
                    let mut cells = registry.cells.borrow_mut();
                    cells
                        .iter()
                        .position(|cell| cell.target.address() == target.address())
                        .map(|index| cells.remove(index).held)
                };
                if let Some(held) = held {
                    ops::call(self, &registry.cleanup, Value::Undefined, &[held])?;
                }
            }
            self.run_jobs()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
