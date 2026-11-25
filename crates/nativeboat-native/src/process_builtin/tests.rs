macro_rules! assert_value {
    ($actual:expr, $expected:expr $(,)?) => {{
        let actual = $actual;
        let expected = $expected;
        assert!(
            crate::same_value(&actual, &expected),
            "{actual:?} != {expected:?}"
        );
    }};
}

use super::*;
use crate::ops;

#[test]
fn cwd_has_stable_identity_and_node_wrapper_call_and_construct_behavior() {
    let mut realm = Realm::new();
    let function = cwd(&mut realm);
    assert!(crate::same_value(&function, &cwd(&mut realm)));
    let mut sibling = realm.sibling();
    assert!(!crate::same_value(&function, &cwd(&mut sibling)));
    assert_value!(
        ops::get(&mut realm, &function, &Key::name("name")).unwrap(),
        Value::str("wrappedCwd")
    );
    assert_value!(
        ops::get(&mut realm, &function, &Key::name("length")).unwrap(),
        Value::Number(0.0)
    );
    assert_value!(
        ops::call(&mut realm, &function, Value::Null, &[Value::Boolean(true)]).unwrap(),
        Value::String(platform::current_directory().unwrap())
    );
    assert!(matches!(
        ops::construct(&mut realm, &function, &[]).unwrap(),
        Value::Object(_)
    ));
    assert_value!(
        exec_path(&realm).unwrap(),
        Value::String(platform::executable_path().unwrap())
    );
}

#[test]
fn chdir_validates_arguments_changes_native_directory_and_maps_uv_errors() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let chdir = ops::get(&mut realm, &process, &Key::name("chdir")).unwrap();
    let mut session = shimforge::Session::new();
    let change = shimforge::mock!(
        session,
        platform::set_current_directory,
        fn(&JsStr) -> Result<(), u32>
    );
    change.expect().times(2).returning(|path| {
        if path == &JsStr::from("C:\\work") {
            Ok(())
        } else {
            Err(3)
        }
    });

    assert_value!(
        ops::call(
            &mut realm,
            &chdir,
            process.clone(),
            &[Value::str("C:\\work")]
        )
        .unwrap(),
        Value::Undefined
    );
    let invalid = ops::call(&mut realm, &chdir, process.clone(), &[]).unwrap_err();
    assert_value!(
        ops::get(&mut realm, &invalid.0, &Key::name("code")).unwrap(),
        Value::str("ERR_INVALID_ARG_TYPE")
    );
    let error = ops::call(&mut realm, &chdir, process, &[Value::str("C:\\missing")]).unwrap_err();
    for (name, value) in [
        ("name", Value::str("Error")),
        ("code", Value::str("ENOENT")),
        ("errno", Value::Number(-4058.0)),
        ("syscall", Value::str("chdir")),
        ("path", Value::str("C:\\missing")),
    ] {
        assert_value!(
            ops::get(&mut realm, &error.0, &Key::name(name)).unwrap(),
            value
        );
    }
}

#[test]
fn binding_exposes_only_the_buffer_binding_needed_by_safer_buffer() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let binding = ops::get(&mut realm, &process, &Key::name("binding")).unwrap();
    let buffer = ops::call(
        &mut realm,
        &binding,
        process.clone(),
        &[Value::str("buffer")],
    )
    .unwrap();
    let maximum = ops::get(&mut realm, &buffer, &Key::name("kStringMaxLength")).unwrap();
    assert_value!(maximum, Value::Number(536_870_888.0));
    let again = ops::call(
        &mut realm,
        &binding,
        process.clone(),
        &[Value::str("buffer")],
    )
    .unwrap();
    assert!(crate::same_value(&buffer, &again));
    let type_error =
        ops::call(&mut realm, &binding, process.clone(), &[Value::Number(1.0)]).unwrap_err();
    assert_value!(
        ops::get(&mut realm, &type_error.0, &Key::name("code")).unwrap(),
        Value::str("ERR_INVALID_ARG_TYPE")
    );
    let error = ops::call(&mut realm, &binding, process, &[Value::str("unsupported")]).unwrap_err();
    assert_value!(
        ops::get(&mut realm, &error.0, &Key::name("message")).unwrap(),
        Value::str("No such binding: unsupported")
    );
}

