//! `ArrayBuffer`, the eleven typed arrays and `DataView`, as
//! ECMAScript 2024 specifies them for fixed-length buffers. Typed arrays
//! are integer-indexed exotic objects: element keys read and write the
//! buffer's little-endian bytes (see the hooks in `object` and `ops`).
//! Methods with generic Array semantics reuse the intrinsic Array
//! functions after validating the receiver. Resizable and transferable
//! buffers are not provided.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{
    index_key, is_callable, is_constructor, length_of, to_integer_or_infinity,
};
use crate::object::{Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};
use alloc::rc::Rc;
use core::cell::RefCell;

/// The bytes of an `ArrayBuffer`, shared with its views.
pub type Bytes = Rc<RefCell<Vec<u8>>>;

/// A typed array's element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
}

pub(crate) const ELEMENTS: [Element; 11] = [
    Element::Int8,
    Element::Uint8,
    Element::Uint8Clamped,
    Element::Int16,
    Element::Uint16,
    Element::Int32,
    Element::Uint32,
    Element::Float32,
    Element::Float64,
    Element::BigInt64,
    Element::BigUint64,
];

impl Element {
    /// `BYTES_PER_ELEMENT`.
    pub fn size(self) -> usize {
        match self {
            Element::Int8 | Element::Uint8 | Element::Uint8Clamped => 1,
            Element::Int16 | Element::Uint16 => 2,
            Element::Int32 | Element::Uint32 | Element::Float32 => 4,
            Element::Float64 | Element::BigInt64 | Element::BigUint64 => 8,
        }
    }

    /// The constructor name.
    pub fn name(self) -> &'static str {
        match self {
            Element::Int8 => "Int8Array",
            Element::Uint8 => "Uint8Array",
            Element::Uint8Clamped => "Uint8ClampedArray",
            Element::Int16 => "Int16Array",
            Element::Uint16 => "Uint16Array",
            Element::Int32 => "Int32Array",
            Element::Uint32 => "Uint32Array",
            Element::Float32 => "Float32Array",
            Element::Float64 => "Float64Array",
            Element::BigInt64 => "BigInt64Array",
            Element::BigUint64 => "BigUint64Array",
        }
    }

    /// The value of little-endian element bytes.
    pub fn read(self, bytes: &[u8]) -> Value {
        let mut word = [0u8; 8];
        word[..bytes.len()].copy_from_slice(bytes);
        let raw = u64::from_le_bytes(word);
        let number = match self {
            Element::BigInt64 => {
                return Value::BigInt(Rc::new(crate::bigint::BigInt::from_i64(raw as i64)));
            }
            Element::BigUint64 => {
                return Value::BigInt(Rc::new(crate::bigint::BigInt::from_u64(raw)));
            }
            Element::Int8 => f64::from(raw as u8 as i8),
            Element::Uint8 | Element::Uint8Clamped => f64::from(raw as u8),
            Element::Int16 => f64::from(raw as u16 as i16),
            Element::Uint16 => f64::from(raw as u16),
            Element::Int32 => f64::from(raw as u32 as i32),
            Element::Uint32 => f64::from(raw as u32),
            Element::Float32 => f64::from(f32::from_bits(raw as u32)),
            Element::Float64 => f64::from_bits(raw),
        };
        Value::Number(number)
    }

    /// Whether this element type has BigInt content.
    pub fn is_bigint(self) -> bool {
        matches!(self, Element::BigInt64 | Element::BigUint64)
    }

    /// Converts application values with the element type's content conversion.
    pub(crate) fn convert(self, realm: &mut Realm, value: &Value) -> Completion {
        if self.is_bigint() {
            crate::bigint_builtin::to_big(realm, value).map(Value::BigInt)
        } else {
            ops::to_number(realm, value).map(Value::Number)
        }
    }

    /// Little-endian bytes of an already converted Number or BigInt value.
    pub fn bytes(self, value: &Value) -> [u8; 8] {
        if let Value::BigInt(value) = value {
            return value
                .as_uint_n(64)
                .expect("64-bit truncation fits")
                .magnitude_u64()
                .expect("64-bit magnitude")
                .to_le_bytes();
        }
        let value = primitive_number(value);
        let raw: u64 = match self {
            Element::Uint8Clamped => u64::from(clamp(value)),
            Element::Float32 => u64::from((value as f32).to_bits()),
            Element::Float64 => value.to_bits(),
            _ => u64::from(ops::wrap32(value)),
        };
        raw.to_le_bytes()
    }
}

/// `ToUint8Clamp`: rounds half to even within 0–255.
fn clamp(value: f64) -> u8 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    let floor = crate::float::floor(value);
    let fraction = value - floor;
    let odd = crate::float::rem_euclid(floor, 2.0) != 0.0;
    if fraction > 0.5 || (fraction == 0.5 && odd) {
        (floor + 1.0) as u8
    } else {
        floor as u8
    }
}

/// A typed array: a fixed window of elements over a buffer.
#[derive(Clone, Debug)]
pub struct View {
    /// The element type.
    pub element: Element,
    /// The `ArrayBuffer` object.
    pub buffer: Object,
    /// The buffer's bytes.
    pub bytes: Bytes,
    /// The byte offset of the first element.
    pub offset: usize,
    /// The number of elements.
    pub length: usize,
}

