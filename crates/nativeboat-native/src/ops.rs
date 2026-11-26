//! Abstract operations and operators of the source language over dynamic
//! values. Generated code calls these directly; there is no instruction
//! dispatch. Property operations follow strict-mode semantics, as modules
//! and classes run strictly.

#[allow(unused_imports)]
use crate::bigint::BigInt;
use crate::prelude::*;

use crate::number;
use crate::object::{Descriptor, Key, Kind, Object, Property, Slot, Wrapped};
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};

/// Which conversion `ToPrimitive` prefers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// No preference, as `+` and `==` ask; ordinary objects prefer `valueOf`.
    Default,
    /// Prefer `valueOf`.
    Number,
    /// Prefer `toString`.
    String,
}

/// `ToBoolean`.
#[inline]
pub fn to_boolean(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => !(value.is_nan() || *value == 0.0),
        Value::String(text) => !text.is_empty(),
        Value::BigInt(value) => !value.is_zero(),
        Value::Symbol(_) | Value::Object(_) => true,
    }
}

/// `ToPrimitive`: `Symbol.toPrimitive`, else `valueOf`/`toString`.
pub fn to_primitive(realm: &mut Realm, value: &Value, hint: Hint) -> Completion {
    let Value::Object(object) = value else {
        return Ok(value.clone());
    };
    let exotic = get(
        realm,
        value,
        &Key::Symbol(realm.symbols.to_primitive.clone()),
    )?;
    if !exotic.is_nullish() {
        let name = match hint {
            Hint::Default => "default",
            Hint::Number => "number",
            Hint::String => "string",
        };
        let result = call(realm, &exotic, value.clone(), &[Value::str(name)])?;
        if matches!(result, Value::Object(_)) {
            return Err(realm.throw(ErrorKind::Type, "Cannot convert object to primitive value"));
        }
        return Ok(result);
    }
    ordinary_to_primitive(realm, object, hint)
}

/// `OrdinaryToPrimitive`: `valueOf` and `toString` in the hint's order.
pub(crate) fn ordinary_to_primitive(realm: &mut Realm, object: &Object, hint: Hint) -> Completion {
    let value = &Value::Object(object.clone());
    let order = match hint {
        Hint::Default | Hint::Number => ["valueOf", "toString"],
        Hint::String => ["toString", "valueOf"],
    };
    for name in order {
        let method = get(realm, value, &Key::name(name))?;
        if let Value::Object(function) = &method
            && function.code().is_some()
        {
            let result = call(realm, &method, Value::Object(object.clone()), &[])?;
            if !matches!(result, Value::Object(_)) {
                return Ok(result);
            }
        }
    }
    Err(realm.throw(ErrorKind::Type, "Cannot convert object to primitive value"))
}

/// `ToNumber`.
pub fn to_number(realm: &mut Realm, value: &Value) -> Result<f64, Thrown> {
    Ok(match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Boolean(value) => f64::from(u8::from(*value)),
        Value::Number(value) => *value,
        Value::String(text) => number::parse(text),
        Value::Symbol(_) => {
            return Err(realm.throw(ErrorKind::Type, "Cannot convert a Symbol value to a number"));
        }
        Value::BigInt(_) => {
            return Err(realm.throw(ErrorKind::Type, "Cannot convert a BigInt value to a number"));
        }
        Value::Object(_) => {
            let primitive = to_primitive(realm, value, Hint::Number)?;
            return to_number(realm, &primitive);
        }
    })
}

/// `ToNumeric`: a BigInt, or a Number for every other value.
pub fn to_numeric(realm: &mut Realm, value: &Value) -> Completion {
    let primitive = to_primitive(realm, value, Hint::Number)?;
    if let Value::BigInt(_) = primitive {
        return Ok(primitive);
    }
    Ok(Value::Number(to_number(realm, &primitive)?))
}

fn mixed(realm: &Realm) -> Thrown {
    realm.throw(
        ErrorKind::Type,
        "Cannot mix BigInt and other types, use explicit conversions",
    )
}

fn too_large(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Range, "Maximum BigInt size exceeded")
}

fn big(value: BigInt) -> Value {
    Value::BigInt(alloc::rc::Rc::new(value))
}

/// The unary `-` operator.
pub fn negate(realm: &mut Realm, value: &Value) -> Completion {
    Ok(match to_numeric(realm, value)? {
        Value::BigInt(value) => big(value.negate()),
        number => Value::Number(-to_number(realm, &number)?),
    })
}

/// The `~` operator.
pub fn bitwise_not(realm: &mut Realm, value: &Value) -> Completion {
    Ok(match to_numeric(realm, value)? {
        Value::BigInt(value) => big(value.not()),
        number => Value::Number(f64::from(!to_int32(realm, &number)?)),
    })
}

/// `++` and `--`: the numeric old value and the value one step away.
#[inline]
pub fn increment(realm: &mut Realm, value: &Value, step: i64) -> Result<(Value, Value), Thrown> {
    if let Value::Number(number) = value {
        return Ok((value.clone(), Value::Number(number + step as f64)));
    }
    increment_slow(realm, value, step)
}

fn increment_slow(realm: &mut Realm, value: &Value, step: i64) -> Result<(Value, Value), Thrown> {
    let old = to_numeric(realm, value)?;
    let new = match &old {
        Value::BigInt(value) => big(value
            .add(&BigInt::from_i64(step))
            .map_err(|_| too_large(realm))?),
        number => Value::Number(to_number(realm, number)? + step as f64),
    };
    Ok((old, new))
}

/// `ToString`.
pub fn to_string(realm: &mut Realm, value: &Value) -> Result<JsStr, Thrown> {
    Ok(match value {
        Value::Undefined => JsStr::from("undefined"),
        Value::Null => JsStr::from("null"),
        Value::Boolean(value) => JsStr::from(if *value { "true" } else { "false" }),
        Value::Number(value) => JsStr::from(number::to_string(*value).as_str()),
        Value::String(text) => text.clone(),
        Value::BigInt(value) => JsStr::from(value.to_radix(10).as_str()),
        Value::Symbol(_) => {
            return Err(realm.throw(ErrorKind::Type, "Cannot convert a Symbol value to a string"));
        }
        Value::Object(_) => {
            let primitive = to_primitive(realm, value, Hint::String)?;
            return to_string(realm, &primitive);
        }
    })
}

