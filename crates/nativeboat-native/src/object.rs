//! Objects: a prototype link, ordered own properties and an optional native
//! behavior (array storage or a compiled function with its environment).
//!
//! Handles are reference counted. Acyclic data is released as soon as its
//! last handle goes; cyclic data needs the bounded cycle detection planned in
//! the design before applications with cycles can claim bounded memory.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::binding::Binding;
use crate::realm::Realm;
use crate::string::JsStr;
use crate::symbol::Symbol;
use crate::table::HashMap;
use crate::value::{Completion, Value};
use alloc::rc::{Rc, Weak};
use core::cell::{Ref, RefCell, RefMut};
use core::fmt;

/// A property key. Canonical array indexes are kept numerically so that own
/// keys enumerate in ascending index order before string keys.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A canonical array index, below 2³² − 1.
    Index(u32),
    /// Any other string key.
    String(JsStr),
    /// A symbol key.
    Symbol(Symbol),
}

impl Key {
    /// The key for a string, canonicalizing array indexes.
    pub fn from_js(text: &JsStr) -> Key {
        let units = text.units();
        let canonical = !units.is_empty()
            && units.len() <= 10
            && units.iter().all(|unit| (0x30..=0x39).contains(unit))
            && (units.len() == 1 || units[0] != 0x30);
        if canonical {
            let value = units
                .iter()
                .fold(0u64, |value, unit| value * 10 + u64::from(unit - 0x30));
            if value < u64::from(u32::MAX) {
                return Key::Index(value as u32);
            }
        }
        Key::String(text.clone())
    }

    /// The key for Rust text.
    pub fn name(text: &str) -> Key {
        Key::from_js(&JsStr::from(text))
    }

    /// Node's internal keys are own-only, hidden from reflection lists.
    pub(crate) fn is_private(&self) -> bool {
        matches!(self, Key::Symbol(symbol) if symbol.is_private())
    }

    /// The key as text, for messages and enumeration; symbols give their
    /// descriptive string.
    pub fn to_js(&self) -> JsStr {
        match self {
            Key::Index(index) => JsStr::from(index.to_string().as_str()),
            Key::String(text) => text.clone(),
            Key::Symbol(symbol) => symbol.descriptive(),
        }
    }
}

/// Where a property's value comes from.
#[derive(Clone, Debug)]
pub enum Slot {
    /// A stored value.
    Data {
        /// The value.
        value: Value,
        /// Whether assignment may change it.
        writable: bool,
    },
    /// Getter and setter functions.
    Accessor {
        /// The getter, if any.
        get: Option<Object>,
        /// The setter, if any.
        set: Option<Object>,
    },
}

/// An own property.
#[derive(Clone, Debug)]
pub struct Property {
    /// The value or accessors.
    pub slot: Slot,
    /// Whether enumeration includes it.
    pub enumerable: bool,
    /// Whether it may be deleted or redefined.
    pub configurable: bool,
}

impl Property {
    /// A writable, enumerable, configurable data property, as assignment
    /// and object literals create.
    pub fn data(value: Value) -> Property {
        Property {
            slot: Slot::Data {
                value,
                writable: true,
            },
            enumerable: true,
            configurable: true,
        }
    }

    /// A writable, configurable, non-enumerable data property, as built-in
    /// methods are defined.
    pub fn hidden(value: Value) -> Property {
        Property {
            enumerable: false,
            ..Property::data(value)
        }
    }
}

/// What [`Object::cached_property`] found.
pub(crate) enum Cached {
    /// The property, on the object or its prototype chain.
    Found(Property),
    /// No object on the chain has it.
    Absent,
    /// The chain has objects whose properties need the general lookup.
    Unknown,
}

/// The longest prototype chain the cached lookup walks.
const CACHED_DEPTH: usize = 8;

/// What [`Object::lookup_own`] found.
pub(crate) enum OwnLookup {
    /// The own property.
    Found(Property),
    /// No own property; the prototype to look in next.
    Missing(Option<Object>),
    /// The object's own properties need [`Object::own_property`].
    Exotic,
}

/// Objects with at most this many property slots find keys by scanning
/// their fingerprints: most objects are small, and comparing a few words is
/// cheaper than probing a table.
const LINEAR_PROPERTIES: usize = 16;

