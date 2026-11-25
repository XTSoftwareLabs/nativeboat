//! Arbitrary-precision integers for the language's BigInt values: a sign
//! and a little-endian magnitude of 32-bit digits without leading zeros.
//! Bitwise operators act on the infinite two's complement form.

#[allow(unused_imports)]
use crate::prelude::*;
use core::cmp::Ordering;

/// The most digits a BigInt may have: 2^30 bits, V8's limit. Unit tests
/// use a small limit so that size errors are cheap to reach.
#[cfg(not(test))]
const MAX_DIGITS: usize = 1 << 25;
#[cfg(test)]
const MAX_DIGITS: usize = 1 << 10;

/// An arbitrary-precision integer.
#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct BigInt {
    negative: bool,
    digits: Vec<u32>,
}

/// The value of a BigInt literal, given in decimal digits.
pub fn literal(decimal: &str) -> crate::Value {
    let units: Vec<u16> = decimal.encode_utf16().collect();
    let value = BigInt::parse(&units).expect("literals are decimal digits");
    crate::Value::BigInt(alloc::rc::Rc::new(value))
}

/// A result too large to represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

impl BigInt {
    fn new(negative: bool, mut digits: Vec<u32>) -> Result<BigInt, TooLarge> {
        while digits.last() == Some(&0) {
            digits.pop();
        }
        if digits.len() > MAX_DIGITS {
            return Err(TooLarge);
        }
        let negative = negative && !digits.is_empty();
        Ok(BigInt { negative, digits })
    }

    fn small(negative: bool, digits: Vec<u32>) -> BigInt {
        BigInt::new(negative, digits).expect("small values fit")
    }

    /// The integer `value`.
    pub fn from_i64(value: i64) -> BigInt {
        let magnitude = value.unsigned_abs();
        BigInt::small(value < 0, vec![magnitude as u32, (magnitude >> 32) as u32])
    }

    /// The unsigned integer `value`, without passing through floating point.
    pub fn from_u64(value: u64) -> BigInt {
        BigInt::small(false, vec![value as u32, (value >> 32) as u32])
    }

    /// The integer a finite integral double denotes exactly.
    pub fn from_integer(value: f64) -> BigInt {
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let fraction = bits & ((1 << 52) - 1);
        if exponent == 0 {
            return BigInt::default();
        }
        let mantissa = BigInt::small(
            false,
            vec![fraction as u32, ((fraction >> 32) as u32) | 1 << 20],
        );
        let shift = exponent - 1075;
        let magnitude = if shift >= 0 {
            mantissa.shift_left(shift as u64).expect("doubles fit")
        } else {
            mantissa.shift_right((-shift) as u64)
        };
        BigInt {
            negative: value < 0.0 && !magnitude.digits.is_empty(),
            ..magnitude
        }
    }

    /// The magnitude when it fits in 64 bits.
    pub fn magnitude_u64(&self) -> Option<u64> {
        (self.digits.len() <= 2).then(|| self.low_u64())
    }

    /// Whether this is zero.
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// Whether this is negative.
    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// The nearest double, rounding ties to even.
    pub fn to_f64(&self) -> f64 {
        let bits = self.bit_length();
        let magnitude = if bits <= 64 {
            self.low_u64() as f64
        } else {
            // The top 64 bits, with a sticky low bit when any bit below
            // them is set, round like the whole magnitude.
            let shift = bits - 64;
            let whole = self.magnitude();
            let top = whole.shift_right(shift);
            let exact = top.shift_left(shift).expect("no larger than the value");
            let sticky = u64::from(exact != whole);
            scale((top.low_u64() | sticky) as f64, shift)
        };
        if self.negative { -magnitude } else { magnitude }
    }

    fn low_u64(&self) -> u64 {
        let digit = |index: usize| u64::from(self.digits.get(index).copied().unwrap_or(0));
        digit(0) | digit(1) << 32
    }