/// `ToPropertyKey`.
pub fn to_key(realm: &mut Realm, value: &Value) -> Result<Key, Thrown> {
    if let Value::Number(number) = value
        && crate::float::fract(*number) == 0.0
        && *number >= 0.0
        && *number < f64::from(u32::MAX)
    {
        return Ok(Key::Index(*number as u32));
    }
    let primitive = to_primitive(realm, value, Hint::String)?;
    if let Value::Symbol(symbol) = primitive {
        return Ok(Key::Symbol(symbol));
    }
    Ok(Key::from_js(&to_string(realm, &primitive)?))
}

/// `ToInt32`.
pub fn to_int32(realm: &mut Realm, value: &Value) -> Result<i32, Thrown> {
    Ok(wrap32(to_number(realm, value)?) as i32)
}

/// `ToUint32`.
pub fn to_uint32(realm: &mut Realm, value: &Value) -> Result<u32, Thrown> {
    Ok(wrap32(to_number(realm, value)?))
}

pub(crate) fn wrap32(number: f64) -> u32 {
    if !number.is_finite() {
        return 0;
    }
    crate::float::rem_euclid(crate::float::trunc(number), 4_294_967_296.0) as u32
}

/// The `typeof` operator.
pub fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Boolean(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Symbol(_) => "symbol",
        Value::BigInt(_) => "bigint",
        Value::Object(object) if object.code().is_some() => "function",
        Value::Object(_) => "object",
    }
}

/// `===`.
#[inline]
pub fn strict_equals(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Boolean(left), Value::Boolean(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Symbol(left), Value::Symbol(right)) => left == right,
        (Value::BigInt(left), Value::BigInt(right)) => left == right,
        (Value::Object(left), Value::Object(right)) => left == right,
        _ => false,
    }
}

/// `==`.
pub fn loose_equals(realm: &mut Realm, left: &Value, right: &Value) -> Result<bool, Thrown> {
    Ok(match (left, right) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
        (Value::Number(_), Value::String(text)) => {
            strict_equals(left, &Value::Number(number::parse(text)))
        }
        (Value::String(text), Value::Number(_)) => {
            strict_equals(&Value::Number(number::parse(text)), right)
        }
        (Value::BigInt(left), Value::Number(right))
        | (Value::Number(right), Value::BigInt(left)) => {
            !right.is_nan() && left.compare_f64(*right) == core::cmp::Ordering::Equal
        }
        (Value::BigInt(left), Value::String(right))
        | (Value::String(right), Value::BigInt(left)) => {
            BigInt::parse(right.units()).is_some_and(|right| **left == right)
        }
        (Value::Boolean(value), _) => {
            loose_equals(realm, &Value::Number(f64::from(u8::from(*value))), right)?
        }
        (_, Value::Boolean(value)) => {
            loose_equals(realm, left, &Value::Number(f64::from(u8::from(*value))))?
        }
        (Value::Object(_), Value::Object(_)) => strict_equals(left, right),
        (Value::Object(_), _) => {
            let left = to_primitive(realm, left, Hint::Default)?;
            loose_equals(realm, &left, right)?
        }
        (_, Value::Object(_)) => {
            let right = to_primitive(realm, right, Hint::Default)?;
            loose_equals(realm, left, &right)?
        }
        _ => strict_equals(left, right),
    })
}

/// The `+` operator.
#[inline]
pub fn add(realm: &mut Realm, left: &Value, right: &Value) -> Completion {
    if let (Value::Number(left), Value::Number(right)) = (left, right) {
        return Ok(Value::Number(left + right));
    }
    add_slow(realm, left, right)
}

fn add_slow(realm: &mut Realm, left: &Value, right: &Value) -> Completion {
    let left = to_primitive(realm, left, Hint::Default)?;
    let right = to_primitive(realm, right, Hint::Default)?;
    if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
        let left = to_string(realm, &left)?;
        let right = to_string(realm, &right)?;
        return Ok(Value::String(left.concat(&right)));
    }
    match (to_numeric(realm, &left)?, to_numeric(realm, &right)?) {
        (Value::BigInt(left), Value::BigInt(right)) => {
            Ok(big(left.add(&right).map_err(|_| too_large(realm))?))
        }
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => Err(mixed(realm)),
        (left, right) => Ok(Value::Number(
            to_number(realm, &left)? + to_number(realm, &right)?,
        )),
    }
}

/// A numeric operator on two BigInts.
fn big_arithmetic(
    realm: &Realm,
    operator: Arithmetic,
    left: &BigInt,
    right: &BigInt,
) -> Completion {
    let large = |_| too_large(realm);
    let shift = |value: &BigInt, count: &BigInt, left_shift: bool| -> Completion {
        // A count beyond 64 bits only fits the largest values' direction.
        let towards_left = left_shift != count.is_negative();
        let bits = count.magnitude_u64();
        Ok(big(match (towards_left, bits) {
            (true, Some(bits)) => value.shift_left(bits).map_err(large)?,
            (true, None) if value.is_zero() => BigInt::default(),
            (true, None) => return Err(too_large(realm)),
            (false, Some(bits)) => value.shift_right(bits),
            (false, None) => value.shift_right(u64::MAX),
        }))
    };
    Ok(big(match operator {
        Arithmetic::Subtract => left.subtract(right).map_err(large)?,
        Arithmetic::Multiply => left.multiply(right).map_err(large)?,
        Arithmetic::Divide | Arithmetic::Remainder => {
            let Some((quotient, remainder)) = left.divide(right) else {
                return Err(realm.throw(ErrorKind::Range, "Division by zero"));
            };
            if operator == Arithmetic::Divide {
                quotient
            } else {
                remainder
            }
        }
        Arithmetic::Exponent => {
            if right.is_negative() {
                // V8's message.
                return Err(realm.throw(ErrorKind::Range, "undefined must be positive"));
            }
            left.power(right).map_err(large)?
        }
        Arithmetic::And => left.bitwise(right, |a, b| a & b),
        Arithmetic::Or => left.bitwise(right, |a, b| a | b),
        Arithmetic::Xor => left.bitwise(right, |a, b| a ^ b),
        Arithmetic::ShiftLeft => return shift(left, right, true),
        Arithmetic::ShiftRight => return shift(left, right, false),
        Arithmetic::ShiftRightUnsigned => {
            return Err(realm.throw(
                ErrorKind::Type,
                "BigInts have no unsigned right shift, use >> instead",
            ));
        }
    }))
}

