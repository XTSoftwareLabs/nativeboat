// Radix formatting adapts V8's algorithm. Copyright 2014, the V8 project authors.
// BSD three-clause notice: ../../nativeboat-start/src/NOTICES.txt.
//! The `Number` and `Boolean` constructors, their static members and
//! prototypes (ECMAScript 2024 §21.1 and §20.3).
//!
//! `toFixed`, `toExponential` and `toPrecision` round the double's exact
//! decimal value as specified (ties choose the larger magnitude), from an
//! exact expansion computed with a small big-integer helper. `toString`
//! with a radix produces the shortest digits that identify the value, by
//! the algorithm V8 uses. `toLocaleString` formats as the default `en-US`
//! number format engines use: grouped integer digits and at most three
//! fraction digits.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::to_integer_or_infinity;
use crate::object::{Key, Kind, Property, Slot, Wrapped};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::value::{Completion, Thrown, Value};

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// A nonnegative big integer, little-endian in base 2³².
#[derive(Debug, Clone)]
struct Big(Vec<u32>);

impl Big {
    fn from_u64(value: u64) -> Big {
        Big(vec![value as u32, (value >> 32) as u32])
    }

    fn mul_small(&mut self, factor: u32) {
        let mut carry = 0u64;
        for word in &mut self.0 {
            let product = u64::from(*word) * u64::from(factor) + carry;
            *word = product as u32;
            carry = product >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }

    fn shift_left(&mut self, bits: u32) {
        let words = (bits / 32) as usize;
        let bits = bits % 32;
        let mut shifted = vec![0u32; words];
        let mut carry = 0u32;
        for &word in &self.0 {
            shifted.push((word << bits) | carry);
            carry = if bits == 0 { 0 } else { word >> (32 - bits) };
        }
        shifted.push(carry);
        self.0 = shifted;
    }

    /// Divides by a small divisor, returning the remainder.
    fn div_small(&mut self, divisor: u32) -> u32 {
        let mut remainder = 0u64;
        for word in self.0.iter_mut().rev() {
            let value = (remainder << 32) | u64::from(*word);
            *word = (value / u64::from(divisor)) as u32;
            remainder = value % u64::from(divisor);
        }
        remainder as u32
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|&word| word == 0)
    }

    /// The decimal digits, most significant first.
    fn decimal(mut self) -> Vec<u8> {
        let mut chunks = Vec::new();
        while !self.is_zero() {
            chunks.push(self.div_small(1_000_000_000));
        }
        let mut digits = Vec::new();
        for (index, chunk) in chunks.iter().rev().enumerate() {
            let text = if index == 0 {
                format!("{chunk}")
            } else {
                format!("{chunk:09}")
            };
            digits.extend(text.bytes().map(|byte| byte - b'0'));
        }
        digits
    }
}

/// The exact decimal value of a positive finite double: digits `d₁d₂…` with
/// `d₁ ≠ 0` and a point position, meaning `0.d₁d₂… × 10^point`.
fn exact_decimal(value: f64) -> (Vec<u8>, i32) {
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let mut big = Big::from_u64(mantissa);
    let mut digits;
    let point;
    if exponent >= 0 {
        big.shift_left(exponent as u32);
        digits = big.decimal();
        point = digits.len() as i32;
    } else {
        // m / 2^k = m × 5^k / 10^k.
        for _ in 0..-exponent {
            big.mul_small(5);
        }
        digits = big.decimal();
        point = digits.len() as i32 + exponent;
    }
    while digits.last() == Some(&0) {
        digits.pop();
    }
    (digits, point)
}

/// Rounds exact digits to `keep` digits, ties away from zero; the result
/// has no digits for zero.
fn round(mut digits: Vec<u8>, mut point: i32, keep: i32) -> (Vec<u8>, i32) {
    if keep < 0 {
        return (Vec::new(), point);
    }
    let keep = keep as usize;
    if keep >= digits.len() {
        return (digits, point);
    }
    let up = digits[keep] >= 5;
    digits.truncate(keep);
    if up {
        let mut index = keep;
        loop {
            if index == 0 {
                digits.insert(0, 1);
                point += 1;
                break;
            }
            index -= 1;
            if digits[index] == 9 {
                digits[index] = 0;
            } else {
                digits[index] += 1;
                break;
            }
        }
    }
    (digits, point)
}

fn digit_text(digits: &[u8]) -> String {
    digits
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect()
}

