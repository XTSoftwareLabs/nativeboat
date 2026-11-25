use super::*;
use crate::ErrorKind;

fn math(realm: &mut Realm) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name("Math")).unwrap()
}

fn call(realm: &mut Realm, name: &str, args: &[Value]) -> f64 {
    let math = math(realm);
    let method = ops::get(realm, &math, &Key::name(name)).unwrap();
    let Value::Number(value) = ops::call(realm, &method, math, args).unwrap() else {
        panic!("numeric result")
    };
    value
}

#[test]
fn intrinsics_have_standard_metadata_tags_and_numeric_identities() {
    let mut realm = Realm::new();
    let math = math(&mut realm);
    let object = math.as_object().unwrap();
    for name in [
        "E", "LN10", "LN2", "LOG10E", "LOG2E", "PI", "SQRT1_2", "SQRT2",
    ] {
        let property = object.own_property(&Key::name(name)).unwrap();
        assert!(!property.enumerable && !property.configurable);
        assert!(matches!(
            property.slot,
            Slot::Data {
                writable: false,
                ..
            }
        ));
    }
    for (name, length) in [
        ("sqrt", 1.0),
        ("min", 2.0),
        ("hypot", 2.0),
        ("sumPrecise", 1.0),
        ("random", 0.0),
    ] {
        let function = ops::get(&mut realm, &math, &Key::name(name)).unwrap();
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &function, &Key::name("name")).unwrap(),
            &Value::str(name)
        ));
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &function, &Key::name("length")).unwrap(),
            &Value::Number(length)
        ));
        assert!(ops::construct(&mut realm, &function, &[]).is_err());
    }
    let to_string = ops::get(&mut realm, &math, &Key::name("toString")).unwrap();
    assert!(ops::strict_equals(
        &ops::call(&mut realm, &to_string, math.clone(), &[]).unwrap(),
        &Value::str("[object Math]")
    ));
    assert!(ops::set(&mut realm, &math, &Key::name("PI"), Value::Null).is_err());
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    assert!(ops::delete(&mut realm, &math, &tag).unwrap());
    assert!(ops::strict_equals(
        &ops::call(&mut realm, &to_string, math, &[]).unwrap(),
        &Value::str("[object Object]")
    ));
    for (name, input, expected) in [
        ("abs", -3.0, 3.0),
        ("acos", 1.0, 0.0),
        ("acosh", 1.0, 0.0),
        ("asin", -0.0, -0.0),
        ("asinh", -0.0, -0.0),
        ("atan", 1.0, std::f64::consts::FRAC_PI_4),
        ("atanh", -0.0, -0.0),
        ("cbrt", -8.0, -2.0),
        ("ceil", -0.2, -0.0),
        ("clz32", 1.0, 31.0),
        ("cos", 0.0, 1.0),
        ("cosh", 0.0, 1.0),
        ("exp", 0.0, 1.0),
        ("expm1", -0.0, -0.0),
        ("floor", -0.2, -1.0),
        ("fround", 16777217.0, 16777216.0),
        ("f16round", 1.0004882812500002, 1.0009765625),
        ("log", std::f64::consts::E, 1.0),
        ("log1p", -0.0, -0.0),
        ("log10", 100.0, 2.0),
        ("log2", 8.0, 3.0),
        ("round", -0.5, -0.0),
        ("sign", -0.0, -0.0),
        ("sin", -0.0, -0.0),
        ("sinh", -0.0, -0.0),
        ("sqrt", -0.0, -0.0),
        ("tan", -0.0, -0.0),
        ("tanh", -0.0, -0.0),
        ("trunc", -0.2, -0.0),
    ] {
        let actual = call(&mut realm, name, &[Value::Number(input)]);
        if expected == 0.0 {
            assert_eq!(actual.to_bits(), expected.to_bits(), "{name}");
        } else {
            assert!(
                (actual - expected).abs() <= 1e-15 * expected.abs(),
                "{name}: {actual}"
            );
        }
    }
    assert!(call(&mut realm, "sqrt", &[]).is_nan());
    assert!(call(&mut realm, "sqrt", &[Value::Number(-1.0)]).is_nan());
    assert_eq!(call(&mut realm, "clz32", &[]), 32.0);
    assert_eq!(call(&mut realm, "abs", &[Value::str("-3")]), 3.0);
    assert_eq!(call(&mut realm, "sign", &[Value::Number(3.0)]), 1.0);
    assert_eq!(call(&mut realm, "sign", &[Value::Number(-3.0)]), -1.0);
    assert!(call(&mut realm, "sign", &[Value::Number(f64::NAN)]).is_nan());
}