#[test]
fn os_path_failures_remain_catchable_and_preserve_the_windows_error() {
    let mut realm = Realm::new();
    let function = cwd(&mut realm);
    let mut session = shimforge::Session::new();
    let directory = shimforge::mock!(
        session,
        platform::current_directory,
        fn() -> Result<JsStr, u32>
    );
    directory.expect().returning(|| Err(3));
    let executable = shimforge::mock!(
        session,
        platform::executable_path,
        fn() -> Result<JsStr, u32>
    );
    executable.expect().returning(|| Err(5));
    let cwd_error = ops::call(&mut realm, &function, Value::Undefined, &[])
        .unwrap_err()
        .0;
    let exe_error = exec_path(&realm).unwrap_err().0;
    for (error, syscall, code) in [(cwd_error, "uv_cwd", 3), (exe_error, "uv_exepath", 5)] {
        assert_value!(
            ops::get(&mut realm, &error, &Key::name("name")).unwrap(),
            Value::str("Error")
        );
        assert_value!(
            ops::get(&mut realm, &error, &Key::name("syscall")).unwrap(),
            Value::str(syscall)
        );
        assert_value!(
            ops::get(&mut realm, &error, &Key::name("win32Code")).unwrap(),
            Value::Number(f64::from(code))
        );
        assert_value!(
            ops::get(&mut realm, &error, &Key::name("message")).unwrap(),
            Value::str(&format!("Windows error {code}, {syscall}"))
        );
    }
}

#[test]
fn native_arguments_are_stable_mutable_and_keep_executable_read_errors() {
    let mut realm = Realm::new();
    let arguments = argv(&mut realm).unwrap();
    assert!(crate::same_value(&arguments, &argv(&mut realm).unwrap()));
    assert_value!(
        ops::get(&mut realm, &arguments, &Key::Index(0)).unwrap(),
        exec_path(&realm).unwrap()
    );
    ops::set(
        &mut realm,
        &arguments,
        &Key::Index(0),
        Value::str("changed"),
    )
    .unwrap();
    let again = argv(&mut realm).unwrap();
    assert_value!(
        ops::get(&mut realm, &again, &Key::Index(0)).unwrap(),
        Value::str("changed")
    );
    let mut sibling = realm.sibling();
    assert!(!crate::same_value(&arguments, &argv(&mut sibling).unwrap()));
    let mut fresh = Realm::new();
    let mut session = shimforge::Session::new();
    let executable = shimforge::mock!(
        session,
        platform::executable_path,
        fn() -> Result<JsStr, u32>
    );
    executable.expect().returning(|| Err(5));
    let error = argv(&mut fresh).unwrap_err().0;
    assert_value!(
        ops::get(&mut fresh, &error, &Key::name("win32Code")).unwrap(),
        Value::Number(5.0)
    );
    assert!(fresh.process_argv.is_none());
}

use crate::{Object, Slot};
use std::sync::{Arc, Mutex};

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap()
}

