//! Exact integer digits, rounded once to binary64 without a big-integer dependency.

#[allow(unused_imports)]
use crate::prelude::*;

pub(crate) fn digit(unit: u16, radix: u32) -> Option<u32> {
    let value = match unit {
        0x30..=0x39 => u32::from(unit - 0x30),
        0x41..=0x5a => u32::from(unit - 0x41) + 10,
        0x61..=0x7a => u32::from(unit - 0x61) + 10,
        _ => return None,
    };
    (value < radix).then_some(value)
}

pub(crate) fn integer(text: &[u16], radix: u32) -> Option<f64> {
    if text.is_empty() {
        return None;
    }
    // Once an unsigned integer exceeds 1024 bits it necessarily rounds to
    // infinity. Keep validating all remaining digits without growing storage.
    let mut words = vec![0u32];
    let mut overflow = false;
    for unit in text {
        let value = digit(*unit, radix)?;
        if !overflow {
            let mut carry = u64::from(value);
            for word in &mut words {
                let product = u64::from(*word) * u64::from(radix) + carry;
                *word = product as u32;
                carry = product >> 32;
            }
            if carry != 0 {
                words.push(carry as u32);
            }
            overflow = words.len() > 32;
        }
    }
    if overflow {
        return Some(f64::INFINITY);
    }
    let top = *words.last().expect("integer storage is nonempty");
    if top == 0 {
        return Some(0.0);
    }
    let mut highest = words.len() * 32 - top.leading_zeros() as usize - 1;
    let bit = |index: usize| (words[index / 32] >> (index % 32)) & 1;
    let shift = highest.saturating_sub(52);
    let mut mantissa = 0u64;
    for index in (shift..=highest).rev() {
        mantissa = (mantissa << 1) | u64::from(bit(index));
    }
    if shift > 0
        && bit(shift - 1) != 0
        && (mantissa & 1 != 0 || (0..shift - 1).any(|index| bit(index) != 0))
    {
        mantissa += 1;
        if mantissa == 1 << 53 {
            mantissa >>= 1;
            highest += 1;
        }
    }
    if highest > 1023 {
        return Some(f64::INFINITY);
    }
    if highest < 52 {
        mantissa <<= 52 - highest;
    }
    Some(f64::from_bits(
        ((highest as u64 + 1023) << 52) | (mantissa & ((1 << 52) - 1)),
    ))
}

#[cfg(test)]
mod tests;