/// The numeric binary operators other than `+`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arithmetic {
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `%`
    Remainder,
    /// `**`
    Exponent,
    /// `&`
    And,
    /// `|`
    Or,
    /// `^`
    Xor,
    /// `<<`
    ShiftLeft,
    /// `>>`
    ShiftRight,
    /// `>>>`
    ShiftRightUnsigned,
}

/// A numeric binary operator.
#[inline]
pub fn arithmetic(
    realm: &mut Realm,
    operator: Arithmetic,
    left: &Value,
    right: &Value,
) -> Completion {
    // Numbers, the common operands, need no conversion.
    if let (Value::Number(a), Value::Number(b)) = (left, right) {
        return Ok(Value::Number(number_arithmetic(operator, *a, *b)));
    }
    arithmetic_slow(realm, operator, left, right)
}

fn arithmetic_slow(
    realm: &mut Realm,
    operator: Arithmetic,
    left: &Value,
    right: &Value,
) -> Completion {
    let left = to_numeric(realm, left)?;
    let right = to_numeric(realm, right)?;
    match (&left, &right) {
        (Value::BigInt(left), Value::BigInt(right)) => {
            return big_arithmetic(realm, operator, left, right);
        }
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => return Err(mixed(realm)),
        _ => {}
    }
    let a = to_number(realm, &left)?;
    let b = to_number(realm, &right)?;
    Ok(Value::Number(number_arithmetic(operator, a, b)))
}

/// A numeric operator on two Numbers.
#[inline]
fn number_arithmetic(operator: Arithmetic, a: f64, b: f64) -> f64 {
    let int = |value: f64| wrap32(value) as i32;
    let shift = wrap32(b) & 31;
    match operator {
        Arithmetic::Subtract => a - b,
        Arithmetic::Multiply => a * b,
        Arithmetic::Divide => a / b,
        // Rust's `%` on f64 is fmod, which matches the source semantics.
        Arithmetic::Remainder => a % b,
        Arithmetic::Exponent => power(a, b),
        Arithmetic::And => f64::from(int(a) & int(b)),
        Arithmetic::Or => f64::from(int(a) | int(b)),
        Arithmetic::Xor => f64::from(int(a) ^ int(b)),
        Arithmetic::ShiftLeft => f64::from(int(a).wrapping_shl(shift)),
        Arithmetic::ShiftRight => f64::from(int(a) >> shift),
        Arithmetic::ShiftRightUnsigned => f64::from(wrap32(a) >> shift),
    }
}

/// `Number::exponentiate`, which differs from IEEE `pow` for NaN exponents
/// and for ±1 raised to ±∞.
pub fn power(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() || (base.abs() == 1.0 && exponent.is_infinite()) {
        return f64::NAN;
    }
    crate::float::pow(base, exponent)
}

/// `IsLessThan` on primitives. `None` means undefined (a NaN operand),
/// which makes every relational comparison false.
fn less_than_primitive(
    realm: &mut Realm,
    left: &Value,
    right: &Value,
) -> Result<Option<bool>, Thrown> {
    if let (Value::String(left), Value::String(right)) = (left, right) {
        return Ok(Some(left.units() < right.units()));
    }
    let text_big = |text: &JsStr| BigInt::parse(text.units());
    match (left, right) {
        (Value::BigInt(left), Value::String(right)) => {
            return Ok(text_big(right).map(|right| **left < right));
        }
        (Value::String(left), Value::BigInt(right)) => {
            return Ok(text_big(left).map(|left| left < **right));
        }
        _ => {}
    }
    let left = to_numeric(realm, left)?;
    let right = to_numeric(realm, right)?;
    match (&left, &right) {
        (Value::BigInt(left), Value::BigInt(right)) => return Ok(Some(left < right)),
        (Value::BigInt(left), Value::Number(right)) => {
            return Ok((!right.is_nan()).then(|| left.compare_f64(*right).is_lt()));
        }
        (Value::Number(left), Value::BigInt(right)) => {
            return Ok((!left.is_nan()).then(|| right.compare_f64(*left).is_gt()));
        }
        _ => {}
    }
    let a = to_number(realm, &left)?;
    let b = to_number(realm, &right)?;
    Ok(if a.is_nan() || b.is_nan() {
        None
    } else {
        Some(a < b)
    })
}

/// `IsLessThan` with the left operand converted first.
pub fn less_than(realm: &mut Realm, left: &Value, right: &Value) -> Result<Option<bool>, Thrown> {
    let left = to_primitive(realm, left, Hint::Number)?;
    let right = to_primitive(realm, right, Hint::Number)?;
    less_than_primitive(realm, &left, &right)
}

/// The relational operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<=`
    LessOrEqual,
    /// `>=`
    GreaterOrEqual,
}

/// A relational operator. Both operands are converted to primitives in
/// source order before comparing, as the language specifies.
#[inline]
pub fn relational(
    realm: &mut Realm,
    relation: Relation,
    left: &Value,
    right: &Value,
) -> Result<bool, Thrown> {
    // Numbers compare directly; NaN is in no relation.
    if let (Value::Number(a), Value::Number(b)) = (left, right) {
        return Ok(match relation {
            Relation::Less => a < b,
            Relation::Greater => a > b,
            Relation::LessOrEqual => a <= b,
            Relation::GreaterOrEqual => a >= b,
        });
    }
    relational_slow(realm, relation, left, right)
}

fn relational_slow(
    realm: &mut Realm,
    relation: Relation,
    left: &Value,
    right: &Value,
) -> Result<bool, Thrown> {
    let left = to_primitive(realm, left, Hint::Number)?;
    let right = to_primitive(realm, right, Hint::Number)?;
    Ok(match relation {
        Relation::Less => less_than_primitive(realm, &left, &right)? == Some(true),
        Relation::Greater => less_than_primitive(realm, &right, &left)? == Some(true),
        Relation::LessOrEqual => less_than_primitive(realm, &right, &left)? == Some(false),
        Relation::GreaterOrEqual => less_than_primitive(realm, &left, &right)? == Some(false),
    })
}