/// A key's fingerprint for finding it among an object's properties: never
/// zero, equal for equal keys. String keys cache theirs.
fn key_fingerprint(key: &Key) -> u64 {
    let mix = |word: u64| {
        let word = (word ^ (word >> 31)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        (word ^ (word >> 29)) | 1
    };
    match key {
        Key::String(text) => text.fingerprint(),
        Key::Index(index) => mix(u64::from(*index) ^ 0x5bd1_e995_0000_0000),
        Key::Symbol(symbol) => mix(symbol.address() as u64),
    }
}

/// Own properties in creation order, with removal tombstones compacted lazily.
#[derive(Debug, Default)]
struct Properties {
    entries: Vec<Option<(Key, Property)>>,
    /// Each slot's key fingerprint; zero for a removed slot.
    fingerprints: Vec<u64>,
    /// Once there are more slots than a scan suits: an open-addressing
    /// table of slot positions plus one (zero is empty), by fingerprint.
    index: Option<Vec<u32>>,
    /// The table's occupied entries, including ones for removed slots.
    indexed: usize,
    removed: usize,
    /// A bit for each slot's fingerprint, so most keys an object lacks,
    /// as in lookups along prototype chains, are refused without a search.
    bloom: u64,
}

/// A fingerprint's bit in [`Properties::bloom`].
fn bloom_bit(fingerprint: u64) -> u64 {
    1 << (fingerprint >> 58)
}

impl Properties {
    /// Room for `count` properties without growing.
    fn with_capacity(count: usize) -> Properties {
        Properties {
            entries: Vec::with_capacity(count),
            fingerprints: Vec::with_capacity(count),
            ..Properties::default()
        }
    }

    fn position(&self, key: &Key) -> Option<usize> {
        let fingerprint = key_fingerprint(key);
        if self.bloom & bloom_bit(fingerprint) == 0 {
            return None;
        }
        let matches = |position: usize| {
            self.fingerprints[position] == fingerprint
                && self.entries[position]
                    .as_ref()
                    .is_some_and(|(candidate, _)| candidate == key)
        };
        match &self.index {
            None => (0..self.fingerprints.len()).find(|position| matches(*position)),
            Some(index) => {
                let mask = index.len() - 1;
                let mut slot = (fingerprint >> 7) as usize & mask;
                loop {
                    match index[slot] {
                        0 => return None,
                        stored if matches(stored as usize - 1) => return Some(stored as usize - 1),
                        _ => slot = (slot + 1) & mask,
                    }
                }
            }
        }
    }

    fn get(&self, key: &Key) -> Option<&Property> {
        let position = self.position(key)?;
        self.entries[position]
            .as_ref()
            .map(|(_, property)| property)
    }

    fn get_mut(&mut self, key: &Key) -> Option<&mut Property> {
        let position = self.position(key)?;
        self.entries[position]
            .as_mut()
            .map(|(_, property)| property)
    }

    fn insert(&mut self, key: Key, property: Property) {
        match self.position(&key) {
            Some(position) => self.entries[position] = Some((key, property)),
            None => {
                let fingerprint = key_fingerprint(&key);
                self.entries.push(Some((key, property)));
                self.fingerprints.push(fingerprint);
                self.bloom |= bloom_bit(fingerprint);
                if self.index.is_some() {
                    if (self.indexed + 1) * 2 > self.index.as_ref().map_or(0, Vec::len) {
                        self.reindex();
                    } else {
                        self.place(self.entries.len() - 1);
                    }
                } else if self.entries.len() > LINEAR_PROPERTIES {
                    self.reindex();
                }
            }
        }
    }

    /// Adds a slot's position to the table.
    fn place(&mut self, position: usize) {
        let fingerprint = self.fingerprints[position];
        let Some(index) = &mut self.index else {
            return;
        };
        let mask = index.len() - 1;
        let mut slot = (fingerprint >> 7) as usize & mask;
        while index[slot] != 0 {
            slot = (slot + 1) & mask;
        }
        index[slot] = position as u32 + 1;
        self.indexed += 1;
    }

    /// Builds the table for the live slots, or drops it for few slots.
    fn reindex(&mut self) {
        self.indexed = 0;
        let live = self.entries.len() - self.removed;
        if self.entries.len() <= LINEAR_PROPERTIES {
            self.index = None;
            return;
        }
        self.index = Some(alloc::vec![0; (live * 4).next_power_of_two()]);
        for position in 0..self.entries.len() {
            if self.fingerprints[position] != 0 {
                self.place(position);
            }
        }
    }

    fn remove(&mut self, key: &Key) {
        let Some(position) = self.position(key) else {
            return;
        };
        // The table keeps the slot's entry, which no longer matches.
        self.entries[position] = None;
        self.fingerprints[position] = 0;
        self.removed += 1;
        if self.removed * 2 > self.entries.len() {
            self.entries.retain(Option::is_some);
            self.fingerprints.retain(|fingerprint| *fingerprint != 0);
            self.bloom = self
                .fingerprints
                .iter()
                .fold(0, |bloom, fingerprint| bloom | bloom_bit(*fingerprint));
            self.removed = 0;
            self.reindex();
        }
    }

    fn keys(&self) -> impl Iterator<Item = &Key> {
        self.entries.iter().flatten().map(|(key, _)| key)
    }
}

/// A compiled function body: the receiver and arguments in, a completion out.
pub type NativeCode = Rc<dyn Fn(&mut Realm, Value, &[Value]) -> Completion>;

/// A native built-in construction operation, with its explicit new target.
pub type ConstructorCode =
    Rc<dyn Fn(&mut Realm, Object, &[Value]) -> Result<Object, crate::Thrown>>;

/// What an object does beyond holding properties.
pub enum Kind {
    /// A plain object.
    Ordinary,
    /// An unmapped strict arguments object with ordinary indexed properties.
    Arguments,
    /// A symbol wrapper with the primitive symbol as its internal data.
    SymbolObject(Symbol),
    /// An array exotic object.
    Array(ArrayData),
    /// A Boolean, Number or String wrapper with its primitive value. String
    /// wrappers expose read-only index and length properties.
    Primitive(Wrapped),
    /// A function compiled to native code.
    Function {
        /// The behavior, with its closure environment captured inside.
        code: NativeCode,
        /// Whether `new` may call it.
        constructor: bool,
    },
    /// A native built-in with distinct call and construction behavior.
    BuiltinConstructor {
        /// The ordinary call behavior.
        code: NativeCode,
        /// The native object-creation behavior.
        construct: ConstructorCode,
    },
    /// A native bound function with its target and leading arguments.
    BoundFunction {
        /// The call behavior, capturing the bound receiver and arguments.
        code: NativeCode,
        /// The original callable object, also used for construction/instanceof.
        target: Object,
        /// Arguments prepended to construction arguments.
        arguments: Rc<[Value]>,
        /// Whether the target supports construction.
        constructor: bool,
    },
    /// An error object.
    Error,
    /// A built-in array or string iterator.
    Iterator(crate::iter::State),
    /// An insertion-ordered native Map or Set.
    Collection(crate::collection_builtin::Collection),
    /// A live native Map/Set iterator.
    CollectionIterator(crate::collection_builtin::CollectionIterator),
    /// A regular expression with its source, flags and compiled pattern.
    RegExp(crate::regexp_builtin::RegExpData),
    /// A RegExp String Iterator, as `matchAll` returns.
    RegExpStringIterator(Rc<core::cell::RefCell<crate::regexp_builtin::StringIterator>>),
    /// A promise and its reactions.
    Promise(crate::promise_builtin::Shared),
    /// A timer or immediate handle with its lifetime state.
    Timer(Rc<crate::event_loop::Timer>),
    /// A generator object and its suspended body.
    Generator(Rc<crate::coroutine::Coroutine>),
    /// An async generator object: its body and queued requests.
    AsyncGenerator(Rc<crate::coroutine::AsyncGenerator>),
    /// A native view of Windows environment variables.
    Environment,
    /// A date and its time value in milliseconds since the epoch, or NaN.
    Date(f64),
    /// An `ArrayBuffer`'s bytes.
    ArrayBuffer(crate::typed_array_builtin::Bytes),
    /// A typed array: an integer-indexed window over a buffer.
    TypedArray(crate::typed_array_builtin::View),
    /// A `DataView` over a buffer.
    DataView(crate::typed_array_builtin::DataView),
    /// A WeakMap or WeakSet.
    WeakCollection(crate::weak_builtin::WeakCollection),
    /// A WeakRef and its target.
    WeakRef(crate::weak_builtin::WeakKey),
    /// A FinalizationRegistry and its registrations.
    FinalizationRegistry(alloc::rc::Rc<crate::weak_builtin::Registry>),
    /// Data a native binding keeps for script code to hold and pass back
    /// opaquely, such as an edit-history snapshot.
    Native(alloc::rc::Rc<dyn core::any::Any>),
    /// A proxy exotic object.
    Proxy(crate::proxy_builtin::Proxy),
}

impl fmt::Debug for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Ordinary => formatter.write_str("Ordinary"),
            Kind::Arguments => formatter.write_str("Arguments"),
            Kind::SymbolObject(symbol) => write!(formatter, "SymbolObject({symbol:?})"),
            Kind::Array(array) => write!(formatter, "Array({})", array.length),
            Kind::Primitive(value) => write!(formatter, "Primitive({value:?})"),
            Kind::Function { constructor, .. } => write!(formatter, "Function({constructor})"),
            Kind::BuiltinConstructor { .. } => formatter.write_str("BuiltinConstructor"),
            Kind::BoundFunction { constructor, .. } => {
                write!(formatter, "BoundFunction({constructor})")
            }
            Kind::Error => formatter.write_str("Error"),
            Kind::Iterator(state) => write!(formatter, "Iterator({state:?})"),
            Kind::Collection(collection) => write!(
                formatter,
                "Collection({}, {})",
                collection.set,
                collection.size()
            ),
            Kind::CollectionIterator(iterator) => {
                write!(formatter, "CollectionIterator({})", iterator.set)
            }
            Kind::RegExp(data) => write!(formatter, "RegExp(/{}/{})", data.source, data.flags),
            Kind::RegExpStringIterator(_) => formatter.write_str("RegExpStringIterator"),
            Kind::Promise(state) => write!(formatter, "Promise({:?})", state.borrow().state),
            Kind::Timer(state) => write!(formatter, "Timer({})", state.id.get()),
            Kind::Generator(coroutine) => write!(formatter, "Generator({coroutine:?})"),
            Kind::AsyncGenerator(_) => write!(formatter, "AsyncGenerator"),
            Kind::Environment => formatter.write_str("Environment"),
            Kind::Date(time) => write!(formatter, "Date({time})"),
            Kind::ArrayBuffer(bytes) => write!(formatter, "ArrayBuffer({})", bytes.borrow().len()),
            Kind::TypedArray(view) => write!(formatter, "{}({})", view.element.name(), view.length),
            Kind::DataView(view) => write!(formatter, "DataView({})", view.length),
            Kind::WeakCollection(collection) => {
                write!(formatter, "WeakCollection({})", collection.set)
            }
            Kind::WeakRef(_) => formatter.write_str("WeakRef"),
            Kind::FinalizationRegistry(_) => formatter.write_str("FinalizationRegistry"),
            Kind::Native(_) => formatter.write_str("Native"),
            Kind::Proxy(_) => formatter.write_str("Proxy"),
        }
    }
}

