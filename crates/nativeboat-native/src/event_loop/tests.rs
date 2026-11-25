//! A fake clock makes timer order and sleeping deterministic.

use super::*;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

#[test]
fn dynamic_import_evaluates_and_settles_only_in_a_promise_job() {
    let mut realm = Realm::new();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    realm.set_dynamic_import_loader(move |realm, entry| {
        assert_eq!(entry, 4);
        observed.set(observed.get() + 1);
        Ok(Value::Object(realm.object()))
    });

    let promise = realm.dynamic_import(4).unwrap();
    let Kind::Promise(state) = &promise.as_object().unwrap().data().kind else {
        panic!("dynamic import returns a Promise");
    };
    let state = state.clone();
    assert!(matches!(
        state.borrow().state,
        crate::promise_builtin::State::Pending
    ));
    assert_eq!(calls.get(), 0, "imported code is deferred from this task");

    realm.run_jobs().unwrap();

    assert_eq!(calls.get(), 1);
    assert!(matches!(
        state.borrow().state,
        crate::promise_builtin::State::Fulfilled(Value::Object(_))
    ));
}

#[test]
fn dynamic_import_rejects_when_its_project_loader_is_unavailable_or_fails() {
    let mut realm = Realm::new();
    let promise = realm.dynamic_import(0).unwrap();
    let Kind::Promise(state) = &promise.as_object().unwrap().data().kind else {
        panic!("dynamic import returns a Promise");
    };
    let state = state.clone();
    assert!(
        realm.run_jobs().is_err(),
        "unhandled import rejection is fatal"
    );
    assert!(matches!(
        state.borrow().state,
        crate::promise_builtin::State::Rejected(_)
    ));

    let mut realm = Realm::new();
    realm.set_dynamic_import_loader(|realm, _| {
        Err(realm.throw(ErrorKind::Type, "module evaluation failed"))
    });
    let promise = realm.dynamic_import(0).unwrap();
    let Kind::Promise(state) = &promise.as_object().unwrap().data().kind else {
        panic!("dynamic import returns a Promise");
    };
    let state = state.clone();
    assert!(
        realm.run_jobs().is_err(),
        "unhandled import rejection is fatal"
    );
    assert!(matches!(
        state.borrow().state,
        crate::promise_builtin::State::Rejected(_)
    ));
}

#[test]
fn native_io_completes_on_realm_thread_after_jobs_and_keeps_loop_alive() {
    let mut realm = Realm::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let main_thread = std::thread::current().id();
    let (release, wait) = std::sync::mpsc::channel();
    let seen = log.clone();
    realm
        .submit_io(
            move || {
                wait.recv().unwrap();
                std::thread::current().id()
            },
            move |realm, worker_thread| {
                assert_ne!(worker_thread, main_thread);
                assert_eq!(std::thread::current().id(), main_thread);
                seen.borrow_mut().push(3);
                let tick = seen.clone();
                realm.enqueue_tick(Box::new(move |_| {
                    tick.borrow_mut().push(4);
                    Ok(())
                }));
                let micro = seen.clone();
                realm.enqueue(Box::new(move |_| {
                    micro.borrow_mut().push(5);
                    Ok(())
                }));
                Ok(())
            },
        )
        .unwrap();
    let seen = log.clone();
    realm.enqueue(Box::new(move |_| {
        seen.borrow_mut().push(1);
        Ok(())
    }));
    let seen = log.clone();
    let immediate = Value::Object(realm.function("release", 0, false, move |_, _, _| {
        seen.borrow_mut().push(2);
        release.send(()).unwrap();
        Ok(Value::Undefined)
    }));
    global(&mut realm, "setImmediate", &[immediate]).unwrap();
    assert!(realm.alive());
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), [1, 2, 3, 4, 5]);
    assert!(!realm.alive());
}

