//! Bodies here are Rust closures standing in for compiled code. They only
//! record what happens: a panic must not unwind across a fiber switch.

use super::*;

type Log = Rc<RefCell<Vec<String>>>;

/// A native iterator method.
type Method = Rc<dyn Fn(&mut Realm, &[Value]) -> Completion>;

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn s(text: &str) -> Value {
    Value::str(text)
}

fn note(log: &Log, text: impl Into<String>) {
    log.borrow_mut().push(text.into());
}

fn show(realm: &mut Realm, value: &Value) -> String {
    crate::inspect::inspect(realm, value, 2).unwrap_or_else(|_| "?".to_owned())
}

fn shown(realm: &mut Realm, result: Completion) -> String {
    match result {
        Ok(value) => show(realm, &value),
        Err(thrown) => format!("throws {}", show(realm, &thrown.0)),
    }
}

/// A generator function whose body is `body`.
fn generator(realm: &Realm, body: impl Fn(&mut Realm) -> Completion + Clone + 'static) -> Value {
    let function = generator_function(realm, "g", 0, move |_, _, _| {
        let body = body.clone();
        Ok(Box::new(move |realm: &mut Realm| body(realm)) as Body)
    });
    Value::Object(function)
}

fn call(realm: &mut Realm, function: &Value) -> Value {
    ops::call(realm, function, Value::Undefined, &[]).unwrap()
}

fn step(realm: &mut Realm, target: &Value, method: &str, args: &[Value]) -> String {
    let function = ops::get(realm, target, &Key::name(method)).unwrap();
    let result = ops::call(realm, &function, target.clone(), args);
    shown(realm, result)
}

#[test]
fn generators_yield_receive_and_finish() {
    let mut realm = Realm::new();
    let log = Log::default();
    let record = log.clone();
    let counting = generator(&realm, move |realm| {
        let first = yield_value(realm, n(1.0))?;
        note(&record, format!("received {}", show(realm, &first)));
        let second = yield_value(realm, n(2.0))?;
        note(&record, format!("received {}", show(realm, &second)));
        Ok(s("end"))
    });
    let generated = call(&mut realm, &counting);
    let steps: Vec<String> = [
        ("next", n(0.0)),
        ("next", s("a")),
        ("next", s("b")),
        ("next", n(0.0)),
        ("return", n(5.0)),
    ]
    .into_iter()
    .map(|(method, value)| step(&mut realm, &generated, method, &[value]))
    .collect();
    assert_eq!(
        steps,
        [
            "{ value: 1, done: false }",
            "{ value: 2, done: false }",
            "{ value: 'end', done: true }",
            "{ value: undefined, done: true }",
            "{ value: 5, done: true }"
        ]
    );
    assert_eq!(*log.borrow(), ["received 'a'", "received 'b'"]);
    assert_eq!(
        step(&mut realm, &generated, "throw", &[s("x")]),
        "throws 'x'"
    );
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = ops::get(&mut realm, &generated, &tag).unwrap();
    assert_eq!(show(&mut realm, &tag), "'Generator'");
}

#[test]
fn return_and_throw_unwind_or_skip_the_body() {
    let mut realm = Realm::new();
    let log = Log::default();
    let record = log.clone();
    // A body like `try { yield 1 } catch (e) { ... } finally { ... }`.
    let guarded = generator(&realm, move |realm| {
        let result = match yield_value(realm, n(1.0)) {
            Err(thrown) if is_return(realm, &thrown) => Err(thrown),
            Err(thrown) => {
                note(&record, format!("caught {}", show(realm, &thrown.0)));
                yield_value(realm, n(2.0))
            }
            Ok(value) => Ok(value),
        };
        note(&record, "finally");
        result
    });
    let returned = call(&mut realm, &guarded);
    step(&mut realm, &returned, "next", &[]);
    assert_eq!(
        step(&mut realm, &returned, "return", &[s("early")]),
        "{ value: 'early', done: true }"
    );
    let thrown = call(&mut realm, &guarded);
    step(&mut realm, &thrown, "next", &[]);
    assert_eq!(
        step(&mut realm, &thrown, "throw", &[s("boom")]),
        "{ value: 2, done: false }"
    );
    assert_eq!(
        step(&mut realm, &thrown, "next", &[]),
        "{ value: undefined, done: true }"
    );
    assert_eq!(*log.borrow(), ["finally", "caught 'boom'", "finally"]);
    // Before starting, return and throw finish without running the body.
    let unstarted = call(&mut realm, &guarded);
    assert_eq!(
        step(&mut realm, &unstarted, "return", &[n(1.0)]),
        "{ value: 1, done: true }"
    );
    let unstarted = call(&mut realm, &guarded);
    assert_eq!(step(&mut realm, &unstarted, "throw", &[n(1.0)]), "throws 1");
    assert_eq!(log.borrow().len(), 3);
    // A body that throws ends the generator with that error.
    let failing = generator(&realm, |realm| Err(realm.throw(ErrorKind::Range, "body")));
    let generated = call(&mut realm, &failing);
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "throws [RangeError: body]"
    );
    // Dropping a suspended generator discards its fiber.
    let suspended = call(&mut realm, &guarded);
    step(&mut realm, &suspended, "next", &[]);
    let debug = format!("{:?}", suspended.as_object().unwrap().data().kind);
    assert_eq!(debug, "Generator(Coroutine(Suspended))");
    drop(suspended);
}

