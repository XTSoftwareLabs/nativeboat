use super::*;
use crate::string::JsStr;

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn call(
    realm: &mut Realm,
    prototype: &crate::Object,
    this: &Value,
    name: &str,
    args: &[Value],
) -> Completion {
    let prototype = Value::Object(prototype.clone());
    let method = ops::get(realm, &prototype, &Key::name(name)).unwrap();
    ops::call(realm, &method, this.clone(), args)
}

/// Calls `Number.prototype[name]` and returns the text.
fn text(realm: &mut Realm, value: f64, name: &str, args: &[Value]) -> String {
    let prototype = realm.number_prototype.clone();
    let result = call(realm, &prototype, &n(value), name, args).unwrap();
    ops::to_string(realm, &result).unwrap().to_rust_lossy()
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

#[test]
fn fixed_exponential_and_precision_round_exact_values() {
    let mut realm = Realm::new();
    let vectors = include_str!("vectors.txt");
    let mut checked = 0;
    for line in vectors.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let value = f64::from_bits(u64::from_str_radix(fields[0], 16).unwrap());
        let digits: f64 = fields[1].parse().unwrap();
        if fields[2] != "-" {
            assert_eq!(
                text(&mut realm, value, "toFixed", &[n(digits)]),
                fields[2],
                "{value:e}.toFixed({digits})"
            );
        }
        assert_eq!(
            text(&mut realm, value, "toExponential", &[n(digits)]),
            fields[3],
            "{value:e}.toExponential({digits})"
        );
        let precision: f64 = fields[4].parse().unwrap();
        assert_eq!(
            text(&mut realm, value, "toPrecision", &[n(precision)]),
            fields[5],
            "{value:e}.toPrecision({precision})"
        );
        checked += 1;
    }
    assert_eq!(checked, 3016);
}

#[test]
fn formatting_methods_handle_special_values_and_arguments() {
    let mut realm = Realm::new();
    assert_eq!(text(&mut realm, 1e21, "toFixed", &[n(2.0)]), "1e+21");
    assert_eq!(text(&mut realm, f64::NAN, "toFixed", &[]), "NaN");
    assert_eq!(text(&mut realm, -0.0, "toFixed", &[n(2.0)]), "0.00");
    assert_eq!(text(&mut realm, -0.0001, "toFixed", &[n(2.0)]), "-0.00");
    assert_eq!(text(&mut realm, 0.5, "toFixed", &[]), "1");
    assert_eq!(
        text(&mut realm, 123.456, "toExponential", &[]),
        "1.23456e+2"
    );
    assert_eq!(text(&mut realm, 0.0, "toExponential", &[]), "0e+0");
    assert_eq!(text(&mut realm, 0.0, "toExponential", &[n(2.0)]), "0.00e+0");
    assert_eq!(text(&mut realm, 5.0, "toExponential", &[]), "5e+0");
    assert_eq!(
        text(&mut realm, f64::INFINITY, "toExponential", &[n(500.0)]),
        "Infinity"
    );
    assert_eq!(text(&mut realm, 0.000123, "toPrecision", &[]), "0.000123");
    assert_eq!(
        text(&mut realm, f64::NEG_INFINITY, "toPrecision", &[n(3.0)]),
        "-Infinity"
    );
    assert_eq!(text(&mut realm, 0.0, "toPrecision", &[n(3.0)]), "0.00");
    assert_eq!(text(&mut realm, 123.0, "toPrecision", &[n(1.0)]), "1e+2");
    let prototype = realm.number_prototype.clone();
    for (name, digits, expected) in [
        (
            "toFixed",
            -1.0,
            "toFixed() argument must be between 0 and 100",
        ),
        (
            "toFixed",
            101.0,
            "toFixed() argument must be between 0 and 100",
        ),
        (
            "toExponential",
            f64::INFINITY,
            "toExponential() argument must be between 0 and 100",
        ),
        (
            "toPrecision",
            0.0,
            "toPrecision() argument must be between 1 and 100",
        ),
    ] {
        let refused = call(&mut realm, &prototype, &n(1.0), name, &[n(digits)]);
        assert_eq!(
            message(&mut realm, refused),
            format!("RangeError: {expected}")
        );
    }
    for name in [
        "toFixed",
        "toExponential",
        "toPrecision",
        "toString",
        "toLocaleString",
        "valueOf",
    ] {
        let refused = call(&mut realm, &prototype, &Value::str("1"), name, &[]);
        assert!(
            message(&mut realm, refused).contains("requires that 'this' be a Number"),
            "{name}"
        );
    }
    let throwing = realm.object();
    let failing = realm.function("valueOf", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "digits"))
    });
    throwing.define(Key::name("valueOf"), Property::data(Value::Object(failing)));
    let throwing = Value::Object(throwing);
    for name in ["toFixed", "toExponential", "toPrecision", "toString"] {
        let thrown = call(
            &mut realm,
            &prototype,
            &n(1.0),
            name,
            std::slice::from_ref(&throwing),
        );
        assert_eq!(message(&mut realm, thrown), "RangeError: digits", "{name}");
    }
    // Wrapper objects are Numbers too.
    let wrapped = Value::Object(realm.primitive_object(Wrapped::Number(2.5)));
    let fixed = call(&mut realm, &prototype, &wrapped, "toFixed", &[n(1.0)]).unwrap();
    assert!(ops::strict_equals(&fixed, &Value::str("2.5")));
}

