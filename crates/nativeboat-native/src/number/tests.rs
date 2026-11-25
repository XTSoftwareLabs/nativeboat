use super::*;

#[test]
fn numbers_format_with_shortest_digits_and_language_thresholds() {
    for (value, text) in [
        (f64::NAN, "NaN"),
        (0.0, "0"),
        (-0.0, "0"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (1.0, "1"),
        (-42.0, "-42"),
        (0.1, "0.1"),
        (0.1 + 0.2, "0.30000000000000004"),
        (123.456, "123.456"),
        (1e21, "1e+21"),
        (1e20, "100000000000000000000"),
        (123456789012345680000.0, "123456789012345680000"),
        (1.5e21, "1.5e+21"),
        (1e-6, "0.000001"),
        (1.5e-6, "0.0000015"),
        (1e-7, "1e-7"),
        (-2.5e-7, "-2.5e-7"),
        (5e-324, "5e-324"),
        (f64::MAX, "1.7976931348623157e+308"),
        (9007199254740993.0, "9007199254740992"),
    ] {
        assert_eq!(to_string(value), text, "{value:e}");
    }
}

fn number(text: &str) -> f64 {
    parse(&JsStr::from(text))
}

#[test]
fn strings_parse_with_the_numeric_string_grammar() {
    for (text, value) in [
        ("", 0.0),
        ("  \n\u{FEFF} ", 0.0),
        ("42", 42.0),
        ("  -42.5e1\t", -425.0),
        ("+.5", 0.5),
        ("5.", 5.0),
        ("1E+2", 100.0),
        ("1e-2", 0.01),
        ("0x1F", 31.0),
        ("0XfF", 255.0),
        ("0o17", 15.0),
        ("0O7", 7.0),
        ("0b101", 5.0),
        ("0B1", 1.0),
        ("Infinity", f64::INFINITY),
        ("+Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        ("007", 7.0),
        ("0", 0.0),
        ("00", 0.0),
        (
            "0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
            340282366920938463463374607431768211455.0,
        ),
    ] {
        assert_eq!(number(text), value, "{text:?}");
    }
    let huge = format!("0x1{}", "0".repeat(40));
    assert_eq!(number(&huge), 2f64.powi(160));
    // An exact integer halfway plus one must round up even beyond u128.
    let sticky = format!("0b1{}1{}1", "0".repeat(52), "0".repeat(106));
    assert_eq!(number(&sticky).to_bits(), (1183u64 << 52) + 1);
    let overflow = format!("0x1{}", "0".repeat(4096));
    assert_eq!(number(&overflow), f64::INFINITY);
    assert!(number(&format!("{overflow}z")).is_nan());
    assert!(number("-0").is_sign_negative());
    for text in [
        "abc", "1a", "0x", "0xG", "-0x1", "infinity", "inf", "NaN", ".", "+", "e5", "1e", "1e+",
        "1..2", "0b2", "0o8", "--1", "1 2", "١",
    ] {
        assert!(number(text).is_nan(), "{text:?}");
    }
}
