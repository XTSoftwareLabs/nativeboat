use super::*;
use std::cell::RefCell;

fn method(realm: &mut Realm, name: &str) -> Value {
    let prototype = Value::Object(realm.function_prototype.clone());
    ops::get(realm, &prototype, &Key::name(name)).unwrap()
}

fn value(realm: &mut Realm, object: &Value, name: &str) -> Value {
    ops::get(realm, object, &Key::name(name)).unwrap()
}

fn equal(actual: &Value, expected: &Value) {
    assert!(
        ops::strict_equals(actual, expected),
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn apply_reads_array_like_properties_in_order_and_preserves_receiver_and_throws() {
    let mut realm = Realm::new();
    let apply = method(&mut realm, "apply");
    for (name, length) in [("apply", 2.0), ("bind", 1.0)] {
        let function = method(&mut realm, name);
        equal(&value(&mut realm, &function, "name"), &Value::str(name));
        equal(
            &value(&mut realm, &function, "length"),
            &Value::Number(length),
        );
        assert!(ops::construct(&mut realm, &function, &[]).is_err());
        let property = realm
            .function_prototype
            .own_property(&Key::name(name))
            .unwrap();
        assert!(!property.enumerable && property.configurable);
        assert!(matches!(property.slot, Slot::Data { writable: true, .. }));
    }
    let receiver = Value::Object(realm.object());
    let expected = receiver.clone();
    let target = Value::Object(realm.function("check", 0, false, move |_, this, args| {
        equal(&this, &expected);
        Ok(Value::str(&format!("{:?}", args)))
    }));
    for absent in [Value::Null, Value::Undefined] {
        equal(
            &ops::call(
                &mut realm,
                &apply,
                target.clone(),
                &[receiver.clone(), absent],
            )
            .unwrap(),
            &Value::str("[]"),
        );
    }
    let trace = Rc::new(RefCell::new(String::new()));
    let parent = realm.object();
    parent.define(Key::Index(1), Property::data(Value::Number(8.0)));
    let array = Object::new(Some(parent), Kind::Ordinary);
    for (key, mark, returned) in [
        (Key::name("length"), 'l', Value::Number(3.9)),
        (Key::Index(0), '0', Value::str("a")),
        (Key::Index(2), '2', Value::Undefined),
    ] {
        let trace = trace.clone();
        let getter = realm.function("get", 0, false, move |_, _, _| {
            trace.borrow_mut().push(mark);
            Ok(returned.clone())
        });
        ops::define_accessor(&array, key, Some(getter), None);
    }
    let result = ops::call(
        &mut realm,
        &apply,
        target.clone(),
        &[receiver.clone(), Value::Object(array.clone())],
    )
    .unwrap();
    equal(
        &result,
        &Value::str("[String(\"a\"), Number(8.0), Undefined]"),
    );
    assert_eq!(&*trace.borrow(), "l02");
    // Callable validation precedes even the array-like length getter.
    trace.borrow_mut().clear();
    assert!(
        ops::call(
            &mut realm,
            &apply,
            Value::Null,
            &[receiver.clone(), Value::Object(array.clone())]
        )
        .is_err()
    );
    assert!(trace.borrow().is_empty());
    for primitive in [
        Value::str("ab"),
        Value::Number(2.0),
        Value::Boolean(true),
        Value::Symbol(crate::Symbol::new(None)),
    ] {
        assert!(
            ops::call(
                &mut realm,
                &apply,
                target.clone(),
                &[receiver.clone(), primitive]
            )
            .is_err()
        );
    }
    for length in [
        Value::Number(f64::NAN),
        Value::Number(-2.0),
        Value::Undefined,
        Value::str("0"),
        Value::Number(f64::NEG_INFINITY),
    ] {
        let array = realm.object();
        array.define(Key::name("length"), Property::data(length));
        equal(
            &ops::call(
                &mut realm,
                &apply,
                target.clone(),
                &[receiver.clone(), Value::Object(array)],
            )
            .unwrap(),
            &Value::str("[]"),
        );
    }
    let primitive = realm.object();
    let trace_conversion = trace.clone();
    realm.method(&primitive, "valueOf", 0, move |_, _, _| {
        trace_conversion.borrow_mut().push('v');
        Ok(Value::Number(1.0))
    });
    array.define(
        Key::name("length"),
        Property::data(Value::Object(primitive)),
    );
    trace.borrow_mut().clear();
    ops::call(
        &mut realm,
        &apply,
        target.clone(),
        &[receiver.clone(), Value::Object(array.clone())],
    )
    .unwrap();
    assert_eq!(&*trace.borrow(), "v0");
    array.define(
        Key::name("length"),
        Property::data(Value::Symbol(crate::Symbol::new(None))),
    );
    assert!(
        ops::call(
            &mut realm,
            &apply,
            target.clone(),
            &[receiver.clone(), Value::Object(array.clone())]
        )
        .is_err()
    );
    array.define(Key::name("length"), Property::data(Value::Number(2.0)));
    let fail = realm.function("get", 0, false, |_, _, _| Err(Thrown(Value::str("index"))));
    ops::define_accessor(&array, Key::Index(0), Some(fail), None);
    let error = ops::call(
        &mut realm,
        &apply,
        target,
        &[receiver, Value::Object(array)],
    )
    .unwrap_err();
    equal(&error.0, &Value::str("index"));
    let failing = Value::Object(realm.function("fail", 0, false, |_, this, _| Err(Thrown(this))));
    equal(
        &ops::call(&mut realm, &apply, failing, &[Value::str("body")])
            .unwrap_err()
            .0,
        &Value::str("body"),
    );
}

#[test]
fn bind_metadata_snapshot_getters_and_nonconstructible_calls() {
    let mut realm = Realm::new();
    let bind = method(&mut realm, "bind");
    assert!(ops::call(&mut realm, &bind, Value::Undefined, &[]).is_err());
    let target = realm.function("target", 3, false, |realm, this, args| {
        let mut values = vec![this];
        values.extend_from_slice(args);
        Ok(Value::Object(realm.array(values)))
    });
    let target_value = Value::Object(target.clone());
    let bound = ops::call(
        &mut realm,
        &bind,
        target_value.clone(),
        &[Value::Null, Value::Number(1.0)],
    )
    .unwrap();
    assert_eq!(ops::type_of(&bound), "function");
    equal(
        &value(&mut realm, &bound, "name"),
        &Value::str("bound target"),
    );
    equal(&value(&mut realm, &bound, "length"), &Value::Number(2.0));
    assert_eq!(
        format!("{:?}", bound.as_object().unwrap().data().kind),
        "BoundFunction(false)"
    );
    assert!(
        bound
            .as_object()
            .unwrap()
            .own_property(&Key::name("prototype"))
            .is_none()
    );
    for key in ["name", "length"] {
        let descriptor = bound
            .as_object()
            .unwrap()
            .own_property(&Key::name(key))
            .unwrap();
        assert!(!descriptor.enumerable && descriptor.configurable);
        assert!(matches!(
            descriptor.slot,
            Slot::Data {
                writable: false,
                ..
            }
        ));
        assert!(ops::set(&mut realm, &bound, &Key::name(key), Value::Undefined).is_err());
    }
    let result = ops::call(
        &mut realm,
        &bound,
        Value::Number(99.0),
        &[Value::Number(2.0)],
    )
    .unwrap();
    equal(&value(&mut realm, &result, "length"), &Value::Number(3.0));
    equal(
        &ops::get(&mut realm, &result, &Key::Index(0)).unwrap(),
        &Value::Null,
    );
    equal(
        &ops::get(&mut realm, &result, &Key::Index(1)).unwrap(),
        &Value::Number(1.0),
    );
    let nested = ops::call(
        &mut realm,
        &bind,
        bound.clone(),
        &[Value::str("ignored"), Value::Number(3.0)],
    )
    .unwrap();
    equal(
        &value(&mut realm, &nested, "name"),
        &Value::str("bound bound target"),
    );
    let result = ops::call(&mut realm, &nested, Value::Undefined, &[]).unwrap();
    equal(
        &ops::get(&mut realm, &result, &Key::Index(0)).unwrap(),
        &Value::Null,
    );
    equal(
        &ops::get(&mut realm, &result, &Key::Index(2)).unwrap(),
        &Value::Number(3.0),
    );
    assert!(ops::construct(&mut realm, &nested, &[]).is_err());
    assert!(!ops::instance_of(&mut realm, &Value::Null, &nested).unwrap());
    let ordinary = Value::Object(realm.object());
    assert!(ops::instance_of(&mut realm, &ordinary, &nested).is_err());
    let object_to_string = {
        let prototype = Value::Object(realm.object_prototype.clone());
        value(&mut realm, &prototype, "toString")
    };
    equal(
        &ops::call(&mut realm, &object_to_string, bound, &[]).unwrap(),
        &Value::str("[object Function]"),
    );
    for (length, expected) in [
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, 0.0),
        (f64::NAN, 0.0),
        (-1.0, 0.0),
        (3.9, 2.0),
    ] {
        target.define(Key::name("length"), Property::data(Value::Number(length)));
        let bound = ops::call(
            &mut realm,
            &bind,
            target_value.clone(),
            &[Value::Undefined, Value::Null],
        )
        .unwrap();
        equal(
            &value(&mut realm, &bound, "length"),
            &Value::Number(expected),
        );
    }
    target.define(Key::name("length"), Property::data(Value::str("5")));
    target.define(
        Key::name("name"),
        Property::data(Value::String(JsStr::from_units(&[0xd800]))),
    );
    let bound = ops::call(&mut realm, &bind, target_value.clone(), &[]).unwrap();
    equal(&value(&mut realm, &bound, "length"), &Value::Number(0.0));
    equal(
        &value(&mut realm, &bound, "name"),
        &Value::String(JsStr::from("bound ").concat(&JsStr::from_units(&[0xd800]))),
    );
    let trace = Rc::new(RefCell::new(String::new()));
    let prototype = realm.object();
    let mutated = realm.object();
    target.data_mut().prototype = Some(prototype.clone());
    let changed_target = target.clone();
    let changed_trace = trace.clone();
    let length_getter = realm.function("length", 0, false, move |_, _, _| {
        changed_trace.borrow_mut().push('l');
        changed_target.data_mut().prototype = Some(mutated.clone());
        Ok(Value::Number(4.0))
    });
    ops::define_accessor(&target, Key::name("length"), Some(length_getter), None);
    let changed_trace = trace.clone();
    let name_getter = realm.function("name", 0, false, move |_, _, _| {
        changed_trace.borrow_mut().push('n');
        Ok(Value::Number(42.0))
    });
    ops::define_accessor(&target, Key::name("name"), Some(name_getter), None);
    let bound = ops::call(&mut realm, &bind, target_value.clone(), &[]).unwrap();
    assert_eq!(
        bound.as_object().unwrap().data().prototype,
        Some(prototype.clone())
    );
    equal(&value(&mut realm, &bound, "name"), &Value::str("bound "));
    assert_eq!(&*trace.borrow(), "ln");
    let fail = realm.function("get", 0, false, |_, _, _| {
        Err(Thrown(Value::str("metadata")))
    });
    ops::define_accessor(&target, Key::name("length"), Some(fail.clone()), None);
    trace.borrow_mut().clear();
    equal(
        &ops::call(&mut realm, &bind, target_value.clone(), &[])
            .unwrap_err()
            .0,
        &Value::str("metadata"),
    );
    assert!(trace.borrow().is_empty());
    assert!(target.delete(&Key::name("length")));
    target.data_mut().prototype = Some(prototype.clone());
    ops::define_accessor(&prototype, Key::name("length"), Some(fail.clone()), None);
    let bound = ops::call(&mut realm, &bind, target_value.clone(), &[]).unwrap();
    equal(&value(&mut realm, &bound, "length"), &Value::Number(0.0));
    ops::define_accessor(&target, Key::name("name"), Some(fail), None);
    equal(
        &ops::call(&mut realm, &bind, target_value, &[])
            .unwrap_err()
            .0,
        &Value::str("metadata"),
    );
}

#[test]
fn bound_construction_new_targets_instance_hooks_and_depth_are_native() {
    let mut realm = Realm::new();
    let bind = method(&mut realm, "bind");
    let target = Value::Object(realm.function("Point", 2, true, |realm, this, args| {
        ops::set(realm, &this, &Key::name("x"), args[0].clone())?;
        ops::set(realm, &this, &Key::name("y"), args[1].clone())?;
        Ok(Value::Undefined)
    }));
    let bound = ops::call(
        &mut realm,
        &bind,
        target.clone(),
        &[Value::Null, Value::Number(3.0)],
    )
    .unwrap();
    assert_eq!(
        format!("{:?}", bound.as_object().unwrap().data().kind),
        "BoundFunction(true)"
    );
    // A user-defined own prototype does not change default bound construction.
    let ignored_prototype = Value::Object(realm.object());
    ops::set(
        &mut realm,
        &bound,
        &Key::name("prototype"),
        ignored_prototype,
    )
    .unwrap();
    let instance = ops::construct(&mut realm, &bound, &[Value::Number(4.0)]).unwrap();
    equal(&value(&mut realm, &instance, "x"), &Value::Number(3.0));
    equal(&value(&mut realm, &instance, "y"), &Value::Number(4.0));
    assert!(ops::instance_of(&mut realm, &instance, &target).unwrap());
    assert!(ops::instance_of(&mut realm, &instance, &bound).unwrap());
    let other = Value::Object(realm.function("Other", 0, true, |_, _, _| Ok(Value::Undefined)));
    let other_instance =
        ops::construct_target(&mut realm, &bound, &[Value::Number(5.0)], &other).unwrap();
    assert!(ops::instance_of(&mut realm, &other_instance, &other).unwrap());
    assert!(!ops::instance_of(&mut realm, &other_instance, &bound).unwrap());
    let nested = ops::call(
        &mut realm,
        &bind,
        bound.clone(),
        &[Value::Undefined, Value::Number(6.0)],
    )
    .unwrap();
    let instance = ops::construct(&mut realm, &nested, &[]).unwrap();
    equal(&value(&mut realm, &instance, "y"), &Value::Number(6.0));
    assert!(ops::instance_of(&mut realm, &instance, &nested).unwrap());
    let hook = realm.function("hasInstance", 1, false, |_, _, args| {
        Ok(Value::Boolean(matches!(args[0], Value::Number(_))))
    });
    target.as_object().unwrap().define(
        Key::Symbol(realm.symbols.has_instance.clone()),
        Property::data(Value::Object(hook)),
    );
    assert!(ops::instance_of(&mut realm, &Value::Number(1.0), &bound).unwrap());
    assert!(!ops::instance_of(&mut realm, &instance, &bound).unwrap());
    let error_constructor = {
        let global = Value::Object(realm.global.clone());
        value(&mut realm, &global, "TypeError")
    };
    let error = ops::call(
        &mut realm,
        &bind,
        error_constructor.clone(),
        &[Value::Null, Value::str("bound error")],
    )
    .unwrap();
    for result in [
        ops::construct(&mut realm, &error, &[]).unwrap(),
        ops::call(&mut realm, &error, Value::Null, &[]).unwrap(),
    ] {
        assert!(ops::instance_of(&mut realm, &result, &error_constructor).unwrap());
        equal(
            &value(&mut realm, &result, "message"),
            &Value::str("bound error"),
        );
    }
    let returned = Value::Object(realm.object());
    let captured = returned.clone();
    let factory =
        Value::Object(realm.function("Factory", 0, true, move |_, _, _| Ok(captured.clone())));
    let factory = ops::call(&mut realm, &bind, factory, &[]).unwrap();
    equal(
        &ops::construct(&mut realm, &factory, &[]).unwrap(),
        &returned,
    );
    let failing = Value::Object(realm.function("Fail", 0, true, |_, _, _| {
        Err(Thrown(Value::str("construct")))
    }));
    let failing = ops::call(&mut realm, &bind, failing, &[]).unwrap();
    equal(
        &ops::construct(&mut realm, &failing, &[]).unwrap_err().0,
        &Value::str("construct"),
    );
    realm.max_depth = 1;
    assert!(ops::construct(&mut realm, &nested, &[]).is_err());
    assert!(ops::call(&mut realm, &nested, Value::Undefined, &[]).is_err());
    assert!(ops::instance_of(&mut realm, &Value::Number(1.0), &nested).is_err());
    realm.max_depth = 10;
    ops::construct(&mut realm, &nested, &[]).unwrap();
    assert!(ops::instance_of(&mut realm, &Value::Number(1.0), &nested).unwrap());
    ops::call(&mut realm, &factory, Value::Undefined, &[]).unwrap();
}

#[test]
fn native_argument_resource_limit_rejects_before_index_reads_or_target_calls() {
    let mut realm = Realm::new();
    let apply = method(&mut realm, "apply");
    let bind = method(&mut realm, "bind");
    let target = Value::Object(realm.function("count", 0, true, |_, _, args| {
        Ok(Value::Number(args.len() as f64))
    }));
    let array = realm.object();
    let fail = realm.function("get", 0, false, |_, _, _| {
        Err(Thrown(Value::str("must not read index")))
    });
    ops::define_accessor(&array, Key::Index(0), Some(fail), None);
    for length in [
        f64::INFINITY,
        (MAX_CALL_ARGUMENTS + 1) as f64,
        9_007_199_254_740_991.0,
    ] {
        array.define(Key::name("length"), Property::data(Value::Number(length)));
        let error = ops::call(
            &mut realm,
            &apply,
            target.clone(),
            &[Value::Undefined, Value::Object(array.clone())],
        )
        .unwrap_err();
        let text = ops::to_string(&mut realm, &error.0).unwrap();
        assert_eq!(
            text.to_rust_lossy(),
            "RangeError: Native call argument limit exceeded"
        );
    }
    array.delete(&Key::Index(0));
    array.define(
        Key::name("length"),
        Property::data(Value::Number(MAX_CALL_ARGUMENTS as f64)),
    );
    equal(
        &ops::call(
            &mut realm,
            &apply,
            target.clone(),
            &[Value::Undefined, Value::Object(array)],
        )
        .unwrap(),
        &Value::Number(MAX_CALL_ARGUMENTS as f64),
    );
    let leading = vec![Value::Undefined; MAX_CALL_ARGUMENTS + 1];
    let bound = ops::call(&mut realm, &bind, target.clone(), &leading).unwrap();
    equal(
        &ops::call(&mut realm, &bound, Value::Undefined, &[]).unwrap(),
        &Value::Number(MAX_CALL_ARGUMENTS as f64),
    );
    assert!(ops::call(&mut realm, &bound, Value::Undefined, &[Value::Null]).is_err());
    assert!(ops::construct(&mut realm, &bound, &[Value::Null]).is_err());
    let excessive = vec![Value::Undefined; MAX_CALL_ARGUMENTS + 2];
    assert!(ops::call(&mut realm, &bind, target, &excessive).is_err());
}

#[test]
fn the_function_constructor_refuses_source_text() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    let constructor = value(&mut realm, &global, "Function");
    let prototype = value(&mut realm, &constructor, "prototype");
    equal(&prototype, &Value::Object(realm.function_prototype.clone()));
    let back = value(&mut realm, &prototype, "constructor");
    equal(&back, &constructor);
    for args in [vec![], vec![Value::str("  ")]] {
        let empty = ops::construct(&mut realm, &constructor, &args).unwrap();
        let result = ops::call(&mut realm, &empty, Value::Undefined, &[]).unwrap();
        equal(&result, &Value::Undefined);
    }
    for body in ["return this", " return this; "] {
        let function = ops::call(
            &mut realm,
            &constructor,
            Value::Undefined,
            &[Value::str(body)],
        );
        let result = ops::call(&mut realm, &function.unwrap(), Value::Undefined, &[]).unwrap();
        equal(&result, &global);
    }
    for args in [
        vec![Value::str("a"), Value::str("return a")],
        vec![Value::str("a"), Value::str("")],
        vec![Value::str("return 1")],
    ] {
        let error = ops::construct(&mut realm, &constructor, &args)
            .unwrap_err()
            .0;
        let text = ops::to_string(&mut realm, &error).unwrap().to_rust_lossy();
        assert_eq!(
            text,
            "EvalError: Code generation from strings disallowed for this context"
        );
    }
    let symbol = Value::Symbol(crate::Symbol::new(None));
    assert!(ops::construct(&mut realm, &constructor, &[symbol]).is_err());
}