/// The state behind an object handle.
#[derive(Debug)]
pub struct ObjectData {
    /// The prototype, if any.
    pub prototype: Option<Object>,
    /// Whether properties may be added.
    pub extensible: bool,
    /// Native behavior.
    pub kind: Kind,
    properties: Properties,
    /// Sloppy arguments indices linked to their formal parameter bindings.
    mapped_arguments: HashMap<u32, Binding>,
    /// Private class elements, keyed by their class's private names. They are
    /// invisible to property operations and reflection.
    private: Vec<(Symbol, Property)>,
}

/// A shared handle to an object. Equality is identity.
#[derive(Clone)]
pub struct Object(Rc<RefCell<ObjectData>>);

impl PartialEq for Object {
    fn eq(&self, other: &Object) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::hash::Hash for Object {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::ptr::hash(Rc::as_ptr(&self.0), state);
    }
}

impl fmt::Debug for Object {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Object({:p})", Rc::as_ptr(&self.0))
    }
}

/// A handle that does not keep its object alive, for an object's own code
/// referring back to the object without a reference cycle.
#[derive(Clone, Debug)]
pub struct WeakObject(Weak<RefCell<ObjectData>>);

impl WeakObject {
    /// The object, while it is alive.
    pub fn upgrade(&self) -> Option<Object> {
        self.0.upgrade().map(Object)
    }

