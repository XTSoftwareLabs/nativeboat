//! Expected values are V8's and Python's exact integer results.

use super::*;

fn big(text: &str) -> BigInt {
    let units: Vec<u16> = text.encode_utf16().collect();
    BigInt::parse(&units).unwrap()
}

fn show(value: &BigInt) -> String {
    value.to_radix(10)
}

#[test]
fn text_parses_and_prints_in_every_radix() {
    for (text, expected) in [
        ("", "0"),
        ("  -42  ", "-42"),
        ("+42", "42"),
        ("0x1F", "31"),
        ("0O777", "511"),
        ("0b1010", "10"),
        ("000123", "123"),
        ("18446744073709551616", "18446744073709551616"),
    ] {
        assert_eq!(show(&big(text)), expected, "{text}");
    }
    for invalid in ["-0x10", "0x", "-", "1e3", "1_0", "12n", "0xg", "1.5"] {
        let units: Vec<u16> = invalid.encode_utf16().collect();
        assert_eq!(BigInt::parse(&units), None, "{invalid}");
    }
    assert_eq!(BigInt::parse(&[0xd800]), None);
    assert_eq!(big("-255").to_radix(16), "-ff");
    assert_eq!(big("255").to_radix(2), "11111111");
    assert_eq!(big("0").to_radix(36), "0");
    assert_eq!(
        big("1267650600228229401496703205376").to_radix(16),
        "10000000000000000000000000"
    );
    assert!(matches!(literal("12"), crate::Value::BigInt(value) if show(&value) == "12"));
}

#[test]
fn doubles_convert_exactly_and_round_to_even() {
    for (value, expected) in [
        (0.0, "0"),
        (-0.0, "0"),
        (1e21, "1000000000000000000000"),
        (-9007199254740992.0, "-9007199254740992"),
        (5e-324, "0"),
        (3.0, "3"),
    ] {
        assert_eq!(show(&BigInt::from_integer(value)), expected, "{value}");
    }
    for (text, expected) in [
        ("18446744073709551617", 18446744073709552000.0),
        ("-9007199254740993", -9007199254740992.0),
        ("9007199254740995", 9007199254740996.0),
        ("36893488147419103233", 36893488147419103000.0),
        ("123", 123.0),
    ] {
        assert_eq!(big(text).to_f64(), expected, "{text}");
    }
    assert_eq!(BigInt::default().to_f64(), 0.0);
    let huge = BigInt::from_i64(1).shift_left(1100).unwrap();
    assert_eq!(huge.to_f64(), f64::INFINITY);
    assert_eq!(huge.negate().to_f64(), f64::NEG_INFINITY);
    assert_eq!(BigInt::from_i64(-1).magnitude_u64(), Some(1));
    assert_eq!(huge.magnitude_u64(), None);
}

#[test]
fn comparisons_with_numbers_are_exact() {
    let five = BigInt::from_i64(5);
    for (number, expected) in [
        (5.0, Ordering::Equal),
        (5.5, Ordering::Less),
        (4.5, Ordering::Greater),
        (f64::INFINITY, Ordering::Less),
        (f64::NEG_INFINITY, Ordering::Greater),
    ] {
        assert_eq!(five.compare_f64(number), expected, "{number}");
    }
    let minus = BigInt::from_i64(-1);
    assert_eq!(minus.compare_f64(-0.5), Ordering::Less);
    assert_eq!(minus.compare_f64(-1.5), Ordering::Greater);
    assert_eq!(
        big("9007199254740993").compare_f64(9007199254740992.0),
        Ordering::Greater
    );
    assert!(BigInt::from_i64(-3) < BigInt::from_i64(-2));
    assert!(BigInt::from_i64(2) > BigInt::from_i64(-2));
    assert!(BigInt::from_i64(-2) < BigInt::from_i64(2));
    assert_eq!(
        BigInt::from_i64(7).cmp(&BigInt::from_i64(7)),
        Ordering::Equal
    );
}

#[test]
fn arithmetic_follows_truncating_division() {
    let a = big("123456789012345678901234567890");
    let b = big("987654321098765432109876543210");
    let product = a.multiply(&b).unwrap();
    assert_eq!(
        show(&product),
        "121932631137021795226185032733622923332237463801111263526900"
    );
    let difference = b.subtract(&a).unwrap();
    let (quotient, remainder) = product.divide(&difference).unwrap();
    assert_eq!(show(&quotient), "141093472973293022679545043194");
    assert_eq!(show(&remainder), "38155482003815548200381554820");
    assert_eq!(
        show(
            &quotient
                .multiply(&difference)
                .unwrap()
                .add(&remainder)
                .unwrap()
        ),
        show(&product)
    );
    for (left, right, quotient, remainder) in [
        ("-5", "2", "-2", "-1"),
        ("5", "-2", "-2", "1"),
        ("3", "5", "0", "3"),
        ("4294967296", "3", "1431655765", "1"),
        // Knuth's algorithm D must add the divisor back for these.
        (
            "39614081257132168796771975171",
            "9903520314283042199192993793",
            "3",
            "9903520314283042199192993792",
        ),
        (
            "2596069201709362459734969208012800",
            "604462909807314587353089",
            "4294836224",
            "604462909807310292516864",
        ),
        // The estimated digit is corrected, once with an overflowing rest.
        (
            "33131011632568156007755195226",
            "10750541312280087032",
            "3081799387",
            "6465208220230945842",
        ),
        (
            "45232442847748964955441488876",
            "11680013813142257662",
            "3872636074",
            "10155962639777389888",
        ),
        // An estimated digit beyond 32 bits.
        (
            "79228162514264337593543950335",
            "18446744073709551615",
            "4294967296",
            "4294967295",
        ),
    ] {
        let (q, r) = big(left).divide(&big(right)).unwrap();
        assert_eq!(
            (show(&q), show(&r)),
            (quotient.to_owned(), remainder.to_owned()),
            "{left} / {right}"
        );
    }
    assert_eq!(big("1").divide(&BigInt::default()), None);
    assert_eq!(show(&big("-7").add(&big("10")).unwrap()), "3");
    assert_eq!(show(&big("7").add(&big("-10")).unwrap()), "-3");
    assert_eq!(show(&big("-7").subtract(&big("-7")).unwrap()), "0");
    assert!(!big("-0").is_negative());
}

