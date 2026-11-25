//! Native Math intrinsics. Numeric methods coerce in source order.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, Key, Property, Realm, Slot, Thrown, Value, ops};
use alloc::rc::Rc;
use core::cell::Cell;

type Method = fn(&mut Realm, Value, &[Value]) -> Completion;

pub(crate) fn install(realm: &mut Realm) {
    let math = realm.object();
    for (name, value) in [
        ("E", core::f64::consts::E),
        ("LN10", core::f64::consts::LN_10),
        ("LN2", core::f64::consts::LN_2),
        ("LOG10E", core::f64::consts::LOG10_E),
        ("LOG2E", core::f64::consts::LOG2_E),
        ("PI", core::f64::consts::PI),
        ("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", core::f64::consts::SQRT_2),
    ] {
        math.define(
            Key::name(name),
            Property {
                slot: Slot::Data {
                    value: Value::Number(value),
                    writable: false,
                },
                enumerable: false,
                configurable: false,
            },
        );
    }
    math.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("Math"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    for &(name, operation) in UNARY {
        realm.method(&math, name, 1, move |realm, _, args| {
            let number = argument(realm, args, 0)?;
            Ok(Value::Number(operation.apply(number)))
        });
    }
    for (name, length, method) in [
        ("atan2", 2, atan2 as Method),
        ("imul", 2, imul),
        ("pow", 2, pow),
        ("hypot", 2, hypot),
        ("min", 2, min),
        ("max", 2, max),
        ("sumPrecise", 1, crate::math_sum::sum_precise),
    ] {
        realm.method(&math, name, length, method);
    }
    let state = Rc::new(Cell::new(random_seed()));
    realm.method(&math, "random", 0, move |_, _, _| {
        let next = state.get().wrapping_add(0x9e3779b97f4a7c15);
        state.set(next);
        let mut word = next;
        word = (word ^ (word >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94d049bb133111eb);
        word ^= word >> 31;
        Ok(Value::Number((word >> 11) as f64 / 9007199254740992.0))
    });
    realm
        .global
        .define(Key::name("Math"), Property::hidden(Value::Object(math)));
}

fn random_seed() -> u64 {
    crate::platform::random()
}

#[derive(Clone, Copy)]
enum Unary {
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Cbrt,
    Ceil,
    Clz32,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Fround,
    F16round,
    Log,
    Log1p,
    Log10,
    Log2,
    Round,
    Sign,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Trunc,
}

const UNARY: &[(&str, Unary)] = &[
    ("abs", Unary::Abs),
    ("acos", Unary::Acos),
    ("acosh", Unary::Acosh),
    ("asin", Unary::Asin),
    ("asinh", Unary::Asinh),
    ("atan", Unary::Atan),
    ("atanh", Unary::Atanh),
    ("cbrt", Unary::Cbrt),
    ("ceil", Unary::Ceil),
    ("clz32", Unary::Clz32),
    ("cos", Unary::Cos),
    ("cosh", Unary::Cosh),
    ("exp", Unary::Exp),
    ("expm1", Unary::Expm1),
    ("floor", Unary::Floor),
    ("fround", Unary::Fround),
    ("f16round", Unary::F16round),
    ("log", Unary::Log),
    ("log1p", Unary::Log1p),
    ("log10", Unary::Log10),
    ("log2", Unary::Log2),
    ("round", Unary::Round),
    ("sign", Unary::Sign),
    ("sin", Unary::Sin),
    ("sinh", Unary::Sinh),
    ("sqrt", Unary::Sqrt),
    ("tan", Unary::Tan),
    ("tanh", Unary::Tanh),
    ("trunc", Unary::Trunc),
];

impl Unary {
    fn apply(self, x: f64) -> f64 {
        match self {
            Self::Abs => x.abs(),
            Self::Acos => crate::float::acos(x),
            Self::Acosh => crate::float::acosh(x),
            Self::Asin => crate::float::asin(x),
            Self::Asinh => crate::float::asinh(x),
            Self::Atan => crate::float::atan(x),
            Self::Atanh => crate::float::atanh(x),
            Self::Cbrt => crate::float::cbrt(x),
            Self::Ceil => crate::float::ceil(x),
            Self::Clz32 => ops::wrap32(x).leading_zeros() as f64,
            Self::Cos => crate::float::cos(x),
            Self::Cosh => crate::float::cosh(x),
            Self::Exp => crate::float::exp(x),
            Self::Expm1 => crate::float::expm1(x),
            Self::Floor => crate::float::floor(x),
            Self::Fround => (x as f32) as f64,
            Self::F16round => half(x),
            Self::Log => crate::float::log(x),
            Self::Log1p => crate::float::log1p(x),
            Self::Log10 => crate::float::log10(x),
            Self::Log2 => crate::float::log2(x),
            Self::Round => round(x),
            Self::Sign => {
                if x.is_nan() || x == 0.0 {
                    x
                } else {
                    x.signum()
                }
            }
            Self::Sin => crate::float::sin(x),
            Self::Sinh => crate::float::sinh(x),
            Self::Sqrt => crate::float::sqrt(x),
            Self::Tan => crate::float::tan(x),
            Self::Tanh => crate::float::tanh(x),
            Self::Trunc => crate::float::trunc(x),
        }
    }
}

fn argument(realm: &mut Realm, args: &[Value], index: usize) -> Result<f64, Thrown> {
    ops::to_number(realm, args.get(index).unwrap_or(&Value::Undefined))
}

fn atan2(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let y = argument(realm, args, 0)?;
    let x = argument(realm, args, 1)?;
    Ok(Value::Number(crate::float::atan2(y, x)))
}

fn imul(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let x = ops::to_uint32(realm, args.first().unwrap_or(&Value::Undefined))?;
    let y = ops::to_uint32(realm, args.get(1).unwrap_or(&Value::Undefined))?;
    Ok(Value::Number(x.wrapping_mul(y) as i32 as f64))
}

fn pow(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let x = argument(realm, args, 0)?;
    let y = argument(realm, args, 1)?;
    Ok(Value::Number(ops::power(x, y)))
}

fn min(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    extrema(realm, args, true)
}
fn max(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    extrema(realm, args, false)
}

fn extrema(realm: &mut Realm, args: &[Value], minimum: bool) -> Completion {
    let mut result = if minimum {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    };
    let mut nan = false;
    for value in args {
        let x = ops::to_number(realm, value)?;
        nan |= x.is_nan();
        if (minimum && x < result)
            || (!minimum && x > result)
            || (x == 0.0 && result == 0.0 && x.is_sign_negative() == minimum)
        {
            result = x;
        }
    }
    Ok(Value::Number(if nan { f64::NAN } else { result }))
}

fn hypot(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let mut values = Vec::new();
    for value in args {
        values.push(ops::to_number(realm, value)?.abs());
    }
    if values.iter().any(|value| value.is_infinite()) {
        return Ok(Value::Number(f64::INFINITY));
    }
    if values.iter().any(|value| value.is_nan()) {
        return Ok(Value::Number(f64::NAN));
    }
    let scale = values.iter().copied().fold(0.0, f64::max);
    if scale == 0.0 {
        return Ok(Value::Number(0.0));
    }
    let mut sum = 0.0;
    let mut correction = 0.0;
    for value in values {
        let scaled = value / scale;
        let term = scaled * scaled - correction;
        let next = sum + term;
        correction = (next - sum) - term;
        sum = next;
    }
    Ok(Value::Number(scale * crate::float::sqrt(sum)))
}

fn round(x: f64) -> f64 {
    if !x.is_finite() || crate::float::fract(x) == 0.0 {
        return x;
    }
    let floor = crate::float::floor(x);
    let nearest = if x - floor < 0.5 { floor } else { floor + 1.0 };
    if nearest == 0.0 {
        0.0f64.copysign(x)
    } else {
        nearest
    }
}

/// Direct binary64 → binary16 rounding avoids binary32 double rounding.
fn half(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let bits = x.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exponent < -25 {
        return 0.0f64.copysign(x);
    }
    if exponent > 15 {
        return f64::INFINITY.copysign(x);
    }
    let significand = (bits & ((1 << 52) - 1)) | (1 << 52);
    let shift = if exponent < -14 { 28 - exponent } else { 42 } as u32;
    let whole = significand >> shift;
    let remainder = significand & ((1 << shift) - 1);
    let midpoint = 1 << (shift - 1);
    let rounded =
        whole + u64::from(remainder > midpoint || (remainder == midpoint && whole & 1 == 1));
    if exponent == 15 && rounded == 2048 {
        return f64::INFINITY.copysign(x);
    }
    let power = if exponent < -14 { -24 } else { exponent - 10 };
    let scale = f64::from_bits(((power + 1023) as u64) << 52);
    (rounded as f64 * scale).copysign(x)
}

#[cfg(test)]
mod tests;
