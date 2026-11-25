use super::*;
use crate::ErrorKind;

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

#[test]
fn standard_errors_have_native_call_construct_descriptors_and_brands() {
    let mut realm = Realm::new();
    let base = global(&mut realm, "Error");
    let is_error = ops::get(&mut realm, &base, &Key::name("isError")).unwrap();
    for (name, length) in [
        ("Error", 1.0),
        ("EvalError", 1.0),
        ("RangeError", 1.0),
        ("ReferenceError", 1.0),
        ("SyntaxError", 1.0),
        ("TypeError", 1.0),
        ("URIError", 1.0),
        ("AggregateError", 2.0),
        ("SuppressedError", 3.0),
    ] {
        let function = global(&mut realm, name);
        assert_eq!(ops::type_of(&function), "function");
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &function, &Key::name("length")).unwrap(),
            &Value::Number(length)
        ));
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &function, &Key::name("name")).unwrap(),
            &Value::str(name)
        ));
        let prototype = ops::get(&mut realm, &function, &Key::name("prototype")).unwrap();
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &prototype, &Key::name("constructor")).unwrap(),
            &function
        ));
        let descriptor = function
            .as_object()
            .unwrap()
            .own_property(&Key::name("prototype"))
            .unwrap();
        assert!(!descriptor.enumerable && !descriptor.configurable);
        assert!(matches!(
            descriptor.slot,
            crate::Slot::Data {
                writable: false,
                ..
            }
        ));
        assert!(ops::strict_equals(
            &ops::call(
                &mut realm,
                &is_error,
                Value::Undefined,
                std::slice::from_ref(&prototype)
            )
            .unwrap(),
            &Value::Boolean(false)
        ));
        let args = match name {
            "AggregateError" => vec![
                Value::Object(realm.array(vec![Value::Number(1.0)])),
                Value::str("message"),
            ],
            "SuppressedError" => vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::str("message"),
            ],
            _ => vec![Value::str("message")],
        };
        let receiver = Value::Object(realm.object());
        for value in [
            ops::call(&mut realm, &function, receiver.clone(), &args).unwrap(),
            ops::construct(&mut realm, &function, &args).unwrap(),
        ] {
            assert!(!ops::strict_equals(&value, &receiver));
            assert!(ops::instance_of(&mut realm, &value, &function).unwrap());
            assert!(ops::instance_of(&mut realm, &value, &base).unwrap());
            assert!(ops::strict_equals(
                &ops::get(&mut realm, &value, &Key::name("name")).unwrap(),
                &Value::str(name)
            ));
            assert!(ops::strict_equals(
                &ops::call(
                    &mut realm,
                    &is_error,
                    Value::Undefined,
                    std::slice::from_ref(&value)
                )
                .unwrap(),
                &Value::Boolean(true)
            ));
            let message = value
                .as_object()
                .unwrap()
                .own_property(&Key::name("message"))
                .unwrap();
            assert!(!message.enumerable && message.configurable);
            let text = ops::to_string(&mut realm, &value).unwrap();
            assert_eq!(text.to_rust_lossy(), format!("{name}: message"));
        }
        assert!(
            format!("{:?}", function.as_object().unwrap().data().kind)
                .contains("BuiltinConstructor")
        );
    }
    for kind in [ErrorKind::Eval, ErrorKind::Syntax, ErrorKind::Uri] {
        assert!(matches!(
            realm.error(kind, "internal").data().kind,
            Kind::Error
        ));
    }
    let empty = ops::call(&mut realm, &base, Value::Null, &[]).unwrap();
    assert!(
        empty
            .as_object()
            .unwrap()
            .own_property(&Key::name("message"))
            .is_none()
    );
    assert!(ops::strict_equals(
        &ops::call(&mut realm, &is_error, Value::Undefined, &[]).unwrap(),
        &Value::Boolean(false)
    ));
    let plain = Value::Object(realm.object());
    assert!(ops::strict_equals(
        &ops::call(&mut realm, &is_error, Value::Undefined, &[plain]).unwrap(),
        &Value::Boolean(false)
    ));
}