/// `thisNumberValue`.
fn this_number(realm: &Realm, this: &Value, method: &str) -> Result<f64, Thrown> {
    let number = match this {
        Value::Number(number) => Some(*number),
        Value::Object(object) => match &object.data().kind {
            Kind::Primitive(Wrapped::Number(number)) => Some(*number),
            _ => None,
        },
        _ => None,
    };
    number.ok_or_else(|| {
        realm.throw(
            ErrorKind::Type,
            &format!("Number.prototype.{method} requires that 'this' be a Number"),
        )
    })
}

/// The digits argument of the formatting methods, from 0 (or 1) to 100.
fn digits_argument(realm: &Realm, digits: f64, minimum: f64, method: &str) -> Result<i32, Thrown> {
    if !(minimum..=100.0).contains(&digits) {
        return Err(realm.throw(
            ErrorKind::Range,
            &format!("{method}() argument must be between {minimum} and 100"),
        ));
    }
    Ok(digits as i32)
}

fn number_string(value: f64) -> Value {
    Value::str(&crate::number::to_string(value))
}

/// `Number.prototype.toFixed`.
pub(crate) fn to_fixed(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let value = this_number(realm, &this, "toFixed")?;
    let fraction = to_integer_or_infinity(realm, &arg(args, 0))?;
    let fraction = digits_argument(realm, fraction, 0.0, "toFixed")?;
    if !value.is_finite() || value.abs() >= 1e21 {
        return Ok(number_string(value));
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, point) = if value == 0.0 {
        (Vec::new(), 0)
    } else {
        let (digits, point) = exact_decimal(value.abs());
        round(digits, point, point + fraction)
    };
    // Write all digits from the first integer place down to the last
    // fraction place.
    let places = |index: i32| -> u8 {
        let position = point - 1 - index;
        if position < 0 || position as usize >= digits.len() {
            0
        } else {
            digits[position as usize]
        }
    };
    let integer: String = if point > 0 {
        (0..point)
            .rev()
            .map(|place| char::from(b'0' + places(place)))
            .collect()
    } else {
        "0".to_owned()
    };
    let decimals: String = (1..=fraction)
        .map(|place| char::from(b'0' + places(-place)))
        .collect();
    let text = if fraction == 0 {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{decimals}")
    };
    Ok(Value::str(&text))
}

/// Digits rounded to `count` significant digits and the decimal exponent.
fn significant(value: f64, count: i32) -> (Vec<u8>, i32) {
    if value == 0.0 {
        return (vec![0; count as usize], 0);
    }
    let (digits, point) = exact_decimal(value);
    let (mut digits, point) = round(digits, point, count);
    digits.resize(count as usize, 0);
    (digits, point - 1)
}

fn exponent_text(exponent: i32) -> String {
    format!(
        "e{}{}",
        if exponent < 0 { '-' } else { '+' },
        exponent.abs()
    )
}

/// `Number.prototype.toExponential`.
fn to_exponential(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let value = this_number(realm, &this, "toExponential")?;
    let requested = arg(args, 0);
    let fraction = to_integer_or_infinity(realm, &requested)?;
    if !value.is_finite() {
        return Ok(number_string(value));
    }
    let fraction = digits_argument(realm, fraction, 0.0, "toExponential")?;
    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, exponent) = if matches!(requested, Value::Undefined) {
        if value == 0.0 {
            (vec![0], 0)
        } else {
            // As many digits as needed to identify the value.
            let shortest = format!("{:e}", value.abs());
            let (mantissa, exponent) = shortest.split_once('e').expect("exponential form");
            let digits = mantissa
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|byte| byte - b'0')
                .collect();
            (digits, exponent.parse().expect("integer exponent"))
        }
    } else {
        significant(value.abs(), fraction + 1)
    };
    let text = digit_text(&digits);
    let body = if digits.len() > 1 {
        format!("{}.{}", &text[..1], &text[1..])
    } else {
        text
    };
    Ok(Value::str(&format!(
        "{sign}{body}{}",
        exponent_text(exponent)
    )))
}

/// `Number.prototype.toPrecision`.
fn to_precision(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let value = this_number(realm, &this, "toPrecision")?;
    if matches!(arg(args, 0), Value::Undefined) {
        return Ok(number_string(value));
    }
    let precision = to_integer_or_infinity(realm, &arg(args, 0))?;
    if !value.is_finite() {
        return Ok(number_string(value));
    }
    let precision = digits_argument(realm, precision, 1.0, "toPrecision")?;
    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, exponent) = significant(value.abs(), precision);
    let text = digit_text(&digits);
    let body = if exponent < -6 || exponent >= precision {
        let mantissa = if precision > 1 {
            format!("{}.{}", &text[..1], &text[1..])
        } else {
            text
        };
        format!("{mantissa}{}", exponent_text(exponent))
    } else if exponent == precision - 1 {
        text
    } else if exponent >= 0 {
        let split = exponent as usize + 1;
        format!("{}.{}", &text[..split], &text[split..])
    } else {
        format!("0.{}{text}", "0".repeat((-(exponent + 1)) as usize))
    };
    Ok(Value::str(&format!("{sign}{body}")))
}

