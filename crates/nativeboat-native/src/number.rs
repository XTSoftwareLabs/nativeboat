//! Number ↔ string conversions with the source language's exact rules:
//! shortest round-trip digits and its exponent thresholds when formatting,
//! and its strict numeric-string grammar when parsing.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::string::{JsStr, is_space};

/// `Number::toString(x)` in radix 10.
pub fn to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == 0.0 {
        // Both zeros print as "0".
        return "0".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    // Rust's exponential form uses the shortest digits that round-trip.
    let exponential = format!("{:e}", value.abs());
    let (mantissa, exponent) = exponential
        .split_once('e')
        .expect("exponential formatting has an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().expect("integer exponent") + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let sign = if n - 1 < 0 { '-' } else { '+' };
        let tail = if k == 1 {
            String::new()
        } else {
            format!(".{}", &digits[1..])
        };
        format!("{}{tail}e{sign}{}", &digits[..1], (n - 1).abs())
    };
    format!("{sign}{body}")
}

fn is_decimal(text: &[u16]) -> bool {
    let digit = |unit: &u16| (u16::from(b'0')..=u16::from(b'9')).contains(unit);
    let mut index = 0;
    let run = |index: &mut usize| {
        let start = *index;
        while text.get(*index).is_some_and(digit) {
            *index += 1;
        }
        *index - start
    };
    if matches!(text.first(), Some(&0x2B | &0x2D)) {
        index += 1;
    }
    let whole = run(&mut index);
    let mut fraction = 0;
    if text.get(index) == Some(&u16::from(b'.')) {
        index += 1;
        fraction = run(&mut index);
    }
    if whole + fraction == 0 {
        return false;
    }
    if matches!(text.get(index), Some(&0x65 | &0x45)) {
        index += 1;
        if matches!(text.get(index), Some(&0x2B | &0x2D)) {
            index += 1;
        }
        if run(&mut index) == 0 {
            return false;
        }
    }
    index == text.len()
}

/// `StringToNumber`: NaN for anything outside the numeric string grammar.
pub fn parse(text: &JsStr) -> f64 {
    let units = text.units();
    let start = units.iter().position(|unit| !is_space(*unit));
    let Some(start) = start else {
        return 0.0;
    };
    let end = units
        .iter()
        .rposition(|unit| !is_space(*unit))
        .unwrap_or(start)
        + 1;
    let trimmed = &units[start..end];
    let ascii: String = String::from_utf16_lossy(trimmed);
    match ascii.as_str() {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if trimmed.len() > 2 && trimmed[0] == u16::from(b'0') {
        let radix = match trimmed[1] {
            0x78 | 0x58 => Some(16),
            0x6F | 0x4F => Some(8),
            0x62 | 0x42 => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            return crate::radix::integer(&trimmed[2..], radix).unwrap_or(f64::NAN);
        }
    }
    if !is_decimal(trimmed) {
        return f64::NAN;
    }
    // The grammar was validated above; Rust's parser rounds correctly.
    ascii.parse::<f64>().expect("validated decimal literal")
}

#[cfg(test)]
mod tests;
