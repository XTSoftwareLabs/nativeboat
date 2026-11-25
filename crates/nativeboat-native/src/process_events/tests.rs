//! `process` events with a scripted `emit`, and wiring with a small
//! EventEmitter standing in for the Node.js library's.

use super::*;
use crate::{Completion, ErrorKind, Property, Slot, same_value};
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<String>>>;

/// What a scripted `emit` does for an event.
type Script = fn(&mut Realm, &str) -> Completion;

fn show(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

/// Reads `process` and gives it an `emit` logging each event with its
/// arguments, whose result comes from `script`.
fn scripted(realm: &mut Realm, log: &Log, script: Script) -> Value {
    let process = crate::process_builtin::process(realm).unwrap();
    let log = log.clone();
    let emit = realm.function("emit", 1, false, move |realm, _, args| {
        let name = show(realm, &args[0]);
        let mut entry = name.clone();
        for value in &args[1..] {
            entry.push(' ');
            entry.push_str(&show(realm, value));
        }
        log.lock().unwrap().push(entry);
        script(realm, &name)
    });
    let target = process.as_object().unwrap();
    target.define(Key::name("emit"), Property::data(Value::Object(emit)));
    process
}

/// Captures what is written to the standard error stream.
fn stderr(session: &mut shimforge::Session) -> Log {
    let written = Log::default();
    let sink = written.clone();
    shimforge::mock!(session, platform::write_stderr, fn(&str) -> bool)
        .expect()
        .returning(move |text| {
            sink.lock().unwrap().push(text.to_owned());
            true
        });
    written
}

fn entries(log: &Log) -> Vec<String> {
    core::mem::take(&mut *log.lock().unwrap())
}

fn error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Error, message)
}

/// Calls a global function such as `setImmediate`.
fn global(realm: &mut Realm, name: &str, args: &[Value]) {
    let global = Value::Object(realm.global.clone());
    let function = ops::get(realm, &global, &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args).unwrap();
}

/// A callback that logs `label`, then throws when `throws` is set.
fn callback(realm: &Realm, log: &Log, label: &'static str, throws: bool) -> Value {
    let log = log.clone();
    Value::Object(realm.function("", 0, false, move |realm, _, _| {
        log.lock().unwrap().push(label.to_owned());
        if throws {
            return Err(error(realm, label));
        }
        Ok(Value::Undefined)
    }))
}

fn handled(_: &mut Realm, name: &str) -> Completion {
    Ok(Value::Boolean(name == "uncaughtException"))
}

fn unheard(_: &mut Realm, _: &str) -> Completion {
    Ok(Value::Boolean(false))
}

#[test]
fn programs_end_with_before_exit_and_exit() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |realm, name| {
        if name == "exit" {
            realm.exit_code = Value::Number(4.0);
        }
        Ok(Value::Boolean(true))
    });
    realm.exit_code = Value::Number(2.0);
    assert_eq!(realm.run_main(|_| Ok(())), 4);
    assert_eq!(entries(&log), ["beforeExit 2", "exit 2"]);
    // `exit` is emitted once.
    assert!(!realm.before_exit().unwrap());
    exit(&mut realm).unwrap();
    assert!(entries(&log).is_empty());
    assert!(entries(&written).is_empty());
}

#[test]
fn before_exit_listeners_may_schedule_more_work() {
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |realm, name| {
        if name == "beforeExit" && matches!(realm.exit_code, Value::Undefined) {
            realm.exit_code = Value::Number(0.0);
            let again = realm.function("", 0, false, |_, _, _| Ok(Value::Undefined));
            global(realm, "setImmediate", &[Value::Object(again)]);
        }
        Ok(Value::Boolean(true))
    });
    assert_eq!(realm.run_main(|_| Ok(())), 0);
    assert_eq!(entries(&log), ["beforeExit 0", "beforeExit 0", "exit 0"]);
}