fn call(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> Completion {
    let function = get(realm, target, name);
    ops::call(realm, &function, target.clone(), args)
}

fn text(realm: &mut Realm, value: &Value) -> String {
    ops::to_string(realm, value).unwrap().to_rust_lossy()
}

fn thrown(realm: &mut Realm, result: Completion) -> String {
    let error = result.unwrap_err().0;
    text(realm, &error)
}

fn global_process(realm: &mut Realm) -> Value {
    let global = Value::Object(realm.global.clone());
    get(realm, &global, "process")
}

/// A function that appends `label` and its arguments to `log`.
fn logger(realm: &Realm, log: &Arc<Mutex<Vec<String>>>, label: &'static str) -> Value {
    let log = log.clone();
    Value::Object(realm.function(label, 0, false, move |realm, _, args| {
        let mut entry = label.to_owned();
        for value in args {
            entry.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
        }
        log.lock().unwrap().push(entry);
        Ok(Value::Undefined)
    }))
}

#[test]
fn the_process_global_is_created_once_and_replaced_by_assignment() {
    let mut realm = Realm::new();
    assert!(realm.process.is_none());
    let process = global_process(&mut realm);
    assert!(crate::same_value(&process, &global_process(&mut realm)));
    assert!(realm.process.is_some());
    let accessor = realm.global.own_property(&Key::name("process")).unwrap();
    assert!(!accessor.enumerable && accessor.configurable);
    let Slot::Accessor {
        set: Some(setter), ..
    } = accessor.slot
    else {
        panic!("process is an accessor");
    };
    let refused = ops::call(&mut realm, &Value::Object(setter), Value::Undefined, &[]);
    assert_eq!(
        thrown(&mut realm, refused),
        "TypeError: Object.defineProperty called on non-object"
    );
    let global = Value::Object(realm.global.clone());
    ops::set(
        &mut realm,
        &global,
        &Key::name("process"),
        Value::Number(5.0),
    )
    .unwrap();
    assert_value!(global_process(&mut realm), Value::Number(5.0));
    let replaced = realm.global.own_property(&Key::name("process")).unwrap();
    assert!(!replaced.enumerable && replaced.configurable);
}

#[test]
fn process_members_follow_node_and_electron_main_processes() {
    let mut realm = Realm::new();
    let plain = process(&mut realm).unwrap();
    let versions = get(&mut realm, &plain, "versions");
    assert_value!(get(&mut realm, &versions, "node"), Value::str(NODE_VERSION));
    assert_value!(get(&mut realm, &versions, "electron"), Value::Undefined);
    let mut realm = Realm::new();
    realm.electron_version = Some("42.1.0");
    let process = process(&mut realm).unwrap();
    let versions = get(&mut realm, &process, "versions");
    assert_value!(get(&mut realm, &versions, "electron"), Value::str("42.1.0"));
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    let prototype = Value::Object(
        process
            .as_object()
            .unwrap()
            .data()
            .prototype
            .clone()
            .unwrap(),
    );
    assert_value!(
        ops::get(&mut realm, &prototype, &tag).unwrap(),
        Value::str("process")
    );
    let exec_path = platform::executable_path().unwrap();
    for (name, expected) in [
        ("version", Value::str(&format!("v{NODE_VERSION}"))),
        ("arch", Value::str("x64")),
        ("platform", Value::str("win32")),
        ("pid", Value::Number(f64::from(platform::process_id()))),
        ("execPath", Value::String(exec_path.clone())),
        (
            "argv0",
            Value::String(platform::command_arguments()[0].clone()),
        ),
        ("type", Value::str("browser")),
        ("resourcesPath", Value::String(resources_path(&exec_path))),
        ("cwd", cwd(&mut realm)),
        ("env", crate::environment_builtin::environment(&mut realm)),
        ("argv", argv(&mut realm).unwrap()),
    ] {
        assert_value!(get(&mut realm, &process, name), expected);
    }
    let exec_argv = get(&mut realm, &process, "execArgv");
    assert_value!(get(&mut realm, &exec_argv, "length"), Value::Number(0.0));
    assert_eq!(
        resources_path(&JsStr::from("C:\\apps\\Mark Text.exe")),
        JsStr::from("C:\\apps\\resources")
    );
    assert_eq!(
        resources_path(&JsStr::from("app.exe")),
        JsStr::from("resources")
    );
}

#[test]
fn hrtime_uses_monotonic_normalized_tuples_and_exposes_bigint() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let clock = get(&mut realm, &process, "hrtime");
    let start = ops::call(&mut realm, &clock, process.clone(), &[]).unwrap();
    let Value::Object(start) = start else {
        panic!("hrtime returns an array");
    };
    assert!(start.is_array());
    let seconds = ops::get(&mut realm, &Value::Object(start.clone()), &Key::Index(0)).unwrap();
    let nanos = ops::get(&mut realm, &Value::Object(start.clone()), &Key::Index(1)).unwrap();
    assert!(matches!(seconds, Value::Number(value) if value >= 0.0));
    assert!(matches!(nanos, Value::Number(value) if (0.0..1_000_000_000.0).contains(&value)));

    let diff = ops::call(&mut realm, &clock, process.clone(), &[Value::Object(start)]).unwrap();
    let Value::Object(diff) = diff else {
        panic!("hrtime returns an array");
    };
    let seconds = ops::get(&mut realm, &Value::Object(diff.clone()), &Key::Index(0)).unwrap();
    let nanos = ops::get(&mut realm, &Value::Object(diff), &Key::Index(1)).unwrap();
    assert!(matches!(seconds, Value::Number(value) if value >= 0.0));
    assert!(matches!(nanos, Value::Number(value) if (0.0..1_000_000_000.0).contains(&value)));

    let future =
        Value::Object(realm.array(vec![Value::Number(1_000_000_000.0), Value::Number(0.0)]));
    let negative = ops::call(&mut realm, &clock, process.clone(), &[future]).unwrap();
    let Value::Object(negative) = negative else {
        panic!("hrtime returns an array");
    };
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(negative.clone()), &Key::Index(0)).unwrap(),
        Value::Number(value) if value < 0.0
    ));
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(negative), &Key::Index(1)).unwrap(),
        Value::Number(value) if (0.0..1_000_000_000.0).contains(&value)
    ));

    let short = Value::Object(realm.array(vec![Value::Number(0.0)]));
    assert!(ops::call(&mut realm, &clock, process.clone(), &[short]).is_err());
    let non_finite =
        Value::Object(realm.array(vec![Value::Number(f64::INFINITY), Value::Number(0.0)]));
    assert!(ops::call(&mut realm, &clock, process.clone(), &[non_finite]).is_err());
    for seconds in [1.5, MAX_SAFE_INTEGER + 1.0] {
        let invalid = Value::Object(realm.array(vec![Value::Number(seconds), Value::Number(0.0)]));
        assert!(ops::call(&mut realm, &clock, process.clone(), &[invalid]).is_err());
    }
    assert!(
        ops::call(
            &mut realm,
            &clock,
            process.clone(),
            &[Value::str("invalid")]
        )
        .is_err()
    );

    let bigint = get(&mut realm, &clock, "bigint");
    assert!(matches!(
        ops::call(&mut realm, &bigint, clock, &[]),
        Ok(Value::BigInt(_))
    ));
}

