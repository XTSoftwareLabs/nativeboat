//! Native ECMAScript 2024 Map/Set: SameValueZero hash lookup, insertion order,
//! live iterators and callback mutation. Deleted slots compact only when no
//! active iterator/callback needs their positions. Cycles need the runtime's
//! separately planned collector, as other native objects do.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::iter::{self, Yield};
use crate::object::{Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::table::HashMap;
use crate::value::{Completion, Thrown, Value};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::hash::{Hash, Hasher};

#[derive(Clone, Debug)]
struct NativeKey(Value);

impl PartialEq for NativeKey {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Value::Number(a), Value::Number(b)) => a == b || (a.is_nan() && b.is_nan()),
            _ => crate::object::same_value(&self.0, &other.0),
        }
    }
}
impl Eq for NativeKey {}

impl Hash for NativeKey {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        core::mem::discriminant(&self.0).hash(hasher);
        match &self.0 {
            Value::Undefined | Value::Null => {}
            Value::Boolean(value) => value.hash(hasher),
            Value::Number(value) => {
                let bits = if value.is_nan() {
                    f64::NAN.to_bits()
                } else if *value == 0.0 {
                    0
                } else {
                    value.to_bits()
                };
                bits.hash(hasher);
            }
            Value::String(value) => value.hash(hasher),
            Value::BigInt(value) => value.hash(hasher),
            Value::Symbol(value) => value.hash(hasher),
            Value::Object(value) => value.hash(hasher),
        }
    }
}

#[derive(Debug, Default)]
struct Data {
    entries: Vec<Option<(Value, Value)>>,
    positions: HashMap<NativeKey, usize>,
    removed: usize,
    active: usize,
}

impl Data {
    fn size(&self) -> usize {
        self.entries.len() - self.removed
    }

    fn compact(&mut self) {
        if self.active == 0 && self.removed > self.entries.len() / 2 {
            self.entries.retain(Option::is_some);
            self.entries.shrink_to_fit();
            self.positions = HashMap::new();
            self.removed = 0;
            for (index, entry) in self.entries.iter().enumerate() {
                let (key, _) = entry.as_ref().expect("compacted entries are present");
                self.positions.insert(NativeKey(key.clone()), index);
            }
        }
    }

    fn insert(&mut self, mut key: Value, value: Value) -> Option<Value> {
        if let Value::Number(number) = &mut key
            && *number == 0.0
        {
            *number = 0.0;
        }
        let lookup = NativeKey(key.clone());
        if let Some(index) = self.positions.get(&lookup) {
            let (_, old) = self.entries[*index]
                .as_mut()
                .expect("indexed entries are present");
            return Some(core::mem::replace(old, value));
        }
        self.positions.insert(lookup, self.entries.len());
        self.entries.push(Some((key, value)));
        None
    }

    fn get(&self, key: &Value) -> Option<Value> {
        let index = self.positions.get(&NativeKey(key.clone()))?;
        self.entries[*index]
            .as_ref()
            .map(|(_, value)| value.clone())
    }

    fn delete(&mut self, key: &Value) -> Option<(Value, Value)> {
        let index = self.positions.remove(&NativeKey(key.clone()))?;
        let removed = self.entries[index].take();
        self.removed += 1;
        self.compact();
        removed
    }

    fn clear(&mut self) -> Vec<(Value, Value)> {
        // Retain values until the caller has released the data borrow: an
        // entry can itself hold an iterator whose destructor releases a lease.
        self.positions = HashMap::new();
        let removed = self.entries.iter_mut().filter_map(Option::take).collect();
        self.removed = self.entries.len();
        self.compact();
        removed
    }
}

/// Native keyed collection storage, shared by its live iterators.
#[derive(Clone, Debug)]
pub struct Collection {
    /// Whether this is a Set rather than a Map.
    pub set: bool,
    data: Rc<RefCell<Data>>,
}

impl Collection {
    /// The live entry count, independent of own properties named `size`.
    pub(crate) fn size(&self) -> usize {
        self.data.borrow().size()
    }