impl View {
    /// `IsValidIntegerIndex`.
    pub fn index(&self, index: f64) -> Option<usize> {
        let integral = index.is_finite() && crate::float::trunc(index) == index;
        let valid = integral && index >= 0.0 && !(index == 0.0 && index.is_sign_negative());
        (valid && index < self.length as f64).then_some(index as usize)
    }

    /// The element at a valid integer index.
    pub fn get(&self, index: f64) -> Option<Value> {
        let index = self.index(index)?;
        let size = self.element.size();
        let start = self.offset + index * size;
        Some(self.element.read(&self.bytes.borrow()[start..start + size]))
    }

    /// Writes a converted primitive at an index; invalid indexes are ignored.
    pub fn set(&self, index: f64, value: &Value) {
        if let Some(index) = self.index(index) {
            let size = self.element.size();
            let start = self.offset + index * size;
            self.bytes.borrow_mut()[start..start + size]
                .copy_from_slice(&self.element.bytes(value)[..size]);
        }
    }

    fn values(&self) -> Vec<Value> {
        (0..self.length)
            .filter_map(|index| self.get(index as f64))
            .collect()
    }
}

/// The numeric index a key names, if it is a canonical numeric string:
/// typed arrays own every such key, valid or not.
pub fn numeric_key(key: &Key) -> Option<f64> {
    match key {
        Key::Index(index) => Some(f64::from(*index)),
        Key::String(text) => {
            if text.units() == [u16::from(b'-'), u16::from(b'0')] {
                return Some(-0.0);
            }
            let number = crate::number::parse(text);
            (crate::number::to_string(number) == text.to_rust_lossy()).then_some(number)
        }
        Key::Symbol(_) => None,
    }
}

/// A primitive's number for element writes made without a realm (property
/// definitions); objects and symbols become NaN there.
pub fn primitive_number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => *number,
        Value::Boolean(flag) => f64::from(u8::from(*flag)),
        Value::Null => 0.0,
        Value::String(text) => crate::number::parse(text),
        _ => f64::NAN,
    }
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn type_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Type, message)
}

fn range_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Range, message)
}

/// The view of a typed array value.
pub fn view_of(value: &Value) -> Option<View> {
    match &value.as_object()?.data().kind {
        Kind::TypedArray(view) => Some(view.clone()),
        _ => None,
    }
}

/// The shared bytes, first byte and byte length an `ArrayBufferView`
/// (typed array or `DataView`) covers.
pub(crate) fn view_range(value: &Value) -> Option<(Bytes, usize, usize)> {
    let object = value.as_object()?;
    match &object.data().kind {
        Kind::TypedArray(view) => Some((
            view.bytes.clone(),
            view.offset,
            view.length * view.element.size(),
        )),
        Kind::DataView(view) => Some((view.bytes.clone(), view.offset, view.length)),
        _ => None,
    }
}

/// A copy of the bytes an `ArrayBufferView` covers.
pub(crate) fn view_bytes(value: &Value) -> Option<Vec<u8>> {
    let (bytes, offset, length) = view_range(value)?;
    let bytes = bytes.borrow();
    Some(bytes[offset..offset + length].to_vec())
}

/// A new `Uint8Array` holding `bytes`, with `prototype` when given.
pub(crate) fn new_bytes(realm: &Realm, bytes: Vec<u8>, prototype: Option<Object>) -> Object {
    let array = new_typed_array(realm, Element::Uint8, 0);
    let (buffer, view_bytes) = buffer_object(realm.array_buffer_prototype.clone(), 0);
    *view_bytes.borrow_mut() = bytes;
    let length = view_bytes.borrow().len();
    array.data_mut().kind = Kind::TypedArray(View {
        element: Element::Uint8,
        buffer,
        bytes: view_bytes,
        offset: 0,
        length,
    });
    if let Some(prototype) = prototype {
        array.data_mut().prototype = Some(prototype);
    }
    array
}

/// A new typed array of `length` zeroed elements.
pub(crate) fn new_typed_array(realm: &Realm, element: Element, length: usize) -> Object {
    let index = ELEMENTS
        .iter()
        .position(|known| *known == element)
        .expect("every element type has a class");
    let prototype = realm.typed_array_prototypes[index].clone();
    allocate(realm, element, prototype, length).expect("small typed arrays allocate")
}

pub(crate) fn buffer_of(value: &Value) -> Option<Bytes> {
    match &value.as_object()?.data().kind {
        Kind::ArrayBuffer(bytes) => Some(bytes.clone()),
        _ => None,
    }
}

/// `ValidateTypedArray`.
fn validate(realm: &Realm, value: &Value, method: &str) -> Result<View, Thrown> {
    view_of(value)
        .ok_or_else(|| type_error(realm, &format!("this is not a typed array. ({method})")))
}

/// `ToIndex`, failing with `message`.
fn to_index(realm: &mut Realm, value: &Value, message: &str) -> Result<usize, Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok(0);
    }
    let integer = to_integer_or_infinity(realm, value)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&integer) {
        return Err(range_error(realm, message));
    }
    Ok(integer as usize)
}

/// Buffers above this many bytes are refused, like V8's limit.
const MAX_BYTES: usize = 1 << 32;

/// A new zero-filled buffer object.
fn new_buffer(realm: &Realm, prototype: Object, length: usize) -> Result<Object, Thrown> {
    if length > MAX_BYTES {
        return Err(range_error(realm, "Array buffer allocation failed"));
    }
    Ok(buffer_object(prototype, length).0)
}

