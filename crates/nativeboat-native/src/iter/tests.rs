use super::*;
use crate::Slot;

mod async_iteration;

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn message(realm: &mut Realm, thrown: Thrown) -> String {
    text(realm, &thrown.0)
}

fn texts(realm: &mut Realm, values: &[Value]) -> Vec<String> {
    values.iter().map(|value| text(realm, value)).collect()
}

#[test]
fn array_iterators_preserve_large_indexes_and_clamp_lengths() {
    let mut realm = Realm::new();
    let maximum = (1_u64 << 53) - 1;
    for index in [u64::from(u32::MAX) - 1, u64::from(u32::MAX), maximum - 1] {
        for yields in [Yield::Keys, Yield::Values, Yield::Entries] {
            let target = realm.object();
            target.define(
                Key::name("length"),
                Property::data(Value::Number((index + 1) as f64)),
            );
            target.define(
                crate::array_builtin::index_key(index),
                Property::data(Value::str("large")),
            );
            // Seed a reachable boundary state instead of executing billions
            // of next() calls. Advance through the public next implementation.
            let iterator = iterator_object(
                &realm,
                State::Array {
                    target: Value::Object(target.clone()),
                    index,
                    yields,
                },
                false,
            );
            let step = next(&mut realm, iterator.clone(), false).unwrap();
            let value = ops::get(&mut realm, &step, &Key::name("value")).unwrap();
            let expected = match yields {
                Yield::Keys => index.to_string(),
                Yield::Values => "large".to_owned(),
                Yield::Entries => format!("{index},large"),
            };
            assert_eq!(text(&mut realm, &value), expected);
            let step = next(&mut realm, iterator.clone(), false).unwrap();
            assert!(ops::to_boolean(
                &ops::get(&mut realm, &step, &Key::name("done")).unwrap()
            ));
            target.define(
                Key::name("length"),
                Property::data(Value::Number(f64::INFINITY)),
            );
            let step = next(&mut realm, iterator, false).unwrap();
            assert!(ops::to_boolean(
                &ops::get(&mut realm, &step, &Key::name("done")).unwrap()
            ));
        }
    }
    for length in [f64::INFINITY, maximum as f64 + 100.0] {
        let target = realm.object();
        target.define(Key::name("length"), Property::data(Value::Number(length)));
        let iterator = iterator_object(
            &realm,
            State::Array {
                target: Value::Object(target),
                index: maximum - 1,
                yields: Yield::Keys,
            },
            false,
        );
        let step = next(&mut realm, iterator.clone(), false).unwrap();
        let value = ops::get(&mut realm, &step, &Key::name("value")).unwrap();
        assert_eq!(text(&mut realm, &value), (maximum - 1).to_string());
        let step = next(&mut realm, iterator, false).unwrap();
        assert!(ops::to_boolean(
            &ops::get(&mut realm, &step, &Key::name("done")).unwrap()
        ));
    }
}

/// An iterable object whose iterator is built from Rust closures.
fn iterable(
    realm: &Realm,
    next: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
    finish: Option<Value>,
) -> Value {
    let iterator = realm.object();
    let next = realm.function("next", 0, false, next);
    iterator.define(Key::name("next"), Property::data(Value::Object(next)));
    if let Some(finish) = finish {
        iterator.define(Key::name("return"), Property::data(finish));
    }
    let source = realm.object();
    let make = realm.function("iterator", 0, false, move |_, _, _| {
        Ok(Value::Object(iterator.clone()))
    });
    source.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(Value::Object(make)),
    );
    Value::Object(source)
}