/// The prototype that property lookups on a primitive start from.
fn base_object(realm: &Realm, value: &Value) -> Option<Object> {
    match value {
        Value::Object(object) => Some(object.clone()),
        Value::String(_) => Some(realm.string_prototype.clone()),
        Value::Number(_) => Some(realm.number_prototype.clone()),
        Value::Boolean(_) => Some(realm.boolean_prototype.clone()),
        Value::Symbol(_) => Some(realm.symbol_prototype.clone()),
        Value::BigInt(_) => Some(realm.bigint_prototype.clone()),
        Value::Undefined | Value::Null => None,
    }
}

fn nullish_error(realm: &Realm, value: &Value, key: &Key, action: &str) -> Thrown {
    let name = if matches!(value, Value::Null) {
        "null"
    } else {
        "undefined"
    };
    realm.throw(
        ErrorKind::Type,
        &format!(
            "Cannot {action} properties of {name} (reading '{}')",
            key.to_js()
        ),
    )
}

/// What a lookup along a prototype chain reaches: a property, a proxy
/// whose internal methods continue the lookup, or nothing.
enum Found {
    Property(Property),
    Proxy(crate::proxy_builtin::Proxy),
    Missing,
}

/// The exception of a `throw` statement, reported when
/// `NATIVEBOAT_TRACE_THROWS` is set.
#[inline]
pub fn thrown(value: Value) -> Thrown {
    crate::trace::thrown(&value);
    Thrown(value)
}

/// The first own property for `key` from `start` up its prototype chain,
/// stopping at a proxy, which answers for itself and its prototypes.
fn find_through(start: &Object, key: &Key) -> Found {
    if key.is_private() {
        return start
            .own_property(key)
            .map_or(Found::Missing, Found::Property);
    }
    // The object itself is looked at in place; its prototypes are held.
    let step = |current: &Object| -> Result<Option<Object>, Found> {
        match current.lookup_own(key) {
            crate::object::OwnLookup::Found(property) => Err(Found::Property(property)),
            crate::object::OwnLookup::Missing(next) => Ok(next),
            crate::object::OwnLookup::Exotic => {
                if let Some(proxy) = crate::proxy_builtin::of(current) {
                    return Err(Found::Proxy(proxy));
                }
                if let Some(property) = current.own_property(key) {
                    return Err(Found::Property(property));
                }
                Ok(current.data().prototype.clone())
            }
        }
    };
    let mut next = match step(start) {
        Ok(next) => next,
        Err(found) => return found,
    };
    while let Some(current) = next {
        next = match step(&current) {
            Ok(next) => next,
            Err(found) => return found,
        };
    }
    Found::Missing
}

/// `base[name]` for a name compiled into the program that is not an array
/// index: its key needs neither allocating nor interning.
pub fn get_static(
    realm: &mut Realm,
    base: &Value,
    name: &'static crate::string::StaticHeader,
) -> Completion {
    get_static_at(realm, base, name, name.lookup())
}

/// [`get_static`] at one place in the program, which remembers where it
/// last found the property in `cache`.
pub fn get_static_at(
    realm: &mut Realm,
    base: &Value,
    name: &'static crate::string::StaticHeader,
    cache: &crate::string::LookupCache,
) -> Completion {
    get_at(realm, base, &name.key(), cache)
}

/// `base[key]` at one place in the program, which remembers where it last
/// found a string key's property in `cache`.
pub fn get_at(
    realm: &mut Realm,
    base: &Value,
    key: &Key,
    cache: &crate::string::LookupCache,
) -> Completion {
    // Objects read where the cache remembers the property.
    if let (Value::Object(object), Key::String(_)) = (base, key) {
        return match object.cached_property(key, cache) {
            crate::object::Cached::Found(Property {
                slot: Slot::Data { value, .. },
                ..
            }) => Ok(value),
            crate::object::Cached::Found(Property {
                slot: Slot::Accessor {
                    get: Some(getter), ..
                },
                ..
            }) => call(realm, &Value::Object(getter), base.clone(), &[]),
            crate::object::Cached::Found(_) | crate::object::Cached::Absent => Ok(Value::Undefined),
            crate::object::Cached::Unknown => get_with_receiver(realm, object, key, base),
        };
    }
    get(realm, base, key)
}

/// `base.name` for a property name fixed in the program.
pub fn get_named(realm: &mut Realm, base: &Value, name: &'static str) -> Completion {
    let key = realm.name_key(name);
    get(realm, base, &key)
}

/// `GetValue` of a property reference: `base[key]`.
pub fn get(realm: &mut Realm, base: &Value, key: &Key) -> Completion {
    // Objects, the common base, are read in place.
    if let Value::Object(object) = base {
        return get_with_receiver(realm, object, key, base);
    }
    if let Value::String(text) = base {
        match key {
            Key::Index(index) => {
                if let Some(unit) = text.units().get(*index as usize) {
                    return Ok(Value::String(JsStr::from_units(&[*unit])));
                }
            }
            Key::String(name) if name.units() == [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68] => {
                return Ok(Value::Number(text.len() as f64));
            }
            Key::String(_) | Key::Symbol(_) => {}
        }
    }
    let Some(start) = base_object(realm, base) else {
        return Err(nullish_error(realm, base, key, "read"));
    };
    get_with_receiver(realm, &start, key, base)
}

/// `[[Get]]` of `object` with an explicit receiver, for getters and proxy
/// traps.
pub fn get_with_receiver(
    realm: &mut Realm,
    object: &Object,
    key: &Key,
    receiver: &Value,
) -> Completion {
    // The native environment's symbol getter suppresses a prototype value,
    // including when an ordinary object inherits the handle.
    if matches!(key, Key::Symbol(_)) {
        let mut current = Some(object.clone());
        while let Some(holder) = current {
            if matches!(holder.data().kind, Kind::Environment) {
                return Ok(Value::Undefined);
            }
            if holder.own_property(key).is_some() || crate::proxy_builtin::of(&holder).is_some() {
                break;
            }
            current = holder.data().prototype.clone();
        }
    }
    match find_through(object, key) {
        Found::Proxy(proxy) => crate::proxy_builtin::get(realm, &proxy, key, receiver),
        Found::Property(Property {
            slot: Slot::Data { value, .. },
            ..
        }) => Ok(value),
        Found::Property(Property {
            slot: Slot::Accessor {
                get: Some(getter), ..
            },
            ..
        }) => call(realm, &Value::Object(getter), receiver.clone(), &[]),
        // Missing properties and accessors without a getter read undefined.
        _ => Ok(Value::Undefined),
    }
}

