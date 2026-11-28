//! Expected values follow ECMAScript 2024 and Node 22's results.

use super::*;
use crate::object::Slot;

type Log = Rc<RefCell<Vec<String>>>;

fn s(text: &str) -> Value {
    Value::str(text)
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn global(realm: &mut Realm, name: &str) -> Value {
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name)).unwrap()
}

fn method(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, target, &Key::name(name))?;
    ops::call(realm, &function, target.clone(), args)
}

fn statics(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let promise = global(realm, "Promise");
    method(realm, &promise, name, args)
}

fn function(
    realm: &Realm,
    constructor: bool,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) -> Value {
    Value::Object(realm.function("f", 1, constructor, code))
}

/// A callback recording `label:value` and returning its argument.
fn recorder(realm: &Realm, log: &Log, label: &'static str) -> Value {
    let log = log.clone();
    function(realm, false, move |realm, _, args| {
        let value = arg(args, 0);
        let text = crate::inspect::inspect(realm, &value, 2)?;
        log.borrow_mut().push(format!("{label}:{text}"));
        Ok(value)
    })
}

fn thrower(realm: &Realm, message: &'static str) -> Value {
    function(realm, false, move |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, message))
    })
}

/// Observes a promise's settlement into the log.
fn observe(realm: &mut Realm, log: &Log, promise: &Value) {
    let fulfilled = recorder(realm, log, "fulfilled");
    let rejected = recorder(realm, log, "rejected");
    method(realm, promise, "then", &[fulfilled, rejected]).unwrap();
}