#[test]
fn native_construct_targets_validate_prototypes_depth_and_error_options() {
    let mut realm = Realm::new();
    let base = global(&mut realm, "Error");
    let other = Value::Object(realm.function("Other", 0, true, |_, _, _| Ok(Value::Undefined)));
    let prototype = ops::get(&mut realm, &other, &Key::name("prototype")).unwrap();
    let value = ops::construct_target(&mut realm, &base, &[], &other).unwrap();
    assert_eq!(
        value.as_object().unwrap().data().prototype.as_ref(),
        prototype.as_object()
    );
    assert!(matches!(
        value.as_object().unwrap().data().kind,
        Kind::Error
    ));
    ops::set(&mut realm, &other, &Key::name("prototype"), Value::Null).unwrap();
    let value = ops::construct_target(&mut realm, &base, &[], &other).unwrap();
    assert_eq!(
        value.as_object().unwrap().data().prototype,
        Some(realm.error_prototype.clone())
    );
    let ordinary = ops::construct_target(&mut realm, &other, &[], &base).unwrap();
    assert_eq!(
        ordinary.as_object().unwrap().data().prototype,
        Some(realm.error_prototype.clone())
    );
    assert!(ops::construct_target(&mut realm, &base, &[], &Value::Null).is_err());
    let plain = Value::Object(realm.object());
    assert!(ops::construct_target(&mut realm, &base, &[], &plain).is_err());
    realm.max_depth = 0;
    assert!(ops::construct(&mut realm, &base, &[]).is_err());
    realm.max_depth = 4000;
    assert!(ops::construct(&mut realm, &base, &[]).is_ok());
    let options = realm.object();
    options.define(Key::name("cause"), Property::data(Value::Undefined));
    let value = ops::construct(
        &mut realm,
        &base,
        &[Value::Undefined, Value::Object(options)],
    )
    .unwrap();
    assert!(
        value
            .as_object()
            .unwrap()
            .own_property(&Key::name("cause"))
            .is_some()
    );
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(ops::call(&mut realm, &base, Value::Undefined, &[symbol]).is_err());
    let aggregate = global(&mut realm, "AggregateError");
    assert!(ops::call(&mut realm, &aggregate, Value::Undefined, &[]).is_err());
}

#[test]
fn error_messages_causes_and_aggregate_iteration_keep_order_and_original_throws() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut realm = Realm::new();
    let base = global(&mut realm, "Error");
    let aggregate = global(&mut realm, "AggregateError");
    let suppressed = global(&mut realm, "SuppressedError");
    let log = Rc::new(RefCell::new(String::new()));
    let message = realm.object();
    let trace = log.clone();
    let convert = realm.function("toString", 0, false, move |_, _, _| {
        trace.borrow_mut().push('m');
        Ok(Value::String(crate::JsStr::from_units(&[0xd800])))
    });
    message.define(
        Key::name("toString"),
        Property::data(Value::Object(convert)),
    );
    let options = realm.object();
    let parent = realm.object();
    options.data_mut().prototype = Some(parent.clone());
    let trace = log.clone();
    let cause = realm.function("get cause", 0, false, move |_, _, _| {
        trace.borrow_mut().push('c');
        Ok(Value::Number(7.0))
    });
    ops::define_accessor(&parent, Key::name("cause"), Some(cause), None);
    let value = ops::construct(
        &mut realm,
        &base,
        &[
            Value::Object(message.clone()),
            Value::Object(options.clone()),
        ],
    )
    .unwrap();
    assert_eq!(&*log.borrow(), "mc");
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &value, &Key::name("cause")).unwrap(),
        &Value::Number(7.0)
    ));
    let Value::String(text) = ops::get(&mut realm, &value, &Key::name("message")).unwrap() else {
        panic!("string")
    };
    assert_eq!(text.units(), &[0xd800]);
    log.borrow_mut().clear();
    let trace = log.clone();
    let getter = realm.function("get iterator", 0, false, move |_, _, _| {
        trace.borrow_mut().push('i');
        Err(Thrown(Value::str("iterator")))
    });
    let input = realm.object();
    ops::define_accessor(
        &input,
        Key::Symbol(realm.symbols.iterator.clone()),
        Some(getter),
        None,
    );
    let thrown = ops::construct(
        &mut realm,
        &aggregate,
        &[
            Value::Object(input),
            Value::Object(message.clone()),
            Value::Object(options.clone()),
        ],
    )
    .unwrap_err();
    assert!(ops::strict_equals(&thrown.0, &Value::str("iterator")));
    assert_eq!(&*log.borrow(), "mci");
    log.borrow_mut().clear();
    let bad = realm.function("get cause", 0, false, |_, _, _| {
        Err(Thrown(Value::str("cause")))
    });
    ops::define_accessor(&parent, Key::name("cause"), Some(bad), None);
    let thrown = ops::call(
        &mut realm,
        &base,
        Value::Undefined,
        &[Value::Object(message), Value::Object(options.clone())],
    )
    .unwrap_err();
    assert!(ops::strict_equals(&thrown.0, &Value::str("cause")));
    assert_eq!(&*log.borrow(), "m");
    let value = ops::construct(
        &mut realm,
        &suppressed,
        &[
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Undefined,
            Value::Object(options),
        ],
    )
    .unwrap();
    assert!(
        value
            .as_object()
            .unwrap()
            .own_property(&Key::name("cause"))
            .is_none()
    );
    assert!(
        value
            .as_object()
            .unwrap()
            .own_property(&Key::name("error"))
            .is_some()
    );
    assert!(
        value
            .as_object()
            .unwrap()
            .own_property(&Key::name("suppressed"))
            .is_some()
    );
    let value = ops::construct(&mut realm, &aggregate, &[Value::str("📒a")]).unwrap();
    let list = ops::get(&mut realm, &value, &Key::name("errors")).unwrap();
    assert!(list.as_object().unwrap().is_array());
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &list, &Key::name("length")).unwrap(),
        &Value::Number(2.0)
    ));
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &list, &Key::Index(0)).unwrap(),
        &Value::str("📒")
    ));
}