#[test]
fn arrays_strings_and_iterators_iterate() {
    let mut realm = Realm::new();
    let array = Value::Object(realm.array(vec![Value::Number(1.0), Value::str("two")]));
    let values = collect(&mut realm, &array).unwrap();
    assert_eq!(texts(&mut realm, &values), ["1", "two"]);
    let word = Value::String(JsStr::from_units(&[0x61, 0xD83D, 0xDE00, 0xD800, 0x62]));
    let points = collect(&mut realm, &word).unwrap();
    let lengths: Vec<usize> = points
        .iter()
        .map(|point| match point {
            Value::String(text) => text.len(),
            _ => 0,
        })
        .collect();
    assert_eq!(lengths, [1, 2, 1, 1]);
    for (method, expected) in [("keys", "0,1"), ("entries", "0,1,1,two")] {
        let function = ops::get(&mut realm, &array, &Key::name(method)).unwrap();
        let iterator = ops::call(&mut realm, &function, array.clone(), &[]).unwrap();
        let items = collect(&mut realm, &iterator).unwrap();
        let joined = Value::Object(realm.array(items));
        assert_eq!(text(&mut realm, &joined), expected);
    }
    // An exhausted iterator stays done; array iterators read live lengths.
    let mut iteration = iterate(&mut realm, &array).unwrap();
    while iteration.step(&mut realm).unwrap().is_some() {}
    assert!(iteration.step(&mut realm).unwrap().is_none());
    let values = ops::get(&mut realm, &array, &Key::name("values")).unwrap();
    let iterator = ops::call(&mut realm, &values, array.clone(), &[]).unwrap();
    let next = ops::get(&mut realm, &iterator, &Key::name("next")).unwrap();
    for _ in 0..3 {
        ops::call(&mut realm, &next, iterator.clone(), &[]).unwrap();
    }
    let after = ops::call(&mut realm, &next, iterator.clone(), &[]).unwrap();
    let done = ops::get(&mut realm, &after, &Key::name("done")).unwrap();
    assert!(ops::to_boolean(&done));
    let string_iterator = iterate(&mut realm, &Value::str("")).unwrap();
    let mut string_iterator = string_iterator;
    assert!(string_iterator.step(&mut realm).unwrap().is_none());
    // Calling the raw String iterator's next after completion also stays done,
    // independently of GetIterator's record caching its own done flag.
    let empty_string = super::string_iterator(&realm, JsStr::from(""));
    for _ in 0..3 {
        let step = super::next(&mut realm, empty_string.clone(), true).unwrap();
        assert!(ops::to_boolean(
            &ops::get(&mut realm, &step, &Key::name("done")).unwrap()
        ));
        assert!(matches!(
            ops::get(&mut realm, &step, &Key::name("value")).unwrap(),
            Value::Undefined
        ));
    }
    // Iterators are iterable and tagged.
    let tag = text(&mut realm, &iterator);
    assert_eq!(tag, "[object Array Iterator]");
    let again = collect(&mut realm, &iterator).unwrap();
    assert!(again.is_empty());
    let generic = realm.object();
    generic.define(Key::name("length"), Property::data(Value::Number(1.0)));
    generic.define(Key::Index(0), Property::data(Value::str("like")));
    let like = ops::call(&mut realm, &values, Value::Object(generic), &[]).unwrap();
    let items = collect(&mut realm, &like).unwrap();
    assert_eq!(texts(&mut realm, &items), ["like"]);
    let other = Value::str("x");
    let string_next = {
        let key = Key::Symbol(realm.symbols.iterator.clone());
        let iterator = ops::get(&mut realm, &other, &key).unwrap();
        let iterator = ops::call(&mut realm, &iterator, other.clone(), &[]).unwrap();
        ops::get(&mut realm, &iterator, &Key::name("next")).unwrap()
    };
    for (function, receiver) in [
        (&next, Value::Null),
        (&string_next, iterator.clone()),
        (&next, Value::Object(realm.object())),
    ] {
        let error = ops::call(&mut realm, function, receiver, &[]).unwrap_err();
        assert_eq!(
            message(&mut realm, error),
            "TypeError: next called on an incompatible receiver"
        );
    }
    assert!(format!("{:?}", *iterator.as_object().unwrap().data()).contains("Iterator(Array"));
}