#[test]
fn process_kill_checks_and_terminates_windows_processes() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let pid = platform::process_id();
    assert_value!(
        call(
            &mut realm,
            &process,
            "kill",
            &[Value::Number(f64::from(pid)), Value::Number(0.0)],
        )
        .unwrap(),
        Value::Boolean(true)
    );

    let unknown = call(
        &mut realm,
        &process,
        "kill",
        &[Value::Number(f64::from(pid)), Value::str("SIGUNKNOWN")],
    )
    .unwrap_err()
    .0;
    assert_value!(
        ops::get(&mut realm, &unknown, &Key::name("code")).unwrap(),
        Value::str("ERR_UNKNOWN_SIGNAL")
    );
    assert!(
        call(
            &mut realm,
            &process,
            "kill",
            &[Value::Number(0.0), Value::Number(0.0)],
        )
        .is_err()
    );

    let missing = call(
        &mut realm,
        &process,
        "kill",
        &[Value::Number(f64::from(u32::MAX)), Value::Number(0.0)],
    )
    .unwrap_err()
    .0;
    assert_value!(
        ops::get(&mut realm, &missing, &Key::name("code")).unwrap(),
        Value::str("ESRCH")
    );

    let mut child = std::process::Command::new("cmd.exe")
        .args(["/c", "ping 127.0.0.1 -n 60 > nul"])
        .spawn()
        .unwrap();
    let sent = call(
        &mut realm,
        &process,
        "kill",
        &[Value::Number(f64::from(child.id())), Value::str("SIGTERM")],
    );
    if sent.is_err() {
        let _ = child.kill();
    }
    assert_value!(sent.unwrap(), Value::Boolean(true));
    assert!(!child.wait().unwrap().success());
}

#[test]
fn process_kill_validates_signals_and_reports_native_access_failures() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    assert!(call(&mut realm, &process, "kill", &[Value::str("42")]).is_err());
    for signal in [
        Value::Number(999.0),
        Value::Boolean(true),
        Value::Number(2.5),
    ] {
        assert!(call(&mut realm, &process, "kill", &[Value::Number(42.0), signal]).is_err());
    }
    {
        let mut session = shimforge::Session::new();
        let terminate = shimforge::mock!(session, platform::kill, fn(u32, i32) -> Result<(), u32>);
        terminate.expect().once().returning(|pid, signal| {
            assert_eq!((pid, signal), (42, 15));
            Ok(())
        });
        assert!(call(&mut realm, &process, "kill", &[Value::Number(42.0)]).is_ok());
    }
    {
        let mut session = shimforge::Session::new();
        let terminate = shimforge::mock!(session, platform::kill, fn(u32, i32) -> Result<(), u32>);
        terminate.expect().once().returning(|pid, signal| {
            assert_eq!((pid, signal), (42, 15));
            Err(5)
        });
        let error = call(
            &mut realm,
            &process,
            "kill",
            &[Value::Number(42.0), Value::Number(15.0)],
        )
        .unwrap_err()
        .0;
        assert_value!(get(&mut realm, &error, "code"), Value::str("EPERM"));
    }
}

#[test]
fn process_creation_failures_are_not_cached() {
    let mut realm = Realm::new();
    {
        let mut session = shimforge::Session::new();
        let executable = shimforge::mock!(
            session,
            platform::executable_path,
            fn() -> Result<JsStr, u32>
        );
        executable.expect().returning(|| Err(5));
        assert!(process(&mut realm).is_err());
        assert!(realm.process.is_none());
    }
    let mut session = shimforge::Session::new();
    let arguments = shimforge::mock!(session, platform::command_arguments, fn() -> Vec<JsStr>);
    arguments.expect().returning(Vec::new);
    let process = process(&mut realm).unwrap();
    assert_value!(get(&mut realm, &process, "argv0"), Value::str(""));
}

#[test]
fn exit_codes_validate_like_node() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let code = Key::name("exitCode");
    for (value, expected) in [
        (Value::str("0x10"), Value::Number(16.0)),
        (Value::Number(-0.0), Value::Number(-0.0)),
        (Value::Null, Value::Undefined),
        (Value::Number(3.0), Value::Number(3.0)),
        (Value::Undefined, Value::Undefined),
    ] {
        ops::set(&mut realm, &process, &code, value).unwrap();
        assert_value!(ops::get(&mut realm, &process, &code).unwrap(), expected);
    }
    for (value, expected) in [
        (
            Value::str("abc"),
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"code\" argument must be of type number. Received type string ('abc')",
        ),
        (
            Value::str(""),
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"code\" argument must be of type number. Received type string ('')",
        ),
        (
            Value::Boolean(true),
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"code\" argument must be of type number. Received type boolean (true)",
        ),
        (
            Value::str("1.5"),
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"code\" argument must be of type number. Received type string ('1.5')",
        ),
        (
            Value::Number(1.5),
            "RangeError [ERR_OUT_OF_RANGE]: The value of \"code\" is out of range. It must be an integer. Received 1.5",
        ),
        (
            Value::Number(f64::NAN),
            "RangeError [ERR_OUT_OF_RANGE]: The value of \"code\" is out of range. It must be an integer. Received NaN",
        ),
        (
            Value::Number(9_007_199_254_740_992.0),
            "RangeError [ERR_OUT_OF_RANGE]: The value of \"code\" is out of range. It must be >= -9007199254740991 && <= 9007199254740991. Received 9_007_199_254_740_992",
        ),
    ] {
        let result = ops::set(&mut realm, &process, &code, value).map(|()| Value::Undefined);
        assert_eq!(thrown(&mut realm, result), expected);
    }
    assert_eq!(realm.exit_status(), 0);
    realm.exit_code = Value::Number(4_294_967_299.0);
    assert_eq!(realm.exit_status(), 3);
    realm.exit_code = Value::Number(-1.0);
    assert_eq!(realm.exit_status(), -1);
}

