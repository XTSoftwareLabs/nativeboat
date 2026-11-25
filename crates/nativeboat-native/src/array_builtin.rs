//! The `Array` constructor, its static functions and `Array.prototype`, as
//! specified by ECMAScript 2024 (§23.1). Methods are generic over array-like
//! receivers: they read `length` with `LengthOfArrayLike`, use property
//! operations and observe holes, getters and species constructors exactly as
//! specified. `join` additionally treats arrays already being joined as
//! empty, as engines do, so cyclic arrays do not recurse forever.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::iter;
use crate::object::{Descriptor, Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};

/// 2⁵³ − 1, the largest length of an array-like object.
const MAX_LENGTH: u64 = (1 << 53) - 1;

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// The property key of an index, which is a canonical array index below
/// 2³² − 1 and a string key above it.
pub(crate) fn index_key(index: u64) -> Key {
    match u32::try_from(index) {
        Ok(index) if index != u32::MAX => Key::Index(index),
        _ => Key::String(JsStr::from(index.to_string().as_str())),
    }
}

fn length_key() -> Key {
    crate::string::static_key!("length")
}

fn number(value: u64) -> Value {
    Value::Number(value as f64)
}

/// `ToIntegerOrInfinity`.
pub(crate) fn to_integer_or_infinity(realm: &mut Realm, value: &Value) -> Result<f64, Thrown> {
    let number = ops::to_number(realm, value)?;
    Ok(if number.is_nan() {
        0.0
    } else if number.is_infinite() {
        number
    } else {
        // Adding zero turns a negative zero into zero.
        crate::float::trunc(number) + 0.0
    })
}

/// `ToLength`.
pub(crate) fn to_length(realm: &mut Realm, value: &Value) -> Result<u64, Thrown> {
    let length = to_integer_or_infinity(realm, value)?;
    Ok(if length <= 0.0 {
        0
    } else {
        length.min(MAX_LENGTH as f64) as u64
    })
}

/// `LengthOfArrayLike`.
pub(crate) fn length_of(realm: &mut Realm, object: &Value) -> Result<u64, Thrown> {
    // An array's own length is the stored one, and nothing can intercept
    // reading it.
    if let Value::Object(array) = object
        && let Kind::Array(data) = &array.data().kind
    {
        return Ok(u64::from(data.length()));
    }
    let length = ops::get_named(realm, object, "length")?;
    to_length(realm, &length)
}

/// A relative index argument resolved against a length; `fallback` applies
/// when the argument is `undefined`.
fn relative(realm: &mut Realm, value: &Value, length: u64, fallback: u64) -> Result<u64, Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok(fallback);
    }
    let relative = to_integer_or_infinity(realm, value)?;
    Ok(if relative < 0.0 {
        (length as f64 + relative).max(0.0) as u64
    } else {
        relative.min(length as f64) as u64
    })
}

/// `IsCallable`.
pub(crate) fn is_callable(value: &Value) -> bool {
    matches!(value, Value::Object(object) if object.code().is_some())
}

/// `IsConstructor`.
pub(crate) fn is_constructor(value: &Value) -> bool {
    matches!(value, Value::Object(object) if matches!(object.code(), Some((_, true))))
}

fn callback(realm: &Realm, args: &[Value], method: &str) -> Result<Value, Thrown> {
    let function = arg(args, 0);
    if is_callable(&function) {
        Ok(function)
    } else {
        Err(realm.throw(
            ErrorKind::Type,
            &format!("Array.prototype.{method} requires a function"),
        ))
    }
}