#[test]
fn generators_reject_reentry_and_other_receivers() {
    let mut realm = Realm::new();
    let own: Rc<RefCell<Value>> = Rc::default();
    let reference = own.clone();
    let reentrant = generator(&realm, move |realm| {
        let target = reference.borrow().clone();
        let next = ops::get(realm, &target, &Key::name("next"))?;
        ops::call(realm, &next, target, &[])
    });
    let generated = call(&mut realm, &reentrant);
    *own.borrow_mut() = generated.clone();
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "throws [TypeError: Generator is already running]"
    );
    let next = ops::get(&mut realm, &generated, &Key::name("next")).unwrap();
    let plain = Value::Object(realm.object());
    for receiver in [n(1.0), plain] {
        let result = ops::call(&mut realm, &next, receiver, &[]);
        assert_eq!(
            shown(&mut realm, result),
            "throws [TypeError: next method called on incompatible receiver]"
        );
    }
    let outside = yield_value(&mut realm, n(1.0));
    assert_eq!(
        shown(&mut realm, outside),
        "throws [SyntaxError: yield and await are only valid in generators and async functions]"
    );
    let outside = await_value(&mut realm, n(1.0));
    assert_eq!(
        shown(&mut realm, outside),
        "throws [SyntaxError: await is only valid in async functions]"
    );
}

#[test]
fn generator_functions_bind_parameters_and_choose_prototypes() {
    let mut realm = Realm::new();
    let failing = Value::Object(generator_function(&realm, "g", 0, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "parameters"))
    }));
    let result = ops::call(&mut realm, &failing, Value::Undefined, &[]);
    assert_eq!(shown(&mut realm, result), "throws [RangeError: parameters]");
    let plain = generator(&realm, |_| Ok(Value::Undefined));
    let generated = call(&mut realm, &plain);
    let prototype = ops::get(&mut realm, &plain, &Key::name("prototype")).unwrap();
    assert_eq!(
        generated.as_object().unwrap().data().prototype.as_ref(),
        prototype.as_object()
    );
    ops::set(&mut realm, &plain, &Key::name("prototype"), n(1.0)).unwrap();
    let generated = call(&mut realm, &plain);
    assert_eq!(
        generated.as_object().unwrap().data().prototype,
        Some(realm.generator_prototype.clone())
    );
}

/// An iterator object whose methods are native functions.
fn iterator(realm: &Realm, methods: &[(&str, Method)]) -> Value {
    let iterator = realm.object();
    for (name, method) in methods {
        let method = method.clone();
        let function = realm.function(name, 1, false, move |realm, _, args| method(realm, args));
        iterator.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let own = Value::Object(iterator.clone());
    let identity = realm.function("iterator", 0, false, move |_, _, _| Ok(own.clone()));
    iterator.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(Value::Object(identity)),
    );
    Value::Object(iterator)
}

fn result(realm: &Realm, value: Value, done: bool) -> Completion {
    Ok(result_object(realm, value, done))
}

