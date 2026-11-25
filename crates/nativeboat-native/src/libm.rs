//! IEEE 754 double-precision math functions, in place of a C runtime's.
//!
//! The transcendental functions are vendored from the `libm` crate 0.2.16, a
//! Rust port of musl and FreeBSD msun, which derive from Sun's fdlibm. V8 also
//! implements `Math` with fdlibm, so compiled applications get the results
//! Electron applications do. The rounding, remainder, scaling, cube-root and
//! square-root functions are written here for the x86-64 target.
//!
//! The `libm` crate is available under the MIT license:
//!
//! > Permission is hereby granted, free of charge, to any person obtaining a
//! > copy of this software and associated documentation files (the
//! > "Software"), to deal in the Software without restriction, including
//! > without limitation the rights to use, copy, modify, merge, publish,
//! > distribute, sublicense, and/or sell copies of the Software, and to
//! > permit persons to whom the Software is furnished to do so, subject to
//! > the following conditions:
//! >
//! > The above copyright notice and this permission notice shall be included
//! > in all copies or substantial portions of the Software.
//! >
//! > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! > OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! > MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
//! > IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
//! > CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
//! > TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
//! > SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! The Sun Microsystems and FreeBSD notices each vendored file carries are
//! preserved there. musl's `cbrt` (MIT, derived from FreeBSD's `s_cbrt.c`,
//! Copyright (C) 1993 Sun Microsystems, freely distributable with its notice
//! preserved) is the origin of `cbrt` below.

#![allow(
    clippy::all,
    clippy::pedantic,
    unused_parens,
    unused_assignments,
    non_upper_case_globals
)]

/// Evaluates an expression whose floating-point side effects (exception
/// flags) the original code relies on.
macro_rules! force_eval {
    ($e:expr) => {
        core::hint::black_box($e)
    };
}

/// Checked array indexing in the vendored code's notation.
macro_rules! i {
    ($array:expr, $index:expr) => {
        $array[$index]
    };
    ($array:expr, $index:expr, = , $rhs:expr) => {
        $array[$index] = $rhs
    };
    ($array:expr, $index:expr, += , $rhs:expr) => {
        $array[$index] += $rhs
    };
    ($array:expr, $index:expr, -= , $rhs:expr) => {
        $array[$index] -= $rhs
    };
    ($array:expr, $index:expr, &= , $rhs:expr) => {
        $array[$index] &= $rhs
    };
    ($array:expr, $index:expr, == , $rhs:expr) => {
        $array[$index] == $rhs
    };
}

macro_rules! div {
    ($a:expr, $b:expr) => {
        $a / $b
    };
}

mod acos;
mod acosh;
mod asin;
mod asinh;
mod atan;
mod atan2;
mod atanh;
mod cos;
mod cosh;
mod exp;
mod expm1;
mod expo2;
mod k_cos;
mod k_expo2;
mod k_sin;
mod k_tan;
mod log;
mod log10;
mod log1p;
mod log2;
mod pow;
mod rem_pio2;
mod rem_pio2_large;
mod sin;
mod sinh;
mod tan;
mod tanh;

pub use acos::acos;
pub use acosh::acosh;
pub use asin::asin;
pub use asinh::asinh;
pub use atan::atan;
pub use atan2::atan2;
pub use atanh::atanh;
pub use cos::cos;
pub use cosh::cosh;
pub use exp::exp;
pub use expm1::expm1;
use expo2::expo2;
use k_cos::k_cos;
use k_expo2::k_expo2;
use k_sin::k_sin;
use k_tan::k_tan;
pub use log::log;
pub use log1p::log1p;
pub use log2::log2;
pub use log10::log10;
pub use pow::pow;
use rem_pio2::rem_pio2;
use rem_pio2_large::rem_pio2_large;
pub use sin::sin;
pub use sinh::sinh;
pub use tan::tan;
pub use tanh::tanh;

const SIGN: u64 = 1 << 63;
const EXPONENT: u64 = 0x7ff << 52;
const IMPLICIT: u64 = 1 << 52;

fn get_high_word(x: f64) -> u32 {
    (x.to_bits() >> 32) as u32
}

fn get_low_word(x: f64) -> u32 {
    x.to_bits() as u32
}

fn with_set_high_word(f: f64, hi: u32) -> f64 {
    f64::from_bits((f.to_bits() & 0x0000_0000_ffff_ffff) | ((hi as u64) << 32))
}

fn with_set_low_word(f: f64, lo: u32) -> f64 {
    f64::from_bits((f.to_bits() & 0xffff_ffff_0000_0000) | lo as u64)
}

fn combine_words(hi: u32, lo: u32) -> f64 {
    f64::from_bits(((hi as u64) << 32) | lo as u64)
}

fn fabs(x: f64) -> f64 {
    x.abs()
}

/// The correctly rounded square root (SSE2 `sqrtsd`).
pub fn sqrt(x: f64) -> f64 {
    use core::arch::x86_64::{_mm_cvtsd_f64, _mm_set_sd, _mm_sqrt_pd};
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { _mm_cvtsd_f64(_mm_sqrt_pd(_mm_set_sd(x))) }
}

/// Rounds toward zero.
pub fn trunc(x: f64) -> f64 {
    let bits = x.to_bits();
    let exponent = ((bits & EXPONENT) >> 52) as i32 - 1023;
    if exponent >= 52 {
        // Integral, infinite or NaN.
        return x;
    }
    if exponent < 0 {
        return f64::from_bits(bits & SIGN);
    }
    f64::from_bits(bits & !((IMPLICIT - 1) >> exponent))
}