/// `CreateDataPropertyOrThrow`.
pub(crate) fn create_data_property(
    realm: &Realm,
    object: &Object,
    key: Key,
    value: Value,
) -> Result<(), Thrown> {
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

/// `DeletePropertyOrThrow` (deleting from an object throws on failure).
fn delete(realm: &mut Realm, object: &Value, key: &Key) -> Result<(), Thrown> {
    ops::delete(realm, object, key).map(drop)
}

fn has(realm: &mut Realm, object: &Value, key: &Key) -> Result<bool, Thrown> {
    ops::has_property(realm, key, object)
}

fn set_length(realm: &mut Realm, object: &Value, length: u64) -> Result<(), Thrown> {
    ops::set(realm, object, &length_key(), number(length))
}

/// `ArrayCreate`.
pub(crate) fn array_create(realm: &Realm, length: u64) -> Result<Object, Thrown> {
    if length > u64::from(u32::MAX) {
        return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
    }
    let array = realm.array(Vec::new());
    if length > 0 {
        let descriptor = Descriptor {
            value: Some(number(length)),
            ..Descriptor::default()
        };
        array.define_own_property(&length_key(), &descriptor);
    }
    Ok(array)
}

/// `GetPrototypeFromConstructor`.
fn prototype_from(realm: &mut Realm, target: &Object, fallback: &Object) -> Result<Object, Thrown> {
    match ops::get(
        realm,
        &Value::Object(target.clone()),
        &crate::string::static_key!("prototype"),
    )? {
        Value::Object(prototype) => Ok(prototype),
        _ => Ok(fallback.clone()),
    }
}

/// `ArraySpeciesCreate`.
fn species_create(realm: &mut Realm, original: &Value, length: u64) -> Result<Object, Thrown> {
    if !is_array(realm, original)? {
        return array_create(realm, length);
    }
    let mut constructor = ops::get_named(realm, original, "constructor")?;
    if matches!(constructor, Value::Object(_)) {
        let species = Key::Symbol(realm.symbols.species.clone());
        constructor = ops::get(realm, &constructor, &species)?;
        if matches!(constructor, Value::Null) {
            constructor = Value::Undefined;
        }
    }
    if matches!(constructor, Value::Undefined) {
        return array_create(realm, length);
    }
    if !is_constructor(&constructor) {
        return Err(realm.throw(
            ErrorKind::Type,
            "Array species constructor is not a constructor",
        ));
    }
    construct_object(realm, &constructor, &[number(length)])
}

/// `Construct`, whose result is always an object.
fn construct_object(
    realm: &mut Realm,
    constructor: &Value,
    args: &[Value],
) -> Result<Object, Thrown> {
    let object = ops::construct(realm, constructor, args)?;
    Ok(object
        .as_object()
        .expect("construction returns an object")
        .clone())
}

/// The target of `Array.from` and `Array.of`: a new instance of the
/// receiver when it is a constructor, otherwise a new array.
fn new_target(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    length: u64,
) -> Result<Object, Thrown> {
    if is_constructor(this) {
        construct_object(realm, this, args)
    } else {
        array_create(realm, length)
    }
}

/// `IsArray`: an array, or a proxy of one, as a reactive array is.
pub(crate) fn is_array(realm: &Realm, value: &Value) -> Result<bool, Thrown> {
    match value {
        Value::Object(object) => crate::proxy_builtin::is_array(realm, object),
        _ => Ok(false),
    }
}

/// `IsConcatSpreadable`.
fn spreadable(realm: &mut Realm, value: &Value) -> Result<bool, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Ok(false);
    }
    let key = Key::Symbol(realm.symbols.is_concat_spreadable.clone());
    match ops::get(realm, value, &key)? {
        Value::Undefined => is_array(realm, value),
        flag => Ok(ops::to_boolean(&flag)),
    }
}

fn too_long(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Type, "Array length exceeds the maximum length")
}

/// Whether `added` more elements fit after `count`.
fn fits(realm: &Realm, count: u64, added: u64) -> Result<(), Thrown> {
    if count + added > MAX_LENGTH {
        Err(too_long(realm))
    } else {
        Ok(())
    }
}

/// The receiver as an object value, for the generic methods.
fn receiver(realm: &mut Realm, this: &Value) -> Result<Value, Thrown> {
    Ok(Value::Object(ops::to_object(realm, this)?))
}

fn at(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let relative = to_integer_or_infinity(realm, &arg(args, 0))?;
    let index = if relative >= 0.0 {
        relative
    } else {
        length as f64 + relative
    };
    if index < 0.0 || index >= length as f64 {
        return Ok(Value::Undefined);
    }
    ops::get(realm, &object, &index_key(index as u64))
}

fn concat(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let result = species_create(realm, &object, 0)?;
    let mut count = 0u64;
    for item in core::iter::once(&object).chain(args) {
        if spreadable(realm, item)? {
            let length = length_of(realm, item)?;
            fits(realm, count, length)?;
            for index in 0..length {
                let key = index_key(index);
                if has(realm, item, &key)? {
                    let value = ops::get(realm, item, &key)?;
                    create_data_property(realm, &result, index_key(count), value)?;
                }
                count += 1;
            }
        } else {
            fits(realm, count, 1)?;
            create_data_property(realm, &result, index_key(count), item.clone())?;
            count += 1;
        }
    }
    let result = Value::Object(result);
    set_length(realm, &result, count)?;
    Ok(result)
}

fn copy_within(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let to = relative(realm, &arg(args, 0), length, 0)?;
    let from = relative(realm, &arg(args, 1), length, 0)?;
    let end = relative(realm, &arg(args, 2), length, length)?;
    let count = end.saturating_sub(from).min(length - to);
    // Overlapping ranges copy from the end when moving forwards.
    let backwards = from < to && to < from + count;
    for step in 0..count {
        let offset = if backwards { count - 1 - step } else { step };
        let source = index_key(from + offset);
        let target = index_key(to + offset);
        if has(realm, &object, &source)? {
            let value = ops::get(realm, &object, &source)?;
            ops::set(realm, &object, &target, value)?;
        } else {
            delete(realm, &object, &target)?;
        }
    }
    Ok(object)
}