#[test]
fn uncaught_exceptions_go_to_listeners_and_the_loop_resumes() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, handled);
    let status = realm.run_main(|realm| {
        let first = callback(realm, &log, "immediate 1", true);
        let second = callback(realm, &log, "immediate 2", false);
        global(realm, "setImmediate", &[first]);
        global(realm, "setImmediate", &[second]);
        let tick = callback(realm, &log, "tick 1", true);
        global(
            realm,
            "queueMicrotask",
            &[callback(realm, &log, "job 1", true)],
        );
        global(
            realm,
            "queueMicrotask",
            &[callback(realm, &log, "job 2", false)],
        );
        let process = crate::process_builtin::process(realm)?;
        let next_tick = ops::get(realm, &process, &Key::name("nextTick"))?;
        ops::call(realm, &next_tick, process.clone(), &[tick])?;
        let tick = callback(realm, &log, "tick 2", false);
        ops::call(realm, &next_tick, process, &[tick])?;
        Err::<(), _>(error(realm, "main"))
    });
    assert_eq!(status, 0);
    assert_eq!(
        entries(&log),
        [
            "uncaughtExceptionMonitor Error: main uncaughtException",
            "uncaughtException Error: main uncaughtException",
            "tick 1",
            "uncaughtExceptionMonitor Error: tick 1 uncaughtException",
            "uncaughtException Error: tick 1 uncaughtException",
            // The rest of the queue waits for the next task.
            "immediate 1",
            "uncaughtExceptionMonitor Error: immediate 1 uncaughtException",
            "uncaughtException Error: immediate 1 uncaughtException",
            "tick 2",
            "job 1",
            "uncaughtExceptionMonitor Error: job 1 uncaughtException",
            "uncaughtException Error: job 1 uncaughtException",
            "job 2",
            "immediate 2",
            "beforeExit 0",
            "exit 0",
        ]
    );
    assert!(entries(&written).is_empty());
}

#[test]
fn unhandled_exceptions_emit_exit_report_and_end_the_program() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |realm, name| {
        if name == "exit" {
            // Exit listeners may change the status, and their errors are ignored.
            realm.exit_code = Value::Number(5.0);
            return Err(error(realm, "ignored"));
        }
        Ok(Value::Boolean(false))
    });
    let status = realm.run_main(|realm| {
        global(
            realm,
            "setImmediate",
            &[callback(realm, &log, "never", false)],
        );
        Err::<(), _>(error(realm, "main"))
    });
    assert_eq!(status, 5);
    assert_eq!(
        entries(&log),
        [
            "uncaughtExceptionMonitor Error: main uncaughtException",
            "uncaughtException Error: main uncaughtException",
            "exit 1",
        ]
    );
    assert_eq!(entries(&written), ["Uncaught Error: main\n"]);
    // An exception after `exit` keeps the status, or 1.
    realm.exit_code = Value::Undefined;
    realm.event_loop.fatal = None;
    assert_eq!(realm.fatal(error(&realm, "late")), Some(1));
    assert_eq!(entries(&written), ["Uncaught Error: late\n"]);
}

#[test]
fn throwing_uncaught_exception_listeners_end_with_seven() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |realm, name| match name {
        "uncaughtException" => Err(error(realm, "again")),
        _ => Ok(Value::Boolean(false)),
    });
    assert_eq!(
        realm.run_main(|realm| Err::<(), _>(error(realm, "first"))),
        7
    );
    assert_eq!(entries(&written), ["Uncaught Error: again\n"]);
    assert_eq!(entries(&log).len(), 2);
}

#[test]
fn programs_without_process_events_report_uncaught_exceptions() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let mut realm = Realm::new();
    assert_eq!(
        realm.run_main(|realm| Err::<(), _>(error(realm, "plain"))),
        1
    );
    assert_eq!(entries(&written), ["Uncaught Error: plain\n"]);
    // A `process` that is not an EventEmitter has no listeners either.
    let mut realm = Realm::new();
    crate::process_builtin::process(&mut realm).unwrap();
    assert!(matches!(emit(&mut realm, "exit", &[]), Ok(None)));
    realm.exit_code = Value::Number(3.0);
    assert_eq!(realm.run_main(|_| Ok(())), 3);
    let thrown = realm.run_main(|realm| Err::<(), _>(error(realm, "after exit")));
    assert_eq!(thrown, 3);
    assert_eq!(entries(&written), ["Uncaught Error: after exit\n"]);
    // An unreadable `emit` is an uncaught exception.
    let mut realm = Realm::new();
    let process = crate::process_builtin::process(&mut realm).unwrap();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(error(realm, "emit getter"))
    });
    process
        .as_object()
        .unwrap()
        .define(Key::name("emit"), getter_only(getter));
    assert_eq!(realm.run_main(|_| Ok(())), 7);
    assert_eq!(entries(&written), ["Uncaught Error: emit getter\n"]);
    // A rejection that ends the program is reported once.
    let mut realm = Realm::new();
    let status = realm.run_main(|realm| {
        let global = Value::Object(realm.global.clone());
        let promise = ops::get(realm, &global, &Key::name("Promise"))?;
        let reject = ops::get(realm, &promise, &Key::name("reject"))?;
        let reason = Value::Object(realm.error(ErrorKind::Error, "rejected"));
        ops::call(realm, &reject, promise, &[reason])
    });
    assert_eq!(status, 1);
    assert_eq!(entries(&written), ["Uncaught Error: rejected\n"]);
}

