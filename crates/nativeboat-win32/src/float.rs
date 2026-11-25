//! Float methods that `std` owns, from `libm` without it.

/// The float methods painting and windowing use.
pub trait Float: Copy {
    fn ceil(self) -> Self;
    fn floor(self) -> Self;
    fn round(self) -> Self;
    fn sqrt(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn exp(self) -> Self;
    fn sin_cos(self) -> (Self, Self)
    where
        Self: Copy,
    {
        (self.sin(), self.cos())
    }
}

impl Float for f32 {
    fn ceil(self) -> f32 {
        libm::ceilf(self)
    }
    fn floor(self) -> f32 {
        libm::floorf(self)
    }
    fn round(self) -> f32 {
        libm::roundf(self)
    }
    fn sqrt(self) -> f32 {
        libm::sqrtf(self)
    }
    fn sin(self) -> f32 {
        libm::sinf(self)
    }
    fn cos(self) -> f32 {
        libm::cosf(self)
    }
    fn exp(self) -> f32 {
        libm::expf(self)
    }
}

impl Float for f64 {
    fn ceil(self) -> f64 {
        libm::ceil(self)
    }
    fn floor(self) -> f64 {
        libm::floor(self)
    }
    fn round(self) -> f64 {
        libm::round(self)
    }
    fn sqrt(self) -> f64 {
        libm::sqrt(self)
    }
    fn sin(self) -> f64 {
        libm::sin(self)
    }
    fn cos(self) -> f64 {
        libm::cos(self)
    }
    fn exp(self) -> f64 {
        libm::exp(self)
    }
}