/// Visits an element: its index, value and the callback's result; true stops.
type Visit<'a> = dyn FnMut(&mut Realm, u64, Value, Value) -> Result<bool, Thrown> + 'a;

/// The shared loop of `every`, `some`, `forEach`, `map` and `filter`: calls
/// the callback for each present element and returns early when `stop`
/// says so.
fn each(
    realm: &mut Realm,
    object: &Value,
    length: u64,
    function: &Value,
    receiver: &Value,
    visit: &mut Visit<'_>,
) -> Result<bool, Thrown> {
    for index in 0..length {
        let key = index_key(index);
        if has(realm, object, &key)? {
            let value = ops::get(realm, object, &key)?;
            let result = ops::call(
                realm,
                function,
                receiver.clone(),
                &[value.clone(), number(index), object.clone()],
            )?;
            if visit(realm, index, value, result)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The receiver, its length and the callback of an iterating method.
fn iterating(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
) -> Result<(Value, u64, Value), Thrown> {
    let object = receiver(realm, this)?;
    let length = length_of(realm, &object)?;
    let function = callback(realm, args, method)?;
    Ok((object, length, function))
}

fn every(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (object, length, function) = iterating(realm, &this, args, "every")?;
    let failed = each(
        realm,
        &object,
        length,
        &function,
        &arg(args, 1),
        &mut |_, _, _, result| Ok(!ops::to_boolean(&result)),
    )?;
    Ok(Value::Boolean(!failed))
}

fn some(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (object, length, function) = iterating(realm, &this, args, "some")?;
    let found = each(
        realm,
        &object,
        length,
        &function,
        &arg(args, 1),
        &mut |_, _, _, result| Ok(ops::to_boolean(&result)),
    )?;
    Ok(Value::Boolean(found))
}

fn for_each(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (object, length, function) = iterating(realm, &this, args, "forEach")?;
    each(
        realm,
        &object,
        length,
        &function,
        &arg(args, 1),
        &mut |_, _, _, _| Ok(false),
    )?;
    Ok(Value::Undefined)
}

fn map(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (object, length, function) = iterating(realm, &this, args, "map")?;
    let result = species_create(realm, &object, length)?;
    each(
        realm,
        &object,
        length,
        &function,
        &arg(args, 1),
        &mut |realm, index, _, mapped| {
            create_data_property(realm, &result, index_key(index), mapped)?;
            Ok(false)
        },
    )?;
    Ok(Value::Object(result))
}

fn filter(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let (object, length, function) = iterating(realm, &this, args, "filter")?;
    let result = species_create(realm, &object, 0)?;
    let mut count = 0;
    each(
        realm,
        &object,
        length,
        &function,
        &arg(args, 1),
        &mut |realm, _, value, keep| {
            if ops::to_boolean(&keep) {
                create_data_property(realm, &result, index_key(count), value)?;
                count += 1;
            }
            Ok(false)
        },
    )?;
    Ok(Value::Object(result))
}

fn fill(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let start = relative(realm, &arg(args, 1), length, 0)?;
    let end = relative(realm, &arg(args, 2), length, length)?;
    for index in start..end {
        ops::set(realm, &object, &index_key(index), arg(args, 0))?;
    }
    Ok(object)
}

/// `find`, `findIndex`, `findLast` and `findLastIndex`, which visit holes as
/// `undefined`.
fn find_in(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    method: &str,
    last: bool,
    index_result: bool,
) -> Completion {
    let object = receiver(realm, this)?;
    let length = length_of(realm, &object)?;
    let function = callback(realm, args, method)?;
    for step in 0..length {
        let index = if last { length - 1 - step } else { step };
        let value = ops::get(realm, &object, &index_key(index))?;
        let found = ops::call(
            realm,
            &function,
            arg(args, 1),
            &[value.clone(), number(index), object.clone()],
        )?;
        if ops::to_boolean(&found) {
            return Ok(if index_result { number(index) } else { value });
        }
    }
    Ok(if index_result {
        Value::Number(-1.0)
    } else {
        Value::Undefined
    })
}

/// `FlattenIntoArray`.
#[allow(clippy::too_many_arguments)]
fn flatten(
    realm: &mut Realm,
    target: &Object,
    source: &Value,
    length: u64,
    start: u64,
    depth: f64,
    mapper: Option<(&Value, &Value)>,
) -> Result<u64, Thrown> {
    let mut target_index = start;
    for index in 0..length {
        let key = index_key(index);
        if !has(realm, source, &key)? {
            continue;
        }
        let mut element = ops::get(realm, source, &key)?;
        if let Some((function, receiver)) = mapper {
            element = ops::call(
                realm,
                function,
                receiver.clone(),
                &[element, number(index), source.clone()],
            )?;
        }
        let flattens = depth > 0.0 && is_array(realm, &element)?;
        if flattens {
            let inner = length_of(realm, &element)?;
            target_index = flatten(
                realm,
                target,
                &element,
                inner,
                target_index,
                depth - 1.0,
                None,
            )?;
        } else {
            fits(realm, target_index, 1)?;
            create_data_property(realm, target, index_key(target_index), element)?;
            target_index += 1;
        }
    }
    Ok(target_index)
}

fn flat(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let depth = match arg(args, 0) {
        Value::Undefined => 1.0,
        value => to_integer_or_infinity(realm, &value)?.max(0.0),
    };
    let result = species_create(realm, &object, 0)?;
    flatten(realm, &result, &object, length, 0, depth, None)?;
    Ok(Value::Object(result))
}

fn flat_map(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let function = callback(realm, args, "flatMap")?;
    let result = species_create(realm, &object, 0)?;
    let receiver = arg(args, 1);
    flatten(
        realm,
        &result,
        &object,
        length,
        0,
        1.0,
        Some((&function, &receiver)),
    )?;
    Ok(Value::Object(result))
}

/// `SameValueZero`.
fn same_value_zero(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            left == right || (left.is_nan() && right.is_nan())
        }
        _ => crate::same_value(left, right),
    }
}

fn includes(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length == 0 {
        return Ok(Value::Boolean(false));
    }
    let start = relative(realm, &arg(args, 1), length, 0)?;
    let target = arg(args, 0);
    for index in start..length {
        let value = ops::get(realm, &object, &index_key(index))?;
        if same_value_zero(&value, &target) {
            return Ok(Value::Boolean(true));
        }
    }
    Ok(Value::Boolean(false))
}

fn index_of(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length == 0 {
        return Ok(Value::Number(-1.0));
    }
    let start = relative(realm, &arg(args, 1), length, 0)?;
    let target = arg(args, 0);
    for index in start..length {
        let key = index_key(index);
        if has(realm, &object, &key)? {
            let value = ops::get(realm, &object, &key)?;
            if ops::strict_equals(&value, &target) {
                return Ok(number(index));
            }
        }
    }
    Ok(Value::Number(-1.0))
}

fn last_index_of(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length == 0 {
        return Ok(Value::Number(-1.0));
    }
    let from = if args.len() > 1 {
        to_integer_or_infinity(realm, &args[1])?
    } else {
        length as f64 - 1.0
    };
    let start = if from >= 0.0 {
        from.min(length as f64 - 1.0)
    } else {
        length as f64 + from
    };
    if start < 0.0 {
        return Ok(Value::Number(-1.0));
    }
    let target = arg(args, 0);
    for index in (0..=start as u64).rev() {
        let key = index_key(index);
        if has(realm, &object, &key)? {
            let value = ops::get(realm, &object, &key)?;
            if ops::strict_equals(&value, &target) {
                return Ok(number(index));
            }
        }
    }
    Ok(Value::Number(-1.0))
}

pub(crate) fn join(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let target = ops::to_object(realm, &this)?;
    let object = Value::Object(target.clone());
    let length = length_of(realm, &object)?;
    let separator = match arg(args, 0) {
        Value::Undefined => JsStr::from(","),
        value => ops::to_string(realm, &value)?,
    };
    if realm.joining.contains(&target) {
        return Ok(Value::String(JsStr::default()));
    }
    realm.joining.push(target.clone());
    let result = join_elements(realm, &object, length, &separator);
    realm.joining.pop();
    result.map(Value::String)
}

fn join_elements(
    realm: &mut Realm,
    object: &Value,
    length: u64,
    separator: &JsStr,
) -> Result<JsStr, Thrown> {
    let mut result = JsStr::default();
    for index in 0..length {
        if index > 0 {
            result = result.concat(separator);
        }
        let element = ops::get(realm, object, &index_key(index))?;
        if !element.is_nullish() {
            result = result.concat(&ops::to_string(realm, &element)?);
        }
    }
    Ok(result)
}

fn pop(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length == 0 {
        set_length(realm, &object, 0)?;
        return Ok(Value::Undefined);
    }
    let last = index_key(length - 1);
    let value = ops::get(realm, &object, &last)?;
    delete(realm, &object, &last)?;
    set_length(realm, &object, length - 1)?;
    Ok(value)
}

fn push(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length + args.len() as u64 > MAX_LENGTH {
        return Err(too_long(realm));
    }
    for (offset, value) in args.iter().enumerate() {
        ops::set(
            realm,
            &object,
            &index_key(length + offset as u64),
            value.clone(),
        )?;
    }
    let length = length + args.len() as u64;
    set_length(realm, &object, length)?;
    Ok(number(length))
}

/// `reduce` and `reduceRight`.
fn reduce_in(realm: &mut Realm, this: &Value, args: &[Value], right: bool) -> Completion {
    let object = receiver(realm, this)?;
    let length = length_of(realm, &object)?;
    let method = if right { "reduceRight" } else { "reduce" };
    let function = callback(realm, args, method)?;
    let order = |step: u64| if right { length - 1 - step } else { step };
    let mut step = 0;
    let mut accumulator = if args.len() > 1 {
        args[1].clone()
    } else {
        loop {
            if step == length {
                return Err(realm.throw(
                    ErrorKind::Type,
                    &format!("Array.prototype.{method} of empty array with no initial value"),
                ));
            }
            let key = index_key(order(step));
            step += 1;
            if has(realm, &object, &key)? {
                break ops::get(realm, &object, &key)?;
            }
        }
    };
    while step < length {
        let index = order(step);
        let key = index_key(index);
        if has(realm, &object, &key)? {
            let value = ops::get(realm, &object, &key)?;
            accumulator = ops::call(
                realm,
                &function,
                Value::Undefined,
                &[accumulator, value, number(index), object.clone()],
            )?;
        }
        step += 1;
    }
    Ok(accumulator)
}

fn reverse(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    for lower in 0..length / 2 {
        let upper = length - 1 - lower;
        let (lower_key, upper_key) = (index_key(lower), index_key(upper));
        let lower_exists = has(realm, &object, &lower_key)?;
        let lower_value = if lower_exists {
            ops::get(realm, &object, &lower_key)?
        } else {
            Value::Undefined
        };
        let upper_exists = has(realm, &object, &upper_key)?;
        let upper_value = if upper_exists {
            ops::get(realm, &object, &upper_key)?
        } else {
            Value::Undefined
        };
        match (lower_exists, upper_exists) {
            (true, true) => {
                ops::set(realm, &object, &lower_key, upper_value)?;
                ops::set(realm, &object, &upper_key, lower_value)?;
            }
            (false, true) => {
                ops::set(realm, &object, &lower_key, upper_value)?;
                delete(realm, &object, &upper_key)?;
            }
            (true, false) => {
                delete(realm, &object, &lower_key)?;
                ops::set(realm, &object, &upper_key, lower_value)?;
            }
            (false, false) => {}
        }
    }
    Ok(object)
}

fn shift(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    if length == 0 {
        set_length(realm, &object, 0)?;
        return Ok(Value::Undefined);
    }
    let first = ops::get(realm, &object, &index_key(0))?;
    move_range(realm, &object, 1, 0, length - 1)?;
    delete(realm, &object, &index_key(length - 1))?;
    set_length(realm, &object, length - 1)?;
    Ok(first)
}

/// Moves `count` elements from `from` to `to`, preserving holes, in the
/// order that never overwrites an unmoved element.
fn move_range(
    realm: &mut Realm,
    object: &Value,
    from: u64,
    to: u64,
    count: u64,
) -> Result<(), Thrown> {
    for step in 0..count {
        let offset = if to > from { count - 1 - step } else { step };
        let source = index_key(from + offset);
        let target = index_key(to + offset);
        if has(realm, object, &source)? {
            let value = ops::get(realm, object, &source)?;
            ops::set(realm, object, &target, value)?;
        } else {
            delete(realm, object, &target)?;
        }
    }
    Ok(())
}

fn unshift(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let count = args.len() as u64;
    if count > 0 {
        if length + count > MAX_LENGTH {
            return Err(too_long(realm));
        }
        move_range(realm, &object, 0, count, length)?;
        for (index, value) in args.iter().enumerate() {
            ops::set(realm, &object, &index_key(index as u64), value.clone())?;
        }
    }
    set_length(realm, &object, length + count)?;
    Ok(number(length + count))
}

fn slice(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let start = relative(realm, &arg(args, 0), length, 0)?;
    let end = relative(realm, &arg(args, 1), length, length)?;
    let count = end.saturating_sub(start);
    let result = species_create(realm, &object, count)?;
    for (offset, index) in (start..end).enumerate() {
        let key = index_key(index);
        if has(realm, &object, &key)? {
            let value = ops::get(realm, &object, &key)?;
            create_data_property(realm, &result, index_key(offset as u64), value)?;
        }
    }
    let result = Value::Object(result);
    set_length(realm, &result, count)?;
    Ok(result)
}

/// `SortCompare` for defined elements (`undefined` sorts last without
/// comparison): without a comparator, elements compare as strings by code
/// units. Whether `right` sorts before `left`.
fn compare(
    realm: &mut Realm,
    comparator: &Value,
    left: &Value,
    right: &Value,
) -> Result<bool, Thrown> {
    if matches!(comparator, Value::Undefined) {
        let left = ops::to_string(realm, left)?;
        let right = ops::to_string(realm, right)?;
        return Ok(right < left);
    }
    let order = ops::call(
        realm,
        comparator,
        Value::Undefined,
        &[left.clone(), right.clone()],
    )?;
    // Only a positive comparison moves the right element first.
    Ok(ops::to_number(realm, &order)? > 0.0)
}

/// A stable merge sort with a fallible comparison that tolerates
/// inconsistent comparators, as user-defined ones may be.
fn sort_values(
    realm: &mut Realm,
    comparator: &Value,
    values: Vec<Value>,
) -> Result<Vec<Value>, Thrown> {
    let (mut defined, undefined): (Vec<Value>, Vec<Value>) = values
        .into_iter()
        .partition(|value| !matches!(value, Value::Undefined));
    let mut buffer = defined.clone();
    let mut width = 1;
    while width < defined.len() {
        let mut start = 0;
        while start < defined.len() {
            let middle = (start + width).min(defined.len());
            let end = (start + 2 * width).min(defined.len());
            let (mut left, mut right, mut out) = (start, middle, start);
            while left < middle && right < end {
                if compare(realm, comparator, &defined[left], &defined[right])? {
                    buffer[out] = defined[right].clone();
                    right += 1;
                } else {
                    buffer[out] = defined[left].clone();
                    left += 1;
                }
                out += 1;
            }
            for index in (left..middle).chain(right..end) {
                buffer[out] = defined[index].clone();
                out += 1;
            }
            start = end;
        }
        core::mem::swap(&mut defined, &mut buffer);
        width *= 2;
    }
    defined.extend(undefined);
    Ok(defined)
}

fn comparator(realm: &Realm, args: &[Value], method: &str) -> Result<Value, Thrown> {
    let comparator = arg(args, 0);
    if matches!(comparator, Value::Undefined) || is_callable(&comparator) {
        Ok(comparator)
    } else {
        Err(realm.throw(
            ErrorKind::Type,
            &format!("The comparison function of Array.prototype.{method} must be a function or undefined"),
        ))
    }
}

/// `SortIndexedProperties` with holes skipped: the present elements.
fn present(realm: &mut Realm, object: &Value, length: u64) -> Result<Vec<Value>, Thrown> {
    let mut values = Vec::new();
    for index in 0..length {
        let key = index_key(index);
        if has(realm, object, &key)? {
            values.push(ops::get(realm, object, &key)?);
        }
    }
    Ok(values)
}

fn sort(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let comparator = comparator(realm, args, "sort")?;
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let values = present(realm, &object, length)?;
    let count = values.len() as u64;
    let sorted = sort_values(realm, &comparator, values)?;
    for (index, value) in sorted.into_iter().enumerate() {
        ops::set(realm, &object, &index_key(index as u64), value)?;
    }
    for index in count..length {
        delete(realm, &object, &index_key(index))?;
    }
    Ok(object)
}

/// The start and deletion count of `splice` and `toSpliced`.
fn splice_range(realm: &mut Realm, args: &[Value], length: u64) -> Result<(u64, u64), Thrown> {
    let start = relative(realm, &arg(args, 0), length, 0)?;
    let count = match args.len() {
        0 => 0,
        1 => length - start,
        _ => {
            let count = to_integer_or_infinity(realm, &args[1])?;
            count.clamp(0.0, (length - start) as f64) as u64
        }
    };
    Ok((start, count))
}

fn splice(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let (start, delete_count) = splice_range(realm, args, length)?;
    let items = args.get(2..).unwrap_or(&[]);
    let insert = items.len() as u64;
    if length + insert - delete_count > MAX_LENGTH {
        return Err(too_long(realm));
    }
    let removed = species_create(realm, &object, delete_count)?;
    for offset in 0..delete_count {
        let key = index_key(start + offset);
        if has(realm, &object, &key)? {
            let value = ops::get(realm, &object, &key)?;
            create_data_property(realm, &removed, index_key(offset), value)?;
        }
    }
    let removed = Value::Object(removed);
    set_length(realm, &removed, delete_count)?;
    let tail = length - start - delete_count;
    if insert < delete_count {
        move_range(realm, &object, start + delete_count, start + insert, tail)?;
        for index in (length - delete_count + insert..length).rev() {
            delete(realm, &object, &index_key(index))?;
        }
    } else if insert > delete_count {
        move_range(realm, &object, start + delete_count, start + insert, tail)?;
    }
    for (offset, item) in items.iter().enumerate() {
        ops::set(
            realm,
            &object,
            &index_key(start + offset as u64),
            item.clone(),
        )?;
    }
    set_length(realm, &object, length - delete_count + insert)?;
    Ok(removed)
}

fn to_locale_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let mut result = JsStr::default();
    for index in 0..length {
        if index > 0 {
            result = result.concat(&JsStr::from(","));
        }
        let element = ops::get(realm, &object, &index_key(index))?;
        if !element.is_nullish() {
            let method = ops::get_named(realm, &element, "toLocaleString")?;
            let text = ops::call(realm, &method, element, &[])?;
            result = result.concat(&ops::to_string(realm, &text)?);
        }
    }
    Ok(Value::String(result))
}