#[test]
fn errors_in_before_exit_and_exit_listeners_are_uncaught_exceptions() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    for (script, status, report) in [
        (
            (|realm: &mut Realm, name: &str| match name {
                "beforeExit" => Err(error(realm, "before")),
                other => handled(realm, other),
            }) as Script,
            0,
            None,
        ),
        (
            |realm: &mut Realm, name: &str| match name {
                "beforeExit" => Err(error(realm, "before")),
                _ => Ok(Value::Boolean(false)),
            },
            1,
            Some("Uncaught Error: before\n"),
        ),
        (
            |realm: &mut Realm, name: &str| match name {
                "exit" => Err(error(realm, "in exit")),
                other => handled(realm, other),
            },
            0,
            None,
        ),
        (
            |realm: &mut Realm, name: &str| match name {
                "exit" => Err(error(realm, "in exit")),
                other => unheard(realm, other),
            },
            1,
            Some("Uncaught Error: in exit\n"),
        ),
    ] {
        let log = Log::default();
        let mut realm = Realm::new();
        scripted(&mut realm, &log, script);
        assert_eq!(realm.run_main(|_| Ok(())), status);
        let report: Vec<String> = report.into_iter().map(str::to_owned).collect();
        assert_eq!(entries(&written), report);
    }
}

#[test]
fn unhandled_rejections_go_to_listeners_or_become_uncaught_exceptions() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let reject = |realm: &mut Realm, reason: Value| {
        let global = Value::Object(realm.global.clone());
        let promise = ops::get(realm, &global, &Key::name("Promise")).unwrap();
        let reject = ops::get(realm, &promise, &Key::name("reject")).unwrap();
        ops::call(realm, &reject, promise, &[reason]).unwrap();
    };
    let log = Log::default();
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |_, name| {
        Ok(Value::Boolean(name == "unhandledRejection"))
    });
    reject(&mut realm, Value::str("first"));
    reject(&mut realm, Value::str("second"));
    realm.run_jobs().unwrap();
    assert_eq!(
        entries(&log),
        [
            "unhandledRejection first [object Promise]",
            "unhandledRejection second [object Promise]"
        ]
    );
    let mut realm = Realm::new();
    scripted(&mut realm, &log, handled);
    reject(&mut realm, Value::str("third"));
    realm.run_jobs().unwrap();
    let explained = "UnhandledPromiseRejection: This error originated either by throwing inside of an async function without a catch block, or by rejecting a promise which was not handled with .catch(). The promise rejected with the reason \"'third'\". unhandledRejection";
    assert_eq!(
        entries(&log),
        [
            "unhandledRejection third [object Promise]".to_owned(),
            format!("uncaughtExceptionMonitor {explained}"),
            format!("uncaughtException {explained}"),
        ]
    );
    let mut realm = Realm::new();
    scripted(&mut realm, &log, |realm, name| match name {
        "unhandledRejection" => Err(error(realm, "listener")),
        _ => Ok(Value::Boolean(false)),
    });
    reject(&mut realm, Value::str("fourth"));
    let thrown = realm.run_jobs().unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "Error: listener");
    assert!(entries(&written).is_empty());
}

#[test]
fn process_exit_emits_exit_once_before_ending() {
    let mut session = shimforge::Session::new();
    let codes = Arc::new(Mutex::new(Vec::new()));
    let seen = codes.clone();
    shimforge::mock!(session, platform::exit_process, fn(u32))
        .expect()
        .returning(move |code| seen.lock().unwrap().push(code));
    let log = Log::default();
    let mut realm = Realm::new();
    let process = scripted(&mut realm, &log, unheard);
    let exit = ops::get(&mut realm, &process, &Key::name("exit")).unwrap();
    for _ in 0..2 {
        ops::call(&mut realm, &exit, process.clone(), &[Value::Number(3.0)]).unwrap();
    }
    assert_eq!(entries(&log), ["exit 3"]);
    assert_eq!(*codes.lock().unwrap(), [3, 3]);
    let exiting = ops::get(&mut realm, &process, &Key::name("_exiting")).unwrap();
    assert!(matches!(exiting, Value::Boolean(true)));
    // Errors in `exit` listeners leave `process.exit`.
    let mut realm = Realm::new();
    let process = scripted(&mut realm, &log, |realm, _| Err(error(realm, "in exit")));
    let exit = ops::get(&mut realm, &process, &Key::name("exit")).unwrap();
    let thrown = ops::call(&mut realm, &exit, process, &[]).unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "Error: in exit");
    assert_eq!(*codes.lock().unwrap(), [3, 3]);
    // So do errors marking the process as exiting.
    let mut realm = Realm::new();
    let process = scripted(&mut realm, &log, unheard);
    let get = realm.function("get", 0, false, |_, _, _| Ok(Value::Boolean(false)));
    let set = realm.function("set", 1, false, |realm, _, _| Err(error(realm, "frozen")));
    let exiting = Property {
        slot: Slot::Accessor {
            get: Some(get),
            set: Some(set),
        },
        enumerable: true,
        configurable: true,
    };
    process
        .as_object()
        .unwrap()
        .define(Key::name("_exiting"), exiting);
    let thrown = super::exit(&mut realm).unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "Error: frozen");
}