/// A zero-filled buffer object of a length already checked, with its bytes.
fn buffer_object(prototype: Object, length: usize) -> (Object, Bytes) {
    let bytes: Bytes = Rc::new(RefCell::new(vec![0; length]));
    let buffer = Object::new(Some(prototype), Kind::ArrayBuffer(bytes.clone()));
    (buffer, bytes)
}

fn prototype_from(realm: &mut Realm, target: &Object, fallback: &Object) -> Result<Object, Thrown> {
    match ops::get(
        realm,
        &Value::Object(target.clone()),
        &Key::name("prototype"),
    )? {
        Value::Object(prototype) => Ok(prototype),
        _ => Ok(fallback.clone()),
    }
}

/// A typed array over a new buffer of `length` elements.
fn allocate(
    realm: &Realm,
    element: Element,
    prototype: Object,
    length: usize,
) -> Result<Object, Thrown> {
    if length > MAX_BYTES / element.size() {
        return Err(range_error(
            realm,
            &format!("Invalid typed array length: {length}"),
        ));
    }
    let (buffer, bytes) = buffer_object(
        realm.array_buffer_prototype.clone(),
        length * element.size(),
    );
    Ok(Object::new(
        Some(prototype),
        Kind::TypedArray(View {
            element,
            buffer,
            bytes,
            offset: 0,
            length,
        }),
    ))
}

/// Writes a value after its content conversion, even for invalid indexes.
fn write(realm: &mut Realm, view: &View, index: f64, value: &Value) -> Result<(), Thrown> {
    let value = view.element.convert(realm, value)?;
    view.set(index, &value);
    Ok(())
}

/// `TypedArray [[Set]]` for a numeric key of the receiver itself.
pub fn set_element(
    realm: &mut Realm,
    view: &View,
    index: f64,
    value: &Value,
) -> Result<(), Thrown> {
    write(realm, view, index, value)
}

fn length_message(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    Ok(format!(
        "Invalid typed array length: {}",
        ops::to_string(realm, value)?.to_rust_lossy()
    ))
}

/// The typed array constructor's behavior for one element type.
fn construct(
    realm: &mut Realm,
    element: Element,
    target: Object,
    args: &[Value],
    fallback: &Object,
) -> Result<Object, Thrown> {
    let prototype = prototype_from(realm, &target, fallback)?;
    let first = arg(args, 0);
    if !matches!(first, Value::Object(_)) {
        let message = length_message(realm, &first)?;
        let length = to_index(realm, &first, &message)?;
        return allocate(realm, element, prototype, length);
    }
    if let Some(bytes) = buffer_of(&first) {
        let size = element.size();
        let offset = to_index(
            realm,
            &arg(args, 1),
            "Start offset is outside the bounds of the buffer",
        )?;
        if offset % size != 0 {
            return Err(range_error(
                realm,
                &format!(
                    "start offset of {} should be a multiple of {size}",
                    element.name()
                ),
            ));
        }
        let available = bytes.borrow().len();
        let length = match arg(args, 2) {
            Value::Undefined => {
                if available % size != 0 {
                    return Err(range_error(
                        realm,
                        &format!(
                            "byte length of {} should be a multiple of {size}",
                            element.name()
                        ),
                    ));
                }
                if offset > available {
                    return Err(range_error(
                        realm,
                        &format!("Start offset {offset} is outside the bounds of the buffer"),
                    ));
                }
                (available - offset) / size
            }
            length => {
                let message = length_message(realm, &length)?;
                let length = to_index(realm, &length, &message)?;
                if offset + length * size > available {
                    return Err(range_error(
                        realm,
                        &format!("Invalid typed array length: {length}"),
                    ));
                }
                length
            }
        };
        let buffer = first.as_object().expect("buffers are objects").clone();
        return Ok(Object::new(
            Some(prototype),
            Kind::TypedArray(View {
                element,
                buffer,
                bytes,
                offset,
                length,
            }),
        ));
    }
    if let Some(view) = view_of(&first) {
        same_content(realm, element, view.element)?;
        let object = allocate(realm, element, prototype, view.length)?;
        let target = view_of(&Value::Object(object.clone())).expect("a new typed array");
        for (index, value) in view.values().into_iter().enumerate() {
            target.set(index as f64, &value);
        }
        return Ok(object);
    }
    // Iterables, then array-likes.
    let key = Key::Symbol(realm.symbols.iterator.clone());
    let method = ops::get(realm, &first, &key)?;
    if method.is_nullish() {
        let length = length_of(realm, &first)?;
        let object = allocate(realm, element, prototype, length as usize)?;
        let target = view_of(&Value::Object(object.clone())).expect("a new typed array");
        for index in 0..length {
            let value = ops::get(realm, &first, &index_key(index))?;
            write(realm, &target, index as f64, &value)?;
        }
        return Ok(object);
    }
    let values = crate::iter::collect(realm, &first)?;
    let object = allocate(realm, element, prototype, values.len())?;
    let target = view_of(&Value::Object(object.clone())).expect("a new typed array");
    for (index, value) in values.iter().enumerate() {
        write(realm, &target, index as f64, value)?;
    }
    Ok(object)
}