#[test]
fn exit_records_the_code_and_ends_the_process() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let invalid = call(&mut realm, &process, "exit", &[Value::str("x")]);
    assert!(thrown(&mut realm, invalid).starts_with("TypeError [ERR_INVALID_ARG_TYPE]"));
    let codes = Arc::new(Mutex::new(Vec::new()));
    let mut session = shimforge::Session::new();
    let exit = shimforge::mock!(session, platform::exit_process, fn(u32));
    let seen = codes.clone();
    exit.expect()
        .returning(move |code| seen.lock().unwrap().push(code));
    for args in [vec![Value::str("7")], vec![], vec![Value::Number(-1.0)]] {
        call(&mut realm, &process, "exit", &args).unwrap();
    }
    assert_eq!(*codes.lock().unwrap(), [7, 7, u32::MAX]);
}

#[test]
fn next_ticks_run_before_promise_jobs() {
    let mut realm = Realm::new();
    let process = process(&mut realm).unwrap();
    let invalid = call(&mut realm, &process, "nextTick", &[Value::Number(1.0)]);
    assert_eq!(
        thrown(&mut realm, invalid),
        "TypeError [ERR_INVALID_ARG_TYPE]: The \"callback\" argument must be of type function. Received type number (1)"
    );
    let log = Arc::new(Mutex::new(Vec::new()));
    let job = logger(&realm, &log, "job");
    let global = Value::Object(realm.global.clone());
    call(&mut realm, &global, "queueMicrotask", &[job]).unwrap();
    let tick = logger(&realm, &log, "tick");
    let (first, second) = (Value::str("a"), Value::str("b"));
    call(&mut realm, &process, "nextTick", &[tick, first, second]).unwrap();
    // A tick queued by a promise job runs after the remaining jobs.
    let later = logger(&realm, &log, "later");
    let process_value = process.clone();
    let queue = realm.function("queue", 0, false, move |realm, _, _| {
        call(
            realm,
            &process_value,
            "nextTick",
            std::slice::from_ref(&later),
        )
    });
    call(
        &mut realm,
        &global,
        "queueMicrotask",
        &[Value::Object(queue)],
    )
    .unwrap();
    let last = logger(&realm, &log, "last");
    call(&mut realm, &global, "queueMicrotask", &[last]).unwrap();
    realm.run_jobs().unwrap();
    assert_eq!(*log.lock().unwrap(), ["tickab", "job", "last", "later"]);
    let failing = realm.function("fail", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "tick"))
    });
    call(&mut realm, &process, "nextTick", &[Value::Object(failing)]).unwrap();
    let result = realm.run_jobs().map(|()| Value::Undefined);
    assert_eq!(thrown(&mut realm, result), "RangeError: tick");
}

/// Captures standard error and standard stream writes for one test.
/// Writes by stream: 1 for standard output, 2 for standard error.
type Written = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

fn capture(session: &mut shimforge::Session) -> Written {
    let written = Arc::new(Mutex::new(Vec::new()));
    let errors = written.clone();
    shimforge::mock!(session, platform::write_stderr, fn(&str) -> bool)
        .expect()
        .returning(move |text| {
            errors.lock().unwrap().push((2, text.as_bytes().to_vec()));
            true
        });
    let streams = written.clone();
    shimforge::mock!(session, platform::write_standard, fn(u32, &[u8]) -> bool)
        .expect()
        .returning(move |descriptor, bytes| {
            streams.lock().unwrap().push((descriptor, bytes.to_vec()));
            true
        });
    written
}

fn errors_written(written: &Written) -> String {
    let written = written.lock().unwrap();
    written
        .iter()
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .collect()
}