#[test]
fn elisions_skip_value_getters_and_keep_iterator_completion() {
    let mut realm = Realm::new();
    let values = iterable(
        &realm,
        |realm, _, _| {
            let result = realm.object();
            let getter = realm.function("value", 0, false, |realm, _, _| {
                Err(realm.throw(ErrorKind::Error, "value read"))
            });
            ops::define_accessor(&result, Key::name("value"), Some(getter), None);
            Ok(Value::Object(result))
        },
        None,
    );
    let mut iteration = iterate(&mut realm, &values).unwrap();
    iteration.skip(&mut realm).unwrap();
    let error = iteration.step(&mut realm).unwrap_err();
    assert_eq!(message(&mut realm, error), "Error: value read");
    iteration.skip(&mut realm).unwrap();
    let array = Value::Object(realm.array(vec![Value::Null]));
    let mut iteration = iterate(&mut realm, &array).unwrap();
    iteration.skip(&mut realm).unwrap();
    iteration.skip(&mut realm).unwrap();
    assert!(iteration.step(&mut realm).unwrap().is_none());
    let broken = iterable(&realm, |_, _, _| Ok(Value::Null), None);
    assert!(
        iterate(&mut realm, &broken)
            .unwrap()
            .skip(&mut realm)
            .is_err()
    );
}

#[test]
fn protocol_errors_and_closing() {
    let mut realm = Realm::new();
    for value in [Value::Undefined, Value::Number(1.0)] {
        let error = iterate(&mut realm, &value).unwrap_err();
        assert_eq!(
            message(&mut realm, error),
            "TypeError: value is not iterable"
        );
    }
    let broken = realm.object();
    let primitive = realm.function("iterator", 0, false, |_, _, _| Ok(Value::Number(1.0)));
    broken.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(Value::Object(primitive)),
    );
    let error = iterate(&mut realm, &Value::Object(broken)).unwrap_err();
    assert_eq!(
        message(&mut realm, error),
        "TypeError: Result of the Symbol.iterator method is not an object"
    );
    let bad_result = iterable(&realm, |_, _, _| Ok(Value::Null), None);
    let error = collect(&mut realm, &bad_result).unwrap_err();
    assert_eq!(
        message(&mut realm, error),
        "TypeError: Iterator result is not an object"
    );
    let throwing = iterable(
        &realm,
        |realm, _, _| Err(realm.throw(ErrorKind::Error, "next")),
        None,
    );
    let mut iteration = iterate(&mut realm, &throwing).unwrap();
    assert!(iteration.step(&mut realm).is_err());
    // A failed iterator is done: nothing more is read or closed.
    assert!(iteration.step(&mut realm).unwrap().is_none());
    iteration.close(&mut realm).unwrap();
    let counting = |realm: &Realm, finish: Option<Value>| {
        iterable(
            realm,
            |realm, _, _| {
                let result = realm.object();
                result.define(Key::name("value"), Property::data(Value::Number(7.0)));
                result.define(Key::name("done"), Property::data(Value::Boolean(false)));
                Ok(Value::Object(result))
            },
            finish,
        )
    };
    let plain = counting(&realm, None);
    let mut iteration = iterate(&mut realm, &plain).unwrap();
    let first = iteration.step(&mut realm).unwrap().unwrap();
    assert!(ops::strict_equals(&first, &Value::Number(7.0)));
    let rest_values = {
        let pair = Value::Object(realm.array(vec![Value::Null, Value::Null]));
        let mut finite = iterate(&mut realm, &pair).unwrap();
        finite.step(&mut realm).unwrap();
        rest(&mut realm, &mut finite).unwrap()
    };
    assert_eq!(rest_values.len(), 1);
    iteration.close(&mut realm).unwrap();
    let closed = realm.object();
    let log = closed.clone();
    let finish = realm.function("return", 0, false, move |_, _, _| {
        log.define(Key::name("closed"), Property::data(Value::Boolean(true)));
        Ok(Value::Object(log.clone()))
    });
    let closable = counting(&realm, Some(Value::Object(finish)));
    let mut iteration = iterate(&mut realm, &closable).unwrap();
    iteration.step(&mut realm).unwrap();
    iteration.close(&mut realm).unwrap();
    assert!(closed.own_property(&Key::name("closed")).is_some());
    let primitive_return = realm.function("return", 0, false, |_, _, _| Ok(Value::Null));
    let bad_close = counting(&realm, Some(Value::Object(primitive_return)));
    let mut iteration = iterate(&mut realm, &bad_close).unwrap();
    let error = iteration.close(&mut realm).unwrap_err();
    assert_eq!(
        message(&mut realm, error),
        "TypeError: Iterator result is not an object"
    );
    let failing_return = realm.function("return", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "return failed"))
    });
    let failing = counting(&realm, Some(Value::Object(failing_return)));
    let mut iteration = iterate(&mut realm, &failing).unwrap();
    let original = realm.throw(ErrorKind::Error, "body");
    let kept = iteration.close_thrown(&mut realm, original);
    assert_eq!(message(&mut realm, kept), "Error: body");
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "getter"))
    });
    let return_getter = realm.object();
    return_getter.define(
        Key::name("return"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    return_getter.define(Key::name("next"), Property::data(Value::Null));
    let source = realm.object();
    let target = return_getter.clone();
    let make = realm.function("iterator", 0, false, move |_, _, _| {
        Ok(Value::Object(target.clone()))
    });
    source.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(Value::Object(make)),
    );
    let mut iteration = iterate(&mut realm, &Value::Object(source)).unwrap();
    assert!(iteration.close(&mut realm).is_err());
}