#[test]
fn native_io_completions_can_submit_more_work_and_resume_after_throw() {
    let mut realm = Realm::new();
    let completed = Rc::new(Cell::new(0));
    let seen = completed.clone();
    realm
        .submit_io(
            || 1,
            move |realm, value| {
                seen.set(value);
                let seen = seen.clone();
                realm
                    .submit_io(
                        || 2,
                        move |_, value| {
                            seen.set(value);
                            Ok(())
                        },
                    )
                    .unwrap();
                Err(realm.throw(ErrorKind::Error, "completion"))
            },
        )
        .unwrap();
    let error = realm.run_event_loop().unwrap_err();
    assert!(
        ops::to_string(&mut realm, &error.0)
            .unwrap()
            .to_rust_lossy()
            .contains("completion")
    );
    assert_eq!(completed.get(), 1);
    assert!(realm.alive());
    realm.run_event_loop().unwrap();
    assert_eq!(completed.get(), 2);
    assert!(!realm.alive());
}

#[test]
fn native_io_checkpoint_failure_preserves_pending_work_and_microtasks() {
    let mut realm = Realm::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let seen = log.clone();
    realm
        .submit_io(
            || (),
            move |realm, ()| {
                seen.borrow_mut().push(1);
                let second = seen.clone();
                realm
                    .submit_io(
                        || (),
                        move |_, ()| {
                            second.borrow_mut().push(2);
                            Ok(())
                        },
                    )
                    .unwrap();
                let micro = seen.clone();
                realm.enqueue(Box::new(move |_| {
                    micro.borrow_mut().push(3);
                    Ok(())
                }));
                realm.enqueue_tick(Box::new(|realm| {
                    Err(realm.throw(ErrorKind::Error, "checkpoint"))
                }));
                Ok(())
            },
        )
        .unwrap();
    let error = realm.run_event_loop().unwrap_err();
    assert!(
        ops::to_string(&mut realm, &error.0)
            .unwrap()
            .to_rust_lossy()
            .contains("checkpoint")
    );
    assert_eq!(*log.borrow(), [1]);
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), [1, 2, 3]);
}

#[test]
fn native_io_chains_do_not_starve_immediates() {
    let mut realm = Realm::new();
    let log = Rc::new(RefCell::new(Vec::new()));
    let seen = log.clone();
    realm
        .submit_io(
            || (),
            move |realm, ()| {
                seen.borrow_mut().push(1);
                // A deterministically ready second request tests the poll boundary,
                // independently of nondeterministic Windows worker completion timing.
                let next = seen.clone();
                let mut ready: Option<Job> = Some(Box::new(move |_| {
                    next.borrow_mut().push(3);
                    Ok(())
                }));
                realm.event_loop.io.push(PendingIo {
                    poll: Box::new(move || ready.take()),
                    referenced: None,
                });
                crate::work_queue::tests::publish_wake(
                    realm.event_loop.io_signal.as_ref().unwrap(),
                );
                let immediate = seen.clone();
                let callback =
                    Value::Object(realm.function("immediate", 0, false, move |_, _, _| {
                        immediate.borrow_mut().push(2);
                        Ok(Value::Undefined)
                    }));
                global(realm, "setImmediate", &[callback]).unwrap();
                Ok(())
            },
        )
        .unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), [1, 2, 3]);
}

#[test]
fn native_io_setup_and_submission_fail_without_queuing_completions() {
    for create in [
        crate::work_queue::tests::failed_create as fn() -> _,
        crate::work_queue::tests::failed_submit,
    ] {
        let mut realm = Realm::new();
        realm.event_loop.io_create = create;
        assert_eq!(
            realm.submit_io(
                || panic!("rejected work"),
                |_, ()| panic!("rejected completion")
            ),
            Err(5)
        );
        assert!(!realm.alive());
        realm.run_event_loop().unwrap();
    }
}

#[test]
fn native_io_wait_failure_propagates_and_pending_request_can_resume() {
    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::failed_wait;
    let (release, wait) = std::sync::mpsc::channel();
    let completed = Rc::new(Cell::new(false));
    let seen = completed.clone();
    realm
        .submit_io(
            move || wait.recv().unwrap(),
            move |_, ()| {
                seen.set(true);
                Ok(())
            },
        )
        .unwrap();
    let error = realm.run_event_loop().unwrap_err();
    assert!(
        ops::to_string(&mut realm, &error.0)
            .unwrap()
            .to_rust_lossy()
            .contains("Windows error 5")
    );
    assert!(!completed.get());
    release.send(()).unwrap();
    // Wait through the real API after checking the injected failure, so the
    // resumed loop finds a published result without another failing wait.
    crate::work_queue::tests::wait_for_worker(realm.event_loop.io_signal.as_ref().unwrap());
    realm.run_event_loop().unwrap();
    assert!(completed.get());
}