#[test]
fn warnings_are_emitted_as_events() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    let process = scripted(&mut realm, &log, unheard);
    let emit_warning = ops::get(&mut realm, &process, &Key::name("emitWarning")).unwrap();
    ops::call(&mut realm, &emit_warning, process, &[Value::str("careful")]).unwrap();
    realm.run_jobs().unwrap();
    assert_eq!(entries(&log), ["warning Warning: careful"]);
    assert!(entries(&written).is_empty());
}

/// `require` of a library whose `events` module is a small EventEmitter:
/// listeners are kept in an array per event on `_events`.
fn library(realm: &mut Realm, name: &str) -> Completion {
    assert_eq!(name, "events");
    let constructor = realm.function("EventEmitter", 0, false, |realm, this, _| {
        let events = Value::Object(realm.object());
        ops::set(realm, &this, &Key::name("_events"), events)?;
        Ok(Value::Undefined)
    });
    let prototype = realm.object();
    let on = realm.function("on", 2, false, |realm, this, args| {
        let events = ops::get(realm, &this, &Key::name("_events"))?;
        let name = ops::to_key(realm, &args[0])?;
        let listeners = match ops::get(realm, &events, &name)? {
            Value::Undefined => Value::Object(realm.array(Vec::new())),
            listeners => listeners,
        };
        let push = ops::get(realm, &listeners, &Key::name("push"))?;
        ops::call(realm, &push, listeners.clone(), &args[1..2])?;
        ops::set(realm, &events, &name, listeners)?;
        Ok(this)
    });
    let emit = realm.function("emit", 1, false, |realm, this, args| {
        let events = ops::get(realm, &this, &Key::name("_events"))?;
        let name = ops::to_key(realm, &args[0])?;
        let listeners = ops::get(realm, &events, &name)?;
        if matches!(listeners, Value::Undefined) {
            return Ok(Value::Boolean(false));
        }
        for listener in crate::iter::collect(realm, &listeners)? {
            ops::call(realm, &listener, this.clone(), &args[1..])?;
        }
        Ok(Value::Boolean(true))
    });
    for (key, function) in [("on", on), ("emit", emit)] {
        prototype.define(Key::name(key), Property::data(Value::Object(function)));
    }
    constructor.define(
        Key::name("prototype"),
        Property::data(Value::Object(prototype)),
    );
    Ok(Value::Object(constructor))
}

/// A library whose `events` module is still loading.
fn loading(realm: &mut Realm, _: &str) -> Completion {
    Ok(Value::Object(realm.object()))
}

fn failing(realm: &mut Realm, _: &str) -> Completion {
    Err(error(realm, "cannot load"))
}