#[test]
fn yield_delegation_forwards_next_throw_and_return() {
    let mut realm = Realm::new();
    let array = Value::Object(realm.array(vec![n(7.0), n(8.0)]));
    let delegating = generator(&realm, move |realm| {
        let value = yield_delegate(realm, array.clone())?;
        yield_value(realm, value)
    });
    let generated = call(&mut realm, &delegating);
    let steps: Vec<String> = (0..4)
        .map(|_| step(&mut realm, &generated, "next", &[]))
        .collect();
    assert_eq!(
        steps,
        [
            "{ value: 7, done: false }",
            "{ value: 8, done: false }",
            "{ value: undefined, done: false }",
            "{ value: undefined, done: true }"
        ]
    );
    // An iterator with every method; results pass through unchanged.
    let full = iterator(
        &realm,
        &[
            (
                "next",
                Rc::new(|realm: &mut Realm, _: &[Value]| result(realm, s("n"), false)),
            ),
            (
                "throw",
                Rc::new(|realm: &mut Realm, args: &[Value]| result(realm, args[0].clone(), true)),
            ),
            (
                "return",
                Rc::new(|realm: &mut Realm, args: &[Value]| {
                    let done = matches!(args[0], Value::Number(_));
                    result(realm, args[0].clone(), done)
                }),
            ),
        ],
    );
    let delegate = |realm: &Realm, target: Value| {
        generator(realm, move |realm| yield_delegate(realm, target.clone()))
    };
    let forwarding = delegate(&realm, full.clone());
    let generated = call(&mut realm, &forwarding);
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "{ value: 'n', done: false }"
    );
    assert_eq!(
        step(&mut realm, &generated, "throw", &[s("t")]),
        "{ value: 't', done: true }"
    );
    let generated = call(&mut realm, &forwarding);
    step(&mut realm, &generated, "next", &[]);
    assert_eq!(
        step(&mut realm, &generated, "return", &[s("more")]),
        "{ value: 'more', done: false }"
    );
    assert_eq!(
        step(&mut realm, &generated, "return", &[n(9.0)]),
        "{ value: 9, done: true }"
    );
    // Without return and throw methods.
    let bare = iterator(
        &realm,
        &[(
            "next",
            Rc::new(|realm: &mut Realm, _: &[Value]| result(realm, s("n"), false)),
        )],
    );
    let function = delegate(&realm, bare.clone());
    let generated = call(&mut realm, &function);
    step(&mut realm, &generated, "next", &[]);
    assert_eq!(
        step(&mut realm, &generated, "return", &[n(3.0)]),
        "{ value: 3, done: true }"
    );
    let function = delegate(&realm, bare);
    let generated = call(&mut realm, &function);
    step(&mut realm, &generated, "next", &[]);
    assert_eq!(
        step(&mut realm, &generated, "throw", &[n(3.0)]),
        "throws [TypeError: The iterator does not provide a 'throw' method]"
    );
    // A missing throw method closes the iterator, checking its result.
    for (closing, expected) in [
        (
            Value::Object(realm.object()),
            "throws [TypeError: The iterator does not provide a 'throw' method]",
        ),
        (
            n(1.0),
            "throws [TypeError: Iterator result is not an object]",
        ),
    ] {
        let closer: Method = Rc::new(move |_: &mut Realm, _: &[Value]| Ok(closing.clone()));
        let closable = iterator(
            &realm,
            &[
                (
                    "next",
                    Rc::new(|realm: &mut Realm, _: &[Value]| result(realm, s("n"), false)),
                ),
                ("return", closer),
            ],
        );
        let function = delegate(&realm, closable);
        let generated = call(&mut realm, &function);
        step(&mut realm, &generated, "next", &[]);
        assert_eq!(step(&mut realm, &generated, "throw", &[n(3.0)]), expected);
    }
    // Results must be objects; operands must be iterable.
    let broken = iterator(
        &realm,
        &[("next", Rc::new(|_: &mut Realm, _: &[Value]| Ok(n(1.0))))],
    );
    let function = delegate(&realm, broken);
    let generated = call(&mut realm, &function);
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "throws [TypeError: Iterator result is not an object]"
    );
    let function = delegate(&realm, n(1.0));
    let generated = call(&mut realm, &function);
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "throws [TypeError: yield* operand is not iterable]"
    );
}

/// An async function whose body is `body`.
fn async_fn(realm: &Realm, body: impl Fn(&mut Realm) -> Completion + Clone + 'static) -> Value {
    let function = async_function(realm, "f", 0, move |_, _, _| {
        let body = body.clone();
        Ok(Box::new(move |realm: &mut Realm| body(realm)) as Body)
    });
    Value::Object(function)
}