unsafe extern "system" fn timed_io_wait(handle: *mut core::ffi::c_void, milliseconds: u32) -> u32 {
    if milliseconds != u32::MAX {
        sleep(f64::from(milliseconds));
        258
    } else {
        unsafe extern "system" {
            fn WaitForSingleObject(handle: *mut core::ffi::c_void, milliseconds: u32) -> u32;
        }
        // SAFETY: the injected wait receives the owning Signal's valid handle.
        unsafe { WaitForSingleObject(handle, 5_000) }
    }
}

fn timed_io_signal() -> Result<Arc<Signal>, u32> {
    crate::work_queue::tests::signal_with_wait(timed_io_wait)
}

#[test]
fn native_io_wait_respects_timer_deadlines_and_rounds_fractional_milliseconds() {
    let mut realm = realm();
    realm.event_loop.io_create = timed_io_signal;
    let (release, wait) = std::sync::mpsc::channel();
    let completed = Rc::new(Cell::new(false));
    let seen = completed.clone();
    realm
        .submit_io(
            move || wait.recv().unwrap(),
            move |_, ()| {
                seen.set(true);
                Ok(())
            },
        )
        .unwrap();
    let callback = Value::Object(realm.function("release", 0, false, move |_, _, _| {
        release.send(()).unwrap();
        Ok(Value::Undefined)
    }));
    global(&mut realm, "setTimeout", &[callback, n(5.0)]).unwrap();
    // Half a millisecond has elapsed before entering the event loop.
    NOW.with(|now| now.set(0.5));
    realm.run_event_loop().unwrap();
    assert!(completed.get());
    assert_eq!(SLEPT.with(|slept| slept.borrow().clone()), [5.0]);
}

type Log = Rc<RefCell<Vec<String>>>;

