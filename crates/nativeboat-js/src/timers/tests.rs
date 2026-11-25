use super::*;

#[test]
fn validates_native_arguments_and_orders_tied_deadlines() {
    let context = &mut Context::default();
    let timers = Timers {
        state: Gc::new(GcRefCell::new(State::default())),
        started: Instant::now(),
    };
    let function = context
        .eval(boa_engine::Source::from_bytes("(function(){})"))
        .unwrap();
    let array: JsValue = JsArray::new(context).unwrap().into();
    assert!(schedule(&JsValue::undefined(), &[], &timers, context).is_err());
    assert!(
        schedule(
            &JsValue::undefined(),
            &[function.clone(), 0.into(), 0.into()],
            &timers,
            context
        )
        .is_err()
    );
    assert!(
        schedule(
            &JsValue::undefined(),
            &[
                function.clone(),
                0.into(),
                JsObject::with_null_proto().into()
            ],
            &timers,
            context
        )
        .is_err()
    );
    for delay in [f64::NAN, -5.0, 2.0] {
        schedule(
            &JsValue::undefined(),
            &[function.clone(), delay.into(), array.clone(), false.into()],
            &timers,
            context,
        )
        .unwrap();
    }
    {
        let mut state = timers.state.borrow_mut();
        for (id, timer) in &mut state.timers {
            timer.at = if *id == 3 { 2.0 } else { 1.0 };
        }
    }
    assert_eq!(timers.due(2.0), vec![1, 2, 3]);
    cancel(&JsValue::undefined(), &[2.into()], &timers, context).unwrap();
    assert!(timers.take(2, 2.0).is_none());
    assert!(timers.take(1, 2.0).is_some());
    for value in [JsValue::undefined(), 0.into(), 99.into()] {
        refresh(&JsValue::undefined(), &[value], &timers, context).unwrap();
    }
    cancel(&JsValue::undefined(), &[3.into()], &timers, context).unwrap();
    assert!(timers.state.borrow().timers.is_empty());
}
