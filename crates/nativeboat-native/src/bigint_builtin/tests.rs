//! Expected results and messages are V8's.

use super::*;
use crate::ops::{Arithmetic, Relation};

fn b(value: i64) -> Value {
    big(BigInt::from_i64(value))
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn shown_result(realm: &mut Realm, result: Completion) -> String {
    match result {
        Ok(Value::BigInt(value)) => format!("{}n", value.to_radix(10)),
        Ok(value) => ops::to_string(realm, &value).unwrap().to_rust_lossy(),
        Err(Thrown(error)) => ops::to_string(realm, &error).unwrap().to_rust_lossy(),
    }
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn call(realm: &mut Realm, target: &Value, name: &str, this: Value, args: &[Value]) -> String {
    let function = ops::get(realm, target, &Key::name(name)).unwrap();
    let result = ops::call(realm, &function, this, args);
    shown_result(realm, result)
}

#[test]
fn the_bigint_function_converts_like_v8() {
    let mut realm = Realm::new();
    let constructor = global(&mut realm, "BigInt");
    let valued = realm.object();
    let value_of = realm.function("valueOf", 0, false, |_, _, _| Ok(Value::str("12")));
    valued.define(
        Key::name("valueOf"),
        Property::data(Value::Object(value_of)),
    );
    let symbol = Value::Symbol(crate::Symbol::new(Some(JsStr::from("s"))));
    for (value, expected) in [
        (n(1e21), "1000000000000000000000n"),
        (n(-0.0), "0n"),
        (
            n(1.5),
            "RangeError: The number 1.5 cannot be converted to a BigInt because it is not an integer",
        ),
        (
            n(f64::NAN),
            "RangeError: The number NaN cannot be converted to a BigInt because it is not an integer",
        ),
        (Value::str(" 0x1F "), "31n"),
        (
            Value::str("1e3"),
            "SyntaxError: Cannot convert 1e3 to a BigInt",
        ),
        (Value::Boolean(true), "1n"),
        (
            Value::Undefined,
            "TypeError: Cannot convert undefined to a BigInt",
        ),
        (Value::Null, "TypeError: Cannot convert null to a BigInt"),
        (symbol, "TypeError: Cannot convert Symbol(s) to a BigInt"),
        (Value::Object(valued), "12n"),
        (b(7), "7n"),
    ] {
        let result = ops::call(&mut realm, &constructor, Value::Undefined, &[value]);
        assert_eq!(shown_result(&mut realm, result), expected);
    }
    let constructed = ops::construct(&mut realm, &constructor, &[n(1.0)]);
    assert_eq!(
        shown_result(&mut realm, constructed),
        "TypeError: BigInt is not a constructor"
    );
    for (name, bits, value, expected) in [
        ("asIntN", n(8.0), b(255), "-1n"),
        ("asUintN", n(8.0), b(-1), "255n"),
        ("asIntN", n(f64::NAN), b(5), "0n"),
        (
            "asIntN",
            n(-1.0),
            b(1),
            "RangeError: Invalid value: not (convertible to) a safe integer",
        ),
        (
            "asUintN",
            n(1e15),
            b(-1),
            "RangeError: Maximum BigInt size exceeded",
        ),
        (
            "asUintN",
            n(8.0),
            n(1.0),
            "TypeError: Cannot convert 1 to a BigInt",
        ),
    ] {
        let shown = call(
            &mut realm,
            &constructor,
            name,
            Value::Undefined,
            &[bits, value],
        );
        assert_eq!(shown, expected, "{name}");
    }
    let prototype = Value::Object(realm.bigint_prototype.clone());
    let wrapper = Value::Object(ops::to_object(&mut realm, &b(-1234567)).unwrap());
    for (name, this, args, expected) in [
        ("toString", b(255), vec![], "255"),
        ("toString", b(-255), vec![n(16.0)], "-ff"),
        (
            "toString",
            b(1),
            vec![n(37.0)],
            "RangeError: toString() radix argument must be between 2 and 36",
        ),
        (
            "toString",
            b(1),
            vec![n(f64::NAN)],
            "RangeError: toString() radix argument must be between 2 and 36",
        ),
        (
            "toString",
            n(1.0),
            vec![],
            "TypeError: BigInt.prototype.toString requires that 'this' be a BigInt",
        ),
        (
            "toString",
            Value::Object(realm.object()),
            vec![],
            "TypeError: BigInt.prototype.toString requires that 'this' be a BigInt",
        ),
        ("toLocaleString", wrapper.clone(), vec![], "-1,234,567"),
        ("toLocaleString", b(123), vec![], "123"),
        ("valueOf", wrapper, vec![], "-1234567n"),
        (
            "valueOf",
            Value::Null,
            vec![],
            "TypeError: BigInt.prototype.valueOf requires that 'this' be a BigInt",
        ),
    ] {
        assert_eq!(
            call(&mut realm, &prototype, name, this, &args),
            expected,
            "{name}"
        );
    }
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = ops::get(&mut realm, &b(1), &tag).unwrap();
    assert!(crate::same_value(&tag, &Value::str("BigInt")));
}

#[test]
fn operators_mix_bigints_only_with_bigints() {
    let mut realm = Realm::new();
    assert!(!ops::to_boolean(&b(0)) && ops::to_boolean(&b(-1)));
    assert_eq!(ops::type_of(&b(1)), "bigint");
    let thrown = ops::to_number(&mut realm, &b(1)).map(n);
    assert_eq!(
        shown_result(&mut realm, thrown),
        "TypeError: Cannot convert a BigInt value to a number"
    );
    let negated = ops::negate(&mut realm, &b(5));
    assert_eq!(shown_result(&mut realm, negated), "-5n");
    let negated = ops::negate(&mut realm, &Value::str("2"));
    assert_eq!(shown_result(&mut realm, negated), "-2");
    let inverted = ops::bitwise_not(&mut realm, &b(5));
    assert_eq!(shown_result(&mut realm, inverted), "-6n");
    let inverted = ops::bitwise_not(&mut realm, &n(5.0));
    assert_eq!(shown_result(&mut realm, inverted), "-6");
    let (old, new) = ops::increment(&mut realm, &b(5), -1).unwrap();
    assert_eq!(
        (
            shown_result(&mut realm, Ok(old)),
            shown_result(&mut realm, Ok(new))
        ),
        ("5n".to_owned(), "4n".to_owned())
    );
    let (old, new) = ops::increment(&mut realm, &Value::str("5"), 1).unwrap();
    assert_eq!(
        (
            shown_result(&mut realm, Ok(old)),
            shown_result(&mut realm, Ok(new))
        ),
        ("5".to_owned(), "6".to_owned())
    );
    // The largest value unit tests allow (1024 digits).
    let half = BigInt::from_i64(1).shift_left(32 * 1024 - 1).unwrap();
    let largest = big(half
        .add(&half.subtract(&BigInt::from_i64(1)).unwrap())
        .unwrap());
    let overflow = ops::increment(&mut realm, &largest, 1).map(|(_, new)| new);
    assert_eq!(
        shown_result(&mut realm, overflow),
        "RangeError: Maximum BigInt size exceeded"
    );
    let overflow = ops::add(&mut realm, &largest, &b(1));
    assert_eq!(
        shown_result(&mut realm, overflow),
        "RangeError: Maximum BigInt size exceeded"
    );
    for (left, right, expected) in [
        (b(1), b(2), "3n"),
        (
            b(1),
            n(2.0),
            "TypeError: Cannot mix BigInt and other types, use explicit conversions",
        ),
        (Value::str("a"), b(1), "a1"),
        (n(1.0), n(2.0), "3"),
    ] {
        let sum = ops::add(&mut realm, &left, &right);
        assert_eq!(shown_result(&mut realm, sum), expected);
    }
    let huge = big(BigInt::from_i64(1).shift_left(80).unwrap());
    for (operator, left, right, expected) in [
        (Arithmetic::Subtract, b(1), b(3), "-2n"),
        (Arithmetic::Multiply, b(-4), b(3), "-12n"),
        (Arithmetic::Divide, b(-5), b(2), "-2n"),
        (Arithmetic::Remainder, b(-5), b(2), "-1n"),
        (
            Arithmetic::Divide,
            b(1),
            b(0),
            "RangeError: Division by zero",
        ),
        (Arithmetic::Exponent, b(2), b(10), "1024n"),
        (
            Arithmetic::Exponent,
            b(2),
            b(-1),
            "RangeError: undefined must be positive",
        ),
        (
            Arithmetic::Exponent,
            b(2),
            huge.clone(),
            "RangeError: Maximum BigInt size exceeded",
        ),
        (Arithmetic::And, b(-6), b(3), "2n"),
        (Arithmetic::Or, b(-6), b(3), "-5n"),
        (Arithmetic::Xor, b(-6), b(3), "-7n"),
        (Arithmetic::ShiftLeft, b(3), b(2), "12n"),
        (Arithmetic::ShiftLeft, b(12), b(-2), "3n"),
        (
            Arithmetic::ShiftLeft,
            b(1),
            huge.clone(),
            "RangeError: Maximum BigInt size exceeded",
        ),
        (Arithmetic::ShiftLeft, b(0), huge.clone(), "0n"),
        (
            Arithmetic::ShiftLeft,
            b(1),
            b(1 << 40),
            "RangeError: Maximum BigInt size exceeded",
        ),
        (Arithmetic::ShiftRight, b(-5), b(1), "-3n"),
        (Arithmetic::ShiftRight, b(3), b(-2), "12n"),
        (Arithmetic::ShiftRight, b(-3), huge.clone(), "-1n"),
        (
            Arithmetic::ShiftRightUnsigned,
            b(1),
            b(0),
            "TypeError: BigInts have no unsigned right shift, use >> instead",
        ),
        (
            Arithmetic::Subtract,
            b(1),
            n(1.0),
            "TypeError: Cannot mix BigInt and other types, use explicit conversions",
        ),
        (
            Arithmetic::Multiply,
            largest.clone(),
            b(2),
            "RangeError: Maximum BigInt size exceeded",
        ),
        (Arithmetic::Subtract, n(3.0), n(1.0), "2"),
    ] {
        let result = ops::arithmetic(&mut realm, operator, &left, &right);
        assert_eq!(shown_result(&mut realm, result), expected, "{operator:?}");
    }
    for (relation, left, right, expected) in [
        (Relation::Less, b(1), n(1.5), true),
        (Relation::Greater, b(2), Value::str("1"), true),
        (Relation::Less, Value::str("1"), b(2), true),
        (Relation::Less, b(1), Value::str("x"), false),
        (Relation::Less, b(1), Value::str("2"), true),
        (Relation::Greater, Value::str("x"), b(1), false),
        (Relation::Less, b(1), b(2), true),
        (Relation::Less, n(f64::NAN), b(1), false),
        (Relation::Less, b(1), n(f64::NAN), false),
        (Relation::GreaterOrEqual, n(4.0), b(4), true),
    ] {
        assert_eq!(
            ops::relational(&mut realm, relation, &left, &right).unwrap(),
            expected
        );
    }
    for (left, right, expected) in [
        (b(1), n(1.0), true),
        (n(1.0), b(1), true),
        (b(1), n(f64::NAN), false),
        (b(1), Value::str("1"), true),
        (Value::str("x"), b(1), false),
        (b(0), Value::Boolean(false), true),
        (b(2), b(2), true),
    ] {
        assert_eq!(
            ops::loose_equals(&mut realm, &left, &right).unwrap(),
            expected
        );
    }
    assert!(ops::strict_equals(&b(3), &b(3)) && !ops::strict_equals(&b(3), &n(3.0)));
    assert!(crate::same_value(&b(0), &b(0)));
}

#[test]
fn bigints_flow_through_objects_text_and_collections() {
    let mut realm = Realm::new();
    let object = ops::to_object(&mut realm, &b(2)).unwrap();
    assert_eq!(
        object.data().prototype,
        Some(realm.bigint_prototype.clone())
    );
    let text = ops::to_string(&mut realm, &b(-12)).unwrap();
    assert_eq!(text.to_rust_lossy(), "-12");
    assert!(
        crate::iter::for_in_keys(&mut realm, &b(1))
            .unwrap()
            .is_empty()
    );
    let shown = crate::inspect::inspect(&mut realm, &b(-3), 2).unwrap();
    assert_eq!(shown, "-3n");
    let shown = crate::inspect::inspect(&mut realm, &Value::Object(object), 2).unwrap();
    assert_eq!(shown, "[BigInt: 2n]");
    let described = crate::node_error::specific_type(&mut realm, &b(5)).unwrap();
    assert_eq!(described, "type bigint (5n)");
    let number = global(&mut realm, "Number");
    let converted = ops::call(&mut realm, &number, Value::Undefined, &[b(123)]);
    assert_eq!(shown_result(&mut realm, converted), "123");
    let set = global(&mut realm, "Set");
    let values = Value::Object(realm.array(vec![b(1), b(1), b(2)]));
    let set = ops::construct(&mut realm, &set, &[values]).unwrap();
    let size = ops::get(&mut realm, &set, &Key::name("size"));
    assert_eq!(shown_result(&mut realm, size), "2");
    let json = global(&mut realm, "JSON");
    let wrapper = Value::Object(ops::to_object(&mut realm, &b(1)).unwrap());
    for value in [b(1), wrapper] {
        let result = call(&mut realm, &json, "stringify", Value::Undefined, &[value]);
        assert_eq!(result, "TypeError: Do not know how to serialize a BigInt");
    }
    let to_json = realm.function("toJSON", 0, false, |realm, this, _| {
        Ok(Value::String(ops::to_string(realm, &this)?))
    });
    realm
        .bigint_prototype
        .define(Key::name("toJSON"), Property::data(Value::Object(to_json)));
    let result = call(&mut realm, &json, "stringify", Value::Undefined, &[b(7)]);
    assert_eq!(result, "\"7\"");
    let object_prototype = Value::Object(realm.object_prototype.clone());
    let tag = call(&mut realm, &object_prototype, "toString", b(1), &[]);
    assert_eq!(tag, "[object BigInt]");
}