#[test]
fn rounding_extrema_binary_operations_and_hypot_preserve_edge_cases() {
    let mut realm = Realm::new();
    for (input, expected) in [
        (0.5, 1.0),
        (-1.5, -1.0),
        (-1.4, -1.0),
        (2.0, 2.0),
        (f64::INFINITY, f64::INFINITY),
    ] {
        assert_eq!(call(&mut realm, "round", &[Value::Number(input)]), expected);
    }
    assert!(call(&mut realm, "round", &[Value::Number(f64::NAN)]).is_nan());
    assert_eq!(call(&mut realm, "min", &[]), f64::INFINITY);
    assert_eq!(call(&mut realm, "max", &[]), f64::NEG_INFINITY);
    for (name, args, expected) in [
        ("min", vec![Value::Number(0.0), Value::Number(-0.0)], -0.0),
        ("max", vec![Value::Number(-0.0), Value::Number(0.0)], 0.0),
        ("min", vec![Value::Number(2.0), Value::Number(-1.0)], -1.0),
        ("max", vec![Value::Number(2.0), Value::Number(-1.0)], 2.0),
        (
            "imul",
            vec![Value::Number(4294967295.0), Value::Number(5.0)],
            -5.0,
        ),
        (
            "atan2",
            vec![Value::Number(-0.0), Value::Number(-0.0)],
            -std::f64::consts::PI,
        ),
        (
            "pow",
            vec![Value::Number(f64::NAN), Value::Number(0.0)],
            1.0,
        ),
    ] {
        assert_eq!(
            call(&mut realm, name, &args).to_bits(),
            expected.to_bits(),
            "{name}"
        );
    }
    assert!(
        call(
            &mut realm,
            "pow",
            &[Value::Number(1.0), Value::Number(f64::INFINITY)]
        )
        .is_nan()
    );
    assert!(call(&mut realm, "pow", &[]).is_nan());
    assert_eq!(call(&mut realm, "imul", &[]), 0.0);
    assert!(call(&mut realm, "atan2", &[]).is_nan());
    assert!(call(&mut realm, "max", &[Value::Number(f64::NAN)]).is_nan());
    assert_eq!(call(&mut realm, "hypot", &[]).to_bits(), 0);
    assert_eq!(
        call(
            &mut realm,
            "hypot",
            &[Value::Number(3.0), Value::Number(4.0)]
        ),
        5.0
    );
    assert_eq!(
        call(
            &mut realm,
            "hypot",
            &[Value::Number(f64::NAN), Value::Number(f64::INFINITY)]
        ),
        f64::INFINITY
    );
    assert!(call(&mut realm, "hypot", &[Value::Number(f64::NAN)]).is_nan());
    let huge = call(
        &mut realm,
        "hypot",
        &[Value::Number(3e200), Value::Number(4e200)],
    );
    assert!((huge / 5e200 - 1.0).abs() < 1e-15);
    let tiny = call(
        &mut realm,
        "hypot",
        &[Value::Number(3e-200), Value::Number(4e-200)],
    );
    assert!((tiny / 5e-200 - 1.0).abs() < 1e-15);
    // A NaN/Infinity result cannot bypass later ToNumber exceptions.
    let bad = realm.object();
    let throwing = realm.function("valueOf", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Type, "coercion"))
    });
    bad.define(
        Key::name("valueOf"),
        Property::data(Value::Object(throwing)),
    );
    for name in ["min", "max", "hypot", "pow", "atan2", "imul"] {
        let math = math(&mut realm);
        let method = ops::get(&mut realm, &math, &Key::name(name)).unwrap();
        assert!(
            ops::call(
                &mut realm,
                &method,
                math,
                &[Value::Number(f64::NAN), Value::Object(bad.clone())]
            )
            .is_err()
        );
    }
}

#[test]
fn half_precision_is_rounded_directly_and_random_sequences_are_realm_owned() {
    let mut realm = Realm::new();
    let halfway = f64::from_bits(998u64 << 52); // 2^-25: half the least f16 subnormal.
    for (input, expected) in [
        (1.00048828125, 1.0),
        (1.00146484375, 1.001953125),
        (halfway, 0.0),
        (
            f64::from_bits(halfway.to_bits() + 1),
            f64::from_bits(999u64 << 52),
        ),
        (0.00000095367431640625, 0.00000095367431640625),
        (0.000061005353927612305, 0.00006103515625),
        (65504.0, 65504.0),
        (65520.0, f64::INFINITY),
        (1e30, f64::INFINITY),
        (-1e-100, -0.0),
        (-0.0, -0.0),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    ] {
        assert_eq!(
            call(&mut realm, "f16round", &[Value::Number(input)]).to_bits(),
            expected.to_bits(),
            "{input}"
        );
    }
    assert!(call(&mut realm, "f16round", &[Value::Number(f64::NAN)]).is_nan());
    let mut other = Realm::new();
    let first: Vec<_> = (0..20).map(|_| call(&mut realm, "random", &[])).collect();
    let second: Vec<_> = (0..20).map(|_| call(&mut other, "random", &[])).collect();
    assert_ne!(first, second);
    assert!(
        first
            .iter()
            .chain(&second)
            .all(|value| *value >= 0.0 && *value < 1.0)
    );
    let global_property = realm.global.own_property(&Key::name("Math")).unwrap();
    assert!(!global_property.enumerable && global_property.configurable);
    assert!(matches!(
        global_property.slot,
        Slot::Data { writable: true, .. }
    ));
}