/// `PutValue` of a property reference in strict code: `base[key] = value`.
pub fn set(realm: &mut Realm, base: &Value, key: &Key, value: Value) -> Result<(), Thrown> {
    put(realm, base, key, value, true, None)
}

/// [`set`] at one place in the program, which remembers in `cache` where
/// it last wrote the property.
pub fn set_at(
    realm: &mut Realm,
    base: &Value,
    key: &Key,
    value: Value,
    cache: &crate::string::LookupCache,
) -> Result<(), Thrown> {
    put(realm, base, key, value, true, Some(cache))
}

/// [`set_sloppy`] at one place in the program, which remembers in `cache`
/// where it last wrote the property.
pub fn set_sloppy_at(
    realm: &mut Realm,
    base: &Value,
    key: &Key,
    value: Value,
    cache: &crate::string::LookupCache,
) -> Result<(), Thrown> {
    put(realm, base, key, value, false, Some(cache))
}

/// `PutValue` of a property reference in sloppy code, where an assignment
/// that cannot be made, such as to a read-only property, does nothing.
pub fn set_sloppy(realm: &mut Realm, base: &Value, key: &Key, value: Value) -> Result<(), Thrown> {
    put(realm, base, key, value, false, None)
}

fn put(
    realm: &mut Realm,
    base: &Value,
    key: &Key,
    value: Value,
    strict: bool,
    cache: Option<&crate::string::LookupCache>,
) -> Result<(), Thrown> {
    // Most assignments replace an own data property's value.
    let value = match base {
        Value::Object(object) => match object.overwrite_data(key, value, cache) {
            Ok(()) => return Ok(()),
            Err(value) => value,
        },
        _ => value,
    };
    // Sloppy code ignores the assignments strict code reports.
    let refuse = |realm: &Realm, message: &str| {
        if strict {
            Err(realm.throw(ErrorKind::Type, message))
        } else {
            Ok(())
        }
    };
    let Some(start) = base_object(realm, base) else {
        return Err(nullish_error(realm, base, key, "set"));
    };
    if matches!(start.data().kind, Kind::Environment) {
        return crate::environment_builtin::set(realm, key, &value);
    }
    // A typed array's own numeric keys perform content conversion, which may
    // run user code, and never reach the prototype chain.
    let view = match &start.data().kind {
        Kind::TypedArray(view) => Some(view.clone()),
        _ => None,
    };
    if let Some(view) = view
        && matches!(base, Value::Object(object) if *object == start)
        && let Some(index) = crate::typed_array_builtin::numeric_key(key)
    {
        return crate::typed_array_builtin::set_element(realm, &view, index, &value);
    }
    // Messages are made only when a write fails: every assignment comes
    // through here.
    let read_only = || {
        format!(
            "Cannot assign to read only property '{}' of object",
            key.to_js()
        )
    };
    match find_through(&start, key) {
        Found::Proxy(proxy) => {
            if crate::proxy_builtin::set(realm, &proxy, key, value, base)? {
                Ok(())
            } else {
                refuse(
                    realm,
                    &format!(
                        "'set' on proxy: trap returned falsish for property '{}'",
                        key.to_js()
                    ),
                )
            }
        }
        Found::Property(Property {
            slot: Slot::Accessor { set, .. },
            ..
        }) => match set {
            Some(setter) => {
                call(realm, &Value::Object(setter), base.clone(), &[value])?;
                Ok(())
            }
            None => refuse(
                realm,
                &format!(
                    "Cannot set property '{}' which has only a getter",
                    key.to_js()
                ),
            ),
        },
        Found::Property(Property {
            slot: Slot::Data {
                writable: false, ..
            },
            ..
        }) => refuse(realm, &read_only()),
        _ => {
            let Value::Object(receiver) = base else {
                return refuse(
                    realm,
                    &format!("Cannot create property '{}' on a primitive", key.to_js()),
                );
            };
            let value = if receiver.is_array() && crate::object::is_length(key) {
                let length = to_number(realm, &value)?;
                if wrap32(length) as f64 != length {
                    return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
                }
                Value::Number(length)
            } else {
                value
            };
            if receiver.write_own(key, value) {
                Ok(())
            } else if receiver.own_property(key).is_none() {
                refuse(
                    realm,
                    &format!(
                        "Cannot add property '{}', object is not extensible",
                        key.to_js()
                    ),
                )
            } else {
                refuse(realm, &read_only())
            }
        }
    }
}

/// The `delete` operator in sloppy code: whether the property is gone.
pub fn delete_sloppy(realm: &mut Realm, base: &Value, key: &Key) -> Result<bool, Thrown> {
    match base {
        Value::Object(object) => delete_property(realm, object, key),
        Value::Undefined | Value::Null => Err(nullish_error(realm, base, key, "delete")),
        _ => Ok(true),
    }
}

/// The `delete` operator in strict code.
pub fn delete(realm: &mut Realm, base: &Value, key: &Key) -> Result<bool, Thrown> {
    match base {
        Value::Object(object) => {
            if delete_property(realm, object, key)? {
                Ok(true)
            } else {
                Err(realm.throw(
                    ErrorKind::Type,
                    &format!("Cannot delete property '{}'", key.to_js()),
                ))
            }
        }
        Value::Undefined | Value::Null => Err(nullish_error(realm, base, key, "delete")),
        _ => Ok(true),
    }
}

/// The `in` operator.
pub fn has_property(realm: &mut Realm, key: &Key, target: &Value) -> Result<bool, Thrown> {
    let Value::Object(object) = target else {
        return Err(realm.throw(
            ErrorKind::Type,
            "Cannot use 'in' operator to search for a key in a primitive",
        ));
    };
    match find_through(object, key) {
        Found::Proxy(proxy) => crate::proxy_builtin::has(realm, &proxy, key),
        Found::Property(_) => Ok(true),
        Found::Missing => Ok(false),
    }
}

