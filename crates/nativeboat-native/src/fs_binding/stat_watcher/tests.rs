use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-stat-watch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn method(realm: &mut Realm, object: &Value, name: &str, args: &[Value]) -> Completion {
    let method = ops::get(realm, object, &Key::name(name))?;
    ops::call(realm, &method, object.clone(), args)
}

fn new(realm: &mut Realm, bigint: bool) -> Value {
    let fs = crate::modules::load(realm, "binding/fs").unwrap();
    let constructor = ops::get(realm, &fs, &Key::name("StatWatcher")).unwrap();
    ops::construct(realm, &constructor, &[Value::Boolean(bigint)]).unwrap()
}

fn number(realm: &mut Realm, stats: &Value, index: usize) -> f64 {
    let value = ops::get(realm, stats, &Key::Index(index as u32)).unwrap();
    ops::to_number(realm, &value).unwrap()
}

fn watchdog(realm: &mut Realm, watcher: Value) -> (Object, Rc<Cell<bool>>) {
    let timed_out = Rc::new(Cell::new(false));
    let failed = timed_out.clone();
    let callback = realm.function("watchdog", 0, false, move |realm, _, _| {
        failed.set(true);
        method(realm, &watcher, "close", &[])?;
        Ok(Value::Undefined)
    });
    (
        realm.native_timeout(callback, 3000.0, Rc::new(Cell::new(true))),
        timed_out,
    )
}

/// Gives a file new contents at once, as a polling watcher must see them:
/// writing in place empties the file before writing, and a poll between
/// the two would see it empty.
fn replace(path: &std::path::Path, contents: &[u8]) {
    let staged = path.with_extension("staged");
    std::fs::write(&staged, contents).unwrap();
    std::fs::rename(&staged, path).unwrap();
}