#[test]
fn async_functions_await_and_settle_promises() {
    let mut realm = Realm::new();
    let log = Log::default();
    let record = log.clone();
    let awaiting = async_fn(&realm, move |realm| {
        let value = await_value(realm, n(1.0))?;
        note(&record, format!("awaited {}", show(realm, &value)));
        let promise = Value::Object(realm.global.clone());
        let promise = ops::get(realm, &promise, &Key::name("Promise"))?;
        let reject = ops::get(realm, &promise, &Key::name("reject"))?;
        let rejected = ops::call(realm, &reject, promise, &[s("no")])?;
        match await_value(realm, rejected) {
            Err(thrown) => note(&record, format!("caught {}", show(realm, &thrown.0))),
            Ok(_) => note(&record, "unexpected"),
        }
        Ok(s("done"))
    });
    let promise = call(&mut realm, &awaiting);
    note(&log, "sync");
    realm.run_jobs().unwrap();
    assert_eq!(*log.borrow(), ["sync", "awaited 1", "caught 'no'"]);
    assert_eq!(show(&mut realm, &promise), "Promise { 'done' }");
    // Failing bodies and parameters reject the promise.
    let failing = async_fn(&realm, |realm| Err(realm.throw(ErrorKind::Range, "body")));
    let rejected = call(&mut realm, &failing);
    let parameters = Value::Object(async_function(&realm, "f", 0, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "parameters"))
    }));
    let rejected_early = call(&mut realm, &parameters);
    let shown_promises = [
        show(&mut realm, &rejected),
        show(&mut realm, &rejected_early),
    ];
    assert_eq!(
        shown_promises,
        [
            "Promise { <rejected> [RangeError: body] }",
            "Promise { <rejected> [RangeError: parameters] }"
        ]
    );
    // A PromiseResolve that throws is thrown into the body.
    let thenable = {
        let promise = Value::Object(realm.global.clone());
        let promise = ops::get(&mut realm, &promise, &Key::name("Promise")).unwrap();
        let resolve = ops::get(&mut realm, &promise, &Key::name("resolve")).unwrap();
        ops::call(&mut realm, &resolve, promise, &[n(1.0)]).unwrap()
    };
    let thrower = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "constructor"))
    });
    thenable.as_object().unwrap().define(
        Key::name("constructor"),
        Property {
            slot: Slot::Accessor {
                get: Some(thrower),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let failing_resolve = async_fn(&realm, move |realm| await_value(realm, thenable.clone()));
    let rejected = call(&mut realm, &failing_resolve);
    assert_eq!(
        show(&mut realm, &rejected),
        "Promise { <rejected> [RangeError: constructor] }"
    );
    // Generators cannot await.
    let awaiting_generator = generator(&realm, |realm| await_value(realm, n(1.0)));
    let generated = call(&mut realm, &awaiting_generator);
    assert_eq!(
        step(&mut realm, &generated, "next", &[]),
        "throws [SyntaxError: await is only valid in async functions]"
    );
    let _ = realm.run_jobs();
}

fn async_generator(
    realm: &Realm,
    body: impl Fn(&mut Realm) -> Completion + Clone + 'static,
) -> Value {
    let function = async_generator_function(realm, "ag", 0, move |_, _, _| {
        let body = body.clone();
        Ok(Box::new(move |realm: &mut Realm| body(realm)) as Body)
    });
    Value::Object(function)
}

/// Requests a step and records how its promise settles.
fn request(realm: &mut Realm, log: &Log, target: &Value, method: &str, args: &[Value]) {
    let function = ops::get(realm, target, &Key::name(method)).unwrap();
    let promise = ops::call(realm, &function, target.clone(), args).unwrap();
    let (fulfilled, rejected) = (log.clone(), log.clone());
    let label = method.to_owned();
    let label_rejected = label.clone();
    let on_fulfilled = realm.function("", 1, false, move |realm, _, args| {
        let shown = show(realm, &args[0]);
        note(&fulfilled, format!("{label} {shown}"));
        Ok(Value::Undefined)
    });
    let on_rejected = realm.function("", 1, false, move |realm, _, args| {
        let shown = show(realm, &args[0]);
        note(&rejected, format!("{label_rejected} rejects {shown}"));
        Ok(Value::Undefined)
    });
    let then = ops::get(realm, &promise, &Key::name("then")).unwrap();
    let handlers = [Value::Object(on_fulfilled), Value::Object(on_rejected)];
    ops::call(realm, &then, promise, &handlers).unwrap();
}

fn rejected_promise(realm: &mut Realm, reason: Value) -> Value {
    let promise = Value::Object(realm.global.clone());
    let promise = ops::get(realm, &promise, &Key::name("Promise")).unwrap();
    let reject = ops::get(realm, &promise, &Key::name("reject")).unwrap();
    ops::call(realm, &reject, promise, &[reason]).unwrap()
}

fn delegate_source(realm: &Realm, sync: bool, iterator: Object) -> Value {
    let source = realm.object();
    let key = if sync {
        realm.symbols.iterator.clone()
    } else {
        realm.symbols.async_iterator.clone()
    };
    let function = realm.function("iterator", 0, false, move |_, _, _| {
        Ok(Value::Object(iterator.clone()))
    });
    source.define(Key::Symbol(key), Property::data(Value::Object(function)));
    Value::Object(source)
}

fn delegated(realm: &mut Realm, source: Value) -> Value {
    let function = async_generator(realm, move |realm| {
        async_yield_delegate(realm, source.clone())
    });
    call(realm, &function)
}

#[test]
fn async_delegates_forward_requests_and_preserve_node_22_value_rules() {
    for sync in [false, true] {
        let mut realm = Realm::new();
        let iterator = realm.object();
        let count = Rc::new(Cell::new(0));
        let record = count.clone();
        realm.method(&iterator, "next", 1, move |realm, _, _| {
            let value = record.get() + 1;
            record.set(value);
            Ok(result_object(realm, n(value as f64), value > 1))
        });
        realm.method(&iterator, "throw", 1, |realm, _, _| {
            Ok(result_object(realm, s("caught"), false))
        });
        realm.method(&iterator, "return", 1, |realm, _, _| {
            Ok(result_object(realm, s("cleanup"), false))
        });
        let source = delegate_source(&realm, sync, iterator);
        let target = delegated(&mut realm, source);
        let log = Log::default();
        request(&mut realm, &log, &target, "next", &[]);
        request(&mut realm, &log, &target, "throw", &[s("throw")]);
        request(&mut realm, &log, &target, "return", &[s("return")]);
        request(&mut realm, &log, &target, "next", &[n(5.0)]);
        realm.run_jobs().unwrap();
        assert_eq!(
            *log.borrow(),
            [
                "next { value: 1, done: false }",
                "throw { value: 'caught', done: false }",
                "return { value: 'cleanup', done: false }",
                "next { value: 2, done: true }"
            ]
        );
    }
}

#[test]
fn async_delegates_close_missing_throw_and_handle_absent_return() {
    for sync in [false, true] {
        for action in ["throw", "return"] {
            for closing in ["absent", "normal", "primitive", "reject"] {
                let mut realm = Realm::new();
                let iterator = realm.object();
                realm.method(&iterator, "next", 1, |realm, _, _| {
                    Ok(result_object(realm, n(1.0), false))
                });
                if closing != "absent" {
                    realm.method(&iterator, "return", 1, move |realm, _, _| match closing {
                        "primitive" => Ok(n(7.0)),
                        "reject" => {
                            let value = rejected_promise(realm, s("close failure"));
                            Ok(if sync {
                                result_object(realm, value, true)
                            } else {
                                value
                            })
                        }
                        _ => Ok(result_object(realm, n(2.0), true)),
                    });
                }
                let source = delegate_source(&realm, sync, iterator);
                let target = delegated(&mut realm, source);
                let log = Log::default();
                request(&mut realm, &log, &target, "next", &[]);
                realm.run_jobs().unwrap();
                request(&mut realm, &log, &target, action, &[s("sent")]);
                realm.run_jobs().unwrap();
                let last = log.borrow().last().unwrap().clone();
                if action == "throw" && sync {
                    assert_eq!(last, "throw rejects 'sent'");
                } else if closing == "primitive" {
                    assert!(last.contains("TypeError"), "{last}");
                } else if closing == "reject" {
                    assert!(last.contains("close failure"), "{last}");
                } else if action == "throw" {
                    assert!(last.contains("TypeError"), "{last}");
                } else {
                    assert!(last.contains("done: true"), "{last}");
                }
            }
        }
    }
}

#[test]
fn async_delegates_validate_results_getters_and_rejected_return_resumption() {
    for sync in [false, true] {
        for method in ["next", "throw", "return"] {
            let mut realm = Realm::new();
            let iterator = realm.object();
            let calls = Rc::new(Cell::new(0));
            let next = calls.clone();
            realm.method(&iterator, "next", 1, move |realm, _, _| {
                let count = next.get();
                next.set(count + 1);
                Ok(if count == 0 {
                    result_object(realm, n(1.0), false)
                } else {
                    n(7.0)
                })
            });
            realm.method(&iterator, "throw", 1, |_, _, _| Ok(n(7.0)));
            realm.method(&iterator, "return", 1, |_, _, _| Ok(n(7.0)));
            let source = delegate_source(&realm, sync, iterator);
            let target = delegated(&mut realm, source);
            let log = Log::default();
            request(&mut realm, &log, &target, "next", &[]);
            realm.run_jobs().unwrap();
            request(&mut realm, &log, &target, method, &[s("sent")]);
            realm.run_jobs().unwrap();
            assert!(log.borrow().last().unwrap().contains("TypeError"));
        }
        let mut realm = Realm::new();
        let iterator = realm.object();
        realm.method(&iterator, "next", 1, |realm, _, args| {
            Ok(result_object(
                realm,
                args.first().cloned().unwrap_or_default(),
                false,
            ))
        });
        let source = delegate_source(&realm, sync, iterator);
        let target = delegated(&mut realm, source);
        let log = Log::default();
        request(&mut realm, &log, &target, "next", &[]);
        realm.run_jobs().unwrap();
        let rejected = rejected_promise(&mut realm, s("return rejection"));
        request(&mut realm, &log, &target, "return", &[rejected]);
        realm.run_jobs().unwrap();
        assert_eq!(
            log.borrow().last().unwrap(),
            "return { value: 'return rejection', done: false }"
        );
        request(&mut realm, &log, &target, "throw", &[s("finish")]);
        realm.run_jobs().unwrap();
    }
}

#[test]
fn async_delegates_propagate_method_and_result_getter_failures() {
    for sync in [false, true] {
        for method in ["next", "return", "throw"] {
            for field in ["done", "value", "method"] {
                let mut realm = Realm::new();
                let iterator = realm.object();
                let bad = realm.object();
                bad.define(Key::name("done"), Property::data(Value::Boolean(true)));
                bad.define(Key::name("value"), Property::data(n(2.0)));
                let getter = realm.function("get", 0, false, move |_, _, _| Err(Thrown(s(field))));
                let property = Property {
                    slot: Slot::Accessor {
                        get: Some(getter),
                        set: None,
                    },
                    enumerable: true,
                    configurable: true,
                };
                if field != "method" {
                    bad.define(Key::name(field), property.clone());
                }
                let calls = Rc::new(Cell::new(0));
                let next = calls.clone();
                let next_bad = bad.clone();
                realm.method(&iterator, "next", 1, move |realm, _, _| {
                    let count = next.get();
                    next.set(count + 1);
                    Ok(if count == 0 {
                        result_object(realm, n(1.0), false)
                    } else {
                        Value::Object(next_bad.clone())
                    })
                });
                for action in ["throw", "return"] {
                    let bad = bad.clone();
                    realm.method(&iterator, action, 1, move |_, _, _| {
                        Ok(Value::Object(bad.clone()))
                    });
                }
                if field == "method" {
                    iterator.define(Key::name(method), property);
                }
                let source = delegate_source(&realm, sync, iterator);
                let target = delegated(&mut realm, source);
                let log = Log::default();
                request(&mut realm, &log, &target, "next", &[]);
                realm.run_jobs().unwrap();
                if !(method == "next" && field == "method") {
                    request(&mut realm, &log, &target, method, &[]);
                    realm.run_jobs().unwrap();
                }
                assert!(
                    log.borrow()
                        .last()
                        .unwrap()
                        .contains(&format!("rejects '{field}'")),
                    "{:?}",
                    log.borrow()
                );
            }
        }
    }
}

#[test]
fn async_generators_queue_requests_and_await_yields() {
    let mut realm = Realm::new();
    let log = Log::default();
    let record = log.clone();
    let counting = async_generator(&realm, move |realm| {
        let first = async_yield(realm, n(1.0))?;
        note(&record, format!("sent {}", show(realm, &first)));
        await_value(realm, n(0.0))?;
        let promise = rejected_promise(realm, s("no"));
        match async_yield(realm, promise) {
            Err(thrown) => note(&record, format!("caught {}", show(realm, &thrown.0))),
            Ok(_) => note(&record, "unexpected"),
        }
        let iterable = Value::Object(realm.array(vec![n(7.0)]));
        yield_delegate(realm, iterable)?;
        Ok(s("end"))
    });
    let generator = call(&mut realm, &counting);
    for args in [vec![], vec![s("a")], vec![], vec![]] {
        request(&mut realm, &log, &generator, "next", &args);
    }
    realm.run_jobs().unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "sent 'a'",
            "next { value: 1, done: false }",
            "caught 'no'",
            "next { value: 7, done: false }",
            "next { value: 'end', done: true }",
            "next { value: undefined, done: true }",
        ]
    );
    log.borrow_mut().clear();
    // Completed generators settle every request without running.
    request(&mut realm, &log, &generator, "throw", &[s("late")]);
    request(&mut realm, &log, &generator, "return", &[n(2.0)]);
    request(&mut realm, &log, &generator, "next", &[]);
    let rejected = rejected_promise(&mut realm, s("why"));
    request(&mut realm, &log, &generator, "return", &[rejected]);
    realm.run_jobs().unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "throw rejects 'late'",
            "return { value: 2, done: true }",
            "next { value: undefined, done: true }",
            "return rejects 'why'",
        ]
    );
}

