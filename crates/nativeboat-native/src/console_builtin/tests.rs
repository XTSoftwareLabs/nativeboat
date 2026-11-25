use super::*;
use crate::object::Object;
use crate::realm::ErrorKind;

#[test]
fn tables_render_rows_columns_and_property_filters() {
    let (mut realm, state) = captured();
    let first = realm.object();
    ops::define_data(&first, Key::name("a"), Value::Number(1.0));
    let second = realm.object();
    ops::define_data(&second, Key::name("a"), Value::Number(2.0));
    ops::define_data(&second, Key::name("b"), Value::str("x"));
    let data = Value::Object(realm.array(vec![
        Value::Object(first),
        Value::Object(second),
        Value::Number(3.0),
    ]));
    call(&mut realm, "table", core::slice::from_ref(&data)).unwrap();
    assert_eq!(
        messages(&state)[0].1,
        "┌─────────┬───┬─────┬────────┐\n│ (index) │ a │ b   │ Values │\n├─────────┼───┼─────┼────────┤\n│ 0       │ 1 │     │        │\n│ 1       │ 2 │ 'x' │        │\n│ 2       │   │     │ 3      │\n└─────────┴───┴─────┴────────┘\n"
    );
    let properties = Value::Object(realm.array(vec![Value::str("b"), Value::str("b")]));
    call(&mut realm, "table", &[data.clone(), properties]).unwrap();
    assert_eq!(
        messages(&state)[0].1,
        "┌─────────┬─────┐\n│ (index) │ b   │\n├─────────┼─────┤\n│ 0       │     │\n│ 1       │ 'x' │\n│ 2       │     │\n└─────────┴─────┘\n"
    );
    assert!(call(&mut realm, "table", &[data, Value::Null]).is_err());
    call(&mut realm, "table", &[Value::Number(1.0)]).unwrap();
    call(&mut realm, "table", &[]).unwrap();
    let empty = Value::Object(realm.array(vec![]));
    call(&mut realm, "table", &[empty]).unwrap();
    assert_eq!(
        messages(&state),
        [
            (Stream::Out, "1\n".into()),
            (Stream::Out, "\n".into()),
            (
                Stream::Out,
                "┌─────────┐\n│ (index) │\n├─────────┤\n└─────────┘\n".into()
            ),
        ]
    );
}

fn captured() -> (Realm, Shared) {
    let mut realm = Realm::new();
    let state = Rc::new(RefCell::new(State {
        captured: Some(Vec::new()),
        ..State::default()
    }));
    install_with(&mut realm, state.clone());
    (realm, state)
}

fn call(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let global = Value::Object(realm.global.clone());
    let console = ops::get(realm, &global, &Key::name("console"))?;
    let method = ops::get(realm, &console, &Key::name(name))?;
    // Methods remain bound when extracted, as Node's do.
    ops::call(realm, &method, Value::Null, args)
}

fn messages(state: &Shared) -> Vec<(Stream, String)> {
    core::mem::take(state.borrow_mut().captured.as_mut().unwrap())
}

fn throwing(realm: &Realm) -> Object {
    realm.function("fail", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "abrupt"))
    })
}

#[test]
fn log_aliases_and_errors_use_their_streams_and_bound_state() {
    let (mut realm, state) = captured();
    for method in ["log", "info", "debug", "dirxml"] {
        assert!(matches!(
            call(
                &mut realm,
                method,
                &[Value::str("n=%d"), Value::Number(3.0)]
            )
            .unwrap(),
            Value::Undefined
        ));
    }
    for method in ["warn", "error"] {
        call(&mut realm, method, &[Value::str("warning")]).unwrap();
    }
    call(&mut realm, "log", &[]).unwrap();
    let mut expected = vec![(Stream::Out, "n=3\n".to_owned()); 4];
    expected.extend([
        (Stream::Err, "warning\n".to_owned()),
        (Stream::Err, "warning\n".to_owned()),
        (Stream::Out, "\n".to_owned()),
    ]);
    assert_eq!(messages(&state), expected);
    // Exercise the real platform sink separately from capture.
    let mut real = Realm::new();
    call(&mut real, "log", &[Value::str("native console stdout")]).unwrap();
    call(&mut real, "error", &[Value::str("native console stderr")]).unwrap();
}

#[test]
fn assertions_traces_and_groups_format_without_throwing() {
    let (mut realm, state) = captured();
    call(&mut realm, "assert", &[Value::Boolean(true)]).unwrap();
    call(&mut realm, "assert", &[]).unwrap();
    call(
        &mut realm,
        "assert",
        &[Value::Boolean(false), Value::str("x=%s"), Value::str("bad")],
    )
    .unwrap();
    call(&mut realm, "assert", &[Value::Null, Value::Number(3.0)]).unwrap();
    call(&mut realm, "trace", &[]).unwrap();
    call(&mut realm, "trace", &[Value::str("message")]).unwrap();
    call(&mut realm, "group", &[Value::str("outer")]).unwrap();
    call(&mut realm, "groupCollapsed", &[]).unwrap();
    call(&mut realm, "log", &[Value::str("a\nb")]).unwrap();
    for _ in 0..3 {
        call(&mut realm, "groupEnd", &[]).unwrap();
    }
    for method in ["clear", "profile", "profileEnd", "timeStamp"] {
        call(&mut realm, method, &[]).unwrap();
    }
    assert_eq!(
        messages(&state),
        [
            (Stream::Err, "Assertion failed\n".into()),
            (Stream::Err, "Assertion failed: x=bad\n".into()),
            (Stream::Err, "Assertion failed 3\n".into()),
            (Stream::Err, "Trace\n".into()),
            (Stream::Err, "Trace: message\n".into()),
            (Stream::Out, "outer\n".into()),
            (Stream::Out, "    a\n    b\n".into()),
        ]
    );
    assert_eq!(state.borrow().indentation, 0);
}