std::thread_local! {
    static NOW: Cell<f64> = const { Cell::new(0.0) };
    static SLEPT: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

fn now() -> f64 {
    NOW.with(Cell::get)
}

fn sleep(milliseconds: f64) {
    NOW.with(|now| now.set(now.get() + milliseconds));
    SLEPT.with(|slept| slept.borrow_mut().push(milliseconds));
}

fn realm() -> Realm {
    NOW.with(|now| now.set(0.0));
    SLEPT.with(|slept| slept.borrow_mut().clear());
    let mut realm = Realm::new();
    realm.event_loop.clock = now;
    realm.event_loop.sleep = sleep;
    realm
}

fn n(value: f64) -> Value {
    Value::Number(value)
}

fn global(realm: &mut Realm, name: &str, args: &[Value]) -> Completion {
    let global = Value::Object(realm.global.clone());
    let function = ops::get(realm, &global, &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

fn method(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, target, &Key::name(name))?;
    ops::call(realm, &function, target.clone(), args)
}

/// A callback recording its label and arguments at the fake time.
fn recorder(realm: &Realm, log: &Log, label: &'static str) -> Value {
    let log = log.clone();
    let function = realm.function("f", 0, false, move |realm, _, args| {
        let mut text = format!("{label}@{}", now());
        for value in args {
            text.push(' ');
            text.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
        }
        log.borrow_mut().push(text);
        Ok(Value::Undefined)
    });
    Value::Object(function)
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let thrown = result.unwrap_err();
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

#[test]
fn timers_run_in_due_order_then_immediates() {
    let mut realm = realm();
    let log = Log::default();
    let late = recorder(&realm, &log, "late");
    global(&mut realm, "setTimeout", &[late, n(10.0), n(7.0)]).unwrap();
    let early = recorder(&realm, &log, "early");
    global(&mut realm, "setTimeout", &[early, n(5.0)]).unwrap();
    let clamped = recorder(&realm, &log, "clamped");
    for delay in [Value::Undefined, n(f64::NAN), n(0.0), n(3e9)] {
        global(&mut realm, "setTimeout", &[clamped.clone(), delay]).unwrap();
    }
    let immediate = recorder(&realm, &log, "immediate");
    global(&mut realm, "setImmediate", &[immediate, n(1.0)]).unwrap();
    let micro = recorder(&realm, &log, "micro");
    global(&mut realm, "queueMicrotask", &[micro]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(
        *log.borrow(),
        [
            "micro@0",
            "immediate@0 1",
            "clamped@1",
            "clamped@1",
            "clamped@1",
            "clamped@1",
            "early@5",
            "late@10 7"
        ]
    );
    SLEPT.with(|slept| assert_eq!(*slept.borrow(), [1.0, 4.0, 5.0]));
}

#[test]
fn overdue_timers_after_an_immediate_run_without_sleeping() {
    let mut realm = realm();
    let log = Log::default();
    let timer = recorder(&realm, &log, "timer");
    global(&mut realm, "setTimeout", &[timer, n(5.0)]).unwrap();
    let immediate = Value::Object(realm.function("busy", 0, false, |_, _, _| {
        NOW.with(|now| now.set(20.0));
        Ok(Value::Undefined)
    }));
    global(&mut realm, "setImmediate", &[immediate]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["timer@20"]);
    assert!(SLEPT.with(|slept| slept.borrow().is_empty()));
}

#[test]
fn intervals_repeat_until_cleared_and_handles_control_them() {
    let mut realm = realm();
    let log = Log::default();
    let ticks = Rc::new(Cell::new(0));
    let handle: Rc<RefCell<Value>> = Rc::default();
    let (counter, own) = (ticks.clone(), handle.clone());
    let logged = log.clone();
    let tick = realm.function("tick", 0, false, move |realm, _, _| {
        counter.set(counter.get() + 1);
        logged.borrow_mut().push(format!("tick@{}", now()));
        if counter.get() == 3 {
            let handle = own.borrow().clone();
            global(realm, "clearInterval", &[handle])?;
        }
        Ok(Value::Undefined)
    });
    let interval = global(&mut realm, "setInterval", &[Value::Object(tick), n(2.0)]).unwrap();
    *handle.borrow_mut() = interval.clone();
    assert_eq!(
        ops::to_number(&mut realm, &interval).unwrap(),
        1.0,
        "handles convert to their id"
    );
    // An unreferenced timer does not keep the loop alive.
    let never = recorder(&realm, &log, "never");
    let unreferenced = global(&mut realm, "setTimeout", &[never, n(100.0)]).unwrap();
    assert_eq!(
        method(&mut realm, &unreferenced, "unref", &[])
            .unwrap()
            .as_object(),
        unreferenced.as_object()
    );
    let has_ref = method(&mut realm, &unreferenced, "hasRef", &[]).unwrap();
    assert!(matches!(has_ref, Value::Boolean(false)));
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["tick@2", "tick@4", "tick@6"]);
    // Timeout reference preference survives completion and cancellation.
    let has_ref = method(&mut realm, &interval, "hasRef", &[]).unwrap();
    assert!(matches!(has_ref, Value::Boolean(true)));
    method(&mut realm, &interval, "ref", &[]).unwrap();
    method(&mut realm, &interval, "refresh", &[]).unwrap();
}

#[test]
fn refresh_close_and_clearing_by_id() {
    let mut realm = realm();
    let log = Log::default();
    let refreshed = recorder(&realm, &log, "refreshed");
    let timer = global(&mut realm, "setTimeout", &[refreshed, n(5.0)]).unwrap();
    NOW.with(|now| now.set(3.0));
    method(&mut realm, &timer, "refresh", &[]).unwrap();
    // Clearing by handle, numeric id, string id, closing, and clearing
    // immediates; other values are ignored.
    let mut cleared = Vec::new();
    for _ in 0..4 {
        let never = recorder(&realm, &log, "never");
        cleared.push(global(&mut realm, "setTimeout", &[never, n(1.0)]).unwrap());
    }
    let never = recorder(&realm, &log, "never");
    let immediate = global(&mut realm, "setImmediate", &[never]).unwrap();
    global(&mut realm, "clearTimeout", &[cleared[0].clone()]).unwrap();
    global(&mut realm, "clearTimeout", &[n(3.0)]).unwrap();
    global(&mut realm, "clearInterval", &[Value::str("4")]).unwrap();
    method(&mut realm, &cleared[3], "close", &[]).unwrap();
    global(
        &mut realm,
        "clearImmediate",
        std::slice::from_ref(&immediate),
    )
    .unwrap();
    let plain = Value::Object(realm.object());
    for ignored in [
        Value::Undefined,
        n(-1.0),
        n(1.5),
        Value::str("x"),
        plain,
        n(99.0),
    ] {
        global(&mut realm, "clearTimeout", &[ignored]).unwrap();
    }
    let kept = recorder(&realm, &log, "kept");
    global(&mut realm, "setTimeout", &[kept, n(1.0)]).unwrap();
    // Cleared during the same timers phase: the later one never runs.
    let target: Rc<RefCell<Value>> = Rc::default();
    let victim = target.clone();
    let clearing = realm.function("clearing", 0, false, move |realm, _, _| {
        let victim = victim.borrow().clone();
        global(realm, "clearTimeout", &[victim]).map(drop)?;
        Ok(Value::Undefined)
    });
    global(&mut realm, "setTimeout", &[Value::Object(clearing), n(2.0)]).unwrap();
    let victim = recorder(&realm, &log, "victim");
    *target.borrow_mut() = global(&mut realm, "setTimeout", &[victim, n(2.0)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["kept@4", "refreshed@8"]);
    let refresh = ops::get(&mut realm, &timer, &Key::name("refresh")).unwrap();
    let result = ops::call(&mut realm, &refresh, Value::Undefined, &[]);
    assert_eq!(message(&mut realm, result), "TypeError: Illegal invocation");
    let debug = format!("{:?}", timer.as_object().unwrap().data().kind);
    assert_eq!(debug, "Timer(1)");
    assert_eq!(
        format!("{:?}", realm.event_loop),
        "EventLoop(0 jobs, 0 tasks)"
    );
}

#[test]
fn callbacks_are_validated_and_their_errors_end_the_loop() {
    let mut realm = realm();
    for name in [
        "setTimeout",
        "setInterval",
        "setImmediate",
        "queueMicrotask",
    ] {
        let result = global(&mut realm, name, &[Value::Number(1.0)]);
        assert_eq!(
            message(&mut realm, result),
            "TypeError: The \"callback\" argument must be of type function. Received 1"
        );
    }
    let failing = realm.function("failing", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "callback failed"))
    });
    let failing = Value::Object(failing);
    global(&mut realm, "setTimeout", &[failing.clone(), n(1.0)]).unwrap();
    let result = realm.run_event_loop().map(|()| Value::Undefined);
    assert_eq!(message(&mut realm, result), "RangeError: callback failed");
    global(&mut realm, "queueMicrotask", std::slice::from_ref(&failing)).unwrap();
    let result = realm.run_event_loop().map(|()| Value::Undefined);
    assert_eq!(message(&mut realm, result), "RangeError: callback failed");
    let delay = Value::Symbol(crate::symbol::Symbol::new(None));
    let result = global(&mut realm, "setTimeout", &[failing, delay]);
    assert!(message(&mut realm, result).starts_with("TypeError"));
}

#[test]
fn the_platform_sleep_waits() {
    let start = crate::platform::now();
    crate::platform::sleep(2.0);
    assert!(crate::platform::now() - start >= 1.0);
    crate::platform::sleep(-1.0);
}

#[test]
fn immediates_queued_by_immediates_run_without_sleeping() {
    let mut realm = realm();
    let log = Log::default();
    let logged = log.clone();
    let outer = realm.function("outer", 0, false, move |realm, _, _| {
        logged.borrow_mut().push("outer".to_owned());
        let inner = recorder(realm, &logged, "inner");
        global(realm, "setImmediate", &[inner]).map(drop)?;
        Ok(Value::Undefined)
    });
    global(&mut realm, "setImmediate", &[Value::Object(outer)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["outer", "inner@0"]);
    SLEPT.with(|slept| assert!(slept.borrow().is_empty()));
    // Handle methods require a handle.
    let callback = recorder(&realm, &log, "x");
    let handle = global(&mut realm, "setImmediate", &[callback]);
    let has_ref = ops::get(&mut realm, &handle.unwrap(), &Key::name("hasRef")).unwrap();
    let plain = Value::Object(realm.object());
    let result = ops::call(&mut realm, &has_ref, plain, &[]);
    assert_eq!(message(&mut realm, result), "TypeError: Illegal invocation");
}

#[test]
fn unprintable_rejection_reasons_are_still_reported() {
    let mut realm = realm();
    let reason = realm.object();
    let thrower = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "tag"))
    });
    reason.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: crate::object::Slot::Accessor {
                get: Some(thrower),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let promise = Value::Object(realm.global.clone());
    let promise = ops::get(&mut realm, &promise, &Key::name("Promise")).unwrap();
    method(&mut realm, &promise, "reject", &[Value::Object(reason)]).unwrap();
    let result = realm.run_jobs().map(|()| Value::Undefined);
    assert!(message(&mut realm, result).ends_with("the reason \"<unprintable>\"."));
}