#[test]
fn radix_strings_are_the_shortest_identifying_digits() {
    let mut realm = Realm::new();
    assert_eq!(text(&mut realm, 255.0, "toString", &[n(16.0)]), "ff");
    assert_eq!(text(&mut realm, 255.0, "toString", &[n(2.0)]), "11111111");
    assert_eq!(text(&mut realm, -255.0, "toString", &[n(36.0)]), "-73");
    assert_eq!(text(&mut realm, 35.0, "toString", &[n(36.0)]), "z");
    assert_eq!(text(&mut realm, 0.5, "toString", &[n(2.0)]), "0.1");
    assert_eq!(text(&mut realm, 3.5, "toString", &[n(2.0)]), "11.1");
    assert_eq!(
        text(&mut realm, 0.1, "toString", &[n(2.0)]),
        "0.0001100110011001100110011001100110011001100110011001101"
    );
    // Rounding to even in the last digit.
    assert_eq!(
        text(&mut realm, 0.5, "toString", &[n(3.0)]),
        "0.1111111111111111111111111111111112"
    );
    // The largest double below one stays below one.
    assert_eq!(
        radix_string(1.0 - f64::EPSILON / 2.0, 3),
        format!("0.{}", "2".repeat(33))
    );
    // Rounded last digits.
    assert_eq!(radix_string(9.088184001853248, 18), "9.1aa53c07f7a6");
    assert_eq!(
        radix_string(0.9999999999999996, 3),
        "0.2222222222222222222222222222222202"
    );
    // Digits beyond the double's precision are zeros.
    assert_eq!(
        text(&mut realm, 2f64.powi(60), "toString", &[n(2.0)]),
        format!("1{}", "0".repeat(60))
    );
    assert_eq!(
        text(&mut realm, 1e21, "toString", &[n(36.0)]),
        radix_string(1e21, 36)
    );
    assert_eq!(text(&mut realm, f64::NAN, "toString", &[n(16.0)]), "NaN");
    assert_eq!(text(&mut realm, 12.5, "toString", &[]), "12.5");
    assert_eq!(text(&mut realm, 12.5, "toString", &[n(10.0)]), "12.5");
    let prototype = realm.number_prototype.clone();
    for radix in [1.0, 37.0] {
        let refused = call(&mut realm, &prototype, &n(1.0), "toString", &[n(radix)]);
        assert_eq!(
            message(&mut realm, refused),
            "RangeError: toString() radix must be between 2 and 36"
        );
    }
}

#[test]
fn locale_strings_group_digits_and_round_to_three_places() {
    let mut realm = Realm::new();
    for (value, expected) in [
        (1_234_567.891, "1,234,567.891"),
        (1234.5678, "1,234.568"),
        (0.1, "0.1"),
        (0.0005, "0.001"),
        (0.0004, "0"),
        (999.9995, "1,000"),
        (-1234.5, "-1,234.5"),
        (1e21, "1,000,000,000,000,000,000,000"),
        (0.0, "0"),
        (-0.0, "-0"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "∞"),
        (f64::NEG_INFINITY, "-∞"),
    ] {
        assert_eq!(
            text(&mut realm, value, "toLocaleString", &[]),
            expected,
            "{value}"
        );
    }
}