#[test]
fn async_generator_returns_and_throws() {
    let mut realm = Realm::new();
    let log = Log::default();
    let record = log.clone();
    let body = async_generator(&realm, move |realm| {
        let result = async_yield(realm, n(1.0));
        if let Err(thrown) = &result
            && is_return(realm, thrown)
        {
            note(&record, "unwinding");
        }
        result?;
        Err(realm.throw(ErrorKind::Range, "body"))
    });
    // `return` while suspended at a yield awaits its value, then unwinds.
    let returning = call(&mut realm, &body);
    request(&mut realm, &log, &returning, "next", &[]);
    request(&mut realm, &log, &returning, "return", &[n(3.0)]);
    // A rejected return value is thrown into the body.
    let rejecting = call(&mut realm, &body);
    request(&mut realm, &log, &rejecting, "next", &[]);
    let rejected = rejected_promise(&mut realm, s("value"));
    request(&mut realm, &log, &rejecting, "return", &[rejected]);
    // Throwing resumes the body with the exception.
    let throwing = call(&mut realm, &body);
    request(&mut realm, &log, &throwing, "next", &[]);
    request(&mut realm, &log, &throwing, "throw", &[s("in")]);
    // A body that finishes by throwing rejects its request.
    let failing = call(&mut realm, &body);
    request(&mut realm, &log, &failing, "next", &[]);
    request(&mut realm, &log, &failing, "next", &[]);
    realm.run_jobs().unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "next { value: 1, done: false }",
            "unwinding",
            "next { value: 1, done: false }",
            "next { value: 1, done: false }",
            "throw rejects 'in'",
            "next { value: 1, done: false }",
            "next rejects [RangeError: body]",
            "return { value: 3, done: true }",
            "return rejects 'value'",
        ]
    );
}