/// Reads an identifier from the global environment, preserving the
/// `ReferenceError` for an unresolvable reference.
pub fn get_global(realm: &mut Realm, name: &str) -> Completion {
    global_reference(realm, name, true).get(realm)
}

/// A global name reference used by compiled ambient reads and assignments.
/// As Node 22/V8 does, availability is checked at the read or write: RHS
/// effects that create or delete the property are visible to a global store.
#[derive(Debug)]
pub struct GlobalReference {
    object: Object,
    key: Key,
    strict: bool,
}

/// Resolve an ambient identifier without invoking its property's getter.
pub fn global_reference(realm: &Realm, name: &str, strict: bool) -> GlobalReference {
    let object = realm.global.clone();
    let key = Key::name(name);
    GlobalReference {
        object,
        key,
        strict,
    }
}

impl GlobalReference {
    /// Bare reads throw when the global property is unavailable.
    pub fn get(&self, realm: &mut Realm) -> Completion {
        if !has_property(realm, &self.key, &Value::Object(self.object.clone()))? {
            return Err(realm.throw(
                ErrorKind::Reference,
                &format!("{} is not defined", self.key.to_js()),
            ));
        }
        get(realm, &Value::Object(self.object.clone()), &self.key)
    }

    /// `PutValue` preserves strict-mode errors and sloppy silent rejections.
    /// User-defined setter exceptions always propagate in either mode.
    pub fn set(&self, realm: &mut Realm, value: Value) -> Result<(), Thrown> {
        if self.strict && !has_property(realm, &self.key, &Value::Object(self.object.clone()))? {
            return Err(realm.throw(
                ErrorKind::Reference,
                &format!("{} is not defined", self.key.to_js()),
            ));
        }
        let receiver = Value::Object(self.object.clone());
        if self.strict {
            set(realm, &receiver, &self.key, value)
        } else {
            set_with_receiver(realm, &self.object, &self.key, value, &receiver).map(|_| ())
        }
    }
}

/// `typeof` on an identifier reference does not throw when it is unresolvable.
pub fn type_of_global(realm: &mut Realm, name: &str) -> Completion {
    let target = Value::Object(realm.global.clone());
    let key = Key::name(name);
    if !has_property(realm, &key, &target)? {
        return Ok(Value::str("undefined"));
    }
    Ok(Value::str(type_of(&get(realm, &target, &key)?)))
}

/// `ToObject`: primitives become wrapper objects; null and undefined throw.
pub fn to_object(realm: &mut Realm, value: &Value) -> Result<Object, Thrown> {
    Ok(match value {
        Value::Object(object) => object.clone(),
        Value::Undefined | Value::Null => {
            return Err(realm.throw(
                ErrorKind::Type,
                "Cannot convert undefined or null to object",
            ));
        }
        Value::Boolean(value) => realm.primitive_object(Wrapped::Boolean(*value)),
        Value::Number(value) => realm.primitive_object(Wrapped::Number(*value)),
        Value::String(text) => realm.primitive_object(Wrapped::String(text.clone())),
        Value::Symbol(symbol) => realm.symbol_object(symbol.clone()),
        Value::BigInt(value) => realm.primitive_object(Wrapped::BigInt(value.clone())),
    })
}

/// Applies sloppy ordinary-function `this` binding before entering its body.
/// Strict functions and arrows receive their original value from the caller.
pub fn sloppy_this(realm: &mut Realm, value: Value) -> Value {
    match value {
        Value::Undefined | Value::Null => Value::Object(realm.global.clone()),
        Value::Object(_) => value,
        primitive => Value::Object(
            to_object(realm, &primitive).expect("non-nullish primitive is object-coercible"),
        ),
    }
}

/// `DefinePropertyOrThrow`. A new array length is converted first, with
/// RangeError for values that are not array lengths.
pub fn define_property(
    realm: &mut Realm,
    object: &Object,
    key: &Key,
    descriptor: Descriptor,
) -> Result<(), Thrown> {
    if try_define_property(realm, object, key, descriptor)? {
        Ok(())
    } else {
        Err(realm.throw(
            ErrorKind::Type,
            &format!("Cannot redefine property: {}", key.to_js()),
        ))
    }
}

/// `[[GetOwnProperty]]`.
pub fn get_own_property(
    realm: &mut Realm,
    object: &Object,
    key: &Key,
) -> Result<Option<Property>, Thrown> {
    match crate::proxy_builtin::of(object) {
        Some(proxy) => crate::proxy_builtin::get_own_property(realm, &proxy, key),
        None => Ok(object.own_property(key)),
    }
}

/// `[[OwnPropertyKeys]]`: integer keys, strings, then symbols.
pub fn own_property_keys(realm: &mut Realm, object: &Object) -> Result<Vec<Key>, Thrown> {
    match crate::proxy_builtin::of(object) {
        Some(proxy) => crate::proxy_builtin::own_keys(realm, &proxy),
        None => Ok(object.own_keys()),
    }
}

/// `[[GetPrototypeOf]]`.
pub fn get_prototype_of(realm: &mut Realm, object: &Object) -> Result<Option<Object>, Thrown> {
    match crate::proxy_builtin::of(object) {
        Some(proxy) => crate::proxy_builtin::get_prototype_of(realm, &proxy),
        None => Ok(object.data().prototype.clone()),
    }
}

/// `[[SetPrototypeOf]]`: whether the prototype could be set.
pub fn set_prototype_of(
    realm: &mut Realm,
    object: &Object,
    prototype: Option<Object>,
) -> Result<bool, Thrown> {
    match crate::proxy_builtin::of(object) {
        Some(proxy) => crate::proxy_builtin::set_prototype_of(realm, &proxy, prototype),
        None => Ok(object.set_prototype(prototype)),
    }
}

/// `[[IsExtensible]]`.
pub fn is_extensible(realm: &mut Realm, object: &Object) -> Result<bool, Thrown> {
    match crate::proxy_builtin::of(object) {
        Some(proxy) => crate::proxy_builtin::is_extensible(realm, &proxy),
        None => Ok(object.is_extensible()),
    }
}