#[test]
fn warnings_validate_and_print_like_node() {
    let mut realm = Realm::new();
    let mut session = shimforge::Session::new();
    let written = capture(&mut session);
    let process = process(&mut realm).unwrap();
    let options = realm.object();
    options.define(Key::name("type"), Property::data(Value::Number(5.0)));
    let array = Value::Object(realm.array(Vec::new()));
    let function = Value::Object(realm.function("f", 0, false, |_, _, _| Ok(Value::Undefined)));
    let plain = Value::Object(realm.object());
    for (args, expected) in [
        (
            vec![Value::Number(1.0)],
            "The \"warning\" argument must be of type string or an instance of Error. Received type number (1)",
        ),
        (
            vec![plain],
            "The \"warning\" argument must be of type string or an instance of Error. Received an instance of Object",
        ),
        (
            vec![Value::str("x"), Value::Number(5.0)],
            "The \"type\" argument must be of type string. Received type number (5)",
        ),
        (
            vec![Value::str("x"), Value::str("T"), Value::Number(5.0)],
            "The \"code\" argument must be of type string. Received type number (5)",
        ),
        (
            vec![Value::str("x"), array],
            "The \"type\" argument must be of type string. Received an instance of Array",
        ),
        (
            vec![Value::str("x"), Value::Object(options)],
            "The \"type\" argument must be of type string. Received type number (5)",
        ),
    ] {
        let result = call(&mut realm, &process, "emitWarning", &args);
        let message = thrown(&mut realm, result);
        assert_eq!(
            message,
            format!("TypeError [ERR_INVALID_ARG_TYPE]: {expected}")
        );
    }
    let detailed = realm.object();
    for (name, value) in [
        ("code", Value::str("C1")),
        ("detail", Value::str("more")),
        ("type", Value::str("Custom")),
    ] {
        detailed.define(Key::name(name), Property::data(value));
    }
    let undetailed = realm.object();
    undetailed.define(Key::name("detail"), Property::data(Value::Number(3.0)));
    let custom = realm.error(ErrorKind::Error, "err");
    custom.define(Key::name("name"), Property::data(Value::str("MyWarning")));
    custom.define(Key::name("toString"), Property::data(Value::Number(1.0)));
    let derived_prototype = Object::new(Some(realm.error_prototype.clone()), crate::Kind::Ordinary);
    let derived = Object::new(Some(derived_prototype), crate::Kind::Ordinary);
    derived.define(
        Key::name("message"),
        Property::hidden(Value::str("derived")),
    );
    for args in [
        vec![Value::str("w1")],
        vec![
            Value::str("w2"),
            Value::str("DeprecationWarning"),
            Value::str("DEP0001"),
        ],
        vec![Value::str("w3"), Value::Object(detailed)],
        vec![Value::str("w4"), Value::Object(undetailed)],
        vec![Value::str("w5"), function.clone(), Value::str("ignored")],
        vec![Value::str("w6"), Value::str(""), function],
        vec![Value::Object(custom)],
        vec![Value::Object(derived)],
    ] {
        call(&mut realm, &process, "emitWarning", &args).unwrap();
    }
    assert!(written.lock().unwrap().is_empty());
    realm.run_jobs().unwrap();
    let pid = platform::process_id();
    let program = basename(&platform::command_arguments()[0].to_rust_lossy()).to_owned();
    assert_eq!(
        errors_written(&written),
        format!(
            "(node:{pid}) Warning: w1\n(Use `{program} --trace-warnings ...` to show where the warning was created)\n\
             (node:{pid}) [DEP0001] DeprecationWarning: w2\n\
             (node:{pid}) [C1] Custom: w3\nmore\n\
             (node:{pid}) Warning: w4\n\
             (node:{pid}) Warning: w5\n\
             (node:{pid}) Warning: w6\n\
             (node:{pid}) MyWarning: err\n\
             (node:{pid}) Error: derived\n"
        )
    );
    // Deprecations honor the process flags; the first shows its own hint.
    written.lock().unwrap().clear();
    let mut realm = Realm::new();
    let process = super::process(&mut realm).unwrap();
    let deprecation = [Value::str("d"), Value::str("DeprecationWarning")];
    ops::set(
        &mut realm,
        &process,
        &Key::name("noDeprecation"),
        Value::Boolean(true),
    )
    .unwrap();
    call(&mut realm, &process, "emitWarning", &deprecation).unwrap();
    realm.run_jobs().unwrap();
    assert!(written.lock().unwrap().is_empty());
    ops::set(
        &mut realm,
        &process,
        &Key::name("noDeprecation"),
        Value::Boolean(false),
    )
    .unwrap();
    call(&mut realm, &process, "emitWarning", &deprecation).unwrap();
    realm.run_jobs().unwrap();
    assert_eq!(
        errors_written(&written),
        format!(
            "(node:{pid}) DeprecationWarning: d\n(Use `{program} --trace-deprecation ...` to show where the warning was created)\n"
        )
    );
    ops::set(
        &mut realm,
        &process,
        &Key::name("throwDeprecation"),
        Value::Boolean(true),
    )
    .unwrap();
    call(&mut realm, &process, "emitWarning", &deprecation).unwrap();
    let result = realm.run_jobs().map(|()| Value::Undefined);
    assert_eq!(thrown(&mut realm, result), "DeprecationWarning: d");
}