#[test]
fn async_generators_before_starting_and_other_receivers() {
    let mut realm = Realm::new();
    let log = Log::default();
    let unused = async_generator(&realm, |_| Ok(Value::Undefined));
    let thrown = call(&mut realm, &unused);
    request(&mut realm, &log, &thrown, "throw", &[s("early")]);
    let returned = call(&mut realm, &unused);
    request(&mut realm, &log, &returned, "return", &[s("r")]);
    // Requests queued while a return value is awaited wait for it.
    request(&mut realm, &log, &returned, "next", &[]);
    // A return value whose promise conversion throws rejects the request.
    let thenable = {
        let promise = Value::Object(realm.global.clone());
        let promise = ops::get(&mut realm, &promise, &Key::name("Promise")).unwrap();
        let resolve = ops::get(&mut realm, &promise, &Key::name("resolve")).unwrap();
        ops::call(&mut realm, &resolve, promise, &[n(1.0)]).unwrap()
    };
    let thrower = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "constructor"))
    });
    thenable.as_object().unwrap().define(
        Key::name("constructor"),
        Property {
            slot: Slot::Accessor {
                get: Some(thrower),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let failing = call(&mut realm, &unused);
    request(&mut realm, &log, &failing, "return", &[thenable]);
    let prototype = Value::Object(realm.async_generator_prototype.clone());
    let plain = Value::Object(realm.object());
    let next = ops::get(&mut realm, &prototype, &Key::name("next")).unwrap();
    let promise = ops::call(&mut realm, &next, plain, &[]).unwrap();
    let catch = ops::get(&mut realm, &promise, &Key::name("catch")).unwrap();
    let record = log.clone();
    let handler = realm.function("", 1, false, move |realm, _, args| {
        let error = ops::to_string(realm, &args[0])?.to_rust_lossy();
        note(&record, format!("receiver rejects {error}"));
        Ok(Value::Undefined)
    });
    ops::call(&mut realm, &catch, promise, &[Value::Object(handler)]).unwrap();
    realm.run_jobs().unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "throw rejects 'early'",
            "return rejects [RangeError: constructor]",
            "receiver rejects TypeError: next method called on incompatible receiver",
            "return { value: 'r', done: true }",
            "next { value: undefined, done: true }",
        ]
    );
    // Prototypes: functions, objects and the async iterator protocol.
    let function = unused.as_object().unwrap();
    assert_eq!(
        function.data().prototype,
        Some(realm.async_generator_function_prototype.clone())
    );
    let iterator = Value::Object(realm.async_iterator_prototype.clone());
    let key = Key::Symbol(realm.symbols.async_iterator.clone());
    let method = ops::get(&mut realm, &iterator, &key).unwrap();
    assert!(crate::same_value(
        &ops::call(&mut realm, &method, n(5.0), &[]).unwrap(),
        &n(5.0)
    ));
    ops::set(&mut realm, &unused, &Key::name("prototype"), n(1.0)).unwrap();
    let fallback = call(&mut realm, &unused);
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        Some(realm.async_generator_prototype.clone())
    );
    let parameters = Value::Object(async_generator_function(&realm, "p", 0, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "parameters"))
    }));
    let result = ops::call(&mut realm, &parameters, Value::Undefined, &[]);
    assert_eq!(shown(&mut realm, result), "throws [RangeError: parameters]");
    let debug = format!("{:?}", fallback.as_object().unwrap().data().kind);
    assert_eq!(debug, "AsyncGenerator");
    if let Kind::AsyncGenerator(generator) = &fallback.as_object().unwrap().data().kind {
        assert_eq!(format!("{generator:?}"), "AsyncGenerator(SuspendedStart)");
    }
}