fn text(realm: &mut Realm, value: &Value, key: &str) -> String {
    let value = ops::get(realm, value, &Key::name(key)).unwrap();
    ops::to_string(realm, &value).unwrap().to_rust_lossy()
}

#[test]
fn errors_have_v8_lazy_stack_headers() {
    let mut realm = Realm::new();
    let base = global(&mut realm, "Error");
    let error = ops::construct(&mut realm, &base, &[Value::str("a")]).unwrap();
    let descriptor = error
        .as_object()
        .unwrap()
        .own_property(&Key::name("stack"))
        .unwrap();
    assert!(!descriptor.enumerable && descriptor.configurable);
    assert!(matches!(descriptor.slot, Slot::Accessor { .. }));
    // The header reads the error when the stack is first read.
    ops::set(&mut realm, &error, &Key::name("message"), Value::str("b")).unwrap();
    assert_eq!(text(&mut realm, &error, "stack"), "Error: b");
    ops::set(&mut realm, &error, &Key::name("message"), Value::str("c")).unwrap();
    assert_eq!(text(&mut realm, &error, "stack"), "Error: b");
    ops::set(
        &mut realm,
        &error,
        &Key::name("stack"),
        Value::str("custom"),
    )
    .unwrap();
    assert_eq!(text(&mut realm, &error, "stack"), "custom");

    let thrown = Value::Object(realm.error(ErrorKind::Type, "x"));
    assert_eq!(text(&mut realm, &thrown, "stack"), "TypeError: x");
    let coded = Value::Object(crate::node_error::object(
        &mut realm,
        ErrorKind::Range,
        "ERR_OUT_OF_RANGE",
        "y",
    ));
    assert!(text(&mut realm, &coded, "stack").ends_with(": y"));

    let (get, set) = realm.error_stack.clone().unwrap();
    assert!(matches!(
        ops::call(&mut realm, &Value::Object(get), Value::Number(1.0), &[]).unwrap(),
        Value::Undefined
    ));
    assert!(matches!(
        ops::call(
            &mut realm,
            &Value::Object(set),
            Value::Number(1.0),
            &[Value::str("z")]
        )
        .unwrap(),
        Value::Undefined
    ));
}

#[test]
fn capture_stack_trace_gives_objects_a_stack() {
    let mut realm = Realm::new();
    let base = global(&mut realm, "Error");
    let capture = ops::get(&mut realm, &base, &Key::name("captureStackTrace")).unwrap();
    let object = Value::Object(realm.object());
    ops::call(
        &mut realm,
        &capture,
        base.clone(),
        std::slice::from_ref(&object),
    )
    .unwrap();
    assert_eq!(text(&mut realm, &object, "stack"), "Error");
    let named = realm.object();
    named.define(Key::name("name"), Property::data(Value::str("N")));
    named.define(Key::name("message"), Property::data(Value::str("M")));
    let named = Value::Object(named);
    ops::call(
        &mut realm,
        &capture,
        base.clone(),
        std::slice::from_ref(&named),
    )
    .unwrap();
    assert_eq!(text(&mut realm, &named, "stack"), "N: M");
    assert!(ops::call(&mut realm, &capture, base.clone(), &[Value::Number(1.0)]).is_err());
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &base, &Key::name("stackTraceLimit")).unwrap(),
        &Value::Number(10.0)
    ));
}