/// The shortest digits of a value in a radix other than ten, by the
/// algorithm V8 uses: fraction digits until the remaining value is within
/// half a unit of the double's precision, rounding the last digit to even.
pub(crate) fn radix_string(value: f64, radix: u32) -> String {
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let negative = value < 0.0;
    let value = value.abs();
    let mut integer = crate::float::floor(value);
    let mut fraction = value - integer;
    let next = f64::from_bits(value.to_bits() + 1);
    let mut delta = (0.5 * (next - value)).max(f64::from_bits(1));
    let radix_f = f64::from(radix);
    let mut fraction_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= radix_f;
            delta *= radix_f;
            let digit = fraction as u32;
            fraction_digits.push(CHARS[digit as usize]);
            fraction -= f64::from(digit);
            if (fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1)) && fraction + delta > 1.0 {
                // Round up: increment the last digit that is not the
                // largest; later (largest) digits become trailing zeros
                // and are dropped, and with none left the integer part
                // carries.
                let digit = |byte: u8| {
                    CHARS
                        .iter()
                        .position(|&c| c == byte)
                        .expect("written digit")
                };
                let keep = fraction_digits
                    .iter()
                    .rposition(|&byte| digit(byte) + 1 < radix as usize);
                fraction_digits.truncate(keep.map_or(0, |index| index + 1));
                integer += f64::from(u8::from(keep.is_none()));
                if let Some(last) = fraction_digits.last_mut() {
                    *last = CHARS[digit(*last) + 1];
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut integer_digits = Vec::new();
    // Digits below the double's precision are zeros.
    while integer / radix_f >= 9_007_199_254_740_992.0 {
        integer /= radix_f;
        integer_digits.push(b'0');
    }
    loop {
        let remainder = crate::libm::fmod(integer, radix_f);
        integer_digits.push(CHARS[remainder as usize]);
        integer = (integer - remainder) / radix_f;
        if integer <= 0.0 {
            break;
        }
    }
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    text.extend(integer_digits.iter().rev().map(|&byte| char::from(byte)));
    if !fraction_digits.is_empty() {
        text.push('.');
        text.extend(fraction_digits.iter().map(|&byte| char::from(byte)));
    }
    text
}

/// `Number.prototype.toString`.
fn to_string(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let value = this_number(realm, &this, "toString")?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10.0,
        radix => to_integer_or_infinity(realm, &radix)?,
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(realm.throw(
            ErrorKind::Range,
            "toString() radix must be between 2 and 36",
        ));
    }
    if radix == 10.0 || !value.is_finite() {
        return Ok(number_string(value));
    }
    Ok(Value::str(&radix_string(value, radix as u32)))
}

/// `Number.prototype.toLocaleString` in the default `en-US` format.
fn to_locale_string(realm: &mut Realm, this: Value, _: &[Value]) -> Completion {
    let value = this_number(realm, &this, "toLocaleString")?;
    if value.is_nan() {
        return Ok(Value::str("NaN"));
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value.is_infinite() {
        return Ok(Value::str(&format!("{sign}∞")));
    }
    let (digits, point) = if value == 0.0 {
        (Vec::new(), 1)
    } else {
        let (digits, point) = exact_decimal(value.abs());
        round(digits, point, point + 3)
    };
    let place = |index: i32| -> u8 {
        let position = point - 1 - index;
        if position < 0 || position as usize >= digits.len() {
            0
        } else {
            digits[position as usize]
        }
    };
    let mut integer = String::new();
    let places = point.max(1);
    for index in (0..places).rev() {
        integer.push(char::from(b'0' + place(index)));
        if index > 0 && index % 3 == 0 {
            integer.push(',');
        }
    }
    let mut fraction: String = (1..=3)
        .map(|index| char::from(b'0' + place(-index)))
        .collect();
    while fraction.ends_with('0') {
        fraction.pop();
    }
    let text = if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    };
    Ok(Value::str(&text))
}