    /// Whether the object is still alive.
    pub fn alive(&self) -> bool {
        self.0.strong_count() > 0
    }

    /// The object's identity; reused only after the object is gone.
    pub fn address(&self) -> usize {
        self.0.as_ptr() as *const u8 as usize
    }
}

/// How far past its dense storage an array index may be written before the
/// element is kept as a sparse property instead of growing the storage.
const DENSE_GAP: usize = 1 << 10;

/// Array storage: elements with default attributes (writable, enumerable and
/// configurable data) are dense; elements with other attributes, and those
/// far beyond the dense storage, are ordinary indexed properties.
#[derive(Debug, Clone)]
pub struct ArrayData {
    elements: Vec<Option<Value>>,
    length: u32,
    length_writable: bool,
}

impl ArrayData {
    /// The array's internal length, without invoking source getters.
    pub(crate) fn length(&self) -> u32 {
        self.length
    }

    /// Dense storage; `None` is a hole.
    pub fn new(elements: Vec<Option<Value>>) -> ArrayData {
        let length = u32::try_from(elements.len()).expect("array lengths fit in u32");
        ArrayData {
            elements,
            length,
            length_writable: true,
        }
    }

    fn dense(&self, index: u32) -> Option<&Value> {
        self.elements.get(index as usize).and_then(Option::as_ref)
    }

    fn fits(&self, index: u32) -> bool {
        let index = index as usize;
        index < self.elements.len() + DENSE_GAP || index < self.elements.len() * 2
    }
}

/// The primitive value a Boolean, Number or String wrapper holds.
#[derive(Debug, Clone)]
pub enum Wrapped {
    /// `[[BooleanData]]`.
    Boolean(bool),
    /// `[[NumberData]]`.
    Number(f64),
    /// `[[StringData]]`.
    String(JsStr),
    /// `[[BigIntData]]`.
    BigInt(alloc::rc::Rc<crate::bigint::BigInt>),
}

/// A property descriptor: each field is present or absent, as in
/// `Object.defineProperty`.
#[derive(Debug, Clone, Default)]
pub struct Descriptor {
    /// `value`.
    pub value: Option<Value>,
    /// `writable`.
    pub writable: Option<bool>,
    /// `get`: a function or `undefined`.
    pub get: Option<Option<Object>>,
    /// `set`: a function or `undefined`.
    pub set: Option<Option<Object>>,
    /// `enumerable`.
    pub enumerable: Option<bool>,
    /// `configurable`.
    pub configurable: Option<bool>,
}

impl Descriptor {
    /// Whether `get` or `set` is present.
    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// Whether `value` or `writable` is present.
    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// The complete descriptor of an existing property.
    /// `CompletePropertyDescriptor`: absent fields take their defaults.
    pub fn complete(&self) -> Property {
        let slot = if self.is_accessor() {
            Slot::Accessor {
                get: self.get.clone().flatten(),
                set: self.set.clone().flatten(),
            }
        } else {
            Slot::Data {
                value: self.value.clone().unwrap_or_default(),
                writable: self.writable.unwrap_or(false),
            }
        };
        Property {
            slot,
            enumerable: self.enumerable.unwrap_or(false),
            configurable: self.configurable.unwrap_or(false),
        }
    }

    pub fn of(property: &Property) -> Descriptor {
        let (value, writable, get, set) = match &property.slot {
            Slot::Data { value, writable } => (Some(value.clone()), Some(*writable), None, None),
            Slot::Accessor { get, set } => (None, None, Some(get.clone()), Some(set.clone())),
        };
        Descriptor {
            value,
            writable,
            get,
            set,
            enumerable: Some(property.enumerable),
            configurable: Some(property.configurable),
        }
    }
}

/// `SameValue`: NaN equals itself and the two zeros differ.
pub fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Symbol(left), Value::Symbol(right)) => left == right,
        (Value::BigInt(left), Value::BigInt(right)) => left == right,
        (Value::Object(left), Value::Object(right)) => left == right,
        _ => false,
    }
}

/// The code units of a String wrapper, if this is one.
fn wrapped_text(kind: &Kind) -> Option<&JsStr> {
    match kind {
        Kind::Primitive(Wrapped::String(text)) => Some(text),
        _ => None,
    }
}

impl Object {
    /// A new object.
    pub fn new(prototype: Option<Object>, kind: Kind) -> Object {
        Object::with_capacity(prototype, kind, 0)
    }

    /// A new object with room for `properties` properties, as an object
    /// literal knows it needs.
    pub fn with_capacity(prototype: Option<Object>, kind: Kind, properties: usize) -> Object {
        Object(Rc::new(RefCell::new(ObjectData {
            prototype,
            extensible: true,
            kind,
            properties: Properties::with_capacity(properties),
            mapped_arguments: HashMap::new(),
            private: Vec::new(),
        })))
    }

    /// A handle that does not keep the object alive.
    pub fn downgrade(&self) -> WeakObject {
        WeakObject(Rc::downgrade(&self.0))
    }

