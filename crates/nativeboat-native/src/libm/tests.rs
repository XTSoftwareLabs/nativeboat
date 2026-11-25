use super::*;
use alloc::vec::Vec;

/// Deterministic pseudo-random bits.
struct Bits(u64);

impl Bits {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Special values and values near the functions' internal thresholds.
fn specials() -> Vec<f64> {
    let mut values = alloc::vec![
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::EPSILON,
        1e-300,
        1e-20,
        1e-10,
        1e-8,
        1e-5,
        0.1,
        0.25,
        0.5,
        0.75,
        1.0,
        1.5,
        2.0,
        3.0,
        10.0,
        20.0,
        22.0,
        40.0,
        100.0,
        700.0,
        709.7,
        710.0,
        745.0,
        1e10,
        1e20,
        1e100,
        1e300,
        f64::MAX,
        f64::INFINITY,
        f64::NAN,
        core::f64::consts::PI,
        core::f64::consts::FRAC_PI_2,
        core::f64::consts::FRAC_PI_4,
        3.0 * core::f64::consts::FRAC_PI_4,
        5.0 * core::f64::consts::FRAC_PI_4,
        7.0 * core::f64::consts::FRAC_PI_4,
        9.0 * core::f64::consts::FRAC_PI_4,
        1e6 * core::f64::consts::PI,
        1e9,
        core::f64::consts::E,
        core::f64::consts::LN_2,
        0.34657359027997264,
        1.0397207708399179,
    ];
    let negatives: Vec<f64> = values.iter().map(|value| -value).collect();
    values.extend(negatives);
    values
}

/// Specials, then random values of every magnitude and of moderate size.
fn inputs() -> Vec<f64> {
    let mut values = specials();
    let mut bits = Bits(0x9e37_79b9_7f4a_7c15);
    for _ in 0..20_000 {
        values.push(f64::from_bits(bits.next()));
    }
    for _ in 0..20_000 {
        let unit = (bits.next() >> 11) as f64 / (1u64 << 53) as f64;
        let scale = [1.0, 4.0, 10.0, 50.0, 800.0][(bits.next() % 5) as usize];
        values.push((unit * 2.0 - 1.0) * scale);
    }
    values
}

/// The distance in units in the last place, treating NaNs as equal.
fn ulps(a: f64, b: f64) -> u64 {
    if a.is_nan() && b.is_nan() {
        return 0;
    }
    if a == b {
        return 0;
    }
    let key = |x: f64| {
        let bits = i128::from(x.to_bits() as i64);
        if bits < 0 {
            i128::from(i64::MIN) - bits
        } else {
            bits
        }
    };
    u64::try_from((key(a) - key(b)).unsigned_abs()).unwrap_or(u64::MAX)
}

fn agrees(name: &str, ours: fn(f64) -> f64, reference: fn(f64) -> f64, tolerance: u64) {
    for x in inputs() {
        let (got, want) = (ours(x), reference(x));
        assert!(
            ulps(got, want) <= tolerance,
            "{name}({x:e}) = {got:e}, expected {want:e}"
        );
    }
}

#[test]
fn vendored_functions_match_their_upstream_bit_for_bit() {
    agrees("acos", acos, ::libm::acos, 0);
    agrees("asin", asin, ::libm::asin, 0);
    agrees("asinh", asinh, ::libm::asinh, 0);
    agrees("atan", atan, ::libm::atan, 0);
    agrees("atanh", atanh, ::libm::atanh, 0);
    agrees("cos", cos, ::libm::cos, 0);
    agrees("cosh", cosh, ::libm::cosh, 0);
    agrees("exp", exp, ::libm::exp, 0);
    agrees("expm1", expm1, ::libm::expm1, 0);
    agrees("log", log, ::libm::log, 0);
    agrees("log1p", log1p, ::libm::log1p, 0);
    agrees("log2", log2, ::libm::log2, 0);
    agrees("log10", log10, ::libm::log10, 0);
    agrees("sin", sin, ::libm::sin, 0);
    agrees("sinh", sinh, ::libm::sinh, 0);
    agrees("tan", tan, ::libm::tan, 0);
    agrees("tanh", tanh, ::libm::tanh, 0);
    // Upstream lets large negative arguments reach a finite result; fdlibm
    // and V8 return NaN for every argument below 1.
    agrees(
        "acosh",
        acosh,
        |x| if x < 1.0 { f64::NAN } else { ::libm::acosh(x) },
        0,
    );
    assert!(acosh(-83946.65).is_nan());
    // Upstream's cube root is correctly rounded; musl's is within an ulp.
    agrees("cbrt", cbrt, ::libm::cbrt, 1);
}

#[test]
fn rounding_roots_and_scaling_are_exact() {
    agrees("sqrt", sqrt, f64::sqrt, 0);
    agrees("trunc", trunc, f64::trunc, 0);
    agrees("floor", floor, f64::floor, 0);
    agrees("ceil", ceil, f64::ceil, 0);
    assert!(floor(-0.0).is_sign_negative());
    assert!(ceil(-0.5).is_sign_negative());
    assert!(trunc(-0.5).is_sign_negative());
    for x in inputs() {
        for n in [-2200, -1100, -1074, -60, -1, 0, 1, 60, 1100, 2200] {
            let want = x * 2f64.powi(n / 2) * 2f64.powi(n - n / 2);
            let got = scalbn(x, n);
            if want.is_finite() && want != 0.0 && want.abs() >= f64::MIN_POSITIVE {
                assert_eq!(got.to_bits(), want.to_bits(), "scalbn({x:e}, {n})");
            }
        }
    }
    assert_eq!(
        scalbn(f64::from_bits(1), 2097),
        f64::from_bits(0x7fe0_0000_0000_0000)
    );
    assert_eq!(scalbn(f64::MAX, -3000), 0.0);
    assert_eq!(scalbn(1.0, 3000), f64::INFINITY);
}

#[test]
fn remainders_are_exact() {
    let values = inputs();
    let mut bits = Bits(7);
    for &x in values.iter().take(4_000) {
        for _ in 0..4 {
            let y = values[(bits.next() as usize) % values.len()];
            let (got, want) = (fmod(x, y), x % y);
            assert_eq!(
                ulps(got, want),
                0,
                "fmod({x:e}, {y:e}) = {got:e}, expected {want:e}"
            );
        }
    }
    assert_eq!(fmod(5.5, f64::INFINITY), 5.5);
    assert!(fmod(f64::INFINITY, 2.0).is_nan());
    assert!(fmod(1.0, 0.0).is_nan());
    assert!(fmod(-6.0, 3.0).is_sign_negative());
    assert_eq!(fmod(f64::MAX, f64::from_bits(1)), 0.0);
    assert_eq!(fmod(7.0, f64::from_bits(3)), 7.0 % f64::from_bits(3));
}

#[test]
fn float_remainders_are_exact() {
    let mut bits = Bits(11);
    for _ in 0..20_000 {
        let x = f32::from_bits(bits.next() as u32);
        let y = f32::from_bits(bits.next() as u32);
        let (got, want) = (fmodf(x, y), x % y);
        assert!(
            got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
            "fmodf({x:e}, {y:e}) = {got:e}, expected {want:e}"
        );
    }
    assert_eq!(fmodf(5.5, 2.0), 1.5);
    assert!(fmodf(-6.0, 3.0).is_sign_negative());
}

#[test]
fn two_argument_functions_match_their_upstream_bit_for_bit() {
    let values = specials();
    let mut bits = Bits(99);
    let mut pairs = Vec::new();
    for &x in &values {
        for &y in &values {
            pairs.push((x, y));
        }
    }
    for _ in 0..40_000 {
        let x = f64::from_bits(bits.next());
        let y = (bits.next() >> 11) as f64 / (1u64 << 53) as f64 * 40.0 - 20.0;
        pairs.push((x, y));
        pairs.push((x.abs() % 10.0, y));
        pairs.push((y, f64::from_bits(bits.next())));
    }
    for (x, y) in pairs {
        let (got, want) = (atan2(x, y), ::libm::atan2(x, y));
        assert!(
            ulps(got, want) <= 1,
            "atan2({x:e}, {y:e}) = {got:e}, expected {want:e}"
        );
        let (got, want) = (pow(x, y), ::libm::pow(x, y));
        assert!(
            ulps(got, want) <= 1,
            "pow({x:e}, {y:e}) = {got:e}, expected {want:e}"
        );
    }
}

#[test]
fn word_helpers_combine_and_replace_halves() {
    let x = combine_words(0x4000_0000, 0x1234_5678);
    assert_eq!(get_high_word(x), 0x4000_0000);
    assert_eq!(get_low_word(x), 0x1234_5678);
    assert_eq!(
        get_high_word(with_set_high_word(x, 0x3ff0_0000)),
        0x3ff0_0000
    );
    assert_eq!(get_low_word(with_set_low_word(x, 7)), 7);
    assert_eq!(fabs(-2.0), 2.0);
}

#[test]
fn rare_reduction_and_power_paths_match_upstream() {
    // The double closest to a multiple of pi/2 needs extra terms of 2/pi
    // and drops zero terms during argument reduction.
    let worst = f64::from_bits(0x7506_ac5b_262c_a1ff);
    for x in [
        worst,
        -worst,
        3.0 * core::f64::consts::FRAC_PI_2,
        -3.0 * core::f64::consts::FRAC_PI_2,
        2.0 * core::f64::consts::PI,
        -2.0 * core::f64::consts::PI,
    ] {
        assert_eq!(sin(x).to_bits(), ::libm::sin(x).to_bits(), "sin({x:e})");
        assert_eq!(cos(x).to_bits(), ::libm::cos(x).to_bits(), "cos({x:e})");
        assert_eq!(tan(x).to_bits(), ::libm::tan(x).to_bits(), "tan({x:e})");
    }
    // Huge exponents of bases near one, and results at the overflow and
    // underflow boundaries.
    for (x, y) in [
        (1.0000001, 3e9),
        (0.9999999, 3e9),
        (1.0000001, -3e9),
        (0.5, 1e20),
        (0.5, -1e20),
        (2.0, 1e20),
        (2.0, -1e20),
        (2.0, 1024.0),
        (2.000004, 1023.997045372036),
        (2.0, -1075.0),
        (2.000003, -1074.9976736610256),
    ] {
        assert_eq!(
            pow(x, y).to_bits(),
            ::libm::pow(x, y).to_bits(),
            "pow({x:e}, {y:e})"
        );
    }
    assert_eq!(scalbn(1.0, 5000), f64::INFINITY);
    assert_eq!(scalbn(1.0, -5000), 0.0);
}