    fn magnitude(&self) -> BigInt {
        BigInt {
            negative: false,
            digits: self.digits.clone(),
        }
    }

    /// The number of bits of the magnitude.
    fn bit_length(&self) -> u64 {
        match self.digits.last() {
            None => 0,
            Some(top) => (self.digits.len() as u64 - 1) * 32 + u64::from(32 - top.leading_zeros()),
        }
    }

    /// `-this`.
    pub fn negate(&self) -> BigInt {
        BigInt {
            negative: !self.negative && !self.is_zero(),
            digits: self.digits.clone(),
        }
    }

    /// Compares values.
    pub fn compare(&self, other: &BigInt) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => compare_digits(&self.digits, &other.digits),
            (true, true) => compare_digits(&other.digits, &self.digits),
        }
    }

    /// Compares with a double that is not NaN.
    pub fn compare_f64(&self, other: f64) -> Ordering {
        if other == f64::INFINITY {
            return Ordering::Less;
        }
        if other == f64::NEG_INFINITY {
            return Ordering::Greater;
        }
        let whole = other - crate::float::fract(other);
        let order = self.compare(&BigInt::from_integer(whole));
        if order != Ordering::Equal || whole == other {
            return order;
        }
        if other > whole {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    /// `this + other`.
    pub fn add(&self, other: &BigInt) -> Result<BigInt, TooLarge> {
        if self.negative == other.negative {
            return BigInt::new(self.negative, add_digits(&self.digits, &other.digits));
        }
        match compare_digits(&self.digits, &other.digits) {
            Ordering::Less => {
                BigInt::new(other.negative, subtract_digits(&other.digits, &self.digits))
            }
            _ => BigInt::new(self.negative, subtract_digits(&self.digits, &other.digits)),
        }
    }

    /// `this - other`.
    pub fn subtract(&self, other: &BigInt) -> Result<BigInt, TooLarge> {
        self.add(&other.negate())
    }

    /// `this * other`.
    pub fn multiply(&self, other: &BigInt) -> Result<BigInt, TooLarge> {
        if self.digits.len() + other.digits.len() > MAX_DIGITS + 1 {
            return Err(TooLarge);
        }
        let mut product = vec![0u32; self.digits.len() + other.digits.len()];
        for (i, left) in self.digits.iter().enumerate() {
            let mut carry = 0u64;
            for (j, right) in other.digits.iter().enumerate() {
                let sum = u64::from(product[i + j]) + u64::from(*left) * u64::from(*right) + carry;
                product[i + j] = sum as u32;
                carry = sum >> 32;
            }
            product[i + other.digits.len()] = carry as u32;
        }
        BigInt::new(self.negative != other.negative, product)
    }

    /// The quotient truncated toward zero and the remainder with this
    /// sign; `None` for a zero divisor.
    pub fn divide(&self, other: &BigInt) -> Option<(BigInt, BigInt)> {
        if other.is_zero() {
            return None;
        }
        let (quotient, remainder) = divide_digits(&self.digits, &other.digits);
        Some((
            BigInt::small(self.negative != other.negative, quotient),
            BigInt::small(self.negative, remainder),
        ))
    }

    /// `this ** exponent` for a non-negative exponent.
    pub fn power(&self, exponent: &BigInt) -> Result<BigInt, TooLarge> {
        if exponent.is_zero() {
            return Ok(BigInt::from_i64(1));
        }
        if self.is_zero() || self.digits == [1] {
            let negative = self.negative && exponent.digits[0] & 1 == 1;
            return Ok(BigInt {
                negative,
                digits: self.digits.clone(),
            });
        }
        if exponent.digits.len() > 1
            || u64::from(exponent.digits[0]) * self.bit_length() > (MAX_DIGITS as u64) * 32
        {
            return Err(TooLarge);
        }
        let mut result = BigInt::from_i64(1);
        let mut base = self.clone();
        let mut remaining = exponent.digits[0];
        loop {
            if remaining & 1 == 1 {
                result = result.multiply(&base)?;
            }
            remaining >>= 1;
            if remaining == 0 {
                return Ok(result);
            }
            base = base.multiply(&base)?;
        }
    }

    /// `this << bits`, for a non-negative count.
    pub fn shift_left(&self, bits: u64) -> Result<BigInt, TooLarge> {
        let words = (bits / 32) as usize;
        if self.is_zero() {
            return Ok(BigInt::default());
        }
        if words > MAX_DIGITS {
            return Err(TooLarge);
        }
        let offset = (bits % 32) as u32;
        let mut digits = vec![0u32; words];
        let mut carry = 0u32;
        for digit in &self.digits {
            if offset == 0 {
                digits.push(*digit);
            } else {
                digits.push(digit << offset | carry);
                carry = digit >> (32 - offset);
            }
        }
        digits.push(carry);
        BigInt::new(self.negative, digits)
    }

    /// `this >> bits` rounding toward negative infinity, for a
    /// non-negative count.
    pub fn shift_right(&self, bits: u64) -> BigInt {
        let words = (bits / 32) as usize;
        if words >= self.digits.len() {
            return if self.negative {
                BigInt::from_i64(-1)
            } else {
                BigInt::default()
            };
        }
        let offset = (bits % 32) as u32;
        let mut digits: Vec<u32> = Vec::new();
        for index in words..self.digits.len() {
            let low = self.digits[index] >> offset;
            let high = if offset == 0 {
                0
            } else {
                self.digits
                    .get(index + 1)
                    .map_or(0, |digit| digit << (32 - offset))
            };
            digits.push(low | high);
        }
        let shifted = BigInt::small(self.negative, digits);
        // A negative value rounds down when it loses any set bit.
        let lost = self.digits[..words].iter().any(|digit| *digit != 0)
            || (offset > 0 && self.digits[words] & ((1 << offset) - 1) != 0);
        if self.negative && lost {
            shifted
                .subtract(&BigInt::from_i64(1))
                .expect("smaller values fit")
        } else {
            shifted
        }
    }

    /// The two's complement digits over `length` digits.
    fn twos_complement(&self, length: usize) -> Vec<u32> {
        let mut digits = self.digits.clone();
        digits.resize(length, 0);
        if self.negative {
            let mut carry = 1u64;
            for digit in &mut digits {
                let sum = u64::from(!*digit) + carry;
                *digit = sum as u32;
                carry = sum >> 32;
            }
        }
        digits
    }

    fn from_twos_complement(mut digits: Vec<u32>) -> BigInt {
        let negative = digits.last().is_some_and(|top| top >> 31 == 1);
        if negative {
            let mut carry = 1u64;
            for digit in &mut digits {
                let sum = u64::from(!*digit) + carry;
                *digit = sum as u32;
                carry = sum >> 32;
            }
        }
        BigInt::small(negative, digits)
    }

    /// A bitwise operator on two's complement forms.
    pub fn bitwise(&self, other: &BigInt, operation: fn(u32, u32) -> u32) -> BigInt {
        let length = self.digits.len().max(other.digits.len()) + 1;
        let left = self.twos_complement(length);
        let right = other.twos_complement(length);
        BigInt::from_twos_complement(
            left.iter()
                .zip(&right)
                .map(|(a, b)| operation(*a, *b))
                .collect(),
        )
    }

    /// `~this`, which is `-this - 1`.
    pub fn not(&self) -> BigInt {
        self.negate()
            .subtract(&BigInt::from_i64(1))
            .expect("one more digit at most")
    }

    /// `BigInt.asUintN(bits, this)`.
    pub fn as_uint_n(&self, bits: u64) -> Result<BigInt, TooLarge> {
        if bits == 0 {
            return Ok(BigInt::default());
        }
        let words = bits.div_ceil(32);
        if words > MAX_DIGITS as u64 {
            return if self.negative {
                Err(TooLarge)
            } else {
                Ok(self.clone())
            };
        }
        let mut digits = self.twos_complement((words as usize).max(self.digits.len() + 1));
        digits.truncate(words as usize);
        if !bits.is_multiple_of(32) {
            let last = digits.len() - 1;
            digits[last] &= (1 << (bits % 32)) - 1;
        }
        BigInt::new(false, digits)
    }

    /// `BigInt.asIntN(bits, this)`.
    pub fn as_int_n(&self, bits: u64) -> Result<BigInt, TooLarge> {
        if bits == 0 {
            return Ok(BigInt::default());
        }
        let unsigned = self.as_uint_n(bits)?;
        if unsigned.bit_length() < bits {
            return Ok(unsigned);
        }
        unsigned.subtract(&BigInt::from_i64(1).shift_left(bits)?)
    }

    /// The digits in a radix from 2 to 36, lowercase, with a `-` sign.
    pub fn to_radix(&self, radix: u32) -> String {
        if self.is_zero() {
            return "0".to_owned();
        }
        let mut digits = self.digits.clone();
        let mut text = Vec::new();
        while !digits.is_empty() {
            let mut remainder = 0u64;
            for digit in digits.iter_mut().rev() {
                let value = remainder << 32 | u64::from(*digit);
                *digit = (value / u64::from(radix)) as u32;
                remainder = value % u64::from(radix);
            }
            while digits.last() == Some(&0) {
                digits.pop();
            }
            text.push(char::from_digit(remainder as u32, radix).expect("radix digit"));
        }
        if self.negative {
            text.push('-');
        }
        text.iter().rev().collect()
    }

    /// `StringToBigInt`: whitespace-trimmed decimal with an optional sign,
    /// or `0x`, `0o` and `0b` digits; empty text is zero.
    pub fn parse(text: &[u16]) -> Option<BigInt> {
        let start = text
            .iter()
            .position(|unit| !crate::string::is_space(*unit))
            .unwrap_or(text.len());
        let end = text
            .iter()
            .rposition(|unit| !crate::string::is_space(*unit))
            .map_or(start, |end| end + 1);
        let text = String::from_utf16(&text[start..end]).ok()?;
        let lower = text.to_ascii_lowercase();
        let (negative, radix, digits) = if let Some(digits) = lower.strip_prefix("0x") {
            (false, 16, digits)
        } else if let Some(digits) = lower.strip_prefix("0o") {
            (false, 8, digits)
        } else if let Some(digits) = lower.strip_prefix("0b") {
            (false, 2, digits)
        } else if let Some(digits) = lower.strip_prefix('-') {
            (true, 10, digits)
        } else {
            (false, 10, lower.strip_prefix('+').unwrap_or(&lower))
        };
        if text.is_empty() {
            return Some(BigInt::default());
        }
        if digits.is_empty() {
            return None;
        }
        let mut value: Vec<u32> = Vec::new();
        for character in digits.chars() {
            let digit = character.to_digit(radix)?;
            let mut carry = u64::from(digit);
            for word in &mut value {
                let product = u64::from(*word) * u64::from(radix) + carry;
                *word = product as u32;
                carry = product >> 32;
            }
            if carry > 0 {
                value.push(carry as u32);
            }
        }
        BigInt::new(negative, value).ok()
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &BigInt) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &BigInt) -> Ordering {
        self.compare(other)
    }
}