    /// Reads the object's state.
    pub fn data(&self) -> Ref<'_, ObjectData> {
        self.0.borrow()
    }

    /// Changes the object's state.
    pub fn data_mut(&self) -> RefMut<'_, ObjectData> {
        self.0.borrow_mut()
    }

    /// The compiled code, if this is a function.
    pub fn code(&self) -> Option<(NativeCode, bool)> {
        match &self.data().kind {
            Kind::Function { code, constructor } => Some((code.clone(), *constructor)),
            Kind::BoundFunction {
                code, constructor, ..
            } => Some((code.clone(), *constructor)),
            Kind::BuiltinConstructor { code, .. } => Some((code.clone(), true)),
            Kind::Proxy(proxy) => proxy
                .call
                .clone()
                .map(|code| (code, proxy.construct.is_some())),
            _ => None,
        }
    }

    /// A built-in's specialized construction operation, if present.
    pub fn constructor_code(&self) -> Option<ConstructorCode> {
        match &self.data().kind {
            Kind::BuiltinConstructor { construct, .. } => Some(construct.clone()),
            Kind::Proxy(proxy) => proxy.construct.clone(),
            _ => None,
        }
    }

    /// Whether this is an array.
    pub fn is_array(&self) -> bool {
        matches!(self.data().kind, Kind::Array(_))
    }

    /// The own property for a key, as a descriptor.
    /// A typed array's view and the numeric index a key names, for keys
    /// the array owns itself.
    fn typed_element(&self, key: &Key) -> Option<(crate::typed_array_builtin::View, f64)> {
        let view = match &self.data().kind {
            Kind::TypedArray(view) => view.clone(),
            _ => return None,
        };
        crate::typed_array_builtin::numeric_key(key).map(|index| (view, index))
    }

    /// An own property lookup in one borrow, for the objects whose own
    /// properties are their stored ones; others, such as proxies, typed
    /// arrays and string wrappers, answer `Exotic` and need
    /// [`Object::own_property`].
    /// The property a string key names on this object or its prototype
    /// chain, read where `cache` remembers it when that slot still holds the
    /// key and the objects before it lack the key, else looked up and
    /// remembered. Objects with exotic properties leave it to the general
    /// lookup.
    pub(crate) fn cached_property(&self, key: &Key, cache: &crate::string::LookupCache) -> Cached {
        let ordinary = |data: &ObjectData| {
            data.mapped_arguments.is_empty()
                && match &data.kind {
                    Kind::Proxy(_)
                    | Kind::Environment
                    | Kind::TypedArray(_)
                    | Kind::Primitive(_) => false,
                    Kind::Array(_) => !matches!(key, Key::Index(_)) && !is_length(key),
                    _ => true,
                }
        };
        let fingerprint = key_fingerprint(key);
        let at = |data: &ObjectData, slot: usize| -> Option<Property> {
            let properties = &data.properties;
            (properties.fingerprints.get(slot) == Some(&fingerprint))
                .then(|| properties.entries[slot].as_ref())
                .flatten()
                .filter(|(candidate, _)| candidate == key)
                .map(|(_, property)| property.clone())
        };
        let places = cache.get();
        // Where the cache says.
        let mut current = self.clone();
        for depth in 0..CACHED_DEPTH {
            let next = {
                let data = current.data();
                if !ordinary(&data) {
                    break;
                }
                let address = Rc::as_ptr(&current.0) as usize;
                let mut here = false;
                for (holder, slot) in places {
                    if holder == address || (holder == 0 && depth == 0) {
                        if let Some(property) = at(&data, slot) {
                            return Cached::Found(property);
                        }
                        here = true;
                    }
                }
                // The key may be on this object, where the cache missed it.
                if here || data.properties.bloom & bloom_bit(fingerprint) != 0 {
                    break;
                }
                data.prototype.clone()
            };
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        // Looked up and remembered.
        let mut current = self.clone();
        for depth in 0..CACHED_DEPTH {
            let next = {
                let data = current.data();
                if !ordinary(&data) {
                    return Cached::Unknown;
                }
                if let Some(position) = data.properties.position(key) {
                    let holder = if depth == 0 {
                        0
                    } else {
                        Rc::as_ptr(&current.0) as usize
                    };
                    cache.set(holder, position);
                    return data.properties.entries[position]
                        .as_ref()
                        .map_or(Cached::Unknown, |(_, property)| {
                            Cached::Found(property.clone())
                        });
                }
                data.prototype.clone()
            };
            match next {
                Some(next) => current = next,
                None => return Cached::Absent,
            }
        }
        Cached::Unknown
    }

    pub(crate) fn lookup_own(&self, key: &Key) -> OwnLookup {
        let data = self.data();
        match &data.kind {
            Kind::Proxy(_) | Kind::Environment | Kind::TypedArray(_) | Kind::Primitive(_) => {
                return OwnLookup::Exotic;
            }
            Kind::Array(array) => {
                if let Key::Index(index) = key
                    && let Some(value) = array.dense(*index)
                {
                    return OwnLookup::Found(Property::data(value.clone()));
                }
                if is_length(key) {
                    return OwnLookup::Found(fixed_length(array.length, array.length_writable));
                }
            }
            _ => {}
        }
        if !data.mapped_arguments.is_empty() {
            return OwnLookup::Exotic;
        }
        match data.properties.get(key) {
            Some(property) => OwnLookup::Found(property.clone()),
            None => OwnLookup::Missing(data.prototype.clone()),
        }
    }

    pub fn own_property(&self, key: &Key) -> Option<Property> {
        if let Some((view, index)) = self.typed_element(key) {
            return view.get(index).map(Property::data);
        }
        let data = self.data();
        if matches!(data.kind, Kind::Environment) {
            return crate::environment_builtin::property(key);
        }
        if let Kind::Array(array) = &data.kind {
            if let Key::Index(index) = key
                && let Some(value) = array.dense(*index)
            {
                return Some(Property::data(value.clone()));
            }
            if is_length(key) {
                return Some(fixed_length(array.length, array.length_writable));
            }
        }
        if let Some(text) = wrapped_text(&data.kind) {
            match key {
                Key::Index(index) if (*index as usize) < text.len() => {
                    let unit = JsStr::from_units(&text.units()[*index as usize..=*index as usize]);
                    return Some(Property {
                        slot: Slot::Data {
                            value: Value::String(unit),
                            writable: false,
                        },
                        enumerable: true,
                        configurable: false,
                    });
                }
                key if is_length(key) => {
                    return Some(fixed_length(text.len() as u32, false));
                }
                _ => {}
            }
        }
        let mut property = data.properties.get(key).cloned()?;
        if let Key::Index(index) = key
            && let Some(binding) = data.mapped_arguments.get(index)
            && let Slot::Data { value, .. } = &mut property.slot
        {
            *value = binding.borrow().clone().unwrap_or_default();
        }
        Some(property)
    }

    /// Links an existing indexed arguments property to its formal parameter.
    pub(crate) fn map_argument(&self, index: u32, binding: Binding) {
        if self.own_property(&Key::Index(index)).is_some() {
            self.data_mut().mapped_arguments.insert(index, binding);
        }
    }

    /// Defines or replaces an own property without the redefinition checks
    /// of `defineProperty`: used for object literals, built-ins and fresh
    /// objects. Returns false for keys an exotic object owns itself (array
    /// `length`, String wrapper indexes and `length`).
    pub fn define(&self, key: Key, mut property: Property) -> bool {
        if key.is_private() {
            property.enumerable = false;
        }
        if self.typed_element(&key).is_some() {
            return self.define_own_property(&key, &Descriptor::of(&property));
        }
        let mut data = self.data_mut();
        if wrapped_text(&data.kind).is_some_and(|text| owned_by_text(text, &key)) {
            return false;
        }
        if let Kind::Array(array) = &mut data.kind {
            if is_length(&key) {
                return false;
            }
            if let Key::Index(index) = key {
                array.length = array.length.max(index + 1);
                // Default-attribute data elements are dense.
                if let Property {
                    slot:
                        Slot::Data {
                            value,
                            writable: true,
                        },
                    enumerable: true,
                    configurable: true,
                } = &property
                    && array.fits(index)
                {
                    let slot = index as usize;
                    if slot >= array.elements.len() {
                        array.elements.resize(slot + 1, None);
                    }
                    array.elements[slot] = Some(value.clone());
                    data.properties.remove(&key);
                    return true;
                }
                if let Some(slot) = array.elements.get_mut(index as usize) {
                    *slot = None;
                }
            }
        }
        data.properties.insert(key, property);
        true
    }

    /// Removes an own property; false if it is not configurable.
    pub fn delete(&self, key: &Key) -> bool {
        if let Some((view, index)) = self.typed_element(key) {
            return view.index(index).is_none();
        }
        let mut data = self.data_mut();
        if wrapped_text(&data.kind).is_some_and(|text| owned_by_text(text, key)) {
            return false;
        }
        if let Kind::Array(array) = &mut data.kind {
            if is_length(key) {
                return false;
            }
            if let Key::Index(index) = key
                && let Some(slot @ Some(_)) = array.elements.get_mut(*index as usize)
            {
                *slot = None;
                return true;
            }
        }
        if data
            .properties
            .get(key)
            .is_some_and(|property| !property.configurable)
        {
            return false;
        }
        data.properties.remove(key);
        if let Key::Index(index) = key {
            data.mapped_arguments.remove(index);
        }
        true
    }

    /// Replaces the value of an own writable data property, the common
    /// assignment, in one lookup, first where `cache` says; gives the value
    /// back when the object or the property needs the general `[[Set]]`.
    pub(crate) fn overwrite_data(
        &self,
        key: &Key,
        value: Value,
        cache: Option<&crate::string::LookupCache>,
    ) -> Result<(), Value> {
        let mut data = self.data_mut();
        let plain = data.mapped_arguments.is_empty()
            && match &data.kind {
                Kind::Proxy(_) | Kind::Environment | Kind::TypedArray(_) | Kind::Primitive(_) => {
                    false
                }
                Kind::Array(_) => !matches!(key, Key::Index(_)) && !is_length(key),
                _ => true,
            };
        if !plain {
            return Err(value);
        }
        let properties = &mut data.properties;
        let cached = cache.and_then(|cache| {
            let fingerprint = key_fingerprint(key);
            cache.get().into_iter().find_map(|(holder, slot)| {
                (holder == 0
                    && properties.fingerprints.get(slot) == Some(&fingerprint)
                    && properties.entries[slot]
                        .as_ref()
                        .is_some_and(|(candidate, _)| candidate == key))
                .then_some(slot)
            })
        });
        let position = match cached {
            Some(slot) => Some(slot),
            None => {
                let position = properties.position(key);
                if let (Some(cache), Some(position)) = (cache, position) {
                    cache.set(0, position);
                }
                position
            }
        };
        let entry = position.and_then(|position| properties.entries[position].as_mut());
        let old = match entry.map(|(_, property)| property) {
            Some(Property {
                slot:
                    Slot::Data {
                        value: slot,
                        writable: true,
                    },
                ..
            }) => core::mem::replace(slot, value),
            _ => return Err(value),
        };
        // The old value is dropped once the object is no longer borrowed.
        drop(data);
        drop(old);
        Ok(())
    }

    /// Writes the value of an existing own writable data property, or adds
    /// a new data property. Returns false if the write is refused.
    pub(crate) fn write_own(&self, key: &Key, value: Value) -> bool {
        let mut guard = self.data_mut();
        let mapped = match key {
            Key::Index(index) => guard.mapped_arguments.get(index).cloned(),
            _ => None,
        };
        let ObjectData {
            extensible,
            kind,
            properties,
            ..
        } = &mut *guard;
        let extensible = *extensible || key.is_private();
        if wrapped_text(kind).is_some_and(|text| owned_by_text(text, key)) {
            return false;
        }
        if let Kind::Array(array) = kind {
            if is_length(key) {
                return array.length_writable
                    && match value {
                        Value::Number(length) => set_length(array, properties, length),
                        _ => false,
                    };
            }
            if let Key::Index(index) = key {
                if let Some(Some(slot)) = array.elements.get_mut(*index as usize) {
                    *slot = value;
                    return true;
                }
                if properties.get(key).is_none()
                    && (!extensible || (*index >= array.length && !array.length_writable))
                {
                    return false;
                }
            }
        }
        match properties.get_mut(key) {
            Some(Property {
                slot:
                    Slot::Data {
                        value: slot,
                        writable,
                    },
                ..
            }) if *writable => {
                *slot = value.clone();
                if let Some(binding) = mapped {
                    *binding.borrow_mut() = Some(value);
                }
                true
            }
            Some(_) => false,
            None if extensible => {
                drop(guard);
                self.define(key.clone(), Property::data(value))
            }
            None => false,
        }
    }

    /// `[[DefineOwnProperty]]` with the language's validation: false when a
    /// non-configurable or non-extensible object refuses the change.
    pub fn define_own_property(&self, key: &Key, descriptor: &Descriptor) -> bool {
        let mut private_descriptor;
        let descriptor = if key.is_private() {
            private_descriptor = descriptor.clone();
            private_descriptor.enumerable = Some(false);
            &private_descriptor
        } else {
            descriptor
        };
        // Typed array elements are writable, enumerable, configurable data.
        if let Some((view, index)) = self.typed_element(key) {
            let allowed = view.index(index).is_some()
                && descriptor.configurable != Some(false)
                && descriptor.enumerable != Some(false)
                && !descriptor.is_accessor()
                && descriptor.writable != Some(false);
            if allowed && let Some(value) = &descriptor.value {
                let value = if view.element.is_bigint() {
                    match value {
                        Value::BigInt(_) => value.clone(),
                        Value::Boolean(flag) => Value::BigInt(alloc::rc::Rc::new(
                            crate::bigint::BigInt::from_i64(i64::from(*flag)),
                        )),
                        Value::String(text) => {
                            let Some(integer) = crate::bigint::BigInt::parse(text.units()) else {
                                return false;
                            };
                            Value::BigInt(alloc::rc::Rc::new(integer))
                        }
                        _ => return false,
                    }
                } else {
                    Value::Number(crate::typed_array_builtin::primitive_number(value))
                };
                view.set(index, &value);
            }
            return allowed;
        }
        {
            let mut guard = self.data_mut();
            let ObjectData {
                kind, properties, ..
            } = &mut *guard;
            if let Kind::Array(array) = kind {
                if is_length(key) {
                    return define_length(array, properties, descriptor);
                }
                if let Key::Index(index) = key
                    && *index >= array.length
                    && !array.length_writable
                {
                    return false;
                }
            }
        }
        let current = self.own_property(key);
        let extensible = self.data().extensible;
        let Some(current) = current else {
            if !extensible {
                return false;
            }
            let slot = if descriptor.is_accessor() {
                Slot::Accessor {
                    get: descriptor.get.clone().flatten(),
                    set: descriptor.set.clone().flatten(),
                }
            } else {
                Slot::Data {
                    value: descriptor.value.clone().unwrap_or_default(),
                    writable: descriptor.writable.unwrap_or(false),
                }
            };
            let property = Property {
                slot,
                enumerable: descriptor.enumerable.unwrap_or(false),
                configurable: descriptor.configurable.unwrap_or(false),
            };
            return self.define(key.clone(), property);
        };
        if !compatible(&current, descriptor) {
            return false;
        }
        let merged = merge(current, descriptor);
        if wrapped_text(&self.data().kind).is_some_and(|text| owned_by_text(text, key)) {
            // A compatible redefinition of a String wrapper's own index or
            // length changes nothing.
            return true;
        }
        let mapped = match key {
            Key::Index(index) => self.data().mapped_arguments.get(index).cloned(),
            _ => None,
        };
        let disconnect = descriptor.is_accessor() || descriptor.writable == Some(false);
        let defined = self.define(key.clone(), merged);
        if defined && let Some(binding) = mapped {
            if let Some(value) = &descriptor.value {
                *binding.borrow_mut() = Some(value.clone());
            }
            if disconnect && let Key::Index(index) = key {
                self.data_mut().mapped_arguments.remove(index);
            }
        }
        defined
    }

    /// `[[SetPrototypeOf]]`: false for a non-extensible object whose
    /// prototype would change, or a change that would create a cycle.
    pub fn set_prototype(&self, prototype: Option<Object>) -> bool {
        if self.data().prototype == prototype {
            return true;
        }
        if !self.is_extensible() {
            return false;
        }
        let mut current = prototype.clone();
        while let Some(object) = current {
            if &object == self {
                return false;
            }
            current = object.data().prototype.clone();
        }
        self.data_mut().prototype = prototype;
        true
    }

    /// A private element, if present.
    pub fn private_property(&self, name: &Symbol) -> Option<Property> {
        self.data()
            .private
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, property)| property.clone())
    }

    /// Adds or replaces a private element.
    pub fn set_private(&self, name: &Symbol, property: Property) {
        let mut data = self.data_mut();
        match data.private.iter_mut().find(|(key, _)| key == name) {
            Some(entry) => entry.1 = property,
            None => data.private.push((name.clone(), property)),
        }
    }

    /// Whether properties may be added.
    pub fn is_extensible(&self) -> bool {
        self.data().extensible
    }

    /// `[[PreventExtensions]]`.
    pub fn prevent_extensions(&self) {
        self.data_mut().extensible = false;
    }

    /// Own property keys in the source language's order: indexes ascending,
    /// then strings in creation order, then symbols in creation order.
    pub fn own_keys(&self) -> Vec<Key> {
        let data = self.data();
        if matches!(data.kind, Kind::Environment) {
            return crate::environment_builtin::keys();
        }
        let mut indexes: Vec<u32> = Vec::new();
        let mut strings = Vec::new();
        let mut symbols = Vec::new();
        if let Kind::Array(array) = &data.kind {
            indexes.extend(
                array
                    .elements
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| value.is_some())
                    .map(|(index, _)| index as u32),
            );
            strings.push(Key::name("length"));
        }
        if let Kind::TypedArray(view) = &data.kind {
            indexes.extend(0..view.length as u32);
        }
        if let Some(text) = wrapped_text(&data.kind) {
            indexes.extend(0..text.len() as u32);
            strings.push(Key::name("length"));
        }
        for key in data.properties.keys() {
            match key {
                Key::Index(index) => indexes.push(*index),
                Key::String(_) => strings.push(key.clone()),
                Key::Symbol(symbol) => {
                    if !symbol.is_private() {
                        symbols.push(key.clone());
                    }
                }
            }
        }
        indexes.sort_unstable();
        indexes
            .into_iter()
            .map(Key::Index)
            .chain(strings)
            .chain(symbols)
            .collect()
    }
}