/// `TypedArraySpeciesCreate`: the created typed array, which must hold at
/// least `minimum` elements.
fn species_create(
    realm: &mut Realm,
    exemplar: &Value,
    view: &View,
    args: &[Value],
    minimum: usize,
) -> Completion {
    let fallback = Value::Object(realm.typed_array_constructors[view.element as usize].clone());
    let constructor = match ops::get_named(realm, exemplar, "constructor")? {
        Value::Undefined => fallback,
        constructor @ Value::Object(_) => {
            let key = Key::Symbol(realm.symbols.species.clone());
            match ops::get(realm, &constructor, &key)? {
                species if species.is_nullish() => fallback,
                species if is_constructor(&species) => species,
                _ => {
                    return Err(type_error(
                        realm,
                        "object.constructor[Symbol.species] is not a constructor",
                    ));
                }
            }
        }
        _ => return Err(type_error(realm, "object.constructor is not an object")),
    };
    let created = ops::construct(realm, &constructor, args)?;
    match view_of(&created) {
        Some(created_view) if created_view.length >= minimum => Ok(created),
        Some(_) => Err(type_error(
            realm,
            "species constructor returned a TypedArray that is too small",
        )),
        None => Err(type_error(
            realm,
            "species constructor did not return a TypedArray object",
        )),
    }
}

/// A new typed array of the same element type holding `values`.
fn copy_of(realm: &mut Realm, view: &View, values: &[Value]) -> Completion {
    let prototype = realm.typed_array_prototypes[view.element as usize].clone();
    let object = allocate(realm, view.element, prototype, values.len())?;
    let target = view_of(&Value::Object(object.clone())).expect("a new typed array");
    for (index, value) in values.iter().enumerate() {
        target.set(index as f64, value);
    }
    Ok(Value::Object(object))
}

fn same_content(realm: &Realm, left: Element, right: Element) -> Result<(), Thrown> {
    if left.is_bigint() != right.is_bigint() {
        Err(type_error(
            realm,
            "Cannot mix BigInt and other types, use explicit conversions",
        ))
    } else {
        Ok(())
    }
}

/// A relative index argument resolved against a length.
fn relative(
    realm: &mut Realm,
    value: &Value,
    length: usize,
    fallback: usize,
) -> Result<usize, Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok(fallback);
    }
    let relative = to_integer_or_infinity(realm, value)?;
    let length = length as f64;
    Ok(if relative < 0.0 {
        (length + relative).max(0.0) as usize
    } else {
        relative.min(length) as usize
    })
}

/// The default numeric order of `sort`: NaN last, -0 before +0.
fn compare_numbers(left: f64, right: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ if left == right => right.is_sign_negative().cmp(&left.is_sign_negative()),
        _ => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
    }
}