#[test]
fn counters_keep_labels_separate_and_report_missing_resets() {
    let (mut realm, state) = captured();
    call(&mut realm, "count", &[]).unwrap();
    call(&mut realm, "count", &[Value::Undefined]).unwrap();
    call(&mut realm, "count", &[Value::Number(7.0)]).unwrap();
    call(&mut realm, "countReset", &[]).unwrap();
    call(&mut realm, "count", &[]).unwrap();
    call(&mut realm, "countReset", &[Value::str("missing")]).unwrap();
    assert_eq!(
        messages(&state),
        [
            (Stream::Out, "default: 1\n".into()),
            (Stream::Out, "default: 2\n".into()),
            (Stream::Out, "7: 1\n".into()),
            (Stream::Out, "default: 1\n".into()),
            (
                Stream::Err,
                "Warning: Count for 'missing' does not exist\n".into()
            ),
        ]
    );
    for unit in [0xd800, 0xd801] {
        call(
            &mut realm,
            "count",
            &[Value::String(JsStr::from_units(&[unit]))],
        )
        .unwrap();
    }
    assert_eq!(state.borrow().counts.len(), 4);
}

#[test]
fn timers_format_elapsed_units_and_reset_only_after_time_end() {
    let (mut realm, state) = captured();
    state.borrow_mut().clock = || 0.0;
    call(&mut realm, "time", &[]).unwrap();
    call(&mut realm, "time", &[]).unwrap();
    state.borrow_mut().clock = || 12.5;
    call(
        &mut realm,
        "timeLog",
        &[Value::Undefined, Value::str("extra")],
    )
    .unwrap();
    state.borrow_mut().clock = || 1100.0;
    call(
        &mut realm,
        "timeEnd",
        &[Value::Undefined, Value::str("ignored")],
    )
    .unwrap();
    call(&mut realm, "timeLog", &[]).unwrap();
    call(&mut realm, "timeEnd", &[]).unwrap();
    assert_eq!(
        messages(&state),
        [
            (
                Stream::Err,
                "Warning: Label 'default' already exists for console.time()\n".into()
            ),
            (Stream::Out, "default: 12.5ms extra\n".into()),
            (Stream::Out, "default: 1.100s\n".into()),
            (
                Stream::Err,
                "Warning: No such label 'default' for console.timeLog()\n".into()
            ),
            (
                Stream::Err,
                "Warning: No such label 'default' for console.timeEnd()\n".into()
            ),
        ]
    );
    for (ms, expected) in [
        (0.0, "0ms"),
        (61_001.0, "1:01.001 (m:ss.mmm)"),
        (3_661_001.0, "1:01:01.001 (h:mm:ss.mmm)"),
    ] {
        assert_eq!(duration(&mut realm, ms).unwrap(), expected);
    }
}

#[test]
fn dir_reads_depth_and_propagates_getters_and_conversions() {
    let (mut realm, state) = captured();
    let inner = realm.object();
    ops::define_data(&inner, Key::name("x"), Value::Number(1.0));
    let outer = realm.object();
    ops::define_data(&outer, Key::name("inner"), Value::Object(inner));
    let value = Value::Object(outer);
    let options = realm.object();
    for depth in [
        Value::Undefined,
        Value::Null,
        Value::Number(-1.0),
        Value::Number(0.0),
        Value::Number(4.0),
    ] {
        ops::define_data(&options, Key::name("depth"), depth);
        call(
            &mut realm,
            "dir",
            &[value.clone(), Value::Object(options.clone())],
        )
        .unwrap();
    }
    call(&mut realm, "dir", &[]).unwrap();
    call(&mut realm, "dir", &[value, Value::Boolean(true)]).unwrap();
    let output = messages(&state);
    assert_eq!(output[0].1, "{ inner: { x: 1 } }\n");
    assert_eq!(output[1].1, output[0].1);
    assert_eq!(output[2].1, "{ inner: [Object] }\n");
    assert_eq!(output[3].1, output[2].1);
    assert_eq!(output[5].1, "undefined\n");
    let getter = throwing(&realm);
    options.define(
        Key::name("depth"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter.clone()),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(call(&mut realm, "dir", &[Value::Null, Value::Object(options)]).is_err());
    let label = realm.object();
    ops::define_data(&label, Key::name("toString"), Value::Object(getter));
    for method in ["count", "countReset", "time", "timeEnd", "timeLog"] {
        assert!(
            call(&mut realm, method, &[Value::Object(label.clone())]).is_err(),
            "{method}"
        );
    }
    assert!(
        call(
            &mut realm,
            "group",
            &[Value::str("%s"), Value::Object(label)]
        )
        .is_err()
    );
    assert!(messages(&state).is_empty());
}

#[test]
fn formatting_callbacks_can_reenter_the_same_console() {
    let (mut realm, state) = captured();
    let label = realm.object();
    let conversion = realm.function("toString", 0, false, |realm, _, _| {
        call(realm, "count", &[Value::str("nested")])?;
        Ok(Value::str("outer"))
    });
    ops::define_data(&label, Key::name("toString"), Value::Object(conversion));
    call(&mut realm, "count", &[Value::Object(label)]).unwrap();
    assert_eq!(
        messages(&state),
        [
            (Stream::Out, "nested: 1\n".into()),
            (Stream::Out, "outer: 1\n".into())
        ]
    );
}