/// `thisBooleanValue`.
fn this_boolean(realm: &Realm, this: &Value, method: &str) -> Result<bool, Thrown> {
    let flag = match this {
        Value::Boolean(flag) => Some(*flag),
        Value::Object(object) => match &object.data().kind {
            Kind::Primitive(Wrapped::Boolean(flag)) => Some(*flag),
            _ => None,
        },
        _ => None,
    };
    flag.ok_or_else(|| {
        realm.throw(
            ErrorKind::Type,
            &format!("Boolean.prototype.{method} requires that 'this' be a Boolean"),
        )
    })
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

/// A wrapper object with the new target's prototype.
fn wrapper(
    realm: &mut Realm,
    value: Wrapped,
    target: crate::Object,
) -> Result<crate::Object, Thrown> {
    let object = realm.primitive_object(value);
    if let Value::Object(prototype) = ops::get_named(realm, &Value::Object(target), "prototype")? {
        object.set_prototype(Some(prototype));
    }
    Ok(object)
}

fn number_value(realm: &mut Realm, args: &[Value]) -> Result<f64, Thrown> {
    match args.first() {
        None => Ok(0.0),
        // BigInts convert to the nearest number.
        Some(value) => match ops::to_numeric(realm, value)? {
            Value::BigInt(value) => Ok(value.to_f64()),
            number => ops::to_number(realm, &number),
        },
    }
}

/// Installs `Number`, `Boolean` and their prototypes.
pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.number_prototype.clone();
    // `Number.prototype` is itself a Number object holding +0.
    prototype.data_mut().kind = Kind::Primitive(Wrapped::Number(0.0));
    let constructor = realm.builtin_constructor(
        "Number",
        1,
        &prototype,
        |realm, _, args| Ok(Value::Number(number_value(realm, args)?)),
        |realm, target, args| {
            let value = number_value(realm, args)?;
            wrapper(realm, Wrapped::Number(value), target)
        },
    );
    for (name, value) in [
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", f64::from_bits(1)),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
    ] {
        constructor.define(Key::name(name), fixed(Value::Number(value)));
    }
    let integer = |value: &Value| match value {
        Value::Number(number) => number.is_finite() && crate::float::trunc(*number) == *number,
        _ => false,
    };
    realm.method(&constructor, "isFinite", 1, |_, _, args| {
        Ok(Value::Boolean(
            matches!(args.first(), Some(Value::Number(number)) if number.is_finite()),
        ))
    });
    realm.method(&constructor, "isInteger", 1, move |_, _, args| {
        Ok(Value::Boolean(integer(&arg(args, 0))))
    });
    realm.method(&constructor, "isNaN", 1, |_, _, args| {
        Ok(Value::Boolean(
            matches!(args.first(), Some(Value::Number(number)) if number.is_nan()),
        ))
    });
    realm.method(&constructor, "isSafeInteger", 1, move |_, _, args| {
        let value = arg(args, 0);
        let safe = integer(&value)
            && matches!(value, Value::Number(number) if number.abs() <= 9_007_199_254_740_991.0);
        Ok(Value::Boolean(safe))
    });
    // `Number.parseFloat` and `Number.parseInt` are the global functions.
    for name in ["parseFloat", "parseInt"] {
        let function = realm
            .global
            .own_property(&Key::name(name))
            .expect("global parse functions are installed first");
        constructor.define(Key::name(name), function);
    }
    realm.global.define(
        Key::name("Number"),
        Property::hidden(Value::Object(constructor)),
    );
    type Method = fn(&mut Realm, Value, &[Value]) -> Completion;
    let methods: [(&str, u32, Method); 6] = [
        ("toExponential", 1, to_exponential),
        ("toFixed", 1, to_fixed),
        ("toLocaleString", 0, to_locale_string),
        ("toPrecision", 1, to_precision),
        ("toString", 1, to_string),
        ("valueOf", 0, |realm, this, _| {
            Ok(Value::Number(this_number(realm, &this, "valueOf")?))
        }),
    ];
    for (name, length, code) in methods {
        realm.method(&prototype, name, length, code);
    }

    let prototype = realm.boolean_prototype.clone();
    // `Boolean.prototype` is itself a Boolean object holding false.
    prototype.data_mut().kind = Kind::Primitive(Wrapped::Boolean(false));
    let constructor = realm.builtin_constructor(
        "Boolean",
        1,
        &prototype,
        |_, _, args| Ok(Value::Boolean(ops::to_boolean(&arg(args, 0)))),
        |realm, target, args| {
            let value = ops::to_boolean(&arg(args, 0));
            wrapper(realm, Wrapped::Boolean(value), target)
        },
    );
    realm.global.define(
        Key::name("Boolean"),
        Property::hidden(Value::Object(constructor)),
    );
    realm.method(&prototype, "toString", 0, |realm, this, _| {
        Ok(Value::str(if this_boolean(realm, &this, "toString")? {
            "true"
        } else {
            "false"
        }))
    });
    realm.method(&prototype, "valueOf", 0, |realm, this, _| {
        Ok(Value::Boolean(this_boolean(realm, &this, "valueOf")?))
    });
}

#[cfg(test)]
mod tests;