/// A new array of `values`, for the copying methods.
/// Callers have checked that the length is valid.
fn from_values(realm: &Realm, values: Vec<Value>) -> Completion {
    Ok(Value::Object(realm.array(values)))
}

/// Every element, with holes read as `undefined`.
fn elements(realm: &mut Realm, object: &Value, length: u64) -> Result<Vec<Value>, Thrown> {
    if length > u64::from(u32::MAX) {
        return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
    }
    (0..length)
        .map(|index| ops::get(realm, object, &index_key(index)))
        .collect()
}

fn to_reversed(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let mut values = elements(realm, &object, length)?;
    values.reverse();
    from_values(realm, values)
}

fn to_sorted(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let comparator = comparator(realm, args, "toSorted")?;
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let values = elements(realm, &object, length)?;
    let sorted = sort_values(realm, &comparator, values)?;
    from_values(realm, sorted)
}

fn to_spliced(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let (start, skip) = splice_range(realm, args, length)?;
    let items = args.get(2..).unwrap_or(&[]);
    let new_length = length + items.len() as u64 - skip;
    if new_length > MAX_LENGTH {
        return Err(too_long(realm));
    }
    if new_length > u64::from(u32::MAX) {
        return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
    }
    let mut values = Vec::new();
    for index in 0..start {
        values.push(ops::get(realm, &object, &index_key(index))?);
    }
    values.extend_from_slice(items);
    for index in start + skip..length {
        values.push(ops::get(realm, &object, &index_key(index))?);
    }
    from_values(realm, values)
}