#[test]
fn warning_getters_and_missing_program_names() {
    let mut realm = Realm::new();
    let mut session = shimforge::Session::new();
    let written = capture(&mut session);
    shimforge::mock!(session, platform::command_arguments, fn() -> Vec<JsStr>)
        .expect()
        .returning(Vec::new);
    let process = process(&mut realm).unwrap();
    call(&mut realm, &process, "emitWarning", &[Value::str("w")]).unwrap();
    realm.run_jobs().unwrap();
    assert!(errors_written(&written).contains("(Use `node --trace-warnings ...`"));
    // Getters on options and warnings run, and their errors propagate.
    let failing = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "getter"))
    });
    let accessor = || Property {
        slot: Slot::Accessor {
            get: Some(failing.clone()),
            set: None,
        },
        enumerable: true,
        configurable: true,
    };
    let options = realm.object();
    options.define(Key::name("ctor"), accessor());
    let result = call(
        &mut realm,
        &process,
        "emitWarning",
        &[Value::str("w"), Value::Object(options)],
    );
    assert_eq!(thrown(&mut realm, result), "RangeError: getter");
    for name in ["name", "code"] {
        let warning = realm.error(ErrorKind::Error, "w");
        warning.define(Key::name(name), accessor());
        let args = [Value::Object(warning)];
        let result = call(&mut realm, &process, "emitWarning", &args);
        let result = result.and_then(|_| realm.run_jobs().map(|()| Value::Undefined));
        assert_eq!(thrown(&mut realm, result), "RangeError: getter", "{name}");
    }
    let warning = realm.error(ErrorKind::Error, "w");
    warning.define(Key::name("detail"), accessor());
    call(
        &mut realm,
        &process,
        "emitWarning",
        &[Value::Object(warning)],
    )
    .unwrap();
    let result = realm.run_jobs().map(|()| Value::Undefined);
    assert_eq!(thrown(&mut realm, result), "RangeError: getter");
    // Process flags are read through getters too.
    for flag in ["noDeprecation", "throwDeprecation"] {
        let process_object = process.as_object().unwrap();
        process_object.define(Key::name(flag), accessor());
        let args = [Value::str("d"), Value::str("DeprecationWarning")];
        let result = call(&mut realm, &process, "emitWarning", &args);
        assert_eq!(thrown(&mut realm, result), "RangeError: getter", "{flag}");
        process_object.define(Key::name(flag), Property::data(Value::Undefined));
    }
}

#[test]
fn program_names_drop_directories_and_the_executable_extension() {
    for (path, expected) in [
        ("C:\\apps\\MarkText.exe", "MarkText"),
        ("C:/apps/tool", "tool"),
        (".exe", ".exe"),
        ("node", "node"),
    ] {
        assert_eq!(basename(path), expected);
    }
}