/// `ArraySetLength`. The caller has converted a new length value.
fn define_length(
    array: &mut ArrayData,
    properties: &mut Properties,
    descriptor: &Descriptor,
) -> bool {
    if descriptor.configurable == Some(true)
        || descriptor.enumerable == Some(true)
        || descriptor.is_accessor()
        || (!array.length_writable && descriptor.writable == Some(true))
    {
        return false;
    }
    let mut done = true;
    if let Some(Value::Number(length)) = &descriptor.value
        && *length != f64::from(array.length)
    {
        done = array.length_writable && set_length(array, properties, *length);
    }
    if descriptor.writable == Some(false) {
        array.length_writable = false;
    }
    done
}

/// Whether a redefinition is allowed by the current property's attributes
/// (`ValidateAndApplyPropertyDescriptor`).
fn compatible(current: &Property, descriptor: &Descriptor) -> bool {
    if current.configurable {
        return true;
    }
    if descriptor.configurable == Some(true)
        || descriptor
            .enumerable
            .is_some_and(|enumerable| enumerable != current.enumerable)
    {
        return false;
    }
    match &current.slot {
        Slot::Accessor { get, set } => {
            !descriptor.is_data()
                && descriptor.get.as_ref().is_none_or(|new| new == get)
                && descriptor.set.as_ref().is_none_or(|new| new == set)
        }
        Slot::Data { value, writable } => {
            !descriptor.is_accessor()
                && (*writable
                    || (descriptor.writable != Some(true)
                        && descriptor
                            .value
                            .as_ref()
                            .is_none_or(|new| same_value(new, value))))
        }
    }
}

