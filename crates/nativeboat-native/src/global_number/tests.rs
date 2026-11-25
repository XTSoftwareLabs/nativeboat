use super::*;
use crate::{JsStr, Key, Property, Symbol};
use std::cell::RefCell;
use std::rc::Rc;

fn call(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let global = Value::Object(realm.global.clone());
    let function = ops::get(realm, &global, &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

fn number(realm: &mut Realm, name: &str, args: &[Value]) -> f64 {
    let Value::Number(value) = call(realm, name, args).unwrap() else {
        panic!("number")
    };
    value
}

#[test]
fn global_numeric_functions_have_native_metadata_and_prefix_grammars() {
    let mut realm = Realm::new();
    for (name, length) in [
        ("parseInt", 2.0),
        ("parseFloat", 1.0),
        ("isNaN", 1.0),
        ("isFinite", 1.0),
    ] {
        let property = realm.global.own_property(&Key::name(name)).unwrap();
        assert!(!property.enumerable && property.configurable);
        let global = Value::Object(realm.global.clone());
        let function = ops::get(&mut realm, &global, &Key::name(name)).unwrap();
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
    for (text, base, expected) in [
        (" \u{feff}-0XfFz", 0.0, -255.0f64),
        ("+0x10", 16.0, 16.0),
        ("0x10", 10.0, 0.0),
        ("0b10", 0.0, 0.0),
        ("0b10", 16.0, 2832.0),
        ("123tail", 10.0, 123.0),
        ("08", 0.0, 8.0),
        ("zZ!", 36.0, 1295.0),
        ("-000", 10.0, -0.0),
        ("17", 4294967304.0, 15.0),
        ("17", 8.9, 15.0),
        ("10", f64::NAN, 10.0),
    ] {
        assert_eq!(
            number(
                &mut realm,
                "parseInt",
                &[Value::str(text), Value::Number(base)]
            )
            .to_bits(),
            expected.to_bits(),
            "{text}"
        );
    }
    for (text, base) in [
        ("", 0.0),
        (" \n", 10.0),
        ("+", 10.0),
        ("-", 0.0),
        ("0x", 16.0),
        ("g", 16.0),
        ("١", 10.0),
        ("1", 1.0),
        ("1", 37.0),
        ("1", -2.0),
    ] {
        assert!(
            number(
                &mut realm,
                "parseInt",
                &[Value::str(text), Value::Number(base)]
            )
            .is_nan()
        );
    }
    assert_eq!(
        number(&mut realm, "parseInt", &[Value::Number(-0.0)]).to_bits(),
        0
    );
    assert!(number(&mut realm, "parseInt", &[]).is_nan());
    for (text, expected) in [
        (" \u{2028}-.5e+2junk", -50.0f64),
        ("1.", 1.0),
        ("1.e2", 100.0),
        ("1e", 1.0),
        ("1E+", 1.0),
        ("1e-2x", 0.01),
        ("0x10", 0.0),
        ("+Infinitytail", f64::INFINITY),
        ("-Infinity!", f64::NEG_INFINITY),
        ("Infinity", f64::INFINITY),
        ("-0x", -0.0),
        ("1e400", f64::INFINITY),
        ("-1e-400", -0.0),
    ] {
        assert_eq!(
            number(&mut realm, "parseFloat", &[Value::str(text)]).to_bits(),
            expected.to_bits(),
            "{text}"
        );
    }
    for text in [
        "", " \n", "+", "-", ".", ".e1", "e1", "inf", "infinity", "NaN", "١",
    ] {
        assert!(number(&mut realm, "parseFloat", &[Value::str(text)]).is_nan());
    }
    assert!(number(&mut realm, "parseFloat", &[]).is_nan());
    let surrogate = Value::String(JsStr::from_units(&[0x31, 0xd800, 0x32]));
    assert_eq!(
        number(&mut realm, "parseFloat", std::slice::from_ref(&surrogate)),
        1.0
    );
    assert_eq!(number(&mut realm, "parseInt", &[surrogate]), 1.0);
}

#[test]
fn numeric_globals_preserve_coercion_order_errors_and_predicates() {
    let mut realm = Realm::new();
    let log = Rc::new(RefCell::new(String::new()));
    let input = realm.object();
    let trace = log.clone();
    let convert = realm.function("toString", 0, false, move |_, _, _| {
        trace.borrow_mut().push('s');
        Ok(Value::str("17"))
    });
    input.define(
        Key::name("toString"),
        Property::data(Value::Object(convert)),
    );
    let base = realm.object();
    let trace = log.clone();
    let convert = realm.function("valueOf", 0, false, move |_, _, _| {
        trace.borrow_mut().push('r');
        Ok(Value::Number(8.0))
    });
    base.define(Key::name("valueOf"), Property::data(Value::Object(convert)));
    assert_eq!(
        number(
            &mut realm,
            "parseInt",
            &[Value::Object(input.clone()), Value::Object(base.clone())]
        ),
        15.0
    );
    assert_eq!(&*log.borrow(), "sr");
    log.borrow_mut().clear();
    let trace = log.clone();
    let throwing = realm.function("valueOf", 0, false, move |_, _, _| {
        trace.borrow_mut().push('r');
        Err(crate::Thrown(Value::str("radix")))
    });
    base.define(
        Key::name("valueOf"),
        Property::data(Value::Object(throwing)),
    );
    let thrown = call(
        &mut realm,
        "parseInt",
        &[Value::str(""), Value::Object(base.clone())],
    )
    .unwrap_err();
    assert!(ops::strict_equals(&thrown.0, &Value::str("radix")));
    assert_eq!(&*log.borrow(), "r");
    log.borrow_mut().clear();
    let throwing = realm.function("toString", 0, false, |_, _, _| {
        Err(crate::Thrown(Value::str("string")))
    });
    input.define(
        Key::name("toString"),
        Property::data(Value::Object(throwing)),
    );
    let thrown = call(
        &mut realm,
        "parseInt",
        &[Value::Object(input.clone()), Value::Object(base)],
    )
    .unwrap_err();
    assert!(ops::strict_equals(&thrown.0, &Value::str("string")));
    assert!(log.borrow().is_empty());
    assert!(call(&mut realm, "parseFloat", &[Value::Object(input)]).is_err());
    for (value, nan, finite) in [
        (Value::Undefined, true, false),
        (Value::Null, false, true),
        (Value::str(""), false, true),
        (Value::str("12"), false, true),
        (Value::str("12x"), true, false),
        (Value::Number(f64::INFINITY), false, false),
        (Value::Number(f64::NAN), true, false),
    ] {
        assert!(ops::strict_equals(
            &call(&mut realm, "isNaN", std::slice::from_ref(&value)).unwrap(),
            &Value::Boolean(nan)
        ));
        assert!(ops::strict_equals(
            &call(&mut realm, "isFinite", &[value]).unwrap(),
            &Value::Boolean(finite)
        ));
    }
    for name in ["parseInt", "parseFloat", "isNaN", "isFinite"] {
        assert!(call(&mut realm, name, &[Value::Symbol(Symbol::new(None))]).is_err());
    }
}
