use super::*;

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn math_functions_match_their_definitions() {
    close(acos(1.0), 0.0);
    close(acosh(1.0), 0.0);
    close(asin(1.0), core::f64::consts::FRAC_PI_2);
    close(asinh(0.0), 0.0);
    close(atan(1.0), core::f64::consts::FRAC_PI_4);
    close(atanh(0.0), 0.0);
    close(atan2(1.0, 1.0), core::f64::consts::FRAC_PI_4);
    close(cbrt(27.0), 3.0);
    close(ceil(1.2), 2.0);
    close(cos(0.0), 1.0);
    close(cosh(0.0), 1.0);
    close(exp(1.0), core::f64::consts::E);
    close(expm1(0.0), 0.0);
    close(floor(-1.2), -2.0);
    close(log(core::f64::consts::E), 1.0);
    close(log1p(0.0), 0.0);
    close(log10(1000.0), 3.0);
    close(log2(8.0), 3.0);
    close(pow(2.0, 10.0), 1024.0);
    close(sin(0.0), 0.0);
    close(sinh(0.0), 0.0);
    close(sqrt(16.0), 4.0);
    close(tan(0.0), 0.0);
    close(tanh(0.0), 0.0);
    close(trunc(-1.7), -1.0);
}

#[test]
fn fractions_and_euclidean_remainders() {
    close(fract(2.75), 0.75);
    close(fract(-2.75), -0.75);
    assert!(fract(f64::INFINITY).is_nan());
    close(rem_euclid(7.0, 4.0), 3.0);
    close(rem_euclid(-7.0, 4.0), 1.0);
    close(rem_euclid(-7.0, -4.0), 1.0);
    close(rem_euclid(-1.0, 4_294_967_296.0), 4_294_967_295.0);
}
