//! Floating-point operations used by layout and SVG geometry.

pub(crate) fn pow(value: f32, exponent: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.powf(exponent)
    }
    #[cfg(not(feature = "std"))]
    libm::powf(value, exponent)
}

pub(crate) fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.sqrt()
    }
    #[cfg(not(feature = "std"))]
    libm::sqrtf(value)
}

pub(crate) fn sin_cos(value: f32) -> (f32, f32) {
    #[cfg(feature = "std")]
    {
        value.sin_cos()
    }
    #[cfg(not(feature = "std"))]
    (libm::sinf(value), libm::cosf(value))
}

pub(crate) fn acos(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.acos()
    }
    #[cfg(not(feature = "std"))]
    libm::acosf(value)
}

pub(crate) fn tan(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.tan()
    }
    #[cfg(not(feature = "std"))]
    libm::tanf(value)
}

pub(crate) fn ceil(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.ceil()
    }
    #[cfg(not(feature = "std"))]
    libm::ceilf(value)
}

pub(crate) fn atan2(y: f32, x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        y.atan2(x)
    }
    #[cfg(not(feature = "std"))]
    libm::atan2f(y, x)
}

pub(crate) fn floor(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.floor()
    }
    #[cfg(not(feature = "std"))]
    libm::floorf(value)
}

/// Rounds half away from zero, as `f32::round` does.
pub(crate) fn round(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.round()
    }
    #[cfg(not(feature = "std"))]
    libm::roundf(value)
}