#[test]
fn process_becomes_an_event_emitter_once_events_loads() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let mut realm = Realm::new();
    realm.node_library = Some(failing);
    let result = crate::process_builtin::process(&mut realm);
    assert_eq!(
        show(&mut realm, &result.unwrap_err().0),
        "Error: cannot load"
    );
    // Read while `events` loads, `process` is a plain object.
    realm.node_library = Some(loading);
    let process = crate::process_builtin::process(&mut realm).unwrap();
    assert!(!realm.process_emitter);
    realm.node_library = Some(library);
    assert!(same_value(
        &crate::process_builtin::process(&mut realm).unwrap(),
        &process
    ));
    assert!(realm.process_emitter);
    // A prototype-less `process` stays so, and cannot take listeners.
    let mut bare = Realm::new();
    bare.node_library = Some(loading);
    let plain = crate::process_builtin::process(&mut bare).unwrap();
    plain.as_object().unwrap().set_prototype(None);
    bare.node_library = Some(library);
    let result = crate::process_builtin::process(&mut bare);
    assert!(show(&mut bare, &result.unwrap_err().0).starts_with("TypeError"));
    assert!(bare.process_emitter);
    assert!(plain.as_object().unwrap().data().prototype.is_none());
    // Reads after wiring do not load `events` again.
    realm.node_library = Some(failing);
    crate::process_builtin::process(&mut realm).unwrap();
    let own = process
        .as_object()
        .unwrap()
        .data()
        .prototype
        .clone()
        .unwrap();
    let emitter = own.data().prototype.clone().unwrap();
    let on = ops::get(&mut realm, &Value::Object(emitter), &Key::name("on")).unwrap();
    assert!(crate::array_builtin::is_callable(&on));
    let events = ops::get(&mut realm, &process, &Key::name("_events")).unwrap();
    let mut names = Vec::new();
    for (name, count) in [
        ("newListener", 2.0),
        ("removeListener", 2.0),
        ("warning", 1.0),
    ] {
        let listeners = ops::get(&mut realm, &events, &Key::name(name)).unwrap();
        let length = ops::get(&mut realm, &listeners, &Key::name("length")).unwrap();
        assert!(same_value(&length, &Value::Number(count)), "{name}");
        names.push(name);
    }
    // Node's listeners: signal bookkeeping does nothing, `onWarning` prints
    // errors only.
    let emit = ops::get(&mut realm, &process, &Key::name("emit")).unwrap();
    for name in ["newListener", "removeListener"] {
        let result = ops::call(
            &mut realm,
            &emit,
            process.clone(),
            &[Value::str(name), Value::str("SIGINT")],
        );
        assert!(matches!(result.unwrap(), Value::Boolean(true)));
    }
    ops::call(
        &mut realm,
        &emit,
        process.clone(),
        &[Value::str("warning"), Value::str("text")],
    )
    .unwrap();
    assert!(entries(&written).is_empty());
    let warning = Value::Object(realm.error(ErrorKind::Error, "shown"));
    ops::call(
        &mut realm,
        &emit,
        process.clone(),
        &[Value::str("warning"), warning],
    )
    .unwrap();
    let shown = entries(&written).concat();
    assert!(shown.contains(") Error: shown\n"), "{shown}");
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(error(realm, "name getter"))
    });
    let failing = realm.error(ErrorKind::Error, "unnamed");
    failing.define(Key::name("name"), getter_only(getter));
    let args = [Value::str("warning"), Value::Object(failing)];
    let thrown = ops::call(&mut realm, &emit, process, &args).unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "Error: name getter");
}

#[test]
fn wiring_failures_surface_from_reading_process() {
    // An `events` module whose emitter cannot be called or listened to.
    fn uncallable(realm: &mut Realm, _: &str) -> Completion {
        Ok(Value::Object(realm.function(
            "EventEmitter",
            0,
            false,
            |realm, _, _| Err(error(realm, "constructor")),
        )))
    }
    let mut realm = Realm::new();
    realm.node_library = Some(uncallable);
    let result = crate::process_builtin::process(&mut realm);
    assert_eq!(
        show(&mut realm, &result.unwrap_err().0),
        "Error: constructor"
    );
    fn no_on(realm: &mut Realm, _: &str) -> Completion {
        Ok(Value::Object(realm.function(
            "EventEmitter",
            0,
            false,
            |_, _, _| Ok(Value::Undefined),
        )))
    }
    let mut realm = Realm::new();
    realm.node_library = Some(no_on);
    let result = crate::process_builtin::process(&mut realm);
    assert!(show(&mut realm, &result.unwrap_err().0).starts_with("TypeError"));
}

fn getter_only(get: Object) -> Property {
    Property {
        slot: Slot::Accessor {
            get: Some(get),
            set: None,
        },
        enumerable: false,
        configurable: true,
    }
}

#[test]
fn preload_realms_report_uncaught_exceptions_and_keep_running() {
    let mut session = shimforge::Session::new();
    let written = stderr(&mut session);
    let log = Log::default();
    let mut realm = Realm::new();
    realm.electron_window = 3;
    scripted(&mut realm, &log, unheard);
    assert_eq!(realm.fatal(error(&realm, "callback")), None);
    assert_eq!(
        entries(&log),
        [
            "uncaughtExceptionMonitor Error: callback uncaughtException",
            "uncaughtException Error: callback uncaughtException",
        ]
    );
    assert_eq!(entries(&written), ["Uncaught Error: callback\n"]);
    // Listeners that handle it leave nothing to report.
    scripted(&mut realm, &log, handled);
    realm.queue_host_error(error(&realm, "heard"));
    assert!(realm.step_host().is_ok());
    assert!(entries(&written).is_empty());
    assert_eq!(realm.event_loop.fatal, None);
    session.restore();
}
