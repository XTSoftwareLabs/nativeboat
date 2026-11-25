//! Floating-point operations shared by standard and engine-free CSS builds.

pub(crate) fn round(value: f32) -> f32 {
    libm::roundf(value)
}

pub(crate) fn floor(value: f32) -> f32 {
    libm::floorf(value)
}

pub(crate) fn pow(value: f32, exponent: f32) -> f32 {
    libm::powf(value, exponent)
}

pub(crate) fn cbrt(value: f32) -> f32 {
    libm::cbrtf(value)
}

pub(crate) fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}

pub(crate) fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

pub(crate) fn sin(value: f32) -> f32 {
    libm::sinf(value)
}

pub(crate) fn cos(value: f32) -> f32 {
    libm::cosf(value)
}

pub(crate) fn fract(value: f64) -> f64 {
    value - libm::trunc(value)
}

pub(crate) fn rem_euclid(value: f32, divisor: f32) -> f32 {
    let remainder = libm::fmodf(value, divisor);
    if remainder < 0.0 {
        if divisor < 0.0 {
            remainder - divisor
        } else {
            remainder + divisor
        }
    } else {
        remainder
    }
}
