//! Native numeric global functions used by compiled dependency behavior.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::string::is_space;
use crate::{Completion, Realm, Value, ops, radix};

type Method = fn(&mut Realm, Value, &[Value]) -> Completion;

pub(crate) fn install(realm: &Realm) {
    for (name, length, code) in [
        ("parseInt", 2, parse_int as Method),
        ("parseFloat", 1, parse_float),
        ("isNaN", 1, is_nan),
        ("isFinite", 1, is_finite),
    ] {
        realm.method(&realm.global, name, length, code);
    }
}

fn is_nan(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let number = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
    Ok(Value::Boolean(number.is_nan()))
}

fn is_finite(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let number = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
    Ok(Value::Boolean(number.is_finite()))
}

pub(crate) fn parse_int(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    // Conversion order is observable even for empty strings or invalid radices.
    let text = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;
    let mut radix = ops::to_int32(realm, args.get(1).unwrap_or(&Value::Undefined))?;
    if radix != 0 && !(2..=36).contains(&radix) {
        return Ok(Value::Number(f64::NAN));
    }
    let units = text.units();
    let start = units
        .iter()
        .position(|unit| !is_space(*unit))
        .unwrap_or(units.len());
    let mut text = &units[start..];
    let negative = text.first() == Some(&0x2d);
    if matches!(text.first(), Some(&0x2b | &0x2d)) {
        text = &text[1..];
    }
    if (radix == 0 || radix == 16)
        && text.first() == Some(&0x30)
        && matches!(text.get(1), Some(&0x78 | &0x58))
    {
        text = &text[2..];
        radix = 16;
    }
    if radix == 0 {
        radix = 10;
    }
    let end = text
        .iter()
        .position(|unit| radix::digit(*unit, radix as u32).is_none())
        .unwrap_or(text.len());
    let number = radix::integer(&text[..end], radix as u32).unwrap_or(f64::NAN);
    Ok(Value::Number(if negative { -number } else { number }))
}

pub(crate) fn parse_float(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let text = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;
    let units = text.units();
    let start = units
        .iter()
        .position(|unit| !is_space(*unit))
        .unwrap_or(units.len());
    let text = &units[start..];
    let mut index = usize::from(matches!(text.first(), Some(&0x2b | &0x2d)));
    if text[index..].starts_with(&[0x49, 0x6e, 0x66, 0x69, 0x6e, 0x69, 0x74, 0x79]) {
        return Ok(Value::Number(if text.first() == Some(&0x2d) {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }));
    }
    let run = |index: &mut usize| {
        let start = *index;
        while text
            .get(*index)
            .is_some_and(|unit| (0x30..=0x39).contains(unit))
        {
            *index += 1;
        }
        *index - start
    };
    let whole = run(&mut index);
    let fraction = if text.get(index) == Some(&0x2e) {
        index += 1;
        run(&mut index)
    } else {
        0
    };
    if whole + fraction == 0 {
        return Ok(Value::Number(f64::NAN));
    }
    let mut end = index;
    if matches!(text.get(index), Some(&0x65 | &0x45)) {
        index += 1;
        if matches!(text.get(index), Some(&0x2b | &0x2d)) {
            index += 1;
        }
        if run(&mut index) != 0 {
            end = index;
        }
    }
    // The longest valid ASCII decimal prefix has already been selected.
    let prefix = String::from_utf16_lossy(&text[..end]);
    Ok(Value::Number(
        prefix.parse::<f64>().expect("validated decimal prefix"),
    ))
}

#[cfg(test)]
mod tests;