    /// Own collection entries for native inspection, in insertion order.
    pub(crate) fn snapshot(&self, limit: usize) -> Vec<(Value, Value)> {
        self.data
            .borrow()
            .entries
            .iter()
            .flatten()
            .take(limit)
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
struct Lease {
    data: Rc<RefCell<Data>>,
    // Preserve the iterated object's properties/prototype as well as entries.
    _owner: Option<Object>,
}

impl Lease {
    fn new(data: &Rc<RefCell<Data>>, owner: Option<Object>) -> Lease {
        data.borrow_mut().active += 1;
        Lease {
            data: data.clone(),
            _owner: owner,
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut data = self.data.borrow_mut();
        data.active -= 1;
        data.compact();
    }
}

#[derive(Debug)]
struct Cursor {
    lease: Option<Lease>,
    index: usize,
    yields: Yield,
}

/// Native Map/Set iterator with shared cursor state and a compaction lease.
#[derive(Clone, Debug)]
pub struct CollectionIterator {
    /// The iterator brand: Map and Set have distinct next operations.
    pub set: bool,
    cursor: Rc<RefCell<Cursor>>,
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn name(set: bool) -> &'static str {
    if set { "Set" } else { "Map" }
}

fn receiver(realm: &Realm, this: &Value, set: bool) -> Result<Collection, Thrown> {
    if let Value::Object(object) = this
        && let Kind::Collection(collection) = &object.data().kind
        && collection.set == set
    {
        return Ok(collection.clone());
    }
    Err(realm.throw(
        ErrorKind::Type,
        &format!("{} method called on an incompatible receiver", name(set)),
    ))
}

fn construct(
    realm: &mut Realm,
    target: Object,
    args: &[Value],
    set: bool,
    fallback: &Object,
) -> Result<Object, Thrown> {
    let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
        Value::Object(prototype) => prototype,
        _ => fallback.clone(),
    };
    let object = Object::new(
        Some(prototype),
        Kind::Collection(Collection {
            set,
            data: Rc::default(),
        }),
    );
    let source = arg(args, 0);
    if source.is_nullish() {
        return Ok(object);
    }
    let this = Value::Object(object.clone());
    let adder = ops::get(realm, &this, &Key::name(if set { "add" } else { "set" }))?;
    if adder.as_object().and_then(Object::code).is_none() {
        return Err(realm.throw(ErrorKind::Type, "collection adder is not callable"));
    }
    let mut iterator = iter::iterate(realm, &source)?;
    while let Some(item) = iterator.step(realm)? {
        let added = (|| -> Result<(), Thrown> {
            let arguments = if set {
                vec![item]
            } else {
                if !matches!(item, Value::Object(_)) {
                    return Err(
                        realm.throw(ErrorKind::Type, "Map iterator value is not an entry object")
                    );
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

fn insert(realm: &mut Realm, this: Value, args: &[Value], set: bool) -> Completion {
    let collection = receiver(realm, &this, set)?;
    let key = arg(args, 0);
    let value = if set { Value::Undefined } else { arg(args, 1) };
    let previous = collection.data.borrow_mut().insert(key, value);
    drop(previous);
    Ok(this)
}

fn for_each(realm: &mut Realm, this: Value, args: &[Value], set: bool) -> Completion {
    let collection = receiver(realm, &this, set)?;
    let callback = arg(args, 0);
    if callback.as_object().and_then(Object::code).is_none() {
        return Err(realm.throw(ErrorKind::Type, "collection callback is not callable"));
    }
    let receiver = arg(args, 1);
    let _lease = Lease::new(&collection.data, None);
    let mut index = 0;
    loop {
        let entry = {
            let data = collection.data.borrow();
            let mut found = None;
            while index < data.entries.len() {
                let entry = data.entries[index].clone();
                index += 1;
                if entry.is_some() {
                    found = entry;
                    break;
                }
            }
            found
        };
        let Some((key, value)) = entry else {
            break;
        };
        let value = if set { key.clone() } else { value };
        ops::call(
            realm,
            &callback,
            receiver.clone(),
            &[value, key, this.clone()],
        )?;
    }
    Ok(Value::Undefined)
}

fn iterator(
    realm: &Realm,
    this: Value,
    set: bool,
    yields: Yield,
    prototype: &Object,
) -> Completion {
    let collection = receiver(realm, &this, set)?;
    let state = CollectionIterator {
        set,
        cursor: Rc::new(RefCell::new(Cursor {
            lease: Some(Lease::new(&collection.data, this.as_object().cloned())),
            index: 0,
            yields,
        })),
    };
    Ok(Value::Object(Object::new(
        Some(prototype.clone()),
        Kind::CollectionIterator(state),
    )))
}

fn next(realm: &mut Realm, this: Value, set: bool) -> Completion {
    let state = this
        .as_object()
        .and_then(|object| match &object.data().kind {
            Kind::CollectionIterator(state) if state.set == set => Some(state.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            realm.throw(
                ErrorKind::Type,
                "collection iterator next called on an incompatible receiver",
            )
        })?;
    let mut cursor = state.cursor.borrow_mut();
    let data = cursor.lease.as_ref().map(|lease| lease.data.clone());
    let entry = data.and_then(|data| {
        let data = data.borrow();
        while cursor.index < data.entries.len() {
            let entry = data.entries[cursor.index].clone();
            cursor.index += 1;
            if entry.is_some() {
                return entry;
            }
        }
        None
    });
    let done = entry.is_none();
    let value = match entry {
        Some((key, value)) => match cursor.yields {
            Yield::Keys => key,
            Yield::Values => {
                if set {
                    key
                } else {
                    value
                }
            }
            Yield::Entries => {
                Value::Object(realm.array(vec![key.clone(), if set { key } else { value }]))
            }
        },
        None => {
            cursor.lease = None;
            Value::Undefined
        }
    };
    let result = realm.object();
    result.define(Key::name("value"), Property::data(value));
    result.define(Key::name("done"), Property::data(Value::Boolean(done)));
    Ok(Value::Object(result))
}

fn accessor(
    realm: &Realm,
    target: &Object,
    key: Key,
    function_name: &str,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) {
    target.define(
        key,
        Property {
            slot: Slot::Accessor {
                get: Some(realm.function(function_name, 0, false, code)),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

fn tag(target: &Object, realm: &Realm, text: &str) {
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

pub(crate) fn install(realm: &Realm) {
    for set in [false, true] {
        let prototype = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
        let fallback = prototype.clone();
        let constructor = realm.builtin_constructor(
            name(set),
            0,
            &prototype,
            move |realm, _, _| {
                Err(realm.throw(
                    ErrorKind::Type,
                    &format!("{} constructor requires new", name(set)),
                ))
            },
            move |realm, target, args| construct(realm, target, args, set, &fallback),
        );
        accessor(
            realm,
            &constructor,
            Key::Symbol(realm.symbols.species.clone()),
            "get [Symbol.species]",
            |_, this, _| Ok(this),
        );
        accessor(
            realm,
            &prototype,
            Key::name("size"),
            "get size",
            move |realm, this, _| Ok(Value::Number(receiver(realm, &this, set)?.size() as f64)),
        );
        tag(&prototype, realm, name(set));
        realm.method(
            &prototype,
            if set { "add" } else { "set" },
            if set { 1 } else { 2 },
            move |realm, this, args| insert(realm, this, args, set),
        );
        if !set {
            realm.method(&prototype, "get", 1, |realm, this, args| {
                let collection = receiver(realm, &this, false)?;
                Ok(collection
                    .data
                    .borrow()
                    .get(&arg(args, 0))
                    .unwrap_or_default())
            });
        }
        realm.method(&prototype, "has", 1, move |realm, this, args| {
            let collection = receiver(realm, &this, set)?;
            Ok(Value::Boolean(
                collection
                    .data
                    .borrow()
                    .positions
                    .get(&NativeKey(arg(args, 0)))
                    .is_some(),
            ))
        });
        realm.method(&prototype, "delete", 1, move |realm, this, args| {
            let collection = receiver(realm, &this, set)?;
            let removed = collection.data.borrow_mut().delete(&arg(args, 0));
            Ok(Value::Boolean(removed.is_some()))
        });
        realm.method(&prototype, "clear", 0, move |realm, this, _| {
            let collection = receiver(realm, &this, set)?;
            let removed = collection.data.borrow_mut().clear();
            drop(removed);
            Ok(Value::Undefined)
        });
        realm.method(&prototype, "forEach", 1, move |realm, this, args| {
            for_each(realm, this, args, set)
        });
        let iterator_prototype =
            Object::new(Some(realm.iterator_prototype.clone()), Kind::Ordinary);
        realm.method(&iterator_prototype, "next", 0, move |realm, this, _| {
            next(realm, this, set)
        });
        tag(
            &iterator_prototype,
            realm,
            &format!("{} Iterator", name(set)),
        );
        let mut default_iterator = None;
        let mut values = None;
        for (method, yields) in [
            ("keys", Yield::Keys),
            ("values", Yield::Values),
            ("entries", Yield::Entries),
        ] {
            if set && method == "keys" {
                continue;
            }
            let iterator_prototype = iterator_prototype.clone();
            let function = realm.function(method, 0, false, move |realm, this, _| {
                iterator(realm, this, set, yields, &iterator_prototype)
            });
            if method == "values" {
                values = Some(function.clone());
            }
            if (set && method == "values") || (!set && method == "entries") {
                default_iterator = Some(function.clone());
            }
            prototype.define(Key::name(method), Property::hidden(Value::Object(function)));
        }
        if set {
            prototype.define(
                Key::name("keys"),
                Property::hidden(Value::Object(values.expect("values installed"))),
            );
        }
        prototype.define(
            Key::Symbol(realm.symbols.iterator.clone()),
            Property::hidden(Value::Object(
                default_iterator.expect("default iterator installed"),
            )),
        );
        realm.set_global(name(set), Value::Object(constructor));
    }
}

#[cfg(test)]
mod tests;
