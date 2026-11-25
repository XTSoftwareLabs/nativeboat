use super::*;

#[test]
fn scalar_fields_validate_handles_indexes_and_disconnected_ancestors() {
    let dom = RefCell::new(Dom::new());
    let context = &mut Context::default();
    let left = dom.borrow_mut().create_text("left");
    let right = dom.borrow_mut().create_text("right");
    let range = dom.borrow_mut().create_range((left, 1), (right, 2));
    for (index, expected) in [(0, left.0), (1, 1), (2, right.0), (3, 2)] {
        assert_eq!(
            field(&dom, &[range.into(), index.into()], context)
                .unwrap()
                .as_number(),
            Some(f64::from(expected))
        );
    }
    assert_eq!(
        field(&dom, &[range.into(), 4.into()], context)
            .unwrap()
            .as_boolean(),
        Some(false)
    );
    assert!(
        field(&dom, &[range.into(), 5.into()], context)
            .unwrap()
            .is_null()
    );
    dom.borrow_mut().set_range(range, (left, 1), (left, 1));
    assert_eq!(
        field(&dom, &[range.into(), 4.into()], context)
            .unwrap()
            .as_boolean(),
        Some(true)
    );
    assert_eq!(
        field(&dom, &[range.into(), 5.into()], context)
            .unwrap()
            .as_number(),
        Some(f64::from(left.0))
    );
    for value in [
        JsValue::undefined(),
        (-1).into(),
        1.5.into(),
        f64::NAN.into(),
        f64::INFINITY.into(),
        4294967296_f64.into(),
        99999.into(),
    ] {
        assert!(field(&dom, &[value, 0.into()], context).is_err());
    }
    for value in [
        JsValue::undefined(),
        (-1).into(),
        1.5.into(),
        f64::NAN.into(),
        6.into(),
    ] {
        assert!(field(&dom, &[range.into(), value], context).is_err());
    }
    dom.borrow_mut().drop_range(range);
    assert!(field(&dom, &[range.into(), 0.into()], context).is_err());
}
