use super::*;

fn total(values: &[f64]) -> f64 {
    let realm = Realm::new();
    let mut sum = Sum::new();
    for value in values {
        sum.add(&realm, &Value::Number(*value)).unwrap();
    }
    sum.finish()
}

#[test]
fn exact_sum_retains_cancellation_subnormals_ties_and_overflow() {
    for (values, expected) in [
        (vec![], -0.0),
        (vec![-0.0, -0.0], -0.0),
        (vec![0.0, -0.0], 0.0),
        (vec![1.0, -1.0], 0.0),
        (vec![1e16, 1.0, -1e16], 1.0),
        (vec![1e308, 1e308, -1e308], 1e308),
        (
            vec![f64::from_bits(1), f64::from_bits(2)],
            f64::from_bits(3),
        ),
        (
            vec![-f64::from_bits(1), -f64::from_bits(2)],
            -f64::from_bits(3),
        ),
        (vec![1.0, f64::from_bits(0x3ca0000000000000)], 1.0),
        (
            vec![1.0, f64::from_bits(0x3ca0000000000000), f64::from_bits(1)],
            f64::from_bits(0x3ff0000000000001),
        ),
        (
            vec![
                f64::from_bits(0x3ff0000000000001),
                f64::from_bits(0x3ca0000000000000),
            ],
            f64::from_bits(0x3ff0000000000002),
        ),
        (
            vec![
                f64::from_bits(0x3fffffffffffffff),
                f64::from_bits(0x3ca0000000000000),
            ],
            2.0,
        ),
        (vec![f64::MAX, f64::MAX], f64::INFINITY),
        (vec![-f64::MAX, -f64::MAX], f64::NEG_INFINITY),
        (vec![f64::INFINITY], f64::INFINITY),
        (vec![f64::NEG_INFINITY], f64::NEG_INFINITY),
    ] {
        assert_eq!(total(&values).to_bits(), expected.to_bits(), "{values:?}");
    }
    assert!(total(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
    assert!(total(&[f64::NAN, 1.0]).is_nan());
    let realm = Realm::new();
    let mut sum = Sum::new();
    sum.count = (1 << 53) - 1;
    assert!(sum.add(&realm, &Value::Number(1.0)).is_err());
    assert!(Sum::new().add(&realm, &Value::str("1")).is_err());
}

#[test]
fn sum_consumes_number_iterables_and_rejects_missing_or_non_numeric_items() {
    let mut realm = Realm::new();
    let array = Value::Object(realm.array(vec![
        Value::Number(1e16),
        Value::Number(1.0),
        Value::Number(-1e16),
    ]));
    assert!(crate::ops::strict_equals(
        &sum_precise(&mut realm, Value::Undefined, &[array]).unwrap(),
        &Value::Number(1.0)
    ));
    assert!(sum_precise(&mut realm, Value::Undefined, &[]).is_err());
    let array = Value::Object(realm.array(vec![Value::str("1")]));
    assert!(sum_precise(&mut realm, Value::Undefined, &[array]).is_err());
}