fn drained(realm: &mut Realm, log: &Log) -> Vec<String> {
    realm.run_jobs().unwrap();
    core::mem::take(&mut *log.borrow_mut())
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

/// `new Promise(executor)` capturing its resolving functions.
fn deferred(realm: &mut Realm) -> (Value, Value, Value) {
    let slots: Rc<RefCell<(Value, Value)>> = Rc::default();
    let captured = slots.clone();
    let executor = function(realm, false, move |_, _, args| {
        *captured.borrow_mut() = (arg(args, 0), arg(args, 1));
        Ok(Value::Undefined)
    });
    let constructor = global(realm, "Promise");
    let promise = ops::construct(realm, &constructor, &[executor]).unwrap();
    let (resolve, reject) = slots.borrow().clone();
    (promise, resolve, reject)
}

fn call(realm: &mut Realm, function: &Value, args: &[Value]) {
    ops::call(realm, function, Value::Undefined, args).unwrap();
}

#[test]
fn construction_runs_the_executor_and_resolves_once() {
    let mut realm = Realm::new();
    let log = Log::default();
    let constructor = global(&mut realm, "Promise");
    let result = ops::call(&mut realm, &constructor, Value::Undefined, &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Promise constructor cannot be invoked without 'new'"
    );
    let result = ops::construct(&mut realm, &constructor, &[n(1.0)]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Promise resolver 1 is not a function"
    );
    let (promise, resolve, reject) = deferred(&mut realm);
    observe(&mut realm, &log, &promise);
    call(&mut realm, &resolve, &[n(1.0)]);
    call(&mut realm, &resolve, &[n(2.0)]);
    call(&mut realm, &reject, &[n(3.0)]);
    assert_eq!(drained(&mut realm, &log), ["fulfilled:1"]);
    // A throwing executor rejects; resolving functions after that do nothing.
    let failing = thrower(&realm, "executor failed");
    let rejected = ops::construct(&mut realm, &constructor, &[failing]).unwrap();
    observe(&mut realm, &log, &rejected);
    assert_eq!(
        drained(&mut realm, &log),
        ["rejected:[RangeError: executor failed]"]
    );
    // A new target's prototype is used; a non-object one falls back.
    let target = function(&realm, true, |_, _, _| Ok(Value::Undefined));
    let prototype = realm.object();
    ops::set(
        &mut realm,
        &target,
        &Key::name("prototype"),
        Value::Object(prototype.clone()),
    )
    .unwrap();
    let executor = function(&realm, false, |_, _, _| Ok(Value::Undefined));
    let derived = ops::construct_target(
        &mut realm,
        &constructor,
        std::slice::from_ref(&executor),
        &target,
    )
    .unwrap();
    assert_eq!(
        derived.as_object().unwrap().data().prototype,
        Some(prototype)
    );
    ops::set(&mut realm, &target, &Key::name("prototype"), n(1.0)).unwrap();
    let plain = ops::construct_target(&mut realm, &constructor, &[executor], &target).unwrap();
    assert_eq!(
        plain.as_object().unwrap().data().prototype,
        Some(realm.promise_prototype.clone())
    );
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let prototype = Value::Object(realm.promise_prototype.clone());
    assert!(matches!(
        ops::get(&mut realm, &prototype, &tag).unwrap(),
        Value::String(_)
    ));
}

#[test]
fn resolution_follows_thenables_and_detects_cycles() {
    let mut realm = Realm::new();
    let log = Log::default();
    // A promise resolved with itself rejects.
    let (promise, resolve, _) = deferred(&mut realm);
    observe(&mut realm, &log, &promise);
    call(&mut realm, &resolve, std::slice::from_ref(&promise));
    assert_eq!(
        drained(&mut realm, &log),
        ["rejected:[TypeError: Chaining cycle detected for promise #<Promise>]"]
    );
    // Objects without a callable then fulfill; a throwing then getter rejects.
    let (promise, resolve, _) = deferred(&mut realm);
    observe(&mut realm, &log, &promise);
    let plain = realm.object();
    plain.define(Key::name("then"), Property::data(n(1.0)));
    call(&mut realm, &resolve, &[Value::Object(plain)]);
    assert_eq!(drained(&mut realm, &log), ["fulfilled:{ then: 1 }"]);
    let (promise, resolve, _) = deferred(&mut realm);
    observe(&mut realm, &log, &promise);
    let getter = realm.object();
    let failing = thrower(&realm, "then getter");
    getter.define(
        Key::name("then"),
        Property {
            slot: Slot::Accessor {
                get: failing.as_object().cloned(),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    call(&mut realm, &resolve, &[Value::Object(getter)]);
    assert_eq!(
        drained(&mut realm, &log),
        ["rejected:[RangeError: then getter]"]
    );
    // A thenable whose then throws rejects through its resolving functions.
    let (promise, resolve, _) = deferred(&mut realm);
    observe(&mut realm, &log, &promise);
    let thenable = realm.object();
    thenable.define(
        Key::name("then"),
        Property::data(thrower(&realm, "then failed")),
    );
    call(&mut realm, &resolve, &[Value::Object(thenable)]);
    assert_eq!(
        drained(&mut realm, &log),
        ["rejected:[RangeError: then failed]"]
    );
}

#[test]
fn then_catch_and_finally_chain_reactions() {
    let mut realm = Realm::new();
    let log = Log::default();
    let resolved = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    // Handlers attached after settlement still run as jobs.
    realm.run_jobs().unwrap();
    observe(&mut realm, &log, &resolved);
    // Missing handlers pass values and reasons through.
    let chained = method(&mut realm, &resolved, "then", &[Value::Undefined]).unwrap();
    observe(&mut realm, &log, &chained);
    let rejected = statics(&mut realm, "reject", &[s("no")]).unwrap();
    let passed = method(&mut realm, &rejected, "then", &[]).unwrap();
    let callback = recorder(&realm, &log, "caught");
    let caught = method(&mut realm, &passed, "catch", &[callback]).unwrap();
    observe(&mut realm, &log, &caught);
    // A throwing handler rejects the derived promise.
    let callback = thrower(&realm, "handler");
    let failing = method(&mut realm, &resolved, "then", &[callback]).unwrap();
    observe(&mut realm, &log, &failing);
    assert_eq!(
        drained(&mut realm, &log),
        [
            "fulfilled:1",
            "fulfilled:1",
            "caught:'no'",
            "rejected:[RangeError: handler]",
            "fulfilled:'no'"
        ]
    );
    // finally keeps the value or reason unless its callback throws.
    let callback = recorder(&realm, &log, "finally");
    let kept = method(&mut realm, &resolved, "finally", &[callback]).unwrap();
    observe(&mut realm, &log, &kept);
    let callback = recorder(&realm, &log, "finally");
    let reason = method(&mut realm, &rejected, "finally", &[callback]).unwrap();
    observe(&mut realm, &log, &reason);
    let callback = thrower(&realm, "finally");
    let replaced = method(&mut realm, &resolved, "finally", &[callback]).unwrap();
    observe(&mut realm, &log, &replaced);
    let callback = thrower(&realm, "finally");
    let replaced = method(&mut realm, &rejected, "finally", &[callback]).unwrap();
    observe(&mut realm, &log, &replaced);
    let plain = method(&mut realm, &resolved, "finally", &[n(1.0)]).unwrap();
    observe(&mut realm, &log, &plain);
    let mut settled = drained(&mut realm, &log);
    settled.sort();
    assert_eq!(
        settled,
        [
            "finally:undefined",
            "finally:undefined",
            "fulfilled:1",
            "fulfilled:1",
            "rejected:'no'",
            "rejected:[RangeError: finally]",
            "rejected:[RangeError: finally]"
        ]
    );
    let then = ops::get(&mut realm, &resolved, &Key::name("then")).unwrap();
    let plain = Value::Object(realm.object());
    let result = ops::call(&mut realm, &then, plain, &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Method Promise.prototype.then called on incompatible receiver"
    );
    let finally = ops::get(&mut realm, &resolved, &Key::name("finally")).unwrap();
    let result = ops::call(&mut realm, &finally, n(1.0), &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Method Promise.prototype.finally called on incompatible receiver"
    );
}

#[test]
fn species_and_custom_constructors_create_capabilities() {
    let mut realm = Realm::new();
    let log = Log::default();
    let promise = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    let key = Key::name("constructor");
    ops::set(&mut realm, &promise, &key, n(1.0)).unwrap();
    let result = method(&mut realm, &promise, "then", &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: object.constructor is not an object"
    );
    let constructor = Value::Object(realm.object());
    ops::set(&mut realm, &promise, &key, constructor.clone()).unwrap();
    let derived = method(&mut realm, &promise, "then", &[]).unwrap();
    observe(&mut realm, &log, &derived);
    let species = Key::Symbol(realm.symbols.species.clone());
    ops::set(&mut realm, &constructor, &species, n(1.0)).unwrap();
    let result = method(&mut realm, &promise, "then", &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: object.constructor[Symbol.species] is not a constructor"
    );
    // A constructor that builds promises through its own executor.
    let intrinsic = global(&mut realm, "Promise");
    let wrapping = intrinsic.clone();
    let custom = function(&realm, true, move |realm, _, args| {
        ops::construct(realm, &wrapping, args)
    });
    ops::set(&mut realm, &constructor, &species, custom.clone()).unwrap();
    let derived = method(&mut realm, &promise, "then", &[]).unwrap();
    observe(&mut realm, &log, &derived);
    ops::set(&mut realm, &promise, &key, Value::Undefined).unwrap();
    let derived = method(&mut realm, &promise, "then", &[]).unwrap();
    observe(&mut realm, &log, &derived);
    assert_eq!(
        drained(&mut realm, &log),
        ["fulfilled:1", "fulfilled:1", "fulfilled:1"]
    );
    // PromiseResolve returns promises of the same constructor unchanged.
    let promise = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    let same = statics(&mut realm, "resolve", std::slice::from_ref(&promise)).unwrap();
    assert_eq!(same.as_object(), promise.as_object());
    let resolve = ops::get(&mut realm, &intrinsic, &Key::name("resolve")).unwrap();
    let wrapped = ops::call(
        &mut realm,
        &resolve,
        custom.clone(),
        std::slice::from_ref(&promise),
    )
    .unwrap();
    assert_ne!(wrapped.as_object(), promise.as_object());
    let result = ops::call(&mut realm, &resolve, n(1.0), &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: PromiseResolve called on non-object"
    );
    let getter = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    getter.as_object().unwrap().define(
        key,
        Property {
            slot: Slot::Accessor {
                get: thrower(&realm, "constructor").as_object().cloned(),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let result = statics(&mut realm, "resolve", &[getter]);
    assert_eq!(message(&mut realm, result), "RangeError: constructor");
    // Capability executors must be called once with callable functions.
    let reject = ops::get(&mut realm, &intrinsic, &Key::name("reject")).unwrap();
    let plain = Value::Object(realm.object());
    let result = ops::call(&mut realm, &reject, plain, &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Promise capability constructor is not a constructor"
    );
    let twice = function(&realm, true, |realm, _, args| {
        let executor = arg(args, 0);
        ops::call(realm, &executor, Value::Undefined, &[n(1.0), n(2.0)])?;
        ops::call(realm, &executor, Value::Undefined, &[n(1.0), n(2.0)])?;
        Ok(Value::Object(realm.object()))
    });
    let result = ops::call(&mut realm, &reject, twice, &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Promise executor has already been invoked with non-undefined arguments"
    );
    let uncallable = function(&realm, true, |realm, _, args| {
        ops::call(realm, &arg(args, 0), Value::Undefined, &[n(1.0)])?;
        Ok(Value::Object(realm.object()))
    });
    let result = ops::call(&mut realm, &reject, uncallable, &[]);
    assert_eq!(
        message(&mut realm, result),
        "TypeError: Promise resolve or reject function is not callable"
    );
    let resolvers = statics(&mut realm, "withResolvers", &[]).unwrap();
    let resolve = ops::get(&mut realm, &resolvers, &Key::name("resolve")).unwrap();
    let promise = ops::get(&mut realm, &resolvers, &Key::name("promise")).unwrap();
    observe(&mut realm, &log, &promise);
    call(&mut realm, &resolve, &[s("ready")]);
    assert_eq!(drained(&mut realm, &log), ["fulfilled:'ready'"]);
}

#[test]
fn combinators_collect_results() {
    let mut realm = Realm::new();
    let log = Log::default();
    let one = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    let no = statics(&mut realm, "reject", &[s("no")]).unwrap();
    let ignore = function(&realm, false, |_, _, _| Ok(Value::Undefined));
    method(&mut realm, &no, "catch", &[ignore]).unwrap();
    let list = |realm: &Realm, values: &[Value]| Value::Object(realm.array(values.to_vec()));
    for (name, values) in [
        ("all", list(&realm, &[one.clone(), n(2.0)])),
        ("all", list(&realm, &[one.clone(), no.clone()])),
        ("all", list(&realm, &[])),
        ("allSettled", list(&realm, &[one.clone(), no.clone()])),
        ("any", list(&realm, &[no.clone(), one.clone()])),
        ("any", list(&realm, std::slice::from_ref(&no))),
        ("any", list(&realm, &[])),
        ("race", list(&realm, &[no.clone(), one.clone()])),
    ] {
        let combined = statics(&mut realm, name, &[values]).unwrap();
        let label = recorder(&realm, &log, name);
        method(&mut realm, &combined, "then", &[label.clone(), label]).unwrap();
        realm.run_jobs().unwrap();
    }
    assert_eq!(
        core::mem::take(&mut *log.borrow_mut()),
        [
            "all:[ 1, 2 ]",
            "all:'no'",
            "all:[]",
            "allSettled:[\n  { status: 'fulfilled', value: 1 },\n  { status: 'rejected', reason: 'no' }\n]",
            "any:1",
            "any:[AggregateError: All promises were rejected] { [errors]: [ 'no' ] }",
            "any:[AggregateError: All promises were rejected] { [errors]: [] }",
            "race:'no'"
        ]
    );
    // Element functions record only their first call.
    let double = realm.object();
    let thenable_then = function(&realm, false, |realm, _, args| {
        for value in [1.0, 2.0] {
            ops::call(realm, &arg(args, 0), Value::Undefined, &[n(value)])?;
        }
        Ok(Value::Undefined)
    });
    double.define(Key::name("then"), Property::data(thenable_then));
    let values = list(&realm, &[Value::Object(double)]);
    let combined = statics(&mut realm, "allSettled", &[values]).unwrap();
    observe(&mut realm, &log, &combined);
    assert_eq!(
        drained(&mut realm, &log),
        ["fulfilled:[ { status: 'fulfilled', value: 1 } ]"]
    );
}

#[test]
fn combinators_reject_abrupt_completions() {
    let mut realm = Realm::new();
    let log = Log::default();
    let promise = global(&mut realm, "Promise");
    let all = ops::get(&mut realm, &promise, &Key::name("all")).unwrap();
    // Not iterable.
    let rejected = statics(&mut realm, "all", &[n(1.0)]).unwrap();
    observe(&mut realm, &log, &rejected);
    // A failing resolve lookup, a non-callable one, and one that throws
    // while iterating, which closes the iterator.
    let custom = function(&realm, true, {
        let promise = promise.clone();
        move |realm, _, args| ops::construct(realm, &promise, args)
    });
    for resolve in [n(1.0), thrower(&realm, "resolve")] {
        ops::set(&mut realm, &custom, &Key::name("resolve"), resolve).unwrap();
        let empty = Value::Object(realm.array(vec![n(1.0)]));
        let rejected = ops::call(&mut realm, &all, custom.clone(), &[empty]).unwrap();
        observe(&mut realm, &log, &rejected);
    }
    let getter = custom.as_object().unwrap();
    getter.define(
        Key::name("resolve"),
        Property {
            slot: Slot::Accessor {
                get: thrower(&realm, "resolve getter").as_object().cloned(),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let rejected = ops::call(&mut realm, &all, custom.clone(), &[n(1.0)]).unwrap();
    observe(&mut realm, &log, &rejected);
    // An iterator whose next throws is not closed.
    let iterable = realm.object();
    let make_iterator = function(&realm, false, |realm, _, _| {
        let iterator = realm.object();
        let next = realm.function("next", 0, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Range, "next"))
        });
        iterator.define(Key::name("next"), Property::data(Value::Object(next)));
        Ok(Value::Object(iterator))
    });
    iterable.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::data(make_iterator),
    );
    let rejected = statics(&mut realm, "race", &[Value::Object(iterable)]).unwrap();
    observe(&mut realm, &log, &rejected);
    // A capability whose resolve throws when the last result arrives.
    let failing_resolve = function(&realm, true, |realm, _, args| {
        let resolve = realm.function("resolve", 1, false, |realm, _, _| {
            Err(realm.throw(ErrorKind::Range, "capability resolve"))
        });
        let reject = realm.function("reject", 1, false, |_, _, _| Ok(Value::Undefined));
        ops::call(
            realm,
            &arg(args, 0),
            Value::Undefined,
            &[Value::Object(resolve), Value::Object(reject)],
        )?;
        Ok(Value::Object(realm.object()))
    });
    let resolve = ops::get(&mut realm, &promise, &Key::name("resolve")).unwrap();
    ops::set(&mut realm, &failing_resolve, &Key::name("resolve"), resolve).unwrap();
    let empty = Value::Object(realm.array(vec![]));
    assert!(ops::call(&mut realm, &all, failing_resolve, &[empty]).is_ok());
    assert_eq!(
        drained(&mut realm, &log),
        [
            "rejected:[TypeError: value is not iterable]",
            "rejected:[TypeError: Promise resolve or reject function is not callable]",
            "rejected:[RangeError: resolve]",
            "rejected:[RangeError: resolve getter]",
            "rejected:[RangeError: next]"
        ]
    );
}

#[test]
fn unhandled_rejections_are_reported_after_jobs() {
    let mut realm = Realm::new();
    // A rejection handled before the jobs drain is not reported.
    let rejected = statics(&mut realm, "reject", &[s("late")]).unwrap();
    realm.run_jobs().unwrap_err();
    let catcher = function(&realm, false, |_, _, _| Ok(Value::Undefined));
    method(
        &mut realm,
        &rejected,
        "catch",
        std::slice::from_ref(&catcher),
    )
    .unwrap();
    method(&mut realm, &rejected, "catch", &[catcher]).unwrap();
    realm.run_jobs().unwrap();
    let error = Value::Object(realm.error(ErrorKind::Type, "lost"));
    statics(&mut realm, "reject", &[error]).unwrap();
    let thrown = realm.run_jobs().unwrap_err();
    assert_eq!(message(&mut realm, Err(thrown)), "TypeError: lost");
    statics(&mut realm, "reject", &[s("text")]).unwrap();
    let thrown = realm.run_jobs().unwrap_err();
    assert_eq!(
        message(&mut realm, Err(thrown)),
        "UnhandledPromiseRejection: This error originated either by throwing inside of an async function without a catch block, or by rejecting a promise which was not handled with .catch(). The promise rejected with the reason \"'text'\"."
    );
    // An object with its own stack, as a DOMException, is the error itself.
    let like = realm.object();
    like.define(Key::name("stack"), Property::data(s("AbortError: gone")));
    statics(&mut realm, "reject", &[Value::Object(like.clone())]).unwrap();
    let thrown = realm.run_jobs().unwrap_err();
    assert!(matches!(&thrown.0, Value::Object(object) if *object == like));
}

#[test]
fn promises_inspect_with_their_state() {
    let mut realm = Realm::new();
    let (pending, _, _) = deferred(&mut realm);
    let fulfilled = statics(&mut realm, "resolve", &[n(1.0)]).unwrap();
    let rejected = statics(&mut realm, "reject", &[s("no")]).unwrap();
    let shown: Vec<String> = [pending, fulfilled, rejected.clone()]
        .iter()
        .map(|value| crate::inspect::inspect(&mut realm, value, 2).unwrap())
        .collect();
    assert_eq!(
        shown,
        [
            "Promise { <pending> }",
            "Promise { 1 }",
            "Promise { <rejected> 'no' }"
        ]
    );
    let debug = format!("{:?}", rejected.as_object().unwrap().data().kind);
    assert_eq!(debug, "Promise(Rejected(String(\"no\")))");
    let _ = realm.run_jobs();
}

#[test]
fn combinators_use_the_constructor_resolve_method() {
    let mut realm = Realm::new();
    let log = Log::default();
    // A constructor whose resolve passes values through unchanged, so the
    // combinators call thenables' own then methods.
    let intrinsic = global(&mut realm, "Promise");
    let custom = function(&realm, true, move |realm, _, args| {
        ops::construct(realm, &intrinsic, args)
    });
    let identity = function(&realm, false, |_, _, args| Ok(arg(args, 0)));
    ops::set(&mut realm, &custom, &Key::name("resolve"), identity).unwrap();
    let all_settled = statics_function(&mut realm, "allSettled");
    let double = realm.object();
    let twice = function(&realm, false, |realm, _, args| {
        for value in [1.0, 2.0] {
            ops::call(realm, &arg(args, 0), Value::Undefined, &[n(value)])?;
        }
        Ok(Value::Undefined)
    });
    double.define(Key::name("then"), Property::data(twice));
    let values = Value::Object(realm.array(vec![Value::Object(double)]));
    let settled = ops::call(&mut realm, &all_settled, custom.clone(), &[values]).unwrap();
    observe(&mut realm, &log, &settled);
    // A then method that throws rejects a race and closes the iterator.
    let race = statics_function(&mut realm, "race");
    let broken = realm.object();
    broken.define(
        Key::name("then"),
        Property::data(thrower(&realm, "race then")),
    );
    let values = Value::Object(realm.array(vec![Value::Object(broken)]));
    let raced = ops::call(&mut realm, &race, custom, &[values]).unwrap();
    observe(&mut realm, &log, &raced);
    assert_eq!(
        drained(&mut realm, &log),
        [
            "fulfilled:[ { status: 'fulfilled', value: 1 } ]",
            "rejected:[RangeError: race then]"
        ]
    );
}

fn statics_function(realm: &mut Realm, name: &str) -> Value {
    let promise = global(realm, "Promise");
    ops::get(realm, &promise, &Key::name(name)).unwrap()
}