fn with(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let length = length_of(realm, &object)?;
    let relative = to_integer_or_infinity(realm, &arg(args, 0))?;
    let index = if relative >= 0.0 {
        relative
    } else {
        length as f64 + relative
    };
    if index < 0.0 || index >= length as f64 {
        return Err(realm.throw(ErrorKind::Range, "Invalid index"));
    }
    let mut values = elements(realm, &object, length)?;
    values[index as usize] = arg(args, 1);
    from_values(realm, values)
}

fn to_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let object = receiver(realm, &this)?;
    let join = ops::get_named(realm, &object, "join")?;
    if is_callable(&join) {
        ops::call(realm, &join, object, &[])
    } else {
        crate::object_builtin::to_string(realm, object, &[])
    }
}

/// `new Array(...)` and `Array(...)` with the new target's prototype.
fn construct(realm: &mut Realm, prototype: &Object, args: &[Value]) -> Result<Object, Thrown> {
    let array = match args {
        [Value::Number(length)] => {
            let integer = ops::wrap32(*length);
            if f64::from(integer) != *length {
                return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
            }
            array_create(realm, u64::from(integer))?
        }
        _ => realm.array(args.to_vec()),
    };
    if *prototype != realm.array_prototype {
        array.set_prototype(Some(prototype.clone()));
    }
    Ok(array)
}

