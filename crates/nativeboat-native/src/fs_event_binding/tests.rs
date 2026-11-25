use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct Directory(std::path::PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nativeboat-fs-event-{}-{}",
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

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap()
}

#[test]
fn watcher_handles_open_emit_changes_close_and_preserve_async_identity() {
    use std::{cell::RefCell, rc::Rc, thread, time::Duration};

    let directory = Directory::new();
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let async_id_fn = get(&mut realm, &watcher, "getAsyncId");
    let async_id = ops::call(&mut realm, &async_id_fn, watcher.clone(), &[]).unwrap();
    assert!(matches!(async_id, Value::Number(id) if id > 0.0));
    for name in ["ref", "unref"] {
        let method = get(&mut realm, &watcher, name);
        assert!(crate::same_value(
            &ops::call(&mut realm, &method, watcher.clone(), &[]).unwrap(),
            &watcher
        ));
    }
    let close = get(&mut realm, &watcher, "close");
    let start = get(&mut realm, &watcher, "start");
    let invalid = ops::call(
        &mut realm,
        &start,
        watcher.clone(),
        &[Value::str("N:\\missing\\watch"), Value::Boolean(true)],
    )
    .unwrap();
    assert!(matches!(invalid, Value::Number(code) if code < 0.0));
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
    ops::call(&mut realm, &close, watcher.clone(), &[]).unwrap();

    // A closed FSEvent cannot be restarted. Use a fresh watcher for the valid
    // path after exercising close on the invalid-path instance above.
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let close = get(&mut realm, &watcher, "close");
    let start = get(&mut realm, &watcher, "start");
    let received = Arc::new(Mutex::new(Vec::<(f64, String, String)>::new()));
    let close_state: Rc<RefCell<Option<(Value, Value)>>> = Rc::new(RefCell::new(None));
    let callback_received = received.clone();
    let callback_close = close_state.clone();
    let callback = realm.function("onchange", 3, false, move |realm, _, args| {
        let status = match args.first() {
            Some(Value::Number(value)) => *value,
            _ => f64::NAN,
        };
        let event =
            ops::to_string(realm, &args.get(1).cloned().unwrap_or_default())?.to_rust_lossy();
        let filename =
            ops::to_string(realm, &args.get(2).cloned().unwrap_or_default())?.to_rust_lossy();
        callback_received
            .lock()
            .unwrap()
            .push((status, event, filename));
        if let Some((watcher, close)) = callback_close.borrow().clone() {
            ops::call(realm, &close, watcher, &[])?;
        }
        Ok(Value::Undefined)
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    *close_state.borrow_mut() = Some((watcher.clone(), close));

    let started = ops::call(
        &mut realm,
        &start,
        watcher.clone(),
        &[
            Value::str(directory.0.to_str().unwrap()),
            Value::Boolean(true),
            Value::Boolean(true),
            Value::str("utf8"),
        ],
    )
    .unwrap();
    assert!(matches!(started, Value::Number(0.0)));
    let initialized = get(&mut realm, &watcher, "initialized");
    assert!(
        matches!(initialized, Value::Boolean(true)),
        "initialized after start: {initialized:?}"
    );
    thread::sleep(Duration::from_millis(60));
    std::fs::write(directory.0.join("note.txt"), b"event").unwrap();
    realm.run_event_loop().unwrap();
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
    assert!(
        received
            .lock()
            .unwrap()
            .iter()
            .any(|(status, event, filename)| {
                *status == 0.0 && event == "rename" && filename == "note.txt"
            })
    );
}

#[test]
fn watcher_start_rejects_non_string_and_nul_paths_without_opening_handles() {
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    assert!(
        ops::call(&mut realm, &constructor, Value::Undefined, &[])
            .unwrap()
            .as_object()
            .is_some()
    );
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let start = get(&mut realm, &watcher, "start");
    for path in [Value::Number(1.0), Value::str("C:\\nul\0path")] {
        let result = ops::call(
            &mut realm,
            &start,
            watcher.clone(),
            &[path, Value::Boolean(true)],
        )
        .unwrap();
        assert!(matches!(result, Value::Number(code) if code == crate::fs_win::UV_EINVAL as f64));
    }
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
}

#[test]
fn file_watcher_filters_parent_directory_events_to_the_requested_file() {
    use std::{cell::RefCell, rc::Rc, thread, time::Duration};

    let directory = Directory::new();
    let watched_path = directory.0.join("Watched.txt");
    let watch_path = directory.0.join("WATCHED.TXT");
    let sibling_path = directory.0.join("sibling.txt");
    std::fs::write(&watched_path, b"before").unwrap();
    std::fs::write(&sibling_path, b"sibling").unwrap();

    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let close = get(&mut realm, &watcher, "close");
    let start = get(&mut realm, &watcher, "start");
    let received = Arc::new(Mutex::new(Vec::<String>::new()));
    let close_state: Rc<RefCell<Option<(Value, Value)>>> = Rc::new(RefCell::new(None));
    let callback_received = received.clone();
    let callback_close = close_state.clone();
    let callback = realm.function("onchange", 3, false, move |realm, _, args| {
        let filename = ops::to_string(realm, &args[2])?.to_rust_lossy();
        callback_received.lock().unwrap().push(filename);
        if let Some((watcher, close)) = callback_close.borrow().clone() {
            ops::call(realm, &close, watcher, &[])?;
        }
        Ok(Value::Undefined)
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    *close_state.borrow_mut() = Some((watcher.clone(), close));
    assert!(matches!(
        ops::call(
            &mut realm,
            &start,
            watcher,
            &[
                Value::str(watch_path.to_str().unwrap()),
                Value::Boolean(true),
                Value::Boolean(false),
                Value::str("utf8"),
            ],
        )
        .unwrap(),
        Value::Number(0.0)
    ));

    thread::sleep(Duration::from_millis(60));
    std::fs::write(&sibling_path, b"changed sibling").unwrap();
    std::fs::write(&watched_path, b"changed target").unwrap();
    realm.run_event_loop().unwrap();

    let received = received.lock().unwrap();
    assert!(
        !received.is_empty(),
        "the target file change must be emitted"
    );
    assert!(
        received.iter().all(|filename| filename == "Watched.txt"),
        "sibling notifications must be filtered: {received:?}"
    );
}

#[test]
fn watcher_stops_and_propagates_errors_from_change_callbacks() {
    use std::{thread, time::Duration};

    let directory = Directory::new();
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let callback = realm.function("onchange", 3, false, |realm, _, _| {
        Err(realm.throw(crate::ErrorKind::Error, "change callback failed"))
    });
    watcher.as_object().unwrap().define(
        Key::name("onchange"),
        Property::data(Value::Object(callback)),
    );
    let start = get(&mut realm, &watcher, "start");
    assert!(matches!(
        ops::call(
            &mut realm,
            &start,
            watcher.clone(),
            &[Value::str(directory.0.to_str().unwrap())],
        )
        .unwrap(),
        Value::Number(0.0)
    ));
    thread::sleep(Duration::from_millis(60));
    std::fs::write(directory.0.join("throws.txt"), b"change").unwrap();
    let error = realm.run_event_loop().unwrap_err();
    assert!(
        crate::ops::to_string(&mut realm, &error.0)
            .unwrap()
            .to_rust_lossy()
            .contains("change callback failed")
    );
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
}

#[test]
fn queue_read_treats_closed_pending_and_unstarted_watchers_as_no_ops() {
    use std::{cell::RefCell, rc::Rc};

    let mut realm = Realm::new();
    let object = realm.object();
    for (closed, pending, handle) in [
        (true, false, None),
        (false, true, Some(1)),
        (false, false, None),
    ] {
        let state = Rc::new(RefCell::new(Watcher {
            handle,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            recursive: false,
            pending,
            closed,
            async_id: 1,
            referenced: Rc::new(core::cell::Cell::new(false)),
            path: Vec::new(),
            filename: None,
        }));
        queue_read(&mut realm, object.clone(), state).unwrap();
    }
    assert!(!realm.alive());
}

#[test]
fn watcher_reports_native_read_errors_and_propagates_callback_failures() {
    use std::{cell::RefCell, rc::Rc, sync::atomic::AtomicBool};

    for callback_behavior in 0..3 {
        let mut realm = Realm::new();
        let object = realm.object();
        let state = Rc::new(RefCell::new(Watcher {
            handle: Some(usize::MAX),
            cancelled: Arc::new(AtomicBool::new(false)),
            recursive: false,
            pending: false,
            closed: false,
            async_id: 1,
            referenced: Rc::new(core::cell::Cell::new(true)),
            path: "missing\0".encode_utf16().collect(),
            filename: None,
        }));
        let seen = Rc::new(RefCell::new(None));
        let callback_seen = seen.clone();
        let callback = realm.function("onchange", 3, false, move |realm, _, args| {
            *callback_seen.borrow_mut() = match args.first() {
                Some(Value::Number(status)) => Some(*status),
                _ => None,
            };
            if callback_behavior == 1 {
                Err(realm.throw(crate::ErrorKind::Error, "watch callback failed"))
            } else {
                Ok(Value::Undefined)
            }
        });
        if callback_behavior == 2 {
            let getter = realm.function("get onchange", 0, false, |realm, _, _| {
                Err(realm.throw(crate::ErrorKind::Error, "watch callback getter failed"))
            });
            object.define(
                Key::name("onchange"),
                Property {
                    slot: Slot::Accessor {
                        get: Some(getter),
                        set: None,
                    },
                    enumerable: true,
                    configurable: true,
                },
            );
        } else {
            object.define(
                Key::name("onchange"),
                Property::data(Value::Object(callback)),
            );
        }

        queue_read(&mut realm, object, state.clone()).unwrap();
        let completion = realm.run_event_loop();
        if callback_behavior != 2 {
            assert!(seen.borrow().is_some_and(|status| status < 0.0));
        }
        assert!(state.borrow().closed);
        assert!(state.borrow().handle.is_none());
        if callback_behavior != 0 {
            assert!(completion.is_err());
        } else {
            completion.unwrap();
        }
    }
}

#[test]
fn watcher_start_rejects_primitive_receivers_and_ignores_repeated_start() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let start = get(&mut realm, &watcher, "start");
    assert!(matches!(
        ops::call(
            &mut realm,
            &start,
            Value::Undefined,
            &[Value::str(directory.0.to_str().unwrap())],
        )
        .unwrap(),
        Value::Number(code) if code == crate::fs_win::UV_EINVAL as f64
    ));
    let args = [Value::str(directory.0.to_str().unwrap())];
    assert!(matches!(
        ops::call(&mut realm, &start, watcher.clone(), &args).unwrap(),
        Value::Number(0.0)
    ));
    assert!(matches!(
        ops::call(&mut realm, &start, watcher.clone(), &args).unwrap(),
        Value::Number(0.0)
    ));
    let close = get(&mut realm, &watcher, "close");
    ops::call(&mut realm, &close, watcher, &[]).unwrap();
    realm.run_event_loop().unwrap();
}

#[test]
fn watcher_closes_after_a_cancelled_pending_read() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let start = get(&mut realm, &watcher, "start");
    let close = get(&mut realm, &watcher, "close");
    assert!(matches!(
        ops::call(
            &mut realm,
            &start,
            watcher.clone(),
            &[Value::str(directory.0.to_str().unwrap())],
        )
        .unwrap(),
        Value::Number(0.0)
    ));
    ops::call(&mut realm, &close, watcher.clone(), &[]).unwrap();
    realm.run_event_loop().unwrap();
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
}

#[test]
fn watcher_ref_state_controls_event_loop_liveness_and_close_drains_cancellation() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let start = get(&mut realm, &watcher, "start");
    let args = [
        Value::str(directory.0.to_str().unwrap()),
        Value::Boolean(false),
        Value::Boolean(false),
        Value::str("utf8"),
    ];
    assert!(matches!(
        ops::call(&mut realm, &start, watcher.clone(), &args).unwrap(),
        Value::Number(0.0)
    ));
    assert!(!realm.alive(), "persistent:false must not keep Node alive");

    let reference = get(&mut realm, &watcher, "ref");
    assert!(matches!(
        ops::call(&mut realm, &reference, watcher.clone(), &[]).unwrap(),
        Value::Object(ref object) if object == watcher.as_object().unwrap()
    ));
    assert!(realm.alive(), "ref() must keep the pending watcher alive");

    let unreference = get(&mut realm, &watcher, "unref");
    ops::call(&mut realm, &unreference, watcher.clone(), &[]).unwrap();
    assert!(!realm.alive(), "unref() must release event-loop liveness");

    let close = get(&mut realm, &watcher, "close");
    ops::call(&mut realm, &close, watcher, &[]).unwrap();
    assert!(realm.alive(), "close must drain cancellation before exit");
    realm.run_event_loop().unwrap();
    assert!(
        !realm.alive(),
        "completed close must release event-loop liveness"
    );
}

#[test]
fn watcher_start_cleans_up_when_io_submission_fails() {
    let directory = Directory::new();
    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::failed_submit;
    let exports = crate::modules::load(&mut realm, "binding/fs_event_wrap").unwrap();
    let constructor = get(&mut realm, &exports, "FSEvent");
    let watcher = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let start = get(&mut realm, &watcher, "start");
    assert!(
        ops::call(
            &mut realm,
            &start,
            watcher.clone(),
            &[Value::str(directory.0.to_str().unwrap())],
        )
        .is_err()
    );
    assert!(matches!(
        get(&mut realm, &watcher, "initialized"),
        Value::Boolean(false)
    ));
    assert!(!realm.alive());
}