/// `magnitude * 2^shift` as a double, infinite when too large.
fn scale(mut magnitude: f64, mut shift: u64) -> f64 {
    while shift > 0 && magnitude.is_finite() {
        let step = shift.min(1000);
        magnitude *= f64::from_bits((1023 + step) << 52);
        shift -= step;
    }
    magnitude
}

fn compare_digits(left: &[u32], right: &[u32]) -> Ordering {
    left.len()
        .cmp(&right.len())
        .then_with(|| left.iter().rev().cmp(right.iter().rev()))
}

fn add_digits(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut sum = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut carry = 0u64;
    for index in 0..left.len().max(right.len()) {
        let value = u64::from(left.get(index).copied().unwrap_or(0))
            + u64::from(right.get(index).copied().unwrap_or(0))
            + carry;
        sum.push(value as u32);
        carry = value >> 32;
    }
    sum.push(carry as u32);
    sum
}

/// `left - right` for `left >= right`.
fn subtract_digits(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut difference = Vec::with_capacity(left.len());
    let mut borrow = 0i64;
    for (index, digit) in left.iter().enumerate() {
        let value = i64::from(*digit) - i64::from(right.get(index).copied().unwrap_or(0)) - borrow;
        borrow = i64::from(value < 0);
        difference.push((value + (borrow << 32)) as u32);
    }
    difference
}