#[test]
fn polling_reports_missing_create_change_delete_with_current_and_previous_stats() {
    let directory = Directory::new();
    let path = directory.0.join("poll-中.txt");
    let mut realm = Realm::new();
    let watcher = new(&mut realm, false);
    let (guard, timed_out) = watchdog(&mut realm, watcher.clone());
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let target = watcher.clone();
    let file = path.clone();
    let callback = realm.function("onchange", 2, false, move |realm, this, args| {
        assert!(crate::same_value(&this, &target));
        let status = ops::to_number(realm, &args[0]).unwrap();
        let current = number(realm, &args[1], 8);
        let previous = number(realm, &args[1], 26);
        match count.get() {
            0 => {
                assert!(status < 0.0);
                assert_eq!((current, previous), (0.0, 0.0));
                replace(&file, b"one");
            }
            1 => {
                assert_eq!((status, current, previous), (0.0, 3.0, 0.0));
                replace(&file, b"changed");
            }
            2 => {
                assert_eq!((status, current, previous), (0.0, 7.0, 3.0));
                std::fs::remove_file(&file).unwrap();
            }
            3 => {
                assert!(status < 0.0);
                assert_eq!((current, previous), (0.0, 7.0));
                method(realm, &target, "close", &[])?;
                realm.clear(&Value::Object(guard.clone()));
            }
            _ => panic!("unexpected poll callback"),
        }
        count.set(count.get() + 1);
        Ok(Value::Undefined)
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    let arguments = [Value::str(path.to_str().unwrap()), Value::Number(0.0)];
    assert!(matches!(
        method(&mut realm, &watcher, "start", &arguments).unwrap(),
        Value::Undefined
    ));
    // Starting an active watcher must not reset its path or interval.
    method(&mut realm, &watcher, "start", &[]).unwrap();
    realm.run_event_loop().unwrap();
    assert!(!timed_out.get());
    assert_eq!(calls.get(), 4);
    assert!(matches!(
        method(&mut realm, &watcher, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
}

#[test]
fn polling_baseline_is_silent_and_bigint_stats_share_the_binding_array() {
    let directory = Directory::new();
    let path = directory.0.join("baseline.txt");
    std::fs::write(&path, b"old").unwrap();
    let mut realm = Realm::new();
    let watcher = new(&mut realm, true);
    let (guard, timed_out) = watchdog(&mut realm, watcher.clone());
    let fs = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let shared = ops::get(&mut realm, &fs, &Key::name("bigintStatValues")).unwrap();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let target = watcher.clone();
    let callback = realm.function("onchange", 2, false, move |realm, _, args| {
        assert!(crate::same_value(&args[1], &shared));
        for (index, expected) in [(8, "7"), (26, "3")] {
            let value = ops::get(realm, &args[1], &Key::Index(index))?;
            assert!(matches!(value, Value::BigInt(_)));
            assert_eq!(ops::to_string(realm, &value)?.to_rust_lossy(), expected);
        }
        count.set(count.get() + 1);
        method(realm, &target, "close", &[])?;
        realm.clear(&Value::Object(guard.clone()));
        Ok(Value::Undefined)
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    method(
        &mut realm,
        &watcher,
        "start",
        &[Value::str(path.to_str().unwrap()), Value::Number(2.0)],
    )
    .unwrap();
    // Wait until the initial stat has completed and the next poll is scheduled.
    // The probe is queued after that completion, avoiding a wall-clock race.
    let file = path.clone();
    let target = watcher.clone();
    let probe = realm.function("changeAfterBaseline", 0, false, move |realm, _, _| {
        method(realm, &target, "unref", &[])?;
        method(realm, &target, "ref", &[])?;
        replace(&file, b"changed");
        Ok(Value::Undefined)
    });
    // 100 ms allows actual Windows pool stat completion; the loop remains
    // bounded by the watchdog if the platform cannot complete the request.
    realm.native_timeout(probe, 100.0, Rc::new(Cell::new(true)));
    realm.run_event_loop().unwrap();
    assert!(!timed_out.get());
    assert_eq!(calls.get(), 1);
}

#[test]
fn polling_validates_start_references_and_submission_failures() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    let fs = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let constructor = ops::get(&mut realm, &fs, &Key::name("StatWatcher")).unwrap();
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
    let watcher = new(&mut realm, false);
    assert!(
        matches!(method(&mut realm, &watcher, "getAsyncId", &[]).unwrap(), Value::Number(id) if id > 0.0)
    );
    for args in [
        vec![],
        vec![Value::str("x")],
        vec![Value::str("x\0"), Value::Number(1.0)],
        vec![Value::str("x"), Value::Number(-1.0)],
        vec![Value::str("x"), Value::Number(0.5)],
        vec![Value::str("x"), Value::Number(f64::NAN)],
        vec![Value::str("x"), Value::Number(4294967296.0)],
    ] {
        assert!(
            matches!(method(&mut realm, &watcher, "start", &args).unwrap(), Value::Number(code) if code == f64::from(crate::fs_win::UV_EINVAL))
        );
    }
    let args = [
        Value::str(directory.0.to_str().unwrap()),
        Value::Number(f64::from(u32::MAX)),
    ];
    realm.event_loop.io_create = crate::work_queue::tests::failed_submit;
    assert!(
        matches!(method(&mut realm, &watcher, "start", &args).unwrap(), Value::Number(code) if code < 0.0)
    );
    realm.event_loop.io_create = crate::work_queue::Signal::new;
    method(&mut realm, &watcher, "unref", &[]).unwrap();
    assert!(matches!(
        method(&mut realm, &watcher, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
    method(&mut realm, &watcher, "ref", &[]).unwrap();
    method(&mut realm, &watcher, "start", &args).unwrap();
    method(&mut realm, &watcher, "close", &[]).unwrap();
    method(&mut realm, &watcher, "ref", &[]).unwrap();
    method(&mut realm, &watcher, "start", &args).unwrap();
    let wait = realm.function("drainClosedPoll", 0, false, |_, _, _| Ok(Value::Undefined));
    realm.native_timeout(wait, 30.0, Rc::new(Cell::new(true)));
    realm.run_event_loop().unwrap();
    assert!(matches!(
        method(&mut realm, &watcher, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
}

#[test]
fn polling_comparison_ignores_access_time_and_link_count_but_checks_change_fields() {
    let previous = crate::fs_win::Stat::default();
    let mut current = previous.clone();
    current.times[0] = (1, 2);
    current.nlink = 2;
    assert!(!changed(&previous, &current));
    for field in 0..5 {
        let mut current = previous.clone();
        match field {
            0 => current.mode = 1,
            1 => current.ino = 1,
            2 => current.dev = 1,
            3 => current.size = 1,
            _ => current.times[2] = (1, 0),
        }
        assert!(changed(&previous, &current));
    }
}

#[test]
fn polling_later_worker_failure_reports_error_and_releases_loop_reference() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::fail_after_first_submission;
    let watcher = new(&mut realm, false);
    method(
        &mut realm,
        &watcher,
        "start",
        &[
            Value::str(directory.0.to_str().unwrap()),
            Value::Number(1.0),
        ],
    )
    .unwrap();
    let error = realm.run_event_loop().unwrap_err();
    let message = ops::get(&mut realm, &error.0, &Key::name("message")).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &message)
            .unwrap()
            .to_rust_lossy(),
        "Could not submit filesystem poll: 5"
    );
    assert!(matches!(
        method(&mut realm, &watcher, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));
    method(&mut realm, &watcher, "close", &[]).unwrap();
}

#[test]
fn polling_remains_active_after_a_callback_exception_and_recovers_on_creation() {
    let directory = Directory::new();
    let path = directory.0.join("recover.txt");
    let mut realm = Realm::new();
    let watcher = new(&mut realm, false);
    let throws = realm.function("onchange", 2, false, |_, _, _| {
        Err(Thrown(Value::str("callback error")))
    });
    watcher
        .as_object()
        .unwrap()
        .define(Key::name("onchange"), Property::data(Value::Object(throws)));
    method(
        &mut realm,
        &watcher,
        "start",
        &[Value::str(path.to_str().unwrap()), Value::Number(1.0)],
    )
    .unwrap();
    assert!(
        matches!(realm.run_event_loop().unwrap_err().0, Value::String(message) if message.to_rust_lossy() == "callback error")
    );
    let (guard, timed_out) = watchdog(&mut realm, watcher.clone());
    std::fs::write(&path, b"recovered").unwrap();
    let target = watcher.clone();
    let received = Rc::new(Cell::new(false));
    let result = received.clone();
    let callback = realm.function("onchange", 2, false, move |realm, _, args| {
        assert_eq!(number(realm, &args[1], 8), 9.0);
        result.set(true);
        method(realm, &target, "close", &[])?;
        realm.clear(&Value::Object(guard.clone()));
        Ok(Value::Undefined)
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    realm.run_event_loop().unwrap();
    assert!(!timed_out.get());
    assert!(received.get());
}

#[test]
fn polling_close_cancels_a_scheduled_timer_and_drains_inflight_stat_safely() {
    let directory = Directory::new();
    let path = directory.0.join("close.txt");
    std::fs::write(&path, b"steady").unwrap();
    let mut realm = Realm::new();
    let watcher = new(&mut realm, false);
    method(
        &mut realm,
        &watcher,
        "start",
        &[Value::str(path.to_str().unwrap()), Value::Number(1000.0)],
    )
    .unwrap();
    let target = watcher.clone();
    let close = realm.function("closeScheduledPoll", 0, false, move |realm, _, _| {
        method(realm, &target, "close", &[])?;
        Ok(Value::Undefined)
    });
    realm.native_timeout(close, 60.0, Rc::new(Cell::new(true)));
    realm.run_event_loop().unwrap();
    assert!(matches!(
        method(&mut realm, &watcher, "hasRef", &[]).unwrap(),
        Value::Boolean(false)
    ));

    let pending = new(&mut realm, false);
    method(
        &mut realm,
        &pending,
        "start",
        &[Value::str(path.to_str().unwrap()), Value::Number(1000.0)],
    )
    .unwrap();
    method(&mut realm, &pending, "close", &[]).unwrap();
    let drain = realm.function("drainInflightStat", 0, false, |_, _, _| {
        Ok(Value::Undefined)
    });
    realm.native_timeout(drain, 30.0, Rc::new(Cell::new(true)));
    realm.run_event_loop().unwrap();
}