/// Sorts numbers with a comparator (stable binary insertion) or
/// numerically.
fn sorted(
    realm: &mut Realm,
    mut values: Vec<Value>,
    comparator: &Value,
) -> Result<Vec<Value>, Thrown> {
    if matches!(comparator, Value::Undefined) {
        values.sort_by(|left, right| match (left, right) {
            (Value::BigInt(left), Value::BigInt(right)) => left.compare(right),
            _ => compare_numbers(primitive_number(left), primitive_number(right)),
        });
        return Ok(values);
    }
    let mut result: Vec<Value> = Vec::with_capacity(values.len());
    for value in values {
        let (mut low, mut high) = (0, result.len());
        while low < high {
            let middle = (low + high) / 2;
            let pair = [value.clone(), result[middle].clone()];
            let order = ops::call(realm, comparator, Value::Undefined, &pair)?;
            if ops::to_number(realm, &order)? < 0.0 {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        result.insert(low, value);
    }
    Ok(result)
}

fn comparator(realm: &Realm, args: &[Value]) -> Result<Value, Thrown> {
    let comparator = arg(args, 0);
    if !matches!(comparator, Value::Undefined) && !is_callable(&comparator) {
        return Err(type_error(
            realm,
            "The comparison function must be either a function or undefined",
        ));
    }
    Ok(comparator)
}

fn callback(realm: &mut Realm, args: &[Value]) -> Result<Value, Thrown> {
    let callback = arg(args, 0);
    if !is_callable(&callback) {
        let text = crate::inspect::inspect(realm, &callback, 0)?;
        return Err(type_error(realm, &format!("{text} is not a function")));
    }
    Ok(callback)
}

/// A native method body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

const METHODS: [(&str, u32, Native); 12] = [
    ("fill", 1, |realm, this, args| {
        let view = validate(realm, &this, "fill")?;
        let value = view.element.convert(realm, &arg(args, 0))?;
        let start = relative(realm, &arg(args, 1), view.length, 0)?;
        let end = relative(realm, &arg(args, 2), view.length, view.length)?;
        for index in start..end.max(start) {
            view.set(index as f64, &value);
        }
        Ok(this)
    }),
    ("set", 1, |realm, this, args| {
        let view = validate(realm, &this, "set")?;
        let offset = to_integer_or_infinity(realm, &arg(args, 1))?;
        if offset < 0.0 {
            return Err(range_error(realm, "offset is out of bounds"));
        }
        let source = arg(args, 0);
        let values = if let Some(source) = view_of(&source) {
            if offset + source.length as f64 > view.length as f64 {
                return Err(range_error(realm, "offset is out of bounds"));
            }
            same_content(realm, view.element, source.element)?;
            if source.element == view.element {
                // The same element type copies bytes, also within one buffer.
                let size = view.element.size();
                let length = source.length * size;
                let to = view.offset + offset as usize * size;
                if Rc::ptr_eq(&source.bytes, &view.bytes) {
                    view.bytes
                        .borrow_mut()
                        .copy_within(source.offset..source.offset + length, to);
                } else {
                    let from = source.bytes.borrow();
                    view.bytes.borrow_mut()[to..to + length]
                        .copy_from_slice(&from[source.offset..source.offset + length]);
                }
                return Ok(Value::Undefined);
            }
            source.values()
        } else {
            let source = Value::Object(ops::to_object(realm, &source)?);
            let length = length_of(realm, &source)?;
            if offset + length as f64 > view.length as f64 {
                return Err(range_error(realm, "offset is out of bounds"));
            }
            for index in 0..length {
                let value = ops::get(realm, &source, &index_key(index))?;
                write(realm, &view, offset + index as f64, &value)?;
            }
            return Ok(Value::Undefined);
        };
        for (index, value) in values.iter().enumerate() {
            write(realm, &view, offset + index as f64, value)?;
        }
        Ok(Value::Undefined)
    }),
    ("subarray", 2, |realm, this, args| {
        let view = validate(realm, &this, "subarray")?;
        let start = relative(realm, &arg(args, 0), view.length, 0)?;
        let end = relative(realm, &arg(args, 1), view.length, view.length)?;
        let count = end.saturating_sub(start);
        let offset = view.offset + start * view.element.size();
        let args = [
            Value::Object(view.buffer.clone()),
            Value::Number(offset as f64),
            Value::Number(count as f64),
        ];
        species_create(realm, &this, &view, &args, 0)
    }),
    ("slice", 2, |realm, this, args| {
        let view = validate(realm, &this, "slice")?;
        let start = relative(realm, &arg(args, 0), view.length, 0)?;
        let end = relative(realm, &arg(args, 1), view.length, view.length)?;
        let count = end.saturating_sub(start);
        let created = species_create(realm, &this, &view, &[Value::Number(count as f64)], count)?;
        let target = view_of(&created).expect("species results are typed arrays");
        // Node 22 permits cross-content buffer views and mapped species; slice
        // checks content only when it copies elements.
        if count > 0 {
            same_content(realm, view.element, target.element)?;
        }
        for (offset, index) in (start..start + count).enumerate() {
            target.set(
                offset as f64,
                &view.get(index as f64).expect("valid element"),
            );
        }
        Ok(created)
    }),
    ("map", 1, |realm, this, args| {
        let view = validate(realm, &this, "map")?;
        let callback = callback(realm, args)?;
        let length = Value::Number(view.length as f64);
        let created = species_create(realm, &this, &view, &[length], view.length)?;
        let target = view_of(&created).expect("species results are typed arrays");
        for index in 0..view.length {
            let value = view.get(index as f64).expect("valid element");
            let call = [value, Value::Number(index as f64), this.clone()];
            let mapped = ops::call(realm, &callback, arg(args, 1), &call)?;
            write(realm, &target, index as f64, &mapped)?;
        }
        Ok(created)
    }),
    ("filter", 1, |realm, this, args| {
        let view = validate(realm, &this, "filter")?;
        let callback = callback(realm, args)?;
        let mut kept = Vec::new();
        for index in 0..view.length {
            let value = view.get(index as f64).expect("valid element");
            let call = [value.clone(), Value::Number(index as f64), this.clone()];
            if ops::to_boolean(&ops::call(realm, &callback, arg(args, 1), &call)?) {
                kept.push(value);
            }
        }
        let length = Value::Number(kept.len() as f64);
        let created = species_create(realm, &this, &view, &[length], kept.len())?;
        let target = view_of(&created).expect("species results are typed arrays");
        for (index, value) in kept.into_iter().enumerate() {
            write(realm, &target, index as f64, &value)?;
        }
        Ok(created)
    }),
    ("sort", 1, |realm, this, args| {
        let comparator = comparator(realm, args)?;
        let view = validate(realm, &this, "sort")?;
        let values = sorted(realm, view.values(), &comparator)?;
        for (index, value) in values.into_iter().enumerate() {
            view.set(index as f64, &value);
        }
        Ok(this)
    }),
    ("toSorted", 1, |realm, this, args| {
        let comparator = comparator(realm, args)?;
        let view = validate(realm, &this, "toSorted")?;
        let values = sorted(realm, view.values(), &comparator)?;
        copy_of(realm, &view, &values)
    }),
    ("toReversed", 0, |realm, this, _| {
        let view = validate(realm, &this, "toReversed")?;
        let mut values = view.values();
        values.reverse();
        copy_of(realm, &view, &values)
    }),
    ("with", 2, |realm, this, args| {
        let view = validate(realm, &this, "with")?;
        let relative = to_integer_or_infinity(realm, &arg(args, 0))?;
        let index = if relative < 0.0 {
            view.length as f64 + relative
        } else {
            relative
        };
        let value = view.element.convert(realm, &arg(args, 1))?;
        let Some(index) = view.index(index) else {
            return Err(range_error(realm, "Invalid typed array index"));
        };
        let mut values = view.values();
        values[index] = value;
        copy_of(realm, &view, &values)
    }),
    ("copyWithin", 2, |realm, this, args| {
        let view = validate(realm, &this, "copyWithin")?;
        let target = relative(realm, &arg(args, 0), view.length, 0)?;
        let start = relative(realm, &arg(args, 1), view.length, 0)?;
        let end = relative(realm, &arg(args, 2), view.length, view.length)?;
        // Elements of one type move as their bytes, as memmove would.
        let count = end.saturating_sub(start).min(view.length - target);
        if count > 0 {
            let size = view.element.size();
            let from = view.offset + start * size;
            let to = view.offset + target * size;
            view.bytes
                .borrow_mut()
                .copy_within(from..from + count * size, to);
        }
        Ok(this)
    }),
    ("reverse", 0, |realm, this, _| {
        let view = validate(realm, &this, "reverse")?;
        let mut values = view.values();
        values.reverse();
        for (index, value) in values.into_iter().enumerate() {
            view.set(index as f64, &value);
        }
        Ok(this)
    }),
];

/// Array methods that are generic over array-likes: typed arrays reuse the
/// intrinsic functions after validating the receiver.
const SHARED: [&str; 18] = [
    "at",
    "entries",
    "every",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "forEach",
    "includes",
    "indexOf",
    "join",
    "keys",
    "lastIndexOf",
    "reduce",
    "reduceRight",
    "some",
    "toLocaleString",
    "values",
];

fn accessor(
    realm: &Realm,
    target: &Object,
    key: Key,
    name: &str,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) {
    let getter = realm.function(&format!("get {name}"), 0, false, code);
    target.define(
        key,
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

fn fixed(value: Value) -> Property {
    Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable: false,
        configurable: false,
    }
}

fn install_typed_arrays(realm: &mut Realm) {
    let prototype = realm.typed_array_prototype.clone();
    // %TypedArray%: abstract, with `from`, `of` and species.
    let abstract_constructor = realm.builtin_constructor(
        "TypedArray",
        0,
        &prototype,
        |realm, _, _| {
            Err(type_error(
                realm,
                "Abstract class TypedArray not directly constructable",
            ))
        },
        |realm, _, _| {
            Err(type_error(
                realm,
                "Abstract class TypedArray not directly constructable",
            ))
        },
    );
    accessor(
        realm,
        &abstract_constructor,
        Key::Symbol(realm.symbols.species.clone()),
        "[Symbol.species]",
        |_, this, _| Ok(this),
    );
    realm.method(&abstract_constructor, "of", 0, |realm, this, args| {
        if !is_constructor(&this) {
            return Err(type_error(realm, "this is not a constructor"));
        }
        let created = ops::construct(realm, &this, &[Value::Number(args.len() as f64)])?;
        let view = view_of(&created)
            .ok_or_else(|| type_error(realm, "constructor did not return a TypedArray object"))?;
        for (index, value) in args.iter().enumerate() {
            write(realm, &view, index as f64, value)?;
        }
        Ok(created)
    });
    realm.method(&abstract_constructor, "from", 1, |realm, this, args| {
        if !is_constructor(&this) {
            return Err(type_error(realm, "this is not a constructor"));
        }
        let mapper = arg(args, 1);
        if !matches!(mapper, Value::Undefined) && !is_callable(&mapper) {
            return Err(type_error(realm, "mapper is not a function"));
        }
        let source = arg(args, 0);
        let key = Key::Symbol(realm.symbols.iterator.clone());
        let method = ops::get(realm, &source, &key)?;
        let (source, values, length) = if method.is_nullish() {
            let source = Value::Object(ops::to_object(realm, &source)?);
            let length = length_of(realm, &source)?;
            (source, None, length)
        } else {
            let values = crate::iter::collect(realm, &source)?;
            let length = values.len() as u64;
            (source, Some(values), length)
        };
        let created = ops::construct(realm, &this, &[Value::Number(length as f64)])?;
        let view = view_of(&created)
            .ok_or_else(|| type_error(realm, "constructor did not return a TypedArray object"))?;
        for index in 0..length {
            let value = match &values {
                Some(values) => values[index as usize].clone(),
                None => ops::get(realm, &source, &index_key(index))?,
            };
            let value = if matches!(mapper, Value::Undefined) {
                value
            } else {
                ops::call(
                    realm,
                    &mapper,
                    arg(args, 2),
                    &[value, Value::Number(index as f64)],
                )?
            };
            write(realm, &view, index as f64, &value)?;
        }
        Ok(created)
    });
    // Accessors and methods of %TypedArray%.prototype.
    for (name, read) in [
        ("buffer", 0usize),
        ("byteLength", 1),
        ("byteOffset", 2),
        ("length", 3),
    ] {
        accessor(
            realm,
            &prototype,
            Key::name(name),
            name,
            move |realm, this, _| {
                let view = validate(realm, &this, name)?;
                Ok(match read {
                    0 => Value::Object(view.buffer),
                    1 => Value::Number((view.length * view.element.size()) as f64),
                    2 => Value::Number(view.offset as f64),
                    _ => Value::Number(view.length as f64),
                })
            },
        );
    }
    accessor(
        realm,
        &prototype,
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        "[Symbol.toStringTag]",
        |_, this, _| {
            Ok(view_of(&this).map_or(Value::Undefined, |view| Value::str(view.element.name())))
        },
    );
    for (name, length, code) in METHODS {
        realm.method(&prototype, name, length, code);
    }
    let array_prototype = Value::Object(realm.array_prototype.clone());
    for name in SHARED {
        let shared = ops::get(realm, &array_prototype, &Key::name(name))
            .expect("Array methods are installed");
        let length = ops::get_named(realm, &shared, "length").expect("functions have lengths");
        let length = ops::to_number(realm, &length).expect("lengths are numbers") as u32;
        let method = realm.function(name, length, false, move |realm, this, args| {
            validate(realm, &this, name)?;
            ops::call(realm, &shared, this, args)
        });
        prototype.define(Key::name(name), Property::hidden(Value::Object(method)));
    }
    let join_to_string =
        ops::get_named(realm, &array_prototype, "toString").expect("Array toString");
    prototype.define(Key::name("toString"), Property::hidden(join_to_string));
    let values = ops::get(
        realm,
        &Value::Object(prototype.clone()),
        &Key::name("values"),
    )
    .expect("values");
    prototype.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::hidden(values),
    );
    // The eleven element types.
    let mut constructors = Vec::new();
    let mut prototypes = Vec::new();
    for element in ELEMENTS {
        let element_prototype = Object::new(Some(prototype.clone()), Kind::Ordinary);
        let fallback = element_prototype.clone();
        let constructor = realm.builtin_constructor(
            element.name(),
            3,
            &element_prototype,
            move |realm, _, _| {
                Err(type_error(
                    realm,
                    &format!("Constructor {} requires 'new'", element.name()),
                ))
            },
            move |realm, target, args| construct(realm, element, target, args, &fallback),
        );
        constructor.data_mut().prototype = Some(abstract_constructor.clone());
        let size = Value::Number(element.size() as f64);
        constructor.define(Key::name("BYTES_PER_ELEMENT"), fixed(size.clone()));
        element_prototype.define(Key::name("BYTES_PER_ELEMENT"), fixed(size));
        realm.set_global(element.name(), Value::Object(constructor.clone()));
        constructors.push(constructor);
        prototypes.push(element_prototype);
    }
    realm.typed_array_constructors = constructors;
    realm.typed_array_prototypes = prototypes;
}

fn install_array_buffer(realm: &mut Realm) {
    let prototype = realm.array_buffer_prototype.clone();
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        "ArrayBuffer",
        1,
        &prototype,
        |realm, _, _| Err(type_error(realm, "Constructor ArrayBuffer requires 'new'")),
        move |realm, target, args| {
            let prototype = prototype_from(realm, &target, &fallback)?;
            let length = to_index(realm, &arg(args, 0), "Invalid array buffer length")?;
            new_buffer(realm, prototype, length)
        },
    );
    accessor(
        realm,
        &constructor,
        Key::Symbol(realm.symbols.species.clone()),
        "[Symbol.species]",
        |_, this, _| Ok(this),
    );
    realm.method(&constructor, "isView", 1, |_, _, args| {
        let value = arg(args, 0);
        let view = value.as_object().is_some_and(|object| {
            matches!(object.data().kind, Kind::TypedArray(_) | Kind::DataView(_))
        });
        Ok(Value::Boolean(view))
    });
    let bytes_of = |realm: &Realm, this: &Value, method: &str| {
        buffer_of(this).ok_or_else(|| {
            type_error(
                realm,
                &format!("Method ArrayBuffer.prototype.{method} called on incompatible receiver"),
            )
        })
    };
    accessor(
        realm,
        &prototype,
        Key::name("byteLength"),
        "byteLength",
        move |realm, this, _| {
            Ok(Value::Number(
                bytes_of(realm, &this, "byteLength")?.borrow().len() as f64,
            ))
        },
    );
    accessor(
        realm,
        &prototype,
        Key::name("maxByteLength"),
        "maxByteLength",
        move |realm, this, _| {
            Ok(Value::Number(
                bytes_of(realm, &this, "maxByteLength")?.borrow().len() as f64,
            ))
        },
    );
    accessor(
        realm,
        &prototype,
        Key::name("resizable"),
        "resizable",
        move |realm, this, _| {
            bytes_of(realm, &this, "resizable")?;
            Ok(Value::Boolean(false))
        },
    );
    accessor(
        realm,
        &prototype,
        Key::name("detached"),
        "detached",
        move |realm, this, _| {
            bytes_of(realm, &this, "detached")?;
            Ok(Value::Boolean(false))
        },
    );
    let fallback_constructor = constructor.downgrade();
    realm.method(&prototype, "slice", 2, move |realm, this, args| {
        let bytes = bytes_of(realm, &this, "slice")?;
        let length = bytes.borrow().len();
        let start = relative(realm, &arg(args, 0), length, 0)?;
        let end = relative(realm, &arg(args, 1), length, length)?;
        let count = end.saturating_sub(start);
        let fallback = Value::Object(
            fallback_constructor
                .upgrade()
                .expect("ArrayBuffer is alive"),
        );
        let constructor = match ops::get_named(realm, &this, "constructor")? {
            Value::Undefined => fallback,
            constructor => {
                let key = Key::Symbol(realm.symbols.species.clone());
                match ops::get(realm, &constructor, &key)? {
                    species if species.is_nullish() => fallback,
                    species => species,
                }
            }
        };
        let created = ops::construct(realm, &constructor, &[Value::Number(count as f64)])?;
        let target = buffer_of(&created).ok_or_else(|| {
            type_error(realm, "species constructor did not return an ArrayBuffer")
        })?;
        if Rc::ptr_eq(&target, &bytes) {
            return Err(type_error(
                realm,
                "ArrayBuffer subclass returned this from species constructor",
            ));
        }
        if target.borrow().len() < count {
            return Err(type_error(
                realm,
                "species constructor returned an ArrayBuffer that is too small",
            ));
        }
        target.borrow_mut()[..count].copy_from_slice(&bytes.borrow()[start..start + count]);
        Ok(created)
    });
    tag(realm, &prototype, "ArrayBuffer");
    realm.set_global("ArrayBuffer", Value::Object(constructor));
}

fn tag(realm: &Realm, target: &Object, name: &str) {
    target.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str(name),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

/// A `DataView`: a byte window over a buffer.
#[derive(Clone, Debug)]
pub struct DataView {
    /// The `ArrayBuffer` object.
    pub buffer: Object,
    /// The buffer's bytes.
    pub bytes: Bytes,
    /// The first byte.
    pub offset: usize,
    /// The number of bytes.
    pub length: usize,
}

fn data_view(realm: &Realm, this: &Value, method: &str) -> Result<DataView, Thrown> {
    match this.as_object().map(|object| match &object.data().kind {
        Kind::DataView(view) => Some(view.clone()),
        _ => None,
    }) {
        Some(Some(view)) => Ok(view),
        _ => Err(type_error(
            realm,
            &format!(
                "Method DataView.prototype.{method} called on incompatible receiver {}",
                crate::weak_builtin::receiver_text(this)
            ),
        )),
    }
}

/// `GetViewValue` and `SetViewValue`.
fn view_access(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    element: Element,
    set: bool,
    method: &str,
) -> Completion {
    let view = data_view(realm, this, method)?;
    let index = to_index(
        realm,
        &arg(args, 0),
        "Offset is outside the bounds of the DataView",
    )?;
    let (value, little) = if set {
        (
            element.convert(realm, &arg(args, 1))?,
            ops::to_boolean(&arg(args, 2)),
        )
    } else {
        (Value::Undefined, ops::to_boolean(&arg(args, 1)))
    };
    let size = element.size();
    if index + size > view.length {
        return Err(range_error(
            realm,
            "Offset is outside the bounds of the DataView",
        ));
    }
    let start = view.offset + index;
    let range = start..start + size;
    if set {
        let mut bytes = element.bytes(&value);
        if !little {
            bytes[..size].reverse();
        }
        view.bytes.borrow_mut()[range].copy_from_slice(&bytes[..size]);
        return Ok(Value::Undefined);
    }
    let mut bytes = view.bytes.borrow()[range].to_vec();
    if !little {
        bytes.reverse();
    }
    Ok(element.read(&bytes))
}

const VIEW_TYPES: [(&str, Element); 10] = [
    ("Int8", Element::Int8),
    ("Uint8", Element::Uint8),
    ("Int16", Element::Int16),
    ("Uint16", Element::Uint16),
    ("Int32", Element::Int32),
    ("Uint32", Element::Uint32),
    ("Float32", Element::Float32),
    ("Float64", Element::Float64),
    ("BigInt64", Element::BigInt64),
    ("BigUint64", Element::BigUint64),
];

fn install_data_view(realm: &mut Realm) {
    let prototype = Object::new(Some(realm.object_prototype.clone()), Kind::Ordinary);
    let fallback = prototype.clone();
    let constructor = realm.builtin_constructor(
        "DataView",
        1,
        &prototype,
        |realm, _, _| Err(type_error(realm, "Constructor DataView requires 'new'")),
        move |realm, target, args| {
            let buffer = arg(args, 0);
            let Some(bytes) = buffer_of(&buffer) else {
                return Err(type_error(
                    realm,
                    "First argument to DataView constructor must be an ArrayBuffer",
                ));
            };
            let offset = to_index(
                realm,
                &arg(args, 1),
                "Start offset is outside the bounds of the buffer",
            )?;
            let available = bytes.borrow().len();
            if offset > available {
                return Err(range_error(
                    realm,
                    &format!("Start offset {offset} is outside the bounds of the buffer"),
                ));
            }
            let length = match arg(args, 2) {
                Value::Undefined => available - offset,
                length => {
                    let length = to_index(realm, &length, "Invalid DataView length")?;
                    if offset + length > available {
                        return Err(range_error(
                            realm,
                            &format!("Invalid DataView length {length}"),
                        ));
                    }
                    length
                }
            };
            let prototype = prototype_from(realm, &target, &fallback)?;
            let buffer = buffer.as_object().expect("buffers are objects").clone();
            Ok(Object::new(
                Some(prototype),
                Kind::DataView(DataView {
                    buffer,
                    bytes,
                    offset,
                    length,
                }),
            ))
        },
    );
    for (name, read) in [("buffer", 0usize), ("byteLength", 1), ("byteOffset", 2)] {
        accessor(
            realm,
            &prototype,
            Key::name(name),
            name,
            move |realm, this, _| {
                let view = data_view(realm, &this, name)?;
                Ok(match read {
                    0 => Value::Object(view.buffer),
                    1 => Value::Number(view.length as f64),
                    _ => Value::Number(view.offset as f64),
                })
            },
        );
    }
    for (name, element) in VIEW_TYPES {
        for set in [false, true] {
            let method = format!("{}{name}", if set { "set" } else { "get" });
            let label = method.clone();
            realm.method(
                &prototype,
                &method,
                if set { 2 } else { 1 },
                move |realm, this, args| view_access(realm, &this, args, element, set, &label),
            );
        }
    }
    tag(realm, &prototype, "DataView");
    realm.set_global("DataView", Value::Object(constructor));
}

pub(crate) fn install(realm: &mut Realm) {
    install_array_buffer(realm);
    install_typed_arrays(realm);
    install_data_view(realm);
}

#[cfg(test)]
mod tests;
