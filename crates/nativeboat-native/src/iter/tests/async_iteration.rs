use super::*;
use alloc::{boxed::Box, rc::Rc};
use core::cell::Cell;

fn run(body: impl Fn(&mut Realm) -> Completion + Clone + 'static) -> String {
    let mut realm = Realm::new();
    let function = crate::coroutine::async_function(&realm, "test", 0, move |_, _, _| {
        let body = body.clone();
        Ok(Box::new(move |realm: &mut Realm| body(realm)) as crate::coroutine::Body)
    });
    let promise = ops::call(&mut realm, &Value::Object(function), Value::Undefined, &[]).unwrap();
    realm.run_jobs().unwrap();
    crate::inspect::inspect(&mut realm, &promise, 2).unwrap()
}

fn reject(realm: &mut Realm, reason: &str) -> Completion {
    let (promise, _, reject) = crate::promise_builtin::capability(realm);
    ops::call(realm, &reject, Value::Undefined, &[Value::str(reason)])?;
    Ok(promise)
}

fn getter(realm: &Realm, object: &Object, name: &str) {
    let function = realm.function("get", 0, false, move |_, _, _| {
        Err(Thrown(Value::str("getter")))
    });
    object.define(
        Key::name(name),
        Property {
            slot: Slot::Accessor {
                get: Some(function),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
}

/// Run failures through a real async body, rather than asserting on a
/// suspended fiber. The final status also records whether cleanup ran.
fn scenario(realm: &mut Realm, asynchronous: bool, mode: &'static str, thrown: bool) -> Completion {
    let returning = mode.ends_with("-signal");
    let mode = mode.strip_suffix("-signal").unwrap_or(mode);
    let count = Rc::new(Cell::new(0));
    let closed = Rc::new(Cell::new(0));
    let iterator = realm.object();
    let counter = count.clone();
    let next = realm.function("next", 0, false, move |realm, _, _| {
        if mode == "next-throw" {
            return Err(Thrown(Value::str("next")));
        }
        if mode == "next-primitive" {
            return Ok(Value::Number(1.0));
        }
        if mode == "reject" && asynchronous {
            return reject(realm, "rejected");
        }
        let value = if mode == "reject" {
            reject(realm, "rejected")?
        } else {
            Value::Number(1.0)
        };
        let step = result(realm, value, counter.replace(counter.get() + 1) != 0);
        if matches!(mode, "done-get" | "value-get") {
            getter(
                realm,
                step.as_object().unwrap(),
                if mode == "done-get" { "done" } else { "value" },
            );
        }
        Ok(step)
    });
    iterator.define(Key::name("next"), Property::data(Value::Object(next)));
    let closes = closed.clone();
    let finish = realm.function("return", 0, false, move |realm, _, _| {
        closes.set(closes.get() + 1);
        match mode {
            "return-throw" => Err(Thrown(Value::str("close"))),
            "return-primitive" => Ok(Value::Number(1.0)),
            "return-reject" if asynchronous => reject(realm, "close rejected"),
            "return-reject" => {
                let value = reject(realm, "close rejected")?;
                Ok(result(realm, value, true))
            }
            _ => Ok(result(realm, Value::Number(2.0), true)),
        }
    });
    if mode != "missing" {
        iterator.define(Key::name("return"), Property::data(Value::Object(finish)));
    }
    if mode == "return-get" {
        getter(realm, &iterator, "return");
    }
    let make = realm.function("iterator", 0, false, move |_, _, _| {
        Ok(Value::Object(iterator.clone()))
    });
    let source = realm.object();
    let symbol = if asynchronous {
        &realm.symbols.async_iterator
    } else {
        &realm.symbols.iterator
    };
    source.define(
        Key::Symbol(symbol.clone()),
        Property::data(Value::Object(make)),
    );
    let mut iteration = iterate_async(realm, &Value::Object(source))?;
    let outcome = (|| {
        let mut values = Vec::new();
        while let Some(value) = iteration.step(realm)? {
            values.push(text(realm, &value));
            if thrown {
                let signal = if returning {
                    Value::Object(realm.return_signal.clone())
                } else {
                    Value::str("original")
                };
                return Err(iteration.close_thrown(realm, Thrown(signal)));
            }
            if mode != "normal" {
                iteration.close(realm)?;
                break;
            }
        }
        // Repeated steps and closes on a finished record are inert.
        if mode == "normal" {
            iteration.step(realm)?;
        }
        iteration.close(realm)?;
        Ok(values.join(","))
    })();
    let status = match outcome {
        Ok(value) => value,
        Err(thrown) => message(realm, thrown),
    };
    Ok(Value::str(&format!("{status}/{}", closed.get())))
}

#[test]
fn async_iteration_cleanup_and_failures() {
    for asynchronous in [false, true] {
        for (mode, expected) in [
            ("normal", "1/0"),
            ("missing", "1/0"),
            ("next-throw", "next/0"),
            (
                "next-primitive",
                "TypeError: Iterator result is not an object/0",
            ),
            ("done-get", "getter/0"),
            ("value-get", "getter/0"),
            ("reject", "rejected/0"),
            ("return-get", "getter/0"),
            ("return-throw", "close/1"),
            (
                "return-primitive",
                "TypeError: Iterator result is not an object/1",
            ),
            ("return-reject", "close rejected/1"),
            ("break", "1/1"),
        ] {
            assert_eq!(
                run(move |realm| scenario(realm, asynchronous, mode, false)),
                format!("Promise {{ '{expected}' }}"),
                "{asynchronous}/{mode}"
            );
        }
        for mode in [
            "break",
            "missing",
            "return-get",
            "return-throw",
            "return-primitive",
            "return-reject",
        ] {
            let expected = if matches!(mode, "missing" | "return-get") {
                0
            } else {
                1
            };
            assert_eq!(
                run(move |realm| scenario(realm, asynchronous, mode, true)),
                format!("Promise {{ 'original/{expected}' }}")
            );
        }
        assert_eq!(
            run(move |realm| scenario(realm, asynchronous, "return-throw-signal", true)),
            "Promise { 'close/1' }"
        );
        assert_eq!(
            run(move |realm| scenario(realm, asynchronous, "break-signal", true)),
            "Promise { '[object Object]/1' }"
        );
    }
}

#[test]
fn sync_adapter_propagates_call_stack_exhaustion() {
    let mut realm = Realm::new();
    let finish = realm.function("return", 0, false, |realm, _, _| {
        Ok(result(realm, Value::Undefined, true))
    });
    let source = iterable(
        &realm,
        |realm, _, _| Ok(result(realm, Value::Number(1.0), false)),
        Some(Value::Object(finish)),
    );
    let mut stepping = iterate_async(&mut realm, &source).unwrap();
    let mut closing = iterate_async(&mut realm, &source).unwrap();
    // At the call limit, both the iterator method and intrinsic rejection
    // fail. Propagate the RangeError without suspending or retrying cleanup.
    realm.max_depth = 1;
    let failure = realm.with_depth(|realm| stepping.step(realm)).unwrap_err();
    assert_eq!(
        message(&mut realm, failure),
        "RangeError: Maximum call stack size exceeded"
    );
    let failure = realm.with_depth(|realm| closing.close(realm)).unwrap_err();
    assert_eq!(
        message(&mut realm, failure),
        "RangeError: Maximum call stack size exceeded"
    );
    assert!(stepping.step(&mut realm).unwrap().is_none());
    closing.close(&mut realm).unwrap();
}

#[test]
fn sync_iterator_cleanup_replaces_generator_returns() {
    let mut realm = Realm::new();
    for fail in [false, true] {
        let finish = realm.function("return", 0, false, move |realm, _, _| {
            if fail {
                Err(Thrown(Value::str("cleanup")))
            } else {
                Ok(result(realm, Value::Null, true))
            }
        });
        let source = iterable(
            &realm,
            |realm, _, _| Ok(result(realm, Value::Number(1.0), false)),
            Some(Value::Object(finish)),
        );
        let mut iteration = iterate(&mut realm, &source).unwrap();
        iteration.step(&mut realm).unwrap();
        let signal = Thrown(Value::Object(realm.return_signal.clone()));
        let completion = iteration.close_thrown(&mut realm, signal);
        if fail {
            assert_eq!(message(&mut realm, completion), "cleanup");
        } else {
            assert!(crate::coroutine::is_return(&realm, &completion));
        }
    }
}

#[test]
fn async_iterator_acquisition_errors_and_fallback() {
    let mut realm = Realm::new();
    let fallback_reads = Rc::new(Cell::new(0));
    let source = realm.object();
    let get_async = realm.function("get", 0, false, |_, _, _| {
        Err(Thrown(Value::str("async getter")))
    });
    ops::define_accessor(
        &source,
        Key::Symbol(realm.symbols.async_iterator.clone()),
        Some(get_async),
        None,
    );
    let reads = fallback_reads.clone();
    let get_sync = realm.function("get", 0, false, move |_, _, _| {
        reads.set(reads.get() + 1);
        Ok(Value::Undefined)
    });
    ops::define_accessor(
        &source,
        Key::Symbol(realm.symbols.iterator.clone()),
        Some(get_sync),
        None,
    );
    let thrown = iterate_async(&mut realm, &Value::Object(source)).unwrap_err();
    assert_eq!(message(&mut realm, thrown), "async getter");
    assert_eq!(fallback_reads.get(), 0);
    for value in [Value::Null, Value::Undefined, Value::Number(1.0)] {
        assert!(iterate_async(&mut realm, &value).is_err());
    }
    let source = realm.object();
    source.define(
        Key::Symbol(realm.symbols.async_iterator.clone()),
        Property::data(Value::Number(1.0)),
    );
    assert!(iterate_async(&mut realm, &Value::Object(source.clone())).is_err());
    source.define(
        Key::Symbol(realm.symbols.async_iterator.clone()),
        Property::data(Value::Object(realm.function(
            "iterator",
            0,
            false,
            |_, _, _| Ok(Value::Number(1.0)),
        ))),
    );
    assert!(iterate_async(&mut realm, &Value::Object(source)).is_err());
    assert_eq!(
        run(|realm| {
            let source = Value::Object(realm.array(vec![Value::Number(3.0)]));
            let mut iterator = iterate_async(realm, &source)?;
            let value = iterator.step(realm)?.unwrap_or_default();
            iterator.close(realm)?;
            Ok(value)
        }),
        "Promise { 3 }"
    );
}
