//! Floating-point functions. The runtime links no Rust `std`, which owns the
//! `f64` methods for these, and no C runtime: they come from the runtime's
//! own `libm` module.

pub use crate::libm::{
    acos, acosh, asin, asinh, atan, atan2, atanh, cbrt, ceil, cos, cosh, exp, expm1, floor, log,
    log1p, log2, log10, pow, sin, sinh, sqrt, tan, tanh, trunc,
};

/// The fractional part, `x - trunc(x)`.
pub fn fract(x: f64) -> f64 {
    x - trunc(x)
}

/// The least nonnegative remainder of `x` divided by `y`.
pub fn rem_euclid(x: f64, y: f64) -> f64 {
    let remainder = crate::libm::fmod(x, y);
    if remainder < 0.0 {
        remainder + y.abs()
    } else {
        remainder
    }
}

#[cfg(test)]
mod tests;