/// `[[PreventExtensions]]`: whether the object is no longer extensible. The
/// native environment view stays extensible.
pub fn prevent_extensions(realm: &mut Realm, object: &Object) -> Result<bool, Thrown> {
    if let Some(proxy) = crate::proxy_builtin::of(object) {
        return crate::proxy_builtin::prevent_extensions(realm, &proxy);
    }
    if matches!(object.data().kind, Kind::Environment) {
        return Ok(false);
    }
    object.prevent_extensions();
    Ok(true)
}

/// `[[Delete]]`: whether the property is gone.
pub fn delete_property(realm: &mut Realm, object: &Object, key: &Key) -> Result<bool, Thrown> {
    if let Some(proxy) = crate::proxy_builtin::of(object) {
        return crate::proxy_builtin::delete(realm, &proxy, key);
    }
    if matches!(object.data().kind, Kind::Environment) {
        crate::environment_builtin::delete(key);
        return Ok(true);
    }
    Ok(object.delete(key))
}

/// `OrdinarySet` with an explicit receiver, through proxies on the
/// prototype chain: whether the assignment happened.
pub fn set_with_receiver(
    realm: &mut Realm,
    object: &Object,
    key: &Key,
    value: Value,
    receiver: &Value,
) -> Result<bool, Thrown> {
    match find_through(object, key) {
        Found::Proxy(proxy) => crate::proxy_builtin::set(realm, &proxy, key, value, receiver),
        Found::Property(Property {
            slot: Slot::Data {
                writable: false, ..
            },
            ..
        })
        | Found::Property(Property {
            slot: Slot::Accessor { set: None, .. },
            ..
        }) => Ok(false),
        Found::Property(Property {
            slot: Slot::Accessor {
                set: Some(setter), ..
            },
            ..
        }) => {
            call(realm, &Value::Object(setter), receiver.clone(), &[value])?;
            Ok(true)
        }
        _ => {
            let Value::Object(receiver) = receiver else {
                return Ok(false);
            };
            let descriptor =
                match get_own_property(realm, receiver, key)?.map(|property| property.slot) {
                    Some(Slot::Accessor { .. })
                    | Some(Slot::Data {
                        writable: false, ..
                    }) => return Ok(false),
                    Some(Slot::Data { .. }) => Descriptor {
                        value: Some(value),
                        ..Descriptor::default()
                    },
                    None => Descriptor::of(&Property::data(value)),
                };
            try_define_property(realm, receiver, key, descriptor)
        }
    }
}

/// `[[DefineOwnProperty]]`: whether the definition was allowed. Invalid
/// array lengths and environment values still throw.
pub(crate) fn try_define_property(
    realm: &mut Realm,
    object: &Object,
    key: &Key,
    mut descriptor: Descriptor,
) -> Result<bool, Thrown> {
    if let Some(proxy) = crate::proxy_builtin::of(object) {
        return crate::proxy_builtin::define_own_property(realm, &proxy, key, descriptor);
    }
    if matches!(object.data().kind, Kind::Environment) {
        return crate::environment_builtin::define(realm, key, &descriptor).map(|()| true);
    }
    if object.is_array()
        && crate::object::is_length(key)
        && let Some(value) = &descriptor.value
    {
        let length = to_uint32(realm, value)?;
        if f64::from(length) != to_number(realm, value)? {
            return Err(realm.throw(ErrorKind::Range, "Invalid array length"));
        }
        descriptor.value = Some(Value::Number(f64::from(length)));
    }
    if let Some(view) = crate::typed_array_builtin::view_of(&Value::Object(object.clone()))
        && let Some(index) = crate::typed_array_builtin::numeric_key(key)
        && view.index(index).is_some()
        && descriptor.configurable != Some(false)
        && descriptor.enumerable != Some(false)
        && !descriptor.is_accessor()
        && descriptor.writable != Some(false)
        && let Some(value) = &descriptor.value
    {
        descriptor.value = Some(view.element.convert(realm, value)?);
    }
    Ok(object.define_own_property(key, &descriptor))
}

/// The `in` operator: the right operand must be an object before the key
/// is converted.
pub fn in_operator(realm: &mut Realm, key: &Value, target: &Value) -> Result<bool, Thrown> {
    if !matches!(target, Value::Object(_)) {
        return Err(realm.throw(
            ErrorKind::Type,
            "Cannot use 'in' operator to search for a key in a primitive",
        ));
    }
    let key = to_key(realm, key)?;
    has_property(realm, &key, target)
}

/// Calls a function value.
pub fn call(realm: &mut Realm, function: &Value, this: Value, args: &[Value]) -> Completion {
    let code = function.as_object().and_then(Object::code);
    let Some((code, _)) = code else {
        return Err(realm.throw(ErrorKind::Type, "value is not a function"));
    };
    if let Some(home) = function
        .as_object()
        .and_then(|object| realm.home_of(object))
    {
        // SAFETY: a realm that keeps its functions home stays where it is
        // until it drops, which removes it, and runs on this thread.
        return unsafe { &mut *home }.enter(&code, this, args);
    }
    realm.enter(&code, this, args)
}

/// A call written in source: a callee that is not a function throws V8's
/// `<callee> is not a function`, naming the callee as written.
pub fn call_site(
    realm: &mut Realm,
    function: &Value,
    this: Value,
    args: &[Value],
    site: &str,
) -> Completion {
    let code = function.as_object().and_then(Object::code);
    let Some((code, _)) = code else {
        return Err(realm.throw(ErrorKind::Type, &format!("{site} is not a function")));
    };
    if let Some(home) = function
        .as_object()
        .and_then(|object| realm.home_of(object))
    {
        // SAFETY: as in `call`.
        return unsafe { &mut *home }.enter(&code, this, args);
    }
    if realm.near_depth_limit() && crate::trace::throws_traced() {
        crate::platform::write_stderr(&format!("nativeboat: deep call {site}\n"));
    }
    if let Some(profile) = realm.call_profile.as_mut() {
        profile.enter();
        let started = crate::platform::now();
        let result = realm.enter(&code, this, args);
        let elapsed = crate::platform::now() - started;
        if let Some(profile) = realm.call_profile.as_mut() {
            profile.record(site, elapsed);
        }
        return result;
    }
    realm.enter(&code, this, args)
}