#[test]
fn powers_and_shifts() {
    let power =
        |base: &str, exponent: &str| big(base).power(&big(exponent)).map(|value| show(&value));
    assert_eq!(power("-7", "3"), Ok("-343".to_owned()));
    assert_eq!(
        power("2", "100"),
        Ok("1267650600228229401496703205376".to_owned())
    );
    assert_eq!(power("5", "0"), Ok("1".to_owned()));
    assert_eq!(power("0", "9"), Ok("0".to_owned()));
    assert_eq!(power("-1", "3"), Ok("-1".to_owned()));
    assert_eq!(power("-1", "4"), Ok("1".to_owned()));
    assert_eq!(power("2", "1099511627776"), Err(TooLarge));
    assert_eq!(power("2", "4294967295"), Err(TooLarge));
    let value = big("-1267650600228229401496703205376");
    assert_eq!(show(&value.shift_right(100)), "-1");
    assert_eq!(
        show(&value.subtract(&big("1")).unwrap().shift_right(100)),
        "-2"
    );
    assert_eq!(show(&big("-5").shift_right(1)), "-3");
    assert_eq!(show(&big("-4").shift_right(1)), "-2");
    assert_eq!(show(&big("-4").shift_right(64)), "-1");
    assert_eq!(show(&big("4").shift_right(64)), "0");
    assert_eq!(show(&big("12884901888").shift_right(32)), "3");
    assert_eq!(show(&big("3").shift_left(32).unwrap()), "12884901888");
    assert_eq!(show(&big("3").shift_left(33).unwrap()), "25769803776");
    assert_eq!(show(&BigInt::default().shift_left(1 << 40).unwrap()), "0");
    assert_eq!(big("1").shift_left(1 << 40), Err(TooLarge));
    assert_eq!(BigInt::new(false, vec![1; MAX_DIGITS + 1]), Err(TooLarge));
    let wide = BigInt::new(false, vec![1; MAX_DIGITS / 2 + 1]).unwrap();
    assert_eq!(wide.multiply(&wide), Err(TooLarge));
}

#[test]
fn bitwise_operators_use_twos_complement() {
    let and = |a: &str, b: &str| show(&big(a).bitwise(&big(b), |x, y| x & y));
    let or = |a: &str, b: &str| show(&big(a).bitwise(&big(b), |x, y| x | y));
    let xor = |a: &str, b: &str| show(&big(a).bitwise(&big(b), |x, y| x ^ y));
    assert_eq!(and("-6", "3"), "2");
    assert_eq!(or("-6", "3"), "-5");
    assert_eq!(xor("-6", "3"), "-7");
    assert_eq!(and("18446744073709551615", "-1"), "18446744073709551615");
    assert_eq!(or("-18446744073709551616", "1"), "-18446744073709551615");
    assert_eq!(show(&big("5").not()), "-6");
    assert_eq!(show(&big("-1").not()), "0");
}

#[test]
fn widths_wrap_like_as_int_n_and_as_uint_n() {
    let unsigned = |value: &str, bits: u64| big(value).as_uint_n(bits).map(|value| show(&value));
    let signed = |value: &str, bits: u64| big(value).as_int_n(bits).map(|value| show(&value));
    assert_eq!(unsigned("-1", 8), Ok("255".to_owned()));
    assert_eq!(
        unsigned("-9223372036854775808", 64),
        Ok("9223372036854775808".to_owned())
    );
    assert_eq!(unsigned("5", 0), Ok("0".to_owned()));
    assert_eq!(unsigned("5", 64), Ok("5".to_owned()));
    assert_eq!(unsigned("5", 1 << 40), Ok("5".to_owned()));
    assert_eq!(unsigned("-5", 1 << 40), Err(TooLarge));
    assert_eq!(signed("255", 8), Ok("-1".to_owned()));
    assert_eq!(signed("18446744073709551615", 64), Ok("-1".to_owned()));
    assert_eq!(signed("127", 8), Ok("127".to_owned()));
    assert_eq!(signed("5", 0), Ok("0".to_owned()));
    assert_eq!(signed("-5", 1 << 40), Err(TooLarge));
}