fn from(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let items = arg(args, 0);
    let mapper = arg(args, 1);
    let mapping = !matches!(mapper, Value::Undefined);
    if mapping && !is_callable(&mapper) {
        return Err(realm.throw(ErrorKind::Type, "Array.from requires a mapping function"));
    }
    let receiver = arg(args, 2);
    let method = if items.is_nullish() {
        Value::Undefined
    } else {
        let key = Key::Symbol(realm.symbols.iterator.clone());
        ops::get(realm, &items, &key)?
    };
    if !method.is_nullish() {
        if !is_callable(&method) {
            return Err(realm.throw(ErrorKind::Type, "Symbol.iterator is not a function"));
        }
        let object = new_target(realm, &this, &[], 0)?;
        let target = Value::Object(object.clone());
        let mut iteration = iter::iterate_method(realm, &items, &method)?;
        let mut index = 0u64;
        loop {
            let Some(value) = iteration.step(realm)? else {
                set_length(realm, &target, index)?;
                return Ok(target);
            };
            let value = if mapping {
                match ops::call(realm, &mapper, receiver.clone(), &[value, number(index)]) {
                    Ok(value) => value,
                    Err(thrown) => return Err(iteration.close_thrown(realm, thrown)),
                }
            } else {
                value
            };
            if let Err(thrown) = create_data_property(realm, &object, index_key(index), value) {
                return Err(iteration.close_thrown(realm, thrown));
            }
            index += 1;
        }
    }
    let source = Value::Object(ops::to_object(realm, &items)?);
    let length = length_of(realm, &source)?;
    let object = new_target(realm, &this, &[number(length)], length)?;
    let target = Value::Object(object.clone());
    for index in 0..length {
        let value = ops::get(realm, &source, &index_key(index))?;
        let value = if mapping {
            ops::call(realm, &mapper, receiver.clone(), &[value, number(index)])?
        } else {
            value
        };
        create_data_property(realm, &object, index_key(index), value)?;
    }
    set_length(realm, &target, length)?;
    Ok(target)
}