/// Applies a compatible descriptor to the current property.
fn merge(current: Property, descriptor: &Descriptor) -> Property {
    let slot = match current.slot {
        Slot::Data { .. } if descriptor.is_accessor() => Slot::Accessor {
            get: descriptor.get.clone().flatten(),
            set: descriptor.set.clone().flatten(),
        },
        Slot::Accessor { .. } if descriptor.is_data() => Slot::Data {
            value: descriptor.value.clone().unwrap_or_default(),
            writable: descriptor.writable.unwrap_or(false),
        },
        Slot::Data { value, writable } => Slot::Data {
            value: descriptor.value.clone().unwrap_or(value),
            writable: descriptor.writable.unwrap_or(writable),
        },
        Slot::Accessor { get, set } => Slot::Accessor {
            get: descriptor.get.clone().unwrap_or(get),
            set: descriptor.set.clone().unwrap_or(set),
        },
    };
    Property {
        slot,
        enumerable: descriptor.enumerable.unwrap_or(current.enumerable),
        configurable: descriptor.configurable.unwrap_or(current.configurable),
    }
}

fn fixed_length(length: u32, writable: bool) -> Property {
    Property {
        slot: Slot::Data {
            value: Value::Number(f64::from(length)),
            writable,
        },
        enumerable: false,
        configurable: false,
    }
}

