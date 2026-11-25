//! Exact binary64 summation with signed integer bins, rounded once at the end.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Completion, ErrorKind, Realm, Thrown, Value, iter};

pub(crate) fn sum_precise(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let mut iterator = iter::iterate(realm, args.first().unwrap_or(&Value::Undefined))?;
    let mut sum = Sum::new();
    while let Some(value) = iterator.step(realm)? {
        if let Err(thrown) = sum.add(realm, &value) {
            return Err(iterator.close_thrown(realm, thrown));
        }
    }
    Ok(Value::Number(sum.finish()))
}

struct Sum {
    bins: Vec<i128>,
    count: u64,
    negative_zeros: bool,
    nan: bool,
    positive_infinity: bool,
    negative_infinity: bool,
}

impl Sum {
    fn new() -> Self {
        // At most 2^53-1 inputs contribute 53-bit mantissas. Each i128 bin
        // fits their exact sum; the remaining bits hold all propagated carries.
        Self {
            bins: vec![0; 2208],
            count: 0,
            negative_zeros: true,
            nan: false,
            positive_infinity: false,
            negative_infinity: false,
        }
    }

    fn add(&mut self, realm: &Realm, value: &Value) -> Result<(), Thrown> {
        if self.count >= (1 << 53) - 1 {
            return Err(realm.throw(ErrorKind::Range, "Too many summation items"));
        }
        let Value::Number(number) = value else {
            return Err(realm.throw(ErrorKind::Type, "Math.sumPrecise requires numbers"));
        };
        self.count += 1;
        self.negative_zeros &= number.to_bits() == 1 << 63;
        if number.is_nan() {
            self.nan = true;
        } else if number.is_infinite() {
            if number.is_sign_positive() {
                self.positive_infinity = true;
            } else {
                self.negative_infinity = true;
            }
        } else {
            let bits = number.to_bits();
            let exponent = ((bits >> 52) & 0x7ff) as usize;
            let fraction = bits & ((1 << 52) - 1);
            let mantissa = if exponent == 0 {
                fraction
            } else {
                fraction | (1 << 52)
            };
            let signed = if number.is_sign_negative() {
                -(mantissa as i128)
            } else {
                mantissa as i128
            };
            self.bins[exponent.saturating_sub(1)] += signed;
        }
        Ok(())
    }

    fn finish(mut self) -> f64 {
        if self.nan || (self.positive_infinity && self.negative_infinity) {
            return f64::NAN;
        }
        if self.positive_infinity {
            return f64::INFINITY;
        }
        if self.negative_infinity {
            return f64::NEG_INFINITY;
        }
        if self.negative_zeros {
            return -0.0;
        }
        let last = self.bins.len() - 1;
        for index in 0..last {
            let digit = self.bins[index].rem_euclid(2);
            let carry = (self.bins[index] - digit) / 2;
            self.bins[index] = digit;
            self.bins[index + 1] += carry;
        }
        let negative = self.bins[last] < 0;
        self.bins[last] = i128::from(negative);
        if negative {
            let mut carry = 1;
            for digit in &mut self.bins {
                let value = 1 - *digit + carry;
                *digit = value & 1;
                carry = value >> 1;
            }
        }
        let Some(mut highest) = self.bins.iter().rposition(|digit| *digit != 0) else {
            return 0.0;
        };
        let shift = highest.saturating_sub(52);
        let mut mantissa = 0u64;
        for index in (shift..=highest).rev() {
            mantissa = (mantissa << 1) | self.bins[index] as u64;
        }
        if shift > 0
            && self.bins[shift - 1] != 0
            && (mantissa & 1 == 1 || self.bins[..shift - 1].iter().any(|digit| *digit != 0))
        {
            mantissa += 1;
            if mantissa == 1 << 53 {
                mantissa >>= 1;
                highest += 1;
            }
        }
        let sign = u64::from(negative) << 63;
        let exponent = if highest < 52 { 0 } else { highest - 51 };
        if exponent >= 2047 {
            return f64::from_bits(sign | (0x7ff << 52));
        }
        f64::from_bits(sign | ((exponent as u64) << 52) | (mantissa & ((1 << 52) - 1)))
    }
}

#[cfg(test)]
mod tests;