#[test]
fn constructors_statics_and_booleans() {
    let mut realm = Realm::new();
    let number = global(&mut realm, "Number");
    let converted =
        ops::call(&mut realm, &number, Value::Undefined, &[Value::str(" 42 ")]).unwrap();
    assert!(ops::strict_equals(&converted, &n(42.0)));
    let zero = ops::call(&mut realm, &number, Value::Undefined, &[]).unwrap();
    assert!(ops::strict_equals(&zero, &n(0.0)));
    let boxed = ops::construct(&mut realm, &number, &[n(7.0)]).unwrap();
    let prototype = realm.number_prototype.clone();
    let value = call(&mut realm, &prototype, &boxed, "valueOf", &[]).unwrap();
    assert!(ops::strict_equals(&value, &n(7.0)));
    let target = realm.function("Target", 0, true, |_, _, _| Ok(Value::Undefined));
    let custom = realm.object();
    target.define(
        Key::name("prototype"),
        Property::data(Value::Object(custom.clone())),
    );
    let instance =
        ops::construct_target(&mut realm, &number, &[], &Value::Object(target.clone())).unwrap();
    assert_eq!(instance.as_object().unwrap().data().prototype, Some(custom));
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "proto"))
    });
    target.define(
        Key::name("prototype"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let thrown = ops::construct_target(&mut realm, &number, &[], &Value::Object(target));
    assert_eq!(message(&mut realm, thrown), "RangeError: proto");
    let symbol = Value::Symbol(realm.symbols.iterator.clone());
    for thrown in [
        ops::call(
            &mut realm,
            &number,
            Value::Undefined,
            std::slice::from_ref(&symbol),
        ),
        ops::construct(&mut realm, &number, &[symbol]),
    ] {
        assert!(message(&mut realm, thrown).starts_with("TypeError"));
    }
    let statics = |realm: &mut Realm, name: &str, value: Value| -> Value {
        let number = global(realm, "Number");
        let function = ops::get(realm, &number, &Key::name(name)).unwrap();
        ops::call(realm, &function, Value::Undefined, &[value]).unwrap()
    };
    for (name, value, expected) in [
        ("isFinite", n(1.0), true),
        ("isFinite", Value::str("1"), false),
        ("isFinite", n(f64::INFINITY), false),
        ("isInteger", n(5.0), true),
        ("isInteger", n(5.5), false),
        ("isInteger", Value::str("5"), false),
        ("isNaN", n(f64::NAN), true),
        ("isNaN", Value::str("x"), false),
        ("isSafeInteger", n(9_007_199_254_740_991.0), true),
        ("isSafeInteger", n(9_007_199_254_740_992.0), false),
        ("isSafeInteger", n(1.5), false),
    ] {
        let result = statics(&mut realm, name, value);
        assert!(
            ops::strict_equals(&result, &Value::Boolean(expected)),
            "{name}"
        );
    }
    let epsilon = ops::get(&mut realm, &number, &Key::name("EPSILON")).unwrap();
    assert!(ops::strict_equals(&epsilon, &n(f64::EPSILON)));
    let descriptor = number
        .as_object()
        .unwrap()
        .own_property(&Key::name("MAX_VALUE"))
        .unwrap();
    assert!(!descriptor.configurable && !descriptor.enumerable);
    let parse = ops::get(&mut realm, &number, &Key::name("parseFloat")).unwrap();
    assert!(ops::strict_equals(
        &parse,
        &global(&mut realm, "parseFloat")
    ));
    // Number.prototype is the Number object holding zero.
    let prototype_value = Value::Object(prototype.clone());
    let zero = call(&mut realm, &prototype, &prototype_value, "valueOf", &[]).unwrap();
    assert!(ops::strict_equals(&zero, &n(0.0)));

    let boolean = global(&mut realm, "Boolean");
    let truthy = ops::call(&mut realm, &boolean, Value::Undefined, &[Value::str("x")]).unwrap();
    assert!(ops::strict_equals(&truthy, &Value::Boolean(true)));
    let boxed = ops::construct(&mut realm, &boolean, &[n(0.0)]).unwrap();
    let booleans = realm.boolean_prototype.clone();
    let value = call(&mut realm, &booleans, &boxed, "valueOf", &[]).unwrap();
    assert!(ops::strict_equals(&value, &Value::Boolean(false)));
    for (receiver, expected) in [(Value::Boolean(true), "true"), (boxed, "false")] {
        let text = call(&mut realm, &booleans, &receiver, "toString", &[]).unwrap();
        assert!(ops::strict_equals(
            &text,
            &Value::String(JsStr::from(expected))
        ));
    }
    for name in ["toString", "valueOf"] {
        let refused = call(&mut realm, &booleans, &n(1.0), name, &[]);
        assert!(
            message(&mut realm, refused).contains("requires that 'this' be a Boolean"),
            "{name}"
        );
        let object = Value::Object(realm.object());
        let refused = call(&mut realm, &booleans, &object, name, &[]);
        assert!(
            message(&mut realm, refused).contains("be a Boolean"),
            "{name}"
        );
    }
    let prototype_value = Value::Object(booleans.clone());
    let flag = call(&mut realm, &booleans, &prototype_value, "valueOf", &[]).unwrap();
    assert!(ops::strict_equals(&flag, &Value::Boolean(false)));
    let object = Value::Object(realm.object());
    let refused = call(&mut realm, &prototype, &object, "valueOf", &[]);
    assert!(message(&mut realm, refused).contains("be a Number"));
}