fn of(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let length = args.len() as u64;
    let object = new_target(realm, &this, &[number(length)], length)?;
    let target = Value::Object(object.clone());
    for (index, value) in args.iter().enumerate() {
        create_data_property(realm, &object, index_key(index as u64), value.clone())?;
    }
    set_length(realm, &target, length)?;
    Ok(target)
}

/// Installs `Array` and `Array.prototype`'s methods.
pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.array_prototype.clone();
    let constructor = realm.builtin_constructor(
        "Array",
        1,
        &prototype,
        |realm, _, args| {
            let prototype = realm.array_prototype.clone();
            Ok(Value::Object(construct(realm, &prototype, args)?))
        },
        |realm, target, args| {
            let fallback = realm.array_prototype.clone();
            let prototype = prototype_from(realm, &target, &fallback)?;
            construct(realm, &prototype, args)
        },
    );
    realm.method(&constructor, "from", 1, from);
    realm.method(&constructor, "of", 0, of);
    realm.method(&constructor, "isArray", 1, |realm, _, args| {
        is_array(realm, &arg(args, 0)).map(Value::Boolean)
    });
    let species = realm.function("get [Symbol.species]", 0, false, |_, this, _| Ok(this));
    constructor.define(
        Key::Symbol(realm.symbols.species.clone()),
        Property {
            slot: Slot::Accessor {
                get: Some(species),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm.global.define(
        crate::string::static_key!("Array"),
        Property::hidden(Value::Object(constructor)),
    );
    type Method = fn(&mut Realm, Value, &[Value]) -> Completion;
    let methods: [(&str, u32, Method); 33] = [
        ("at", 1, at),
        ("concat", 1, concat),
        ("copyWithin", 2, copy_within),
        ("every", 1, every),
        ("fill", 1, fill),
        ("filter", 1, filter),
        ("find", 1, |realm, this, args| {
            find_in(realm, &this, args, "find", false, false)
        }),
        ("findIndex", 1, |realm, this, args| {
            find_in(realm, &this, args, "findIndex", false, true)
        }),
        ("findLast", 1, |realm, this, args| {
            find_in(realm, &this, args, "findLast", true, false)
        }),
        ("findLastIndex", 1, |realm, this, args| {
            find_in(realm, &this, args, "findLastIndex", true, true)
        }),
        ("flat", 0, flat),
        ("flatMap", 1, flat_map),
        ("forEach", 1, for_each),
        ("includes", 1, includes),
        ("indexOf", 1, index_of),
        ("join", 1, join),
        ("lastIndexOf", 1, last_index_of),
        ("map", 1, map),
        ("pop", 0, pop),
        ("push", 1, push),
        ("reduce", 1, |realm, this, args| {
            reduce_in(realm, &this, args, false)
        }),
        ("reduceRight", 1, |realm, this, args| {
            reduce_in(realm, &this, args, true)
        }),
        ("reverse", 0, reverse),
        ("shift", 0, shift),
        ("slice", 2, slice),
        ("some", 1, some),
        ("sort", 1, sort),
        ("splice", 2, splice),
        ("toLocaleString", 0, to_locale_string),
        ("toReversed", 0, to_reversed),
        ("toSorted", 1, to_sorted),
        ("toSpliced", 2, to_spliced),
        ("unshift", 1, unshift),
    ];
    for (name, length, code) in methods {
        realm.method(&prototype, name, length, code);
    }
    realm.method(&prototype, "with", 2, with);
    realm.method(&prototype, "toString", 0, to_string);
    // `with` statements skip these names on arrays.
    let unscopables = Object::new(None, Kind::Ordinary);
    for name in [
        "at",
        "copyWithin",
        "entries",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "flat",
        "flatMap",
        "includes",
        "keys",
        "toReversed",
        "toSorted",
        "toSpliced",
        "values",
    ] {
        unscopables.define(Key::name(name), Property::data(Value::Boolean(true)));
    }
    prototype.define(
        Key::Symbol(realm.symbols.unscopables.clone()),
        Property {
            slot: Slot::Data {
                value: Value::Object(unscopables),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

#[cfg(test)]
mod tests;