pub(crate) fn is_length(key: &Key) -> bool {
    matches!(key, Key::String(name) if name.units() == LENGTH)
}

/// Whether a String wrapper owns this key itself.
fn owned_by_text(text: &JsStr, key: &Key) -> bool {
    matches!(key, Key::Index(index) if (*index as usize) < text.len()) || is_length(key)
}

const LENGTH: &[u16] = &[0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68];

/// Sets an array's length: an integral number up to 2³² − 1. Shrinking
/// deletes elements from the end and stops at a non-configurable one, which
/// keeps the length just above it; that partial result returns false.
fn set_length(array: &mut ArrayData, properties: &mut Properties, length: f64) -> bool {
    if crate::float::fract(length) != 0.0 || !(0.0..4_294_967_296.0).contains(&length) {
        return false;
    }
    let length = length as u32;
    if length < array.length {
        let mut sparse: Vec<u32> = properties
            .keys()
            .filter_map(|key| match key {
                Key::Index(index) if *index >= length => Some(*index),
                _ => None,
            })
            .collect();
        sparse.sort_unstable_by(|a, b| b.cmp(a));
        for index in sparse {
            let key = Key::Index(index);
            if properties
                .get(&key)
                .is_some_and(|property| !property.configurable)
            {
                array.length = index + 1;
                array.elements.truncate((index + 1) as usize);
                return false;
            }
            properties.remove(&key);
        }
        array.elements.truncate(length as usize);
    }
    array.length = length;
    true
}

#[cfg(test)]
mod tests;