#[test]
fn completed_timeouts_refresh_but_cleared_handles_cannot_restart() {
    let mut realm = realm();
    let log = Log::default();
    let callback = recorder(&realm, &log, "again");
    let weak = callback.as_object().unwrap().downgrade();
    let timer = global(&mut realm, "setTimeout", &[callback, n(2.9), n(7.0)]).unwrap();
    let old = ops::to_number(&mut realm, &timer).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["again@2 7"]);
    assert!(realm.event_loop.tasks.is_empty());
    assert!(
        weak.upgrade().is_some(),
        "a live handle retains its callback for refresh"
    );
    assert!(matches!(
        method(&mut realm, &timer, "hasRef", &[]).unwrap(),
        Value::Boolean(true)
    ));
    assert_eq!(
        method(&mut realm, &timer, "refresh", &[])
            .unwrap()
            .as_object(),
        timer.as_object()
    );
    let new = ops::to_number(&mut realm, &timer).unwrap();
    assert_ne!(old, new);
    global(&mut realm, "clearTimeout", &[n(old)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["again@2 7", "again@4 7"]);
    // The expired numeric id cannot cancel the retained callback.
    global(&mut realm, "clearTimeout", &[n(new)]).unwrap();
    method(&mut realm, &timer, "refresh", &[]).unwrap();
    global(&mut realm, "clearTimeout", std::slice::from_ref(&timer)).unwrap();
    assert!(
        weak.upgrade().is_none(),
        "clearing releases captured data even with a live handle"
    );
    method(&mut realm, &timer, "unref", &[]).unwrap();
    assert!(matches!(
        method(&mut realm, &timer, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
    method(&mut realm, &timer, "ref", &[]).unwrap();
    assert!(matches!(
        method(&mut realm, &timer, "hasRef", &[]).unwrap(),
        Value::Boolean(true)
    ));
    method(&mut realm, &timer, "refresh", &[]).unwrap();
    realm.run_event_loop().unwrap();
    assert!(realm.event_loop.tasks.is_empty());
    assert_eq!(log.borrow().len(), 2);
    // Dropping a completed handle also releases its retained callback.
    let callback = recorder(&realm, &log, "dropped");
    let weak = callback.as_object().unwrap().downgrade();
    let timer = global(&mut realm, "setTimeout", &[callback]).unwrap();
    realm.run_event_loop().unwrap();
    drop(timer);
    assert!(weak.upgrade().is_none());
}

#[test]
fn timeouts_can_refresh_or_cancel_themselves_and_immediates_stay_finished() {
    let mut realm = realm();
    let ticks = Rc::new(Cell::new(0));
    let count = ticks.clone();
    let callback = realm.function("tick", 0, false, move |realm, this, _| {
        count.set(count.get() + 1);
        let id = ops::to_number(realm, &this)?;
        if count.get() == 1 {
            method(realm, &this, "refresh", &[])?;
            assert_eq!(
                ops::to_number(realm, &this)?,
                id,
                "refresh during the callback keeps the id"
            );
        } else {
            global(realm, "clearTimeout", std::slice::from_ref(&this))?;
            method(realm, &this, "refresh", &[])?;
        }
        Ok(Value::Undefined)
    });
    global(&mut realm, "setTimeout", &[Value::Object(callback)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(ticks.get(), 2);
    assert_eq!(now(), 2.0);
    let log = Log::default();
    let callback = recorder(&realm, &log, "immediate");
    let immediate = global(&mut realm, "setImmediate", &[callback]).unwrap();
    realm.run_event_loop().unwrap();
    assert!(matches!(
        method(&mut realm, &immediate, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
    method(&mut realm, &immediate, "ref", &[]).unwrap();
    assert!(matches!(
        method(&mut realm, &immediate, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
    let prototype = Value::Object(realm.timeout_prototype.clone());
    let refresh_fn = ops::get(&mut realm, &prototype, &Key::name("refresh"));
    let refresh_fn = refresh_fn.unwrap();
    ops::call(&mut realm, &refresh_fn, immediate, &[]).unwrap();
    assert!(realm.event_loop.tasks.is_empty());
}

#[test]
fn callbacks_refresh_selected_deadlines_and_equal_deadlines_append() {
    let mut realm = realm();
    let log = Log::default();
    let target: Rc<RefCell<Value>> = Rc::default();
    let refreshed = target.clone();
    let callback = realm.function("refresh", 0, false, move |realm, _, _| {
        let timer = refreshed.borrow().clone();
        method(realm, &timer, "refresh", &[])?;
        Ok(Value::Undefined)
    });
    global(&mut realm, "setTimeout", &[Value::Object(callback), n(2.0)]).unwrap();
    let callback = recorder(&realm, &log, "moved");
    *target.borrow_mut() = global(&mut realm, "setTimeout", &[callback, n(2.0)]).unwrap();
    let callback = recorder(&realm, &log, "existing");
    global(&mut realm, "setTimeout", &[callback, n(4.0)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["existing@4", "moved@4"]);
}

#[test]
fn invalid_callbacks_do_not_coerce_delays_and_unreferenced_tasks_do_not_start_loop() {
    let mut realm = realm();
    let reads = Rc::new(Cell::new(0));
    let observed = reads.clone();
    let delay = realm.object();
    let conversion = realm.function("valueOf", 0, false, move |_, _, _| {
        observed.set(observed.get() + 1);
        Ok(n(1.9))
    });
    delay.define(
        Key::name("valueOf"),
        Property::data(Value::Object(conversion)),
    );
    for name in ["setTimeout", "setInterval"] {
        assert!(global(&mut realm, name, &[n(7.0), Value::Object(delay.clone())]).is_err());
    }
    assert_eq!(reads.get(), 0);
    let log = Log::default();
    let callback = recorder(&realm, &log, "timer");
    let timer = global(&mut realm, "setTimeout", &[callback, Value::Object(delay)]).unwrap();
    assert_eq!(reads.get(), 1);
    let callback = recorder(&realm, &log, "immediate");
    let immediate = global(&mut realm, "setImmediate", &[callback]).unwrap();
    for handle in [&timer, &immediate] {
        method(&mut realm, handle, "unref", &[]).unwrap();
    }
    NOW.with(|clock| clock.set(2.0));
    realm.run_event_loop().unwrap();
    assert!(
        log.borrow().is_empty(),
        "even due unreferenced tasks cannot start the loop"
    );
    method(&mut realm, &timer, "ref", &[]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*log.borrow(), ["timer@2", "immediate@2"]);
}

#[test]
fn imminent_work_covers_jobs_immediates_and_timers_due_soon() {
    let mut realm = realm();
    assert!(!realm.has_imminent_work(4));
    let callback = Value::Object(realm.function("noop", 0, false, |_, _, _| Ok(Value::Undefined)));
    let later = global(&mut realm, "setTimeout", &[callback.clone(), n(50.0)]).unwrap();
    // A timer 50ms away is not imminent until the clock nears it.
    assert!(!realm.has_imminent_work(4));
    NOW.with(|now| now.set(47.0));
    assert!(realm.has_imminent_work(4));
    global(&mut realm, "clearTimeout", &[later]).unwrap();
    assert!(!realm.has_imminent_work(4));
    global(&mut realm, "setImmediate", &[callback]).unwrap();
    assert!(realm.has_imminent_work(0));
}

#[test]
fn next_wakes_follow_work_scheduled_since_the_last_step() {
    let mut realm = realm();
    assert_eq!(realm.next_wake(), None);
    let callback = Value::Object(realm.function("noop", 0, false, |_, _, _| Ok(Value::Undefined)));
    global(&mut realm, "setTimeout", &[callback.clone(), n(30.0)]).unwrap();
    assert_eq!(realm.next_wake(), Some(30));
    NOW.with(|now| now.set(29.5));
    assert_eq!(realm.next_wake(), Some(1));
    global(&mut realm, "setImmediate", &[callback]).unwrap();
    assert_eq!(realm.next_wake(), Some(0));
}