#[test]
fn eval_has_stable_identity_without_evaluating_or_coercing_input() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    let intrinsic = value(&mut realm, &global, "eval");
    equal(&value(&mut realm, &global, "eval"), &intrinsic);
    equal(&value(&mut realm, &intrinsic, "name"), &Value::str("eval"));
    equal(
        &value(&mut realm, &intrinsic, "length"),
        &Value::Number(1.0),
    );
    let object = realm.object();
    let coercion = realm.function("toString", 0, false, |_, _, _| {
        panic!("eval must not coerce non-string input")
    });
    object.define(
        Key::name("toString"),
        Property::data(Value::Object(coercion)),
    );
    for input in [
        Value::Undefined,
        Value::Null,
        Value::Boolean(true),
        Value::Number(42.0),
        Value::Object(object),
        Value::Symbol(crate::Symbol::new(None)),
    ] {
        let result = ops::call(
            &mut realm,
            &intrinsic,
            Value::Undefined,
            core::slice::from_ref(&input),
        )
        .unwrap();
        equal(&result, &input);
    }
    equal(
        &ops::call(&mut realm, &intrinsic, Value::Undefined, &[]).unwrap(),
        &Value::Undefined,
    );
    for source in ["", "globalThis.ran = true", "1 + 1"] {
        let thrown = ops::call(
            &mut realm,
            &intrinsic,
            Value::Undefined,
            &[Value::str(source)],
        )
        .unwrap_err();
        assert_eq!(
            ops::to_string(&mut realm, &thrown.0)
                .unwrap()
                .to_rust_lossy(),
            "EvalError: Code generation from strings disallowed for this context"
        );
    }
    assert!(ops::construct(&mut realm, &intrinsic, &[]).is_err());
    equal(&value(&mut realm, &global, "ran"), &Value::Undefined);
}