/// Rounds toward negative infinity.
pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t > x { t - 1.0 } else { t }
}

/// Rounds toward positive infinity.
pub fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if t < x { t + 1.0 } else { t }
}

/// `x * 2^n`, exactly unless the result overflows or underflows.
pub fn scalbn(x: f64, mut n: i32) -> f64 {
    let x1p1023 = f64::from_bits(0x7fe0_0000_0000_0000);
    let x1p53 = f64::from_bits(0x4340_0000_0000_0000);
    let x1p_1022 = f64::from_bits(0x0010_0000_0000_0000);
    let mut y = x;
    if n > 1023 {
        y *= x1p1023;
        n -= 1023;
        if n > 1023 {
            y *= x1p1023;
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        // A final scale below 2^-53 avoids double rounding of subnormals.
        y *= x1p_1022 * x1p53;
        n += 1022 - 53;
        if n < -1022 {
            y *= x1p_1022 * x1p53;
            n += 1022 - 53;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    y * f64::from_bits(((0x3ff + n) as u64) << 52)
}

/// The exact remainder of `x / y` with the sign of `x` (C `fmodf`), which
/// the `%` operator on `f32` calls, as in the packaged layout libraries. The
/// exact remainder of two `f32` values is an `f32`, so computing it exactly in
/// `f64` and narrowing loses nothing.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn fmodf(x: f32, y: f32) -> f32 {
    fmod(f64::from(x), f64::from(y)) as f32
}

/// The exact remainder of `x / y` with the sign of `x` (C `fmod`). Native
/// builds export it under its C name, which the `%` operator on `f64` calls.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn fmod(x: f64, y: f64) -> f64 {
    let sign = x.to_bits() & SIGN;
    let ux = x.to_bits() & !SIGN;
    let uy = y.to_bits() & !SIGN;
    // NaN or infinite `x`, and NaN or zero `y`, give NaN.
    if ux & EXPONENT == EXPONENT || uy.wrapping_sub(1) & EXPONENT == EXPONENT {
        return (x * y) / (x * y);
    }
    if ux < uy {
        return x;
    }
    // Significands with the implicit bit and exponents above the least.
    let split = |bits: u64| {
        let above = bits.saturating_sub(IMPLICIT);
        (bits - (above & EXPONENT), (above >> 52) as u32)
    };
    let (num, ex) = split(ux);
    let (div, ey) = split(uy);
    // `(num << (ex - ey)) % div`, shifting at most 64 bits at a time.
    let mut remainder = num % div;
    let mut shift = ex - ey;
    while shift > 0 {
        let step = shift.min(64);
        remainder = ((u128::from(remainder) << step) % u128::from(div)) as u64;
        shift -= step;
    }
    if remainder == 0 {
        return f64::from_bits(sign);
    }
    let up = ey.min(52 - remainder.ilog2());
    let bits = (remainder << up) + (u64::from(ey - up) << 52);
    f64::from_bits(sign | bits)
}

/// The cube root.
pub fn cbrt(x: f64) -> f64 {
    const B1: u32 = 715_094_163; // (1023-1023/3-0.03306235651)*2**20
    const B2: u32 = 696_219_795; // (1023-1023/3-54/3-0.03306235651)*2**20
    // |1/cbrt(x) - p(x)| < 2**-23.5 (~[-7.93e-8, 7.929e-8]).
    const P0: f64 = 1.87595182427177009643; // 0x3ffe03e6, 0x0f61e692
    const P1: f64 = -1.88497979543377169875; // 0xbffe28e0, 0x92f02420
    const P2: f64 = 1.621429720105354466140; // 0x3ff9f160, 0x4a49d6c2
    const P3: f64 = -0.758397934778766047437; // 0xbfe844cb, 0xbee751d9
    const P4: f64 = 0.145996192886612446982; // 0x3fc2b000, 0xd4e4edd7
    let x1p54 = f64::from_bits(0x4350_0000_0000_0000);
    let mut bits = x.to_bits();
    let mut hx = (bits >> 32) as u32 & 0x7fff_ffff;
    if hx >= 0x7ff0_0000 {
        // NaN and infinities are their own cube roots.
        return x + x;
    }
    if hx < 0x0010_0000 {
        // Zero or subnormal.
        bits = (x * x1p54).to_bits();
        hx = (bits >> 32) as u32 & 0x7fff_ffff;
        if hx == 0 {
            return x;
        }
        hx = hx / 3 + B2;
    } else {
        hx = hx / 3 + B1;
    }
    bits &= SIGN;
    bits |= u64::from(hx) << 32;
    let mut t = f64::from_bits(bits);
    // A polynomial approximation of 1/cbrt(r) to 23 bits, then rounding to
    // 32 bits so that t*t is exact, then one Newton step to 53 bits.
    let r = (t * t) * (t / x);
    t *= (P0 + r * (P1 + r * P2)) + ((r * r) * r) * (P3 + r * P4);
    t = f64::from_bits((t.to_bits() + 0x8000_0000) & 0xffff_ffff_c000_0000);
    let s = t * t;
    let r = x / s;
    let w = t + t;
    let r = (r - t) / (w + r);
    t + t * r
}

#[cfg(test)]
mod tests;