#[test]
fn for_in_enumerates_enumerable_string_keys_once() {
    let mut realm = Realm::new();
    let names = |realm: &mut Realm, value: &Value| -> Vec<String> {
        for_in_keys(realm, value)
            .unwrap()
            .iter()
            .map(|key| key.to_js().to_rust_lossy())
            .collect()
    };
    assert!(names(&mut realm, &Value::Undefined).is_empty());
    assert!(names(&mut realm, &Value::Null).is_empty());
    realm
        .string_prototype
        .define(Key::name("extra"), Property::data(Value::Null));
    assert_eq!(names(&mut realm, &Value::str("ab")), ["0", "1", "extra"]);
    for value in [
        Value::Number(1.0),
        Value::Boolean(true),
        Value::Symbol(crate::Symbol::new(None)),
    ] {
        assert!(names(&mut realm, &value).is_empty());
    }
    let parent = realm.object();
    parent.define(Key::name("inherited"), Property::data(Value::Null));
    parent.define(Key::name("shadowed"), Property::data(Value::Null));
    let child = Object::new(Some(parent), Kind::Ordinary);
    child.define(Key::name("own"), Property::data(Value::Null));
    child.define(Key::name("shadowed"), Property::hidden(Value::Null));
    child.define(
        Key::Symbol(crate::Symbol::new(None)),
        Property::data(Value::Null),
    );
    assert_eq!(
        names(&mut realm, &Value::Object(child)),
        ["own", "inherited"]
    );
    let _ = &mut realm;
}

#[test]
fn copying_properties_for_spread_and_rest() {
    let mut realm = Realm::new();
    let source = realm.object();
    source.define(Key::name("a"), Property::data(Value::Number(1.0)));
    source.define(Key::name("hidden"), Property::hidden(Value::Null));
    source.define(Key::name("skip"), Property::data(Value::Null));
    let symbol = crate::Symbol::new(None);
    source.define(Key::Symbol(symbol.clone()), Property::data(Value::Null));
    let target = realm.object();
    copy_properties(
        &mut realm,
        &target,
        &Value::Object(source),
        &[Key::name("skip")],
    )
    .unwrap();
    let keys: Vec<_> = target
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    assert_eq!(keys, ["a", "Symbol()"]);
    let letters = realm.object();
    copy_properties(&mut realm, &letters, &Value::str("hi"), &[]).unwrap();
    assert_eq!(letters.own_keys().len(), 2);
    let nothing = realm.object();
    copy_properties(&mut realm, &nothing, &Value::Null, &[]).unwrap();
    copy_properties(&mut realm, &nothing, &Value::Number(3.0), &[]).unwrap();
    assert!(nothing.own_keys().is_empty());
    let failing = realm.object();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "get"))
    });
    failing.define(
        Key::name("trap"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert!(copy_properties(&mut realm, &nothing, &Value::Object(failing), &[]).is_err());
}