/// Long division of magnitudes (Knuth's algorithm D).
fn divide_digits(dividend: &[u32], divisor: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if compare_digits(dividend, divisor) == Ordering::Less {
        return (Vec::new(), dividend.to_vec());
    }
    if divisor.len() == 1 {
        let divisor = u64::from(divisor[0]);
        let mut quotient = vec![0u32; dividend.len()];
        let mut remainder = 0u64;
        for index in (0..dividend.len()).rev() {
            let value = remainder << 32 | u64::from(dividend[index]);
            quotient[index] = (value / divisor) as u32;
            remainder = value % divisor;
        }
        return (quotient, vec![remainder as u32]);
    }
    // Normalize so the divisor's top digit has its high bit set.
    let shift = divisor.last().expect("non-empty").leading_zeros();
    let normalize = |digits: &[u32], extra: usize| {
        let mut shifted = vec![0u32; digits.len() + extra];
        for (index, digit) in digits.iter().enumerate() {
            shifted[index] |= digit << shift;
            if shift > 0 {
                shifted[index + 1] |= digit >> (32 - shift);
            }
        }
        shifted
    };
    let v = normalize(divisor, 1);
    let v = &v[..divisor.len()];
    let mut u = normalize(dividend, 1);
    let n = v.len();
    let m = dividend.len() - n;
    let mut quotient = vec![0u32; m + 1];
    let top = u64::from(v[n - 1]);
    let next = u64::from(v[n - 2]);
    for j in (0..=m).rev() {
        let numerator = u64::from(u[j + n]) << 32 | u64::from(u[j + n - 1]);
        let mut estimate = numerator / top;
        let mut rest = numerator % top;
        while estimate >= 1 << 32 || estimate * next > (rest << 32 | u64::from(u[j + n - 2])) {
            estimate -= 1;
            rest += top;
            if rest >= 1 << 32 {
                break;
            }
        }
        // Multiply and subtract.
        let mut borrow = 0i64;
        let mut carry = 0u64;
        for i in 0..n {
            let product = estimate * u64::from(v[i]) + carry;
            carry = product >> 32;
            let value = i64::from(u[i + j]) - (product & 0xffff_ffff) as i64 - borrow;
            u[i + j] = value as u32;
            borrow = i64::from(value < 0);
        }
        let value = i64::from(u[j + n]) - carry as i64 - borrow;
        u[j + n] = value as u32;
        if value < 0 {
            // The estimate was one too large: add the divisor back.
            estimate -= 1;
            let mut carry = 0u64;
            for i in 0..n {
                let sum = u64::from(u[i + j]) + u64::from(v[i]) + carry;
                u[i + j] = sum as u32;
                carry = sum >> 32;
            }
            u[j + n] = u[j + n].wrapping_add(carry as u32);
        }
        quotient[j] = estimate as u32;
    }
    // Unnormalize the remainder.
    let mut remainder = vec![0u32; n];
    for index in 0..n {
        remainder[index] = u[index] >> shift;
        if shift > 0 {
            remainder[index] |= u[index + 1] << (32 - shift);
        }
    }
    (quotient, remainder)
}

#[cfg(test)]
mod tests;