#[test]
fn to_string_returns_kept_source_text_or_the_native_code_form() {
    let mut realm = Realm::new();
    let to_string = method(&mut realm, "toString");
    equal(
        &value(&mut realm, &to_string, "name"),
        &Value::str("toString"),
    );
    equal(
        &value(&mut realm, &to_string, "length"),
        &Value::Number(0.0),
    );
    let property = realm
        .function_prototype
        .own_property(&Key::name("toString"))
        .unwrap();
    assert!(!property.enumerable && property.configurable);

    // Functions without kept text read as native functions do.
    let plain = Value::Object(realm.function("greet", 1, false, |_, _, _| Ok(Value::Undefined)));
    let text = ops::call(&mut realm, &to_string, plain.clone(), &[]).unwrap();
    equal(&text, &Value::str("function greet() { [native code] }"));
    let bind = method(&mut realm, "bind");
    let bound = ops::call(&mut realm, &bind, plain, &[Value::Undefined]).unwrap();
    let text = ops::call(&mut realm, &to_string, bound, &[]).unwrap();
    equal(&text, &Value::str("function () { [native code] }"));

    // A function the compiler kept the text of returns that text, also when
    // another realm of the agent reads it.
    let source = "function fib(n) {\n\treturn n <= 2 ? n : fib(n - 1) + fib(n - 2);\n}";
    let kept = with_source_text(
        &realm,
        Value::Object(realm.function("fib", 1, true, |_, _, _| Ok(Value::Undefined))),
        source,
    );
    let text = ops::call(&mut realm, &to_string, kept.clone(), &[]).unwrap();
    equal(&text, &Value::str(source));
    let concatenated = ops::add(&mut realm, &Value::str("("), &kept).unwrap();
    equal(&concatenated, &Value::str(&format!("({source}")));
    assert!(
        ops::own_property_keys(&mut realm, kept.as_object().unwrap())
            .unwrap()
            .iter()
            .all(|key| !matches!(key, Key::Symbol(_))),
        "the kept text is not a visible property"
    );

    // Other values are refused.
    for receiver in [
        Value::Object(realm.object()),
        Value::Number(1.0),
        Value::Undefined,
    ] {
        let error = ops::call(&mut realm, &to_string, receiver, &[]).unwrap_err();
        let message = format!("{:?}", error);
        assert!(!message.is_empty());
    }
}
