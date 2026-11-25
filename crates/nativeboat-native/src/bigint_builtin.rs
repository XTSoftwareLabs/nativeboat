//! The `BigInt` function, its static methods and `BigInt.prototype`, with
//! V8's error messages.

use crate::bigint::BigInt;
use crate::object::{Kind, Property, Slot, Wrapped};
use crate::ops::{self, Hint};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, ErrorKind, JsStr, Key, Realm, Thrown, Value};
use alloc::rc::Rc;

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn big(value: BigInt) -> Value {
    Value::BigInt(Rc::new(value))
}

/// How V8 names a value in conversion errors.
fn shown(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    Ok(match value {
        Value::Symbol(symbol) => symbol.descriptive().to_rust_lossy(),
        _ => ops::to_string(realm, value)?.to_rust_lossy(),
    })
}

/// `ToBigInt` of a primitive.
pub(crate) fn to_big(realm: &mut Realm, value: &Value) -> Result<Rc<BigInt>, Thrown> {
    let primitive = ops::to_primitive(realm, value, Hint::Number)?;
    match &primitive {
        Value::BigInt(value) => Ok(value.clone()),
        Value::Boolean(flag) => Ok(Rc::new(BigInt::from_i64(i64::from(*flag)))),
        Value::String(text) => BigInt::parse(text.units()).map(Rc::new).ok_or_else(|| {
            let text = text.to_rust_lossy();
            realm.throw(
                ErrorKind::Syntax,
                &format!("Cannot convert {text} to a BigInt"),
            )
        }),
        _ => {
            let text = shown(realm, &primitive)?;
            Err(realm.throw(
                ErrorKind::Type,
                &format!("Cannot convert {text} to a BigInt"),
            ))
        }
    }
}

/// `BigInt(value)`: integral numbers convert exactly; other values as
/// `ToBigInt` converts them.
fn construct_value(realm: &mut Realm, value: &Value) -> Completion {
    let primitive = ops::to_primitive(realm, value, Hint::Number)?;
    if let Value::Number(number) = primitive {
        if !number.is_finite() || crate::float::fract(number) != 0.0 {
            let text = crate::number::to_string(number);
            return Err(realm.throw(
                ErrorKind::Range,
                &format!(
                    "The number {text} cannot be converted to a BigInt because it is not an integer"
                ),
            ));
        }
        return Ok(big(BigInt::from_integer(number)));
    }
    Ok(Value::BigInt(to_big(realm, &primitive)?))
}

/// `thisBigIntValue`.
fn this_value(realm: &Realm, this: &Value, method: &str) -> Result<Rc<BigInt>, Thrown> {
    match this {
        Value::BigInt(value) => return Ok(value.clone()),
        Value::Object(object) => {
            if let Kind::Primitive(Wrapped::BigInt(value)) = &object.data().kind {
                return Ok(value.clone());
            }
        }
        _ => {}
    }
    Err(realm.throw(
        ErrorKind::Type,
        &format!("BigInt.prototype.{method} requires that 'this' be a BigInt"),
    ))
}

/// `ToIndex` with V8's message.
fn to_index(realm: &mut Realm, value: &Value) -> Result<u64, Thrown> {
    let number = ops::to_number(realm, value)?;
    let integer = if number.is_nan() {
        0.0
    } else {
        number - crate::float::fract(number)
    };
    if !(0.0..=9_007_199_254_740_991.0).contains(&integer) {
        return Err(realm.throw(
            ErrorKind::Range,
            "Invalid value: not (convertible to) a safe integer",
        ));
    }
    Ok(integer as u64)
}

/// Digits grouped by threes with commas, as the default `en-US` locale
/// formats integers.
fn grouped(text: &str) -> String {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", text),
    };
    let mut result = String::from(sign);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.bigint_prototype.clone();
    let constructor = realm.builtin_constructor(
        "BigInt",
        1,
        &prototype,
        |realm, _, args| construct_value(realm, &arg(args, 0)),
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "BigInt is not a constructor")),
    );
    let width = |realm: &mut Realm, args: &[Value], signed: bool| -> Completion {
        let bits = to_index(realm, &arg(args, 0))?;
        let value = to_big(realm, &arg(args, 1))?;
        let result = if signed {
            value.as_int_n(bits)
        } else {
            value.as_uint_n(bits)
        };
        result
            .map(big)
            .map_err(|_| realm.throw(ErrorKind::Range, "Maximum BigInt size exceeded"))
    };
    realm.method(&constructor, "asIntN", 2, move |realm, _, args| {
        width(realm, args, true)
    });
    realm.method(&constructor, "asUintN", 2, move |realm, _, args| {
        width(realm, args, false)
    });
    realm.method(&prototype, "toString", 0, |realm, this, args| {
        let value = this_value(realm, &this, "toString")?;
        let radix = match arg(args, 0) {
            Value::Undefined => 10.0,
            radix => ops::to_number(realm, &radix)?,
        };
        let radix = if radix.is_nan() {
            0.0
        } else {
            radix - crate::float::fract(radix)
        };
        if !(2.0..=36.0).contains(&radix) {
            return Err(realm.throw(
                ErrorKind::Range,
                "toString() radix argument must be between 2 and 36",
            ));
        }
        Ok(Value::str(&value.to_radix(radix as u32)))
    });
    realm.method(&prototype, "toLocaleString", 0, |realm, this, _| {
        let value = this_value(realm, &this, "toLocaleString")?;
        Ok(Value::str(&grouped(&value.to_radix(10))))
    });
    realm.method(&prototype, "valueOf", 0, |realm, this, _| {
        Ok(Value::BigInt(this_value(realm, &this, "valueOf")?))
    });
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::String(JsStr::from("BigInt")),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm.set_global("BigInt", Value::Object(constructor));
}

#[cfg(test)]
mod tests;