#[test]
fn finished_bodies_leave_their_fibers_to_later_ones() {
    let mut realm = Realm::new();
    let counting = generator(&realm, |realm| {
        yield_value(realm, n(1.0))?;
        Ok(s("end"))
    });
    // Each finished body parks its fiber; the next body takes it up.
    for _ in 0..40 {
        let generated = call(&mut realm, &counting);
        assert_eq!(
            step(&mut realm, &generated, "next", &[]),
            "{ value: 1, done: false }"
        );
        assert_eq!(
            step(&mut realm, &generated, "next", &[]),
            "{ value: 'end', done: true }"
        );
    }
    assert_eq!(realm.idle_fibers.0.len(), 1);
    // Bodies running at once each have a fiber; the pool keeps a few of
    // them once they finish, and the rest go with their coroutines.
    let running: Vec<Value> = (0..IDLE_FIBERS + 4)
        .map(|_| {
            let generated = call(&mut realm, &counting);
            step(&mut realm, &generated, "next", &[]);
            generated
        })
        .collect();
    assert!(realm.idle_fibers.0.is_empty());
    for generated in &running {
        assert_eq!(
            step(&mut realm, generated, "next", &[]),
            "{ value: 'end', done: true }"
        );
        // A finished body is not run again.
        assert_eq!(
            step(&mut realm, generated, "next", &[]),
            "{ value: undefined, done: true }"
        );
    }
    assert_eq!(realm.idle_fibers.0.len(), IDLE_FIBERS);
    drop(running);
    // A suspended body abandoned keeps its fiber until its coroutine goes.
    let abandoned = call(&mut realm, &counting);
    step(&mut realm, &abandoned, "next", &[]);
    assert_eq!(realm.idle_fibers.0.len(), IDLE_FIBERS - 1);
}