/// The `new` operator, using the same function as its new target.
pub fn construct(realm: &mut Realm, function: &Value, args: &[Value]) -> Completion {
    construct_target(realm, function, args, function)
}

/// A `new` written in source: a callee that is not a constructor throws
/// V8's `<callee> is not a constructor`, naming the callee as written.
pub fn construct_site(
    realm: &mut Realm,
    function: &Value,
    args: &[Value],
    site: &str,
) -> Completion {
    if !matches!(function.as_object().and_then(Object::code), Some((_, true))) {
        return Err(realm.throw(ErrorKind::Type, &format!("{site} is not a constructor")));
    }
    construct_target(realm, function, args, function)
}

/// Constructs using an explicit new target in this realm.
pub fn construct_target(
    realm: &mut Realm,
    function: &Value,
    args: &[Value],
    new_target: &Value,
) -> Completion {
    let code = function.as_object().and_then(Object::code);
    let Some((code, true)) = code else {
        return Err(realm.throw(ErrorKind::Type, "value is not a constructor"));
    };
    let Some((target_code, true)) = new_target.as_object().and_then(Object::code) else {
        return Err(realm.throw(ErrorKind::Type, "new target is not a constructor"));
    };
    drop(target_code);
    let target = new_target
        .as_object()
        .expect("validated object constructor");
    let object = function.as_object().expect("validated callable object");
    let bound = match &object.data().kind {
        Kind::BoundFunction {
            target, arguments, ..
        } => Some((target.clone(), arguments.clone())),
        _ => None,
    };
    if let Some((bound_target, leading)) = bound {
        return realm.with_depth(|realm| {
            let args = crate::function::prepend(realm, &leading, args)?;
            let new_target = if object == target {
                Value::Object(bound_target.clone())
            } else {
                new_target.clone()
            };
            construct_target(realm, &Value::Object(bound_target), &args, &new_target)
        });
    }
    if let Some(construct) = function.as_object().and_then(Object::constructor_code) {
        return realm
            .enter_constructor(&construct, target.clone(), args)
            .map(Value::Object);
    }
    let key = realm.name_key("prototype");
    let prototype = match get(realm, new_target, &key)? {
        Value::Object(prototype) => prototype,
        _ => realm.object_prototype.clone(),
    };
    let instance = Value::Object(Object::new(Some(prototype), Kind::Ordinary));
    let result = realm.enter_target(&code, instance.clone(), args, new_target.clone())?;
    Ok(if matches!(result, Value::Object(_)) {
        result
    } else {
        instance
    })
}

/// The `instanceof` operator: `Symbol.hasInstance`, else the prototype chain.
pub fn instance_of(realm: &mut Realm, value: &Value, function: &Value) -> Result<bool, Thrown> {
    if !matches!(function, Value::Object(_)) {
        return Err(realm.throw(
            ErrorKind::Type,
            "Right-hand side of 'instanceof' is not an object",
        ));
    }
    let custom = get(
        realm,
        function,
        &Key::Symbol(realm.symbols.has_instance.clone()),
    )?;
    if !custom.is_nullish() {
        let result = call(
            realm,
            &custom,
            function.clone(),
            core::slice::from_ref(value),
        )?;
        return Ok(to_boolean(&result));
    }
    if function.as_object().and_then(Object::code).is_none() {
        return Err(realm.throw(
            ErrorKind::Type,
            "Right-hand side of 'instanceof' is not callable",
        ));
    }
    ordinary_has_instance(realm, value, function)
}

/// The intrinsic Function.prototype hasInstance operation. Unlike the
/// instanceof operator's missing-hook case, an uncallable receiver returns false.
pub(crate) fn ordinary_has_instance(
    realm: &mut Realm,
    value: &Value,
    function: &Value,
) -> Result<bool, Thrown> {
    if function.as_object().and_then(Object::code).is_none() {
        return Ok(false);
    }
    let bound_target = match &function
        .as_object()
        .expect("validated callable object")
        .data()
        .kind
    {
        Kind::BoundFunction { target, .. } => Some(target.clone()),
        _ => None,
    };
    if let Some(target) = bound_target {
        return realm.with_depth(|realm| instance_of(realm, value, &Value::Object(target)));
    }
    let Value::Object(object) = value else {
        return Ok(false);
    };
    let key = realm.name_key("prototype");
    let Value::Object(prototype) = get(realm, function, &key)? else {
        return Err(realm.throw(
            ErrorKind::Type,
            "Function has non-object prototype in instanceof check",
        ));
    };
    let mut current = get_prototype_of(realm, object)?;
    while let Some(object) = current {
        if object == prototype {
            return Ok(true);
        }
        current = get_prototype_of(realm, &object)?;
    }
    Ok(false)
}

/// An object literal property: always defines, never runs setters.
/// Names an anonymous function after the computed property key it is
/// defined under, with a prefix such as `get `: a symbol key names it
/// `[description]`. Functions with names keep them.
pub fn name_function(value: &Value, key: &Key, prefix: &str) {
    let Value::Object(function) = value else {
        return;
    };
    if function.code().is_none() {
        return;
    }
    let name_key = crate::string::static_key!("name");
    let named = function.own_property(&name_key).is_some_and(|property| {
        matches!(property.slot, Slot::Data { value: Value::String(ref name), .. } if !name.is_empty())
    });
    if named {
        return;
    }
    let text = match key {
        Key::Symbol(symbol) => symbol
            .description()
            .map_or_else(String::new, |description| {
                format!("[{}]", description.to_rust_lossy())
            }),
        key => key.to_js().to_rust_lossy(),
    };
    function.define(
        name_key,
        Property {
            slot: Slot::Data {
                value: Value::str(&format!("{prefix}{text}")),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

pub fn define_data(object: &Object, key: Key, value: Value) {
    object.define(key, Property::data(value));
}

/// An object literal getter or setter. A getter and a setter for the same
/// key form one accessor property; a later data property replaces it.
pub fn define_accessor(object: &Object, key: Key, get: Option<Object>, set: Option<Object>) {
    let (get, set) = match object.own_property(&key) {
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
    object.define(
        key,
        Property {
            slot: Slot::Accessor { get, set },
            enumerable: true,
            configurable: true,
        },
    );
}

#[cfg(test)]
mod tests;