#[test]
fn standard_streams_write_strings_and_views() {
    let mut realm = Realm::new();
    let mut session = shimforge::Session::new();
    let written = capture(&mut session);
    shimforge::mock!(session, platform::is_console, fn(u32) -> bool)
        .expect()
        .returning(|descriptor| descriptor == 1);
    let process = process(&mut realm).unwrap();
    let stdout = get(&mut realm, &process, "stdout");
    let stderr = get(&mut realm, &process, "stderr");
    assert!(crate::same_value(
        &stdout,
        &get(&mut realm, &process, "stdout")
    ));
    assert_value!(get(&mut realm, &stdout, "fd"), Value::Number(1.0));
    assert_value!(get(&mut realm, &stdout, "isTTY"), Value::Boolean(true));
    assert_value!(get(&mut realm, &stderr, "isTTY"), Value::Undefined);
    let log = Arc::new(Mutex::new(Vec::new()));
    let callback = logger(&realm, &log, "done");
    let bytes = {
        let array = Value::Object(realm.array(vec![Value::Number(65.0), Value::Number(66.0)]));
        let global = Value::Object(realm.global.clone());
        let constructor = get(&mut realm, &global, "Uint8Array");
        ops::construct(&mut realm, &constructor, &[array]).unwrap()
    };
    let view = {
        let buffer = get(&mut realm, &bytes, "buffer");
        let global = Value::Object(realm.global.clone());
        let constructor = get(&mut realm, &global, "DataView");
        ops::construct(&mut realm, &constructor, &[buffer, Value::Number(1.0)]).unwrap()
    };
    for args in [
        vec![Value::str("é"), callback.clone()],
        vec![Value::str("é"), Value::str("latin1"), callback.clone()],
        vec![Value::str("x"), Value::str(""), Value::Number(1.0)],
        vec![bytes.clone(), Value::str("buffer")],
        vec![view],
    ] {
        assert_value!(
            call(&mut realm, &stdout, "write", &args).unwrap(),
            Value::Boolean(true)
        );
    }
    call(&mut realm, &stderr, "write", &[Value::str("e")]).unwrap();
    assert_eq!(
        *written.lock().unwrap(),
        [
            (1, "é".as_bytes().to_vec()),
            (1, vec![0xe9]),
            (1, b"x".to_vec()),
            (1, b"AB".to_vec()),
            (1, b"B".to_vec()),
            (2, b"e".to_vec()),
        ]
    );
    realm.run_jobs().unwrap();
    assert_eq!(*log.lock().unwrap(), ["donenull", "donenull"]);
    let symbol = Value::Symbol(crate::Symbol::new(Some(JsStr::from("e"))));
    let plain = Value::Object(realm.object());
    for (args, expected) in [
        (
            vec![Value::str("x"), Value::str("nope")],
            "TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: nope",
        ),
        (
            vec![Value::str("x"), Value::str("buffer")],
            "TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: buffer",
        ),
        (
            vec![Value::str("x"), Value::Number(5.0)],
            "TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: 5",
        ),
        (
            vec![Value::str("x"), symbol],
            "TypeError [ERR_UNKNOWN_ENCODING]: Unknown encoding: Symbol(e)",
        ),
        (
            vec![Value::Null],
            "TypeError [ERR_STREAM_NULL_VALUES]: May not write null values to stream",
        ),
        (
            vec![plain],
            "TypeError [ERR_INVALID_ARG_TYPE]: The \"chunk\" argument must be of type string or an instance of Buffer, TypedArray, or DataView. Received an instance of Object",
        ),
    ] {
        let result = call(&mut realm, &stdout, "write", &args);
        assert_eq!(thrown(&mut realm, result), expected);
    }
    let encoding = realm.object();
    let to_string = realm.function("toString", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "encoding"))
    });
    encoding.define(
        Key::name("toString"),
        Property::data(Value::Object(to_string)),
    );
    let result = call(
        &mut realm,
        &stdout,
        "write",
        &[Value::str("x"), Value::Object(encoding)],
    );
    assert_eq!(thrown(&mut realm, result), "RangeError: encoding");
}

fn test_event_library(realm: &mut Realm, module: &str) -> Completion {
    assert_eq!(module, "events");
    let emitter = realm.function("EventEmitter", 0, false, |_, _, _| Ok(Value::Undefined));
    let prototype = realm.object();
    let on = realm.function("on", 2, false, |realm, this, args| {
        let event =
            ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
        let callback = args.get(1).cloned().unwrap_or_default();
        ops::set(
            realm,
            &this,
            &Key::name(&format!("__event_{event}")),
            callback,
        )?;
        Ok(this)
    });
    let emit = realm.function("emit", 1, false, |realm, this, args| {
        let event =
            ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
        let callback = ops::get(realm, &this, &Key::name(&format!("__event_{event}")))?;
        if crate::array_builtin::is_callable(&callback) {
            ops::call(realm, &callback, Value::Undefined, &args[1..])?;
            Ok(Value::Boolean(true))
        } else {
            Ok(Value::Boolean(false))
        }
    });
    prototype.define(Key::name("on"), Property::data(Value::Object(on)));
    prototype.define(Key::name("emit"), Property::data(Value::Object(emit)));
    emitter.define(
        Key::name("prototype"),
        Property::data(Value::Object(prototype)),
    );
    Ok(Value::Object(emitter))
}

#[test]
fn process_standard_streams_inherit_node_event_emitter_listeners() {
    let mut realm = Realm::new();
    realm.node_library = Some(test_event_library);
    let process = process(&mut realm).unwrap();
    let stdout = get(&mut realm, &process, "stdout");
    let stderr = get(&mut realm, &process, "stderr");
    let calls = Arc::new(Mutex::new(Vec::new()));
    for (stream, name) in [(stdout, "stdout"), (stderr, "stderr")] {
        let calls = calls.clone();
        let listener = realm.function("listener", 1, false, move |realm, _, args| {
            let text = args
                .first()
                .map(|value| ops::to_string(realm, value))
                .transpose()?
                .map(|text| text.to_rust_lossy())
                .unwrap_or_default();
            calls.lock().unwrap().push((name, text));
            Ok(Value::Undefined)
        });
        call(
            &mut realm,
            &stream,
            "on",
            &[Value::str("error"), Value::Object(listener)],
        )
        .unwrap();
        call(
            &mut realm,
            &stream,
            "emit",
            &[Value::str("error"), Value::str("closed")],
        )
        .unwrap();
    }
    assert_eq!(
        *calls.lock().unwrap(),
        [
            ("stdout", "closed".to_owned()),
            ("stderr", "closed".to_owned())
        ]
    );
}
