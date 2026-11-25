use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

trait Equal {
    fn equal(&self, other: &Self) -> bool;
}
impl Equal for Value {
    fn equal(&self, other: &Self) -> bool {
        ops::strict_equals(self, other)
    }
}
impl Equal for Vec<Value> {
    fn equal(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other)
                .all(|(a, b)| ops::strict_equals(a, b))
    }
}
fn equal<T: Equal>(left: &T, right: &T) -> bool {
    left.equal(right)
}
macro_rules! assert_eq {
    ($left:expr,$right:expr $(,)?) => {{
        let left = &$left;
        let right = &$right;
        assert!(equal(left, right), "{left:?} != {right:?}");
    }};
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nativeboat-fs-binding-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Fixture(root)
    }
    fn path(&self, name: &str) -> Value {
        Value::str(self.0.join(name).to_str().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn call(realm: &mut Realm, binding: &Value, name: &str, args: &[Value]) -> Completion {
    let method = ops::get(realm, binding, &Key::name(name))?;
    ops::call(realm, &method, Value::Undefined, args)
}
fn get(realm: &mut Realm, value: &Value, name: &str) -> Value {
    ops::get(realm, value, &Key::name(name)).unwrap()
}

#[test]
fn real_unicode_file_io_metadata_and_descriptor_lifecycle() {
    let fixture = Fixture::new();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let file = fixture.path("中🚀.txt");
    let flags = Value::Number(f64::from(
        crate::fs_win::O_CREAT | crate::fs_win::O_TRUNC | crate::fs_win::O_WRONLY,
    ));
    call(
        &mut realm,
        &binding,
        "writeFileUtf8",
        &[
            file.clone(),
            Value::str("nativeboat"),
            flags,
            Value::Number(0o666 as f64),
        ],
    )
    .unwrap();
    std::assert_eq!(
        std::fs::read(fixture.0.join("中🚀.txt")).unwrap(),
        b"nativeboat"
    );
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readFileUtf8",
            &[file.clone(), Value::Number(0.0)]
        )
        .unwrap(),
        Value::str("nativeboat")
    );
    let stat = call(
        &mut realm,
        &binding,
        "stat",
        &[file.clone(), Value::Boolean(false), Value::Undefined],
    )
    .unwrap();
    assert_eq!(stat, get(&mut realm, &binding, "statValues"));
    assert_eq!(
        ops::get(&mut realm, &stat, &Key::Index(8)).unwrap(),
        Value::Number(10.0)
    );
    ops::set(&mut realm, &stat, &Key::Index(18), Value::Number(12345.0)).unwrap();
    call(
        &mut realm,
        &binding,
        "lstat",
        &[file.clone(), Value::Boolean(false), Value::Undefined],
    )
    .unwrap();
    assert_eq!(
        ops::get(&mut realm, &stat, &Key::Index(18)).unwrap(),
        Value::Number(12345.0)
    );
    let big = call(
        &mut realm,
        &binding,
        "stat",
        &[file.clone(), Value::Boolean(true), Value::Undefined],
    )
    .unwrap();
    assert_eq!(big, get(&mut realm, &binding, "bigintStatValues"));
    assert!(
        matches!(ops::get(&mut realm,&big,&Key::Index(8)).unwrap(),Value::BigInt(value) if value.magnitude_u64()==Some(10))
    );
    let fd = call(
        &mut realm,
        &binding,
        "open",
        &[
            file.clone(),
            Value::Number(0.0),
            Value::Number(0o666 as f64),
        ],
    )
    .unwrap();
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "fstat",
            &[fd.clone(), Value::Boolean(false)]
        )
        .unwrap(),
        stat
    );
    call(&mut realm, &binding, "close", core::slice::from_ref(&fd)).unwrap();
    let error = call(&mut realm, &binding, "fstat", &[fd, Value::Boolean(false)])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("EBADF"));
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("fstat"));
    let missing = fixture.path("missing");
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "stat",
            &[
                missing.clone(),
                Value::Boolean(false),
                Value::Undefined,
                Value::Boolean(false)
            ]
        )
        .unwrap(),
        Value::Undefined
    );
    let error = call(
        &mut realm,
        &binding,
        "stat",
        &[missing.clone(), Value::Boolean(false)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "path"), missing);
    assert!(
        matches!(error,Value::Object(object) if object.data().prototype==Some(realm.error_prototype.clone()))
    );
    let root = fixture.path("");
    let statfs = call(
        &mut realm,
        &binding,
        "statfs",
        &[root, Value::Boolean(false)],
    )
    .unwrap();
    assert_eq!(statfs, get(&mut realm, &binding, "statFsValues"));
}

#[test]
fn native_file_handle_owns_close_and_release_lifecycle() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("handle.txt"), b"file handle").unwrap();
    let mut realm = Realm::new();
    let failed = realm.throw(ErrorKind::Error, "close failed");
    let propagated = finish_file_handle_close(
        &realm.object(),
        &Symbol::private("close.promise"),
        Err(failed.clone()),
    )
    .unwrap_err();
    assert_eq!(propagated.0, failed.0);
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let handle = call(
        &mut realm,
        &binding,
        "openFileHandle",
        &[
            fixture.path("handle.txt"),
            number(0),
            number(0o666),
            Value::Undefined,
        ],
    )
    .unwrap();
    let fd = get(&mut realm, &handle, "fd");
    assert!(matches!(
        get(&mut realm, &handle, "getAsyncId"),
        Value::Object(_)
    ));
    let async_id_method = get(&mut realm, &handle, "getAsyncId");
    let async_id = ops::call(&mut realm, &async_id_method, handle.clone(), &[]).unwrap();
    assert!(matches!(async_id, Value::Number(value) if value > 0.0));
    let close = get(&mut realm, &handle, "close");
    let first = ops::call(&mut realm, &close, handle.clone(), &[]).unwrap();
    let repeated = ops::call(&mut realm, &close, handle.clone(), &[]).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(get(&mut realm, &handle, "fd"), number(-1));
    realm.run_event_loop().unwrap();
    let error = call(&mut realm, &binding, "fstat", &[fd, Value::Boolean(false)])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("EBADF"));

    let error = call(
        &mut realm,
        &binding,
        "openFileHandle",
        &[
            fixture.path("missing.txt"),
            number(0),
            number(0o666),
            Value::Undefined,
        ],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("open"));

    let transferred = call(
        &mut realm,
        &binding,
        "openFileHandle",
        &[
            fixture.path("handle.txt"),
            number(0),
            number(0o666),
            Value::Undefined,
        ],
    )
    .unwrap();
    let release = get(&mut realm, &transferred, "releaseFD");
    let transferred_fd = ops::call(&mut realm, &release, transferred.clone(), &[]).unwrap();
    assert!(matches!(transferred_fd, Value::Number(value) if value >= 3.0));
    assert_eq!(get(&mut realm, &transferred, "fd"), number(-1));
    let close_released = get(&mut realm, &transferred, "close");
    let released_promise =
        ops::call(&mut realm, &close_released, transferred.clone(), &[]).unwrap();
    assert!(matches!(released_promise, Value::Object(_)));
    assert!(
        call(
            &mut realm,
            &binding,
            "fstat",
            &[transferred_fd.clone(), Value::Boolean(false)]
        )
        .is_ok()
    );
    call(&mut realm, &binding, "close", &[transferred_fd]).unwrap();

    let invalid_receiver = Value::Object(realm.object());
    let invalid = ops::call(&mut realm, &close_released, invalid_receiver, &[]).unwrap_err();
    assert_eq!(get(&mut realm, &invalid.0, "name"), Value::str("TypeError"));
    let invalid_receiver = Value::Object(realm.object());
    let invalid = ops::call(&mut realm, &release, invalid_receiver, &[]).unwrap_err();
    assert_eq!(get(&mut realm, &invalid.0, "name"), Value::str("TypeError"));
}

#[test]
fn private_fs_dir_binding_reads_unicode_batches_and_closes_cursors() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("alpha.txt"), b"a").unwrap();
    std::fs::write(fixture.0.join("原稿.txt"), b"b").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs_dir").unwrap();
    let handle = call(
        &mut realm,
        &binding,
        "opendirSync",
        &[fixture.path(""), Value::Undefined],
    )
    .unwrap();
    let async_id = get(&mut realm, &handle, "getAsyncId");
    assert!(matches!(
        ops::call(&mut realm, &async_id, handle.clone(), &[]).unwrap(),
        Value::Number(value) if value > 0.0
    ));

    let read = get(&mut realm, &handle, "read");
    let invalid_receiver = Value::Object(realm.object());
    let invalid_receiver = ops::call(
        &mut realm,
        &read,
        invalid_receiver,
        &[Value::str("utf8"), number(1)],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &invalid_receiver, "name"),
        Value::str("TypeError")
    );
    let invalid_size = ops::call(
        &mut realm,
        &read,
        handle.clone(),
        &[Value::str("utf8"), number(0)],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &invalid_size, "name"),
        Value::str("RangeError")
    );
    let invalid_size = ops::call(
        &mut realm,
        &read,
        handle.clone(),
        &[Value::str("utf8"), Value::str("1")],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &invalid_size, "name"),
        Value::str("TypeError")
    );
    let invalid_encoding = ops::call(
        &mut realm,
        &read,
        handle.clone(),
        &[Value::str("not-an-encoding"), number(1)],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &invalid_encoding, "name"),
        Value::str("TypeError")
    );
    let mut names = Vec::new();
    for _ in 0..2 {
        let batch = ops::call(
            &mut realm,
            &read,
            handle.clone(),
            &[Value::str("utf8"), number(1)],
        )
        .unwrap();
        assert_eq!(get(&mut realm, &batch, "length"), number(2));
        let name = ops::get(&mut realm, &batch, &Key::Index(0)).unwrap();
        names.push(ops::to_string(&mut realm, &name).unwrap().to_rust_lossy());
    }
    names.sort();
    std::assert_eq!(names, ["alpha.txt", "原稿.txt"]);

    let buffer_handle = call(
        &mut realm,
        &binding,
        "opendirSync",
        &[fixture.path(""), Value::Undefined],
    )
    .unwrap();
    let read = get(&mut realm, &buffer_handle, "read");
    let batch = ops::call(
        &mut realm,
        &read,
        buffer_handle.clone(),
        &[Value::str("BUFFER"), number(3)],
    )
    .unwrap();
    let Value::Object(batch) = batch else {
        panic!("buffer-encoded directory read did not return a batch")
    };
    let mut buffer_names = Vec::new();
    for index in [0, 2] {
        let name = ops::get(
            &mut realm,
            &Value::Object(batch.clone()),
            &Key::Index(index),
        )
        .unwrap();
        let Value::Object(name) = name else {
            panic!("buffer-encoded directory name was not a byte array")
        };
        buffer_names.push(
            typed_array_builtin::view_of(&Value::Object(name))
                .unwrap()
                .bytes
                .borrow()
                .clone(),
        );
    }
    assert!(buffer_names.iter().any(|name| name == b"alpha.txt"));
    let close = get(&mut realm, &buffer_handle, "close");
    ops::call(&mut realm, &close, buffer_handle, &[]).unwrap();

    assert_eq!(
        ops::call(
            &mut realm,
            &read,
            handle.clone(),
            &[Value::str("utf8"), number(1)],
        )
        .unwrap(),
        Value::Null
    );

    let close = get(&mut realm, &handle, "close");
    ops::call(&mut realm, &close, handle.clone(), &[]).unwrap();
    let closed = ops::call(&mut realm, &close, handle.clone(), &[])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &closed, "code"), Value::str("EBADF"));
    let closed = ops::call(&mut realm, &read, handle, &[Value::str("utf8"), number(1)])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &closed, "code"), Value::str("EBADF"));

    let missing = call(
        &mut realm,
        &binding,
        "opendirSync",
        &[fixture.path("missing"), Value::Undefined],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &missing, "code"), Value::str("ENOENT"));
    assert_eq!(get(&mut realm, &missing, "syscall"), Value::str("opendir"));
    let file = fixture.path("alpha.txt");
    let not_directory = call(
        &mut realm,
        &binding,
        "opendirSync",
        &[file, Value::Undefined],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &not_directory, "code"),
        Value::str("ENOTDIR")
    );

    let constructor = get(&mut realm, &binding, "DirHandle");
    let call_error = ops::call(&mut realm, &constructor, Value::Undefined, &[]).unwrap_err();
    assert_eq!(
        get(&mut realm, &call_error.0, "name"),
        Value::str("TypeError")
    );
    let uninitialized = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let read = get(&mut realm, &uninitialized, "read");
    let error = ops::call(
        &mut realm,
        &read,
        uninitialized,
        &[Value::str("utf8"), number(1)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("EBADF"));
}

#[test]
fn directory_handles_reject_corrupt_private_ids() {
    let mut realm = Realm::new();
    let key = Symbol::private("nativeboat.fs.directory.id");
    let handle = realm.object();
    handle.set_private(&key, Property::data(Value::Number(f64::INFINITY)));
    let error = directory_handle_id(&mut realm, &Value::Object(handle), &key).unwrap_err();
    assert_eq!(get(&mut realm, &error.0, "name"), Value::str("TypeError"));
}

#[test]
fn private_fs_dir_binding_completes_open_read_and_close_requests_on_the_realm() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("entry"), b"x").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs_dir").unwrap();
    let fs = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let constructor = get(&mut realm, &fs, "FSReqCallback");
    let make_request = |realm: &mut Realm| {
        let request = ops::construct(realm, &constructor, &[]).unwrap();
        let output = Rc::new(core::cell::RefCell::new(Vec::new()));
        let callback_output = output.clone();
        let callback = realm.function("complete", 2, false, move |_, _, args| {
            *callback_output.borrow_mut() = args.to_vec();
            Ok(Value::Undefined)
        });
        ops::set(
            realm,
            &request,
            &Key::name("oncomplete"),
            Value::Object(callback),
        )
        .unwrap();
        (request, output)
    };

    let (open_request, open_output) = make_request(&mut realm);
    call(
        &mut realm,
        &binding,
        "opendir",
        &[fixture.path(""), Value::str("utf8"), open_request],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    let open_args = open_output.borrow();
    assert_eq!(open_args[0], Value::Null);
    let Value::Object(handle) = open_args[1].clone() else {
        panic!("open callback did not return a DirHandle")
    };
    drop(open_args);

    let (read_request, read_output) = make_request(&mut realm);
    let read = get(&mut realm, &Value::Object(handle.clone()), "read");
    ops::call(
        &mut realm,
        &read,
        Value::Object(handle.clone()),
        &[Value::str("utf8"), number(1), read_request],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    let read_args = read_output.borrow();
    assert_eq!(read_args[0], Value::Null);
    let Value::Object(entries) = &read_args[1] else {
        panic!("read callback did not return a batch")
    };
    assert_eq!(
        ops::get(&mut realm, &Value::Object(entries.clone()), &Key::Index(0)).unwrap(),
        Value::str("entry")
    );
    drop(read_args);

    let (close_request, close_output) = make_request(&mut realm);
    let close = get(&mut realm, &Value::Object(handle.clone()), "close");
    ops::call(&mut realm, &close, Value::Object(handle), &[close_request]).unwrap();
    realm.run_event_loop().unwrap();
    let close_args = close_output.borrow();
    std::assert_eq!(close_args.len(), 1);
    assert_eq!(close_args[0], Value::Null);

    let (missing_request, missing_output) = make_request(&mut realm);
    call(
        &mut realm,
        &binding,
        "opendir",
        &[fixture.path("missing"), Value::Undefined, missing_request],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    let missing_args = missing_output.borrow();
    assert_eq!(
        get(&mut realm, &missing_args[0], "code"),
        Value::str("ENOENT")
    );

    let invalid_request = call(
        &mut realm,
        &binding,
        "opendir",
        &[fixture.path(""), Value::Undefined, number(7)],
    )
    .unwrap_err()
    .0;
    assert_eq!(
        get(&mut realm, &invalid_request, "name"),
        Value::str("TypeError")
    );
}

#[test]
fn private_fs_dir_binding_delivers_capture_and_worker_setup_errors() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("entry"), b"x").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs_dir").unwrap();
    let fs = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let constructor = get(&mut realm, &fs, "FSReqCallback");
    let request = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let output = seen.clone();
    let callback = realm.function("complete", 2, false, move |_, _, args| {
        *output.borrow_mut() = args.to_vec();
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    let invalid_path = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        1,
    ));
    typed_array_builtin::view_of(&invalid_path)
        .unwrap()
        .bytes
        .borrow_mut()[0] = 0xff;
    call(
        &mut realm,
        &binding,
        "opendir",
        &[invalid_path, Value::Undefined, request],
    )
    .unwrap();
    std::assert_eq!(seen.borrow().len(), 1);
    assert_eq!(
        get(&mut realm, &seen.borrow()[0], "code"),
        Value::str("ENOENT")
    );
    assert_eq!(
        get(&mut realm, &seen.borrow()[0], "syscall"),
        Value::str("opendir")
    );
    assert_eq!(get(&mut realm, &seen.borrow()[0], "path"), Value::Undefined);

    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::failed_create;
    let binding = crate::modules::load(&mut realm, "binding/fs_dir").unwrap();
    let fs = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let constructor = get(&mut realm, &fs, "FSReqCallback");
    let request = ops::construct(&mut realm, &constructor, &[]).unwrap();
    let output = Rc::new(core::cell::RefCell::new(Vec::new()));
    let callback_output = output.clone();
    let callback = realm.function("complete", 2, false, move |_, _, args| {
        *callback_output.borrow_mut() = args.to_vec();
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "opendir",
        &[fixture.path(""), Value::Undefined, request],
    )
    .unwrap();
    std::assert_eq!(output.borrow().len(), 1);
    assert_eq!(
        get(&mut realm, &output.borrow()[0], "code"),
        Value::str("EPERM")
    );
}

#[test]
fn worker_callbacks_and_promises_publish_on_the_realm_thread() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"nativeboat").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    let req = ops::construct(&mut realm, &ctor, &[Value::Boolean(false)]).unwrap();
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let output = seen.clone();
    let thread = std::thread::current().id();
    let callback = realm.function("complete", 2, false, move |_, this, args| {
        output
            .borrow_mut()
            .push((std::thread::current().id() == thread, this, args.to_vec()));
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &req,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    let shared = get(&mut realm, &binding, "statValues");
    let file = fixture.path("file");
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "stat",
            &[file.clone(), Value::Boolean(false), req.clone()]
        )
        .unwrap(),
        Value::Undefined
    );
    assert!(seen.borrow().is_empty());
    let promise_key = get(&mut realm, &binding, "kUsePromises");
    let promise = call(
        &mut realm,
        &binding,
        "stat",
        &[file.clone(), Value::Boolean(false), promise_key],
    )
    .unwrap();
    let promise_seen = Rc::new(core::cell::RefCell::new(None));
    let output = promise_seen.clone();
    let fulfill = realm.function("fulfilled", 1, false, move |_, _, args| {
        *output.borrow_mut() = args.first().cloned();
        Ok(Value::Undefined)
    });
    let then = get(&mut realm, &promise, "then");
    ops::call(&mut realm, &then, promise, &[Value::Object(fulfill)]).unwrap();
    realm.run_event_loop().unwrap();
    std::assert_eq!(seen.borrow().len(), 1);
    assert!(seen.borrow()[0].0);
    assert_eq!(seen.borrow()[0].1, req);
    assert_eq!(seen.borrow()[0].2, vec![Value::Null, shared.clone()]);
    let separate = promise_seen.borrow().clone().unwrap();
    assert!(!ops::strict_equals(&separate, &shared));
    assert_eq!(get(&mut realm, &separate, "length"), Value::Number(18.0));
    assert_eq!(
        ops::get(&mut realm, &separate, &Key::Index(8)).unwrap(),
        Value::Number(10.0)
    );
}

#[test]
fn positioned_buffers_recursive_directories_and_native_write_error_context() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abcdef").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let path = fixture.path("file");
    let fd = call(
        &mut realm,
        &binding,
        "open",
        &[
            path,
            Value::Number(f64::from(crate::fs_win::O_RDWR)),
            Value::Number(0o666 as f64),
        ],
    )
    .unwrap();
    let buffer = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        8,
    ));
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    view.bytes.borrow_mut().fill(99);
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "read",
            &[
                fd.clone(),
                buffer.clone(),
                Value::Number(2.0),
                Value::Number(3.0),
                Value::Number(1.0)
            ]
        )
        .unwrap(),
        Value::Number(3.0)
    );
    std::assert_eq!(&*view.bytes.borrow(), b"ccbcdccc");
    let text = Value::str("XY");
    let ctx = Value::Object(realm.object());
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "writeString",
            &[
                fd.clone(),
                text,
                Value::Number(2.0),
                Value::str("utf8"),
                Value::Undefined,
                ctx.clone()
            ]
        )
        .unwrap(),
        Value::Number(2.0)
    );
    std::assert_eq!(std::fs::read(fixture.0.join("file")).unwrap(), b"abXYef");
    let first = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        2,
    ));
    let second = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        3,
    ));
    let vectors = Value::Object(realm.array(vec![first.clone(), second.clone()]));
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readBuffers",
            &[fd.clone(), vectors, Value::Number(0.0)]
        )
        .unwrap(),
        Value::Number(5.0)
    );
    std::assert_eq!(
        &*typed_array_builtin::view_of(&first).unwrap().bytes.borrow(),
        b"ab"
    );
    std::assert_eq!(
        &*typed_array_builtin::view_of(&second)
            .unwrap()
            .bytes
            .borrow(),
        b"XYe"
    );
    call(&mut realm, &binding, "close", core::slice::from_ref(&fd)).unwrap();
    let error = call(
        &mut realm,
        &binding,
        "writeBuffer",
        &[
            fd,
            buffer,
            Value::Number(0.0),
            Value::Number(1.0),
            Value::Null,
            Value::Undefined,
            ctx.clone(),
        ],
    )
    .unwrap();
    assert_eq!(error, Value::Number(f64::from(crate::fs_win::UV_EBADF)));
    assert_eq!(
        get(&mut realm, &ctx, "errno"),
        Value::Number(f64::from(crate::fs_win::UV_EBADF))
    );
    assert_eq!(get(&mut realm, &ctx, "code"), Value::Undefined);
    let directory = fixture.path("nested/a/b");
    let created = call(
        &mut realm,
        &binding,
        "mkdir",
        &[
            directory.clone(),
            Value::Number(0o777 as f64),
            Value::Boolean(true),
        ],
    )
    .unwrap();
    assert_eq!(created, fixture.path("nested"));
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "mkdir",
            &[directory, Value::Number(0o777 as f64), Value::Boolean(true)]
        )
        .unwrap(),
        Value::Undefined
    );
    let root = fixture.path("");
    let names = call(
        &mut realm,
        &binding,
        "readdir",
        &[root.clone(), Value::str("utf8"), Value::Boolean(false)],
    )
    .unwrap();
    assert_eq!(get(&mut realm, &names, "length"), Value::Number(2.0));
    let names = call(
        &mut realm,
        &binding,
        "readdir",
        &[root, Value::str("buffer"), Value::Boolean(true)],
    )
    .unwrap();
    assert_eq!(get(&mut realm, &names, "length"), Value::Number(2.0));
    let array = ops::get(&mut realm, &names, &Key::Index(0)).unwrap();
    let name = ops::get(&mut realm, &array, &Key::Index(0)).unwrap();
    assert!(typed_array_builtin::view_of(&name).is_some());
}

fn number(value: i32) -> Value {
    Value::Number(f64::from(value))
}

#[test]
fn mutations_links_times_and_temporary_names_reach_the_windows_filesystem() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abcdef").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let file = fixture.path("file");
    call(&mut realm, &binding, "access", &[file.clone(), number(0)]).unwrap();
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "existsSync",
            core::slice::from_ref(&file)
        )
        .unwrap(),
        Value::Boolean(true)
    );
    call(
        &mut realm,
        &binding,
        "copyFile",
        &[file.clone(), fixture.path("copy"), number(0)],
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "link",
        &[file.clone(), fixture.path("hard")],
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "rename",
        &[fixture.path("copy"), fixture.path("renamed")],
    )
    .unwrap();
    std::assert_eq!(std::fs::read(fixture.0.join("renamed")).unwrap(), b"abcdef");
    let error = call(
        &mut realm,
        &binding,
        "rename",
        &[fixture.path("missing"), fixture.path("dest")],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "dest"), fixture.path("dest"));
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    call(
        &mut realm,
        &binding,
        "chmod",
        &[file.clone(), number(0o666)],
    )
    .unwrap();
    for name in ["utimes", "lutimes"] {
        call(
            &mut realm,
            &binding,
            name,
            &[file.clone(), number(1_700_000_000), number(1_700_000_001)],
        )
        .unwrap();
    }
    let descriptor = call(
        &mut realm,
        &binding,
        "open",
        &[file.clone(), number(crate::fs_win::O_RDWR), number(0o666)],
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "fchmod",
        &[descriptor.clone(), number(0o666)],
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "futimes",
        &[
            descriptor.clone(),
            number(1_700_000_002),
            number(1_700_000_003),
        ],
    )
    .unwrap();
    let stat = call(
        &mut realm,
        &binding,
        "fstat",
        &[descriptor.clone(), Value::Boolean(false)],
    )
    .unwrap();
    assert_eq!(
        ops::get(&mut realm, &stat, &Key::Index(12)).unwrap(),
        number(1_700_000_003)
    );
    for name in ["fsync", "fdatasync"] {
        call(
            &mut realm,
            &binding,
            name,
            core::slice::from_ref(&descriptor),
        )
        .unwrap();
    }
    call(
        &mut realm,
        &binding,
        "ftruncate",
        &[descriptor.clone(), number(3)],
    )
    .unwrap();
    std::assert_eq!(std::fs::read(fixture.0.join("hard")).unwrap(), b"abc");
    call(&mut realm, &binding, "close", &[descriptor]).unwrap();
    let directory = fixture.path("directory");
    call(
        &mut realm,
        &binding,
        "mkdir",
        &[directory.clone(), number(0o777), Value::Boolean(false)],
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "symlink",
        &[
            directory.clone(),
            fixture.path("junction"),
            number(crate::fs_win::SYMLINK_JUNCTION),
        ],
    )
    .unwrap();
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readlink",
            &[fixture.path("junction"), Value::str("utf8")]
        )
        .unwrap(),
        directory
    );
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "realpath",
            &[fixture.path("junction")]
        )
        .unwrap(),
        Value::str(
            std::fs::canonicalize(fixture.0.join("directory"))
                .unwrap()
                .to_str()
                .unwrap()
                .strip_prefix(r"\\?\")
                .unwrap()
        )
    );
    let bytes = call(
        &mut realm,
        &binding,
        "readlink",
        &[fixture.path("junction"), Value::str("buffer")],
    )
    .unwrap();
    std::assert_eq!(
        &*typed_array_builtin::view_of(&bytes).unwrap().bytes.borrow(),
        fixture.0.join("directory").to_str().unwrap().as_bytes()
    );
    call(&mut realm, &binding, "unlink", &[fixture.path("junction")]).unwrap();
    call(&mut realm, &binding, "rmdir", &[fixture.path("directory")]).unwrap();
    let temporary = call(
        &mut realm,
        &binding,
        "mkdtemp",
        &[fixture.path("temp-"), Value::str("utf8")],
    )
    .unwrap();
    // Like Node's binding, six random characters follow the prefix.
    assert!(matches!(&temporary, Value::String(path) if {
        let path = std::path::PathBuf::from(String::from_utf16_lossy(path.units()));
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        path.is_dir() && name.len() == 11 && name.starts_with("temp-")
    }));
    let module_stat = |realm: &mut Realm, path: Value| {
        let receiver = binding.clone();
        call(realm, &binding, "internalModuleStat", &[receiver, path]).unwrap()
    };
    assert_eq!(
        module_stat(&mut realm, temporary.clone()),
        Value::Number(1.0)
    );
    assert_eq!(module_stat(&mut realm, file.clone()), Value::Number(0.0));
    assert_eq!(
        module_stat(&mut realm, fixture.path("missing")),
        Value::Number(-4058.0)
    );
    call(&mut realm, &binding, "rmdir", &[temporary]).unwrap();
    call(&mut realm, &binding, "unlink", core::slice::from_ref(&file)).unwrap();
    assert_eq!(
        call(&mut realm, &binding, "existsSync", &[file]).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn recursive_creation_reports_file_ancestors_and_handles_long_paths_iteratively() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"owned").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    for (path, code) in [("file", "EEXIST"), ("file/child", "ENOTDIR")] {
        let error = call(
            &mut realm,
            &binding,
            "mkdir",
            &[fixture.path(path), number(0o777), Value::Boolean(true)],
        )
        .unwrap_err()
        .0;
        assert_eq!(get(&mut realm, &error, "code"), Value::str(code));
    }
    let suffix = vec!["segment"; 70].join("/");
    let path = fixture.path(&suffix);
    let first = call(
        &mut realm,
        &binding,
        "mkdir",
        &[path.clone(), number(0o777), Value::Boolean(true)],
    )
    .unwrap();
    assert_eq!(first, fixture.path("segment"));
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "mkdir",
            &[path, number(0o777), Value::Boolean(true)]
        )
        .unwrap(),
        Value::Undefined
    );
    std::assert_eq!(std::fs::read(fixture.0.join("file")).unwrap(), b"owned");
}

#[test]
fn invalid_inputs_and_failed_fast_io_preserve_error_contracts_and_owned_descriptors() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abc").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    for path in [Value::Null, Value::str("a\0b")] {
        assert!(call(&mut realm, &binding, "stat", &[path, Value::Boolean(false)]).is_err());
    }
    let error = call(
        &mut realm,
        &binding,
        "stat",
        &[Value::str(""), Value::Boolean(false)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "path"), Value::str(""));
    for descriptor in [Value::str("3"), number(-1), Value::Number(1.5)] {
        assert!(call(&mut realm, &binding, "close", &[descriptor]).is_err());
    }
    for encoding in [Value::str("unknown"), Value::Null] {
        assert!(
            call(
                &mut realm,
                &binding,
                "readdir",
                &[fixture.path(""), encoding]
            )
            .is_err()
        );
    }
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    assert!(ops::call(&mut realm, &ctor, Value::Undefined, &[]).is_err());
    for request in [Value::Null, Value::Object(realm.object())] {
        assert!(
            call(
                &mut realm,
                &binding,
                "stat",
                &[fixture.path("file"), Value::Boolean(false), request]
            )
            .is_err()
        );
    }
    assert!(
        call(
            &mut realm,
            &binding,
            "writeFileUtf8",
            &[fixture.path("file"), number(1), number(0), number(0o666)]
        )
        .is_err()
    );
    let error = call(
        &mut realm,
        &binding,
        "readFileUtf8",
        &[fixture.path("missing"), number(0)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("open"));
    let descriptor = call(
        &mut realm,
        &binding,
        "open",
        &[fixture.path("file"), number(0), number(0)],
    )
    .unwrap();
    let error = call(
        &mut realm,
        &binding,
        "writeFileUtf8",
        &[
            descriptor.clone(),
            Value::str("x"),
            number(0),
            number(0o666),
        ],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("write"));
    assert_eq!(get(&mut realm, &error, "path"), Value::Undefined);
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readFileUtf8",
            &[descriptor.clone(), number(0)]
        )
        .unwrap(),
        Value::str("abc")
    );
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readFileUtf8",
            &[descriptor.clone(), number(0)]
        )
        .unwrap(),
        Value::str("")
    );
    call(
        &mut realm,
        &binding,
        "close",
        core::slice::from_ref(&descriptor),
    )
    .unwrap();
    let error = call(
        &mut realm,
        &binding,
        "readFileUtf8",
        &[descriptor, number(0)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("read"));
    let write_only = call(
        &mut realm,
        &binding,
        "open",
        &[
            fixture.path("file"),
            number(crate::fs_win::O_WRONLY),
            number(0),
        ],
    )
    .unwrap();
    let error = call(
        &mut realm,
        &binding,
        "readFileUtf8",
        &[write_only.clone(), number(0)],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("read"));
    assert_eq!(get(&mut realm, &error, "path"), Value::Undefined);
    call(&mut realm, &binding, "close", &[write_only]).unwrap();
    let path_bytes = fixture.0.join("file").to_str().unwrap().as_bytes().to_vec();
    let path = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        path_bytes.len(),
    ));
    typed_array_builtin::view_of(&path)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(&path_bytes);
    assert_eq!(
        call(&mut realm, &binding, "readFileUtf8", &[path, number(0)]).unwrap(),
        Value::str("abc")
    );
}

#[test]
fn worker_reads_publish_before_callback_and_failures_reject_promises() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abcdef").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let descriptor = call(
        &mut realm,
        &binding,
        "open",
        &[
            fixture.path("file"),
            number(crate::fs_win::O_RDWR),
            number(0),
        ],
    )
    .unwrap();
    let buffer = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        5,
    ));
    let view = typed_array_builtin::view_of(&buffer).unwrap();
    view.bytes.borrow_mut().fill(99);
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    let request = ops::construct(&mut realm, &ctor, &[Value::Boolean(false)]).unwrap();
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let output = seen.clone();
    let bytes = view.bytes.clone();
    let callback = realm.function("readComplete", 2, false, move |_, _, args| {
        std::assert_eq!(&*bytes.borrow(), b"cefcc");
        *output.borrow_mut() = args.to_vec();
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "read",
        &[
            descriptor.clone(),
            buffer.clone(),
            number(1),
            number(4),
            crate::bigint::literal("4"),
            request,
        ],
    )
    .unwrap();
    std::assert_eq!(&*view.bytes.borrow(), b"ccccc");
    realm.run_event_loop().unwrap();
    assert_eq!(*seen.borrow(), vec![Value::Null, number(2)]);
    let key = get(&mut realm, &binding, "kUsePromises");
    let rejected = call(
        &mut realm,
        &binding,
        "access",
        &[fixture.path("missing"), number(0), key.clone()],
    )
    .unwrap();
    let failure_seen = Rc::new(core::cell::RefCell::new(None));
    let output = failure_seen.clone();
    let reject = realm.function("reject", 1, false, move |_, _, args| {
        *output.borrow_mut() = args.first().cloned();
        Ok(Value::Undefined)
    });
    let then = get(&mut realm, &rejected, "then");
    ops::call(
        &mut realm,
        &then,
        rejected,
        &[Value::Undefined, Value::Object(reject)],
    )
    .unwrap();
    let vectors = Value::Object(realm.array(vec![buffer.clone()]));
    let write = call(
        &mut realm,
        &binding,
        "writeBuffers",
        &[descriptor.clone(), vectors, number(0), key],
    )
    .unwrap();
    let write_seen = Rc::new(core::cell::RefCell::new(None));
    let output = write_seen.clone();
    let fulfill = realm.function("writeComplete", 1, false, move |_, _, args| {
        *output.borrow_mut() = args.first().cloned();
        Ok(Value::Undefined)
    });
    let then = get(&mut realm, &write, "then");
    ops::call(&mut realm, &then, write, &[Value::Object(fulfill)]).unwrap();
    realm.run_event_loop().unwrap();
    let error = failure_seen.borrow().clone().unwrap();
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    assert_eq!(write_seen.borrow().clone().unwrap(), number(5));
    std::assert_eq!(std::fs::read(fixture.0.join("file")).unwrap(), b"cefccf");
    call(&mut realm, &binding, "close", &[descriptor]).unwrap();
}

#[test]
fn transfer_validation_bigint_positions_and_empty_vectors_preserve_file_content() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abcdef").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let descriptor = call(
        &mut realm,
        &binding,
        "open",
        &[
            fixture.path("file"),
            number(crate::fs_win::O_RDWR),
            number(0),
        ],
    )
    .unwrap();
    let buffer = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        2,
    ));
    assert!(
        call(
            &mut realm,
            &binding,
            "read",
            &[descriptor.clone(), Value::Null, number(0), number(1)]
        )
        .is_err()
    );
    for (start, count) in [
        (number(-1), number(1)),
        (number(0), number(3)),
        (Value::Number(0.5), number(1)),
    ] {
        assert!(
            call(
                &mut realm,
                &binding,
                "read",
                &[descriptor.clone(), buffer.clone(), start, count]
            )
            .is_err()
        );
    }
    assert!(
        call(
            &mut realm,
            &binding,
            "writeString",
            &[
                descriptor.clone(),
                Value::str("x"),
                number(0),
                Value::str("unknown")
            ]
        )
        .is_err()
    );
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "read",
            &[
                descriptor.clone(),
                buffer.clone(),
                number(0),
                number(2),
                crate::bigint::literal("2")
            ]
        )
        .unwrap(),
        number(2)
    );
    std::assert_eq!(
        &*typed_array_builtin::view_of(&buffer)
            .unwrap()
            .bytes
            .borrow(),
        b"cd"
    );
    let context = Value::Object(realm.object());
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "writeBuffer",
            &[
                descriptor.clone(),
                buffer,
                number(0),
                number(2),
                Value::Number(f64::NAN),
                Value::Undefined,
                context
            ]
        )
        .unwrap(),
        number(2)
    );
    std::assert_eq!(std::fs::read(fixture.0.join("file")).unwrap(), b"cdcdef");
    let empty = Value::Object(realm.array(vec![]));
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "readBuffers",
            &[descriptor.clone(), empty, Value::Null]
        )
        .unwrap(),
        number(0)
    );
    call(&mut realm, &binding, "close", &[descriptor]).unwrap();
}

#[test]
fn failed_worker_setup_completes_once_and_never_performs_the_requested_mutation() {
    for create in [
        crate::work_queue::tests::failed_create as fn() -> _,
        crate::work_queue::tests::failed_submit,
    ] {
        let fixture = Fixture::new();
        let mut realm = Realm::new();
        realm.event_loop.io_create = create;
        let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
        let ctor = get(&mut realm, &binding, "FSReqCallback");
        let request = ops::construct(&mut realm, &ctor, &[]).unwrap();
        let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
        let output = seen.clone();
        let callback = realm.function("failed", 1, false, move |_, _, args| {
            output.borrow_mut().push(args[0].clone());
            Ok(Value::Undefined)
        });
        ops::set(
            &mut realm,
            &request,
            &Key::name("oncomplete"),
            Value::Object(callback),
        )
        .unwrap();
        call(
            &mut realm,
            &binding,
            "mkdir",
            &[
                fixture.path("never"),
                number(0o777),
                Value::Boolean(false),
                request,
            ],
        )
        .unwrap();
        realm.run_event_loop().unwrap();
        std::assert_eq!(seen.borrow().len(), 1);
        let error = seen.borrow()[0].clone();
        assert_eq!(get(&mut realm, &error, "code"), Value::str("EPERM"));
        assert!(!fixture.0.join("never").exists());
    }
}

#[test]
fn normalized_namespace_paths_and_native_resolution_failures_keep_error_paths() {
    let mut realm = Realm::new();
    for (input, plain) in [
        (r"\\server\share\file", r"\\server\share\file"),
        (r"\\?\UNC\server\share\file", r"\\server\share\file"),
        (r"\\?\C:\owned\file", r"C:\owned\file"),
    ] {
        let (units, text) = validated_path(&realm, &Value::str(input), "stat").unwrap();
        assert_eq!(Value::String(text), Value::str(plain));
        assert!(units.starts_with(&r"\\?\".encode_utf16().collect::<Vec<_>>()));
    }
    unsafe extern "system" fn failed(_: *const u16, _: u32, _: *mut u16, _: *mut *mut u16) -> u32 {
        0
    }
    unsafe extern "system" fn denied() -> u32 {
        5
    }
    let error = resolve_path(&realm, &Value::str("owned"), "stat", failed, denied)
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("EPERM"));
    assert_eq!(get(&mut realm, &error, "path"), Value::str("owned"));
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("stat"));
    let context = Context {
        syscall: "realpath",
        path: None,
        destination: None,
        reads: vec![],
        encoding: None,
        buffer_names: false,
        with_types: false,
    };
    assert_eq!(
        named_output(
            &realm,
            &context,
            r"\\?\UNC\server\share\file".encode_utf16().collect()
        ),
        Value::str(r"\\server\share\file")
    );
    let prototype = realm.object();
    realm.buffer_prototype = Some(prototype.clone());
    let context = Context {
        buffer_names: true,
        ..context
    };
    let name = named_output(&realm, &context, vec![0x4e2d]);
    assert!(matches!(name, Value::Object(buffer) if buffer.data().prototype == Some(prototype)));
}

#[test]
fn callback_void_bigint_promises_and_throwing_completion_are_preserved() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("file"), b"abc").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    let request = ops::construct(&mut realm, &ctor, &[]).unwrap();
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let output = seen.clone();
    let callback = realm.function("done", 1, false, move |_, _, args| {
        *output.borrow_mut() = args.to_vec();
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "access",
        &[fixture.path("file"), number(0), request],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(*seen.borrow(), vec![Value::Null]);
    let key = get(&mut realm, &binding, "kUsePromises");
    let promise = call(
        &mut realm,
        &binding,
        "statfs",
        &[fixture.path(""), Value::Boolean(true), key],
    )
    .unwrap();
    let output = seen.clone();
    let callback = realm.function("done", 1, false, move |_, _, args| {
        *output.borrow_mut() = args.to_vec();
        Ok(Value::Undefined)
    });
    let then = get(&mut realm, &promise, "then");
    ops::call(&mut realm, &then, promise, &[Value::Object(callback)]).unwrap();
    realm.run_event_loop().unwrap();
    let stat = seen.borrow()[0].clone();
    assert_eq!(get(&mut realm, &stat, "length"), number(7));
    assert!(matches!(
        ops::get(&mut realm, &stat, &Key::Index(1)).unwrap(),
        Value::BigInt(_)
    ));
    // A fresh realm is required because the preceding requests already own a signal.
    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::failed_create;
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    let request = ops::construct(&mut realm, &ctor, &[]).unwrap();
    let callback = realm.function("throw", 1, false, |_, _, _| {
        Err(Thrown(Value::str("completion failed")))
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "access",
            &[fixture.path("file"), number(0), request]
        )
        .unwrap_err()
        .0,
        Value::str("completion failed")
    );
}

#[test]
fn zero_progress_write_releases_its_owned_descriptor_and_context_setter_errors_propagate() {
    let fixture = Fixture::new();
    let mut realm = Realm::new();
    let state = Binding {
        files: Arc::new(Files::new()),
        stats: Statistics::new(&realm, 18, 36),
        statfs: Statistics::new(&realm, 7, 7),
        promise: Symbol::new(None),
        request: Symbol::private("request"),
        handle_fd: Symbol::private("handle.fd"),
        handle_close_promise: Symbol::private("handle.closePromise"),
        directory_id: Symbol::private("directory.id"),
        next_async_id: AtomicU64::new(1),
    };
    let error = utf8_file_with(
        &state,
        &mut realm,
        &[
            fixture.path("file"),
            Value::str("abc"),
            number(crate::fs_win::O_CREAT | crate::fs_win::O_WRONLY),
            number(0o666),
        ],
        true,
        |_, _, _| Ok(0),
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("EIO"));
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("write"));
    assert!(state.files.with(|files| files.handle(3).is_err()));
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let buffer = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        1,
    ));
    let context = realm.object();
    let setter = realm.function("errno", 1, false, |_, _, _| {
        Err(Thrown(Value::str("setter failed")))
    });
    context.define(
        Key::name("errno"),
        Property {
            slot: crate::Slot::Accessor {
                get: None,
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "writeBuffer",
            &[
                number(999),
                buffer,
                number(0),
                number(1),
                Value::Null,
                Value::Undefined,
                Value::Object(context)
            ]
        )
        .unwrap_err()
        .0,
        Value::str("setter failed")
    );
}

#[test]
fn recursive_mkdir_stops_at_roots_and_propagates_native_terminal_errors() {
    fn missing(_: &[u16]) -> Result<(), i32> {
        Err(crate::fs_win::UV_ENOENT)
    }
    fn denied(_: &[u16]) -> Result<(), i32> {
        Err(crate::fs_win::UV_EPERM)
    }
    for path in ["child", "C:\\", "C:\\child\\"] {
        let units: Vec<u16> = path.encode_utf16().chain([0]).collect();
        std::assert_eq!(
            mkdir_with(&units, true, missing).err(),
            Some(crate::fs_win::UV_ENOENT)
        );
    }
    let units: Vec<u16> = "C:\\owned".encode_utf16().chain([0]).collect();
    std::assert_eq!(
        mkdir_with(&units, true, denied).err(),
        Some(crate::fs_win::UV_EPERM)
    );
}

#[test]
fn raw_windows_names_round_trip_through_buffer_paths_without_unicode_replacement() {
    use std::os::windows::ffi::OsStringExt;
    let fixture = Fixture::new();
    let name = [u16::from(b'n'), 0xd800, 0xdfff];
    let path = fixture.0.join(std::ffi::OsString::from_wide(&name));
    std::fs::write(&path, b"owned").unwrap();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let names = call(
        &mut realm,
        &binding,
        "readdir",
        &[
            fixture.path(""),
            Value::str("buffer"),
            Value::Boolean(false),
        ],
    )
    .unwrap();
    let first = ops::get(&mut realm, &names, &Key::Index(0)).unwrap();
    let view = typed_array_builtin::view_of(&first).unwrap();
    // D800 DFFF is a valid pair; seed a second, unpaired name as well.
    let expected = wtf8_bytes(&name);
    std::assert_eq!(&*view.bytes.borrow(), &expected);
    let unpaired = [u16::from(b'x'), 0xd800];
    std::fs::write(
        fixture.0.join(std::ffi::OsString::from_wide(&unpaired)),
        b"raw",
    )
    .unwrap();
    let names = call(
        &mut realm,
        &binding,
        "readdir",
        &[fixture.path(""), Value::str("utf8"), Value::Boolean(false)],
    )
    .unwrap();
    assert_eq!(
        ops::get(&mut realm, &names, &Key::Index(1)).unwrap(),
        Value::str("x���")
    );
    let mut raw = fixture.0.to_str().unwrap().as_bytes().to_vec();
    raw.extend_from_slice(b"\\x\xed\xa0\x80");
    let path = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        raw.len(),
    ));
    typed_array_builtin::view_of(&path)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(&raw);
    assert_eq!(
        call(&mut realm, &binding, "readFileUtf8", &[path, number(0)]).unwrap(),
        Value::str("raw")
    );
    let mut invalid = fixture.0.to_str().unwrap().as_bytes().to_vec();
    invalid.extend_from_slice(b"\\bad\xff");
    let path = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        invalid.len(),
    ));
    typed_array_builtin::view_of(&path)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(&invalid);
    let error = call(&mut realm, &binding, "stat", &[path, Value::Boolean(false)])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    assert_eq!(get(&mut realm, &error, "syscall"), Value::str("stat"));
}

#[test]
fn pinned_libuv_path_decoding_preserves_surrogates_and_rejects_bad_byte_sequences() {
    for (bytes, units) in [
        (&b"ascii"[..], vec![97, 115, 99, 105, 105]),
        (&b"\xc2\xa2"[..], vec![0xa2]),
        (&b"\xed\xa0\x80"[..], vec![0xd800]),
        (&b"\xed\xbf\xbf"[..], vec![0xdfff]),
        (&b"\xf0\x9f\x9a\x80"[..], vec![0xd83d, 0xde80]),
        (&b"\xe0\x80\xaf"[..], vec![47]),
        (&b"\xf0\x80\x80\xaf"[..], vec![47]),
    ] {
        std::assert_eq!(wtf8_units(bytes), Some(units));
    }
    for bytes in [
        &b"\x80"[..],
        &b"\xc0\xaf"[..],
        &b"\xc2x"[..],
        &b"\xe2\x82"[..],
        &b"\xf0\x9f\x9a"[..],
        &b"\xf5\x80\x80\x80"[..],
        &b"\xf4\x90\x80\x80"[..],
    ] {
        std::assert_eq!(wtf8_units(bytes), None);
    }
    std::assert_eq!(
        wtf8_bytes(&[0xd800, 0x61, 0xdfff]),
        b"\xed\xa0\x80a\xed\xbf\xbf"
    );
}

#[test]
fn raw_symlink_target_error_paths_decode_like_node_strings() {
    let fixture = Fixture::new();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let mut bytes = fixture.0.to_str().unwrap().as_bytes().to_vec();
    bytes.extend_from_slice(b"\\x\xed\xa0\x80");
    let target = Value::Object(typed_array_builtin::new_typed_array(
        &realm,
        Element::Uint8,
        bytes.len(),
    ));
    typed_array_builtin::view_of(&target)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(&bytes);
    let error = call(
        &mut realm,
        &binding,
        "symlink",
        &[
            target,
            fixture.path("missing/link"),
            number(crate::fs_win::SYMLINK_JUNCTION),
        ],
    )
    .unwrap_err()
    .0;
    assert_eq!(get(&mut realm, &error, "path"), fixture.path("x���"));
    assert_eq!(
        get(&mut realm, &error, "dest"),
        fixture.path(r"missing\link")
    );
}

fn invalid_path(realm: &Realm, fixture: &Fixture) -> Value {
    let mut bytes = fixture.0.to_str().unwrap().as_bytes().to_vec();
    bytes.extend_from_slice(b"\\invalid-\xff");
    let value = Value::Object(typed_array_builtin::new_typed_array(
        realm,
        Element::Uint8,
        bytes.len(),
    ));
    typed_array_builtin::view_of(&value)
        .unwrap()
        .bytes
        .borrow_mut()
        .copy_from_slice(&bytes);
    value
}

#[test]
fn native_capture_errors_are_suppressed_or_completed_immediately_without_worker_setup() {
    let fixture = Fixture::new();
    let mut realm = Realm::new();
    realm.event_loop.io_create = crate::work_queue::tests::failed_create;
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let path = invalid_path(&realm, &fixture);
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "existsSync",
            core::slice::from_ref(&path)
        )
        .unwrap(),
        Value::Boolean(false)
    );
    for name in ["stat", "lstat"] {
        assert_eq!(
            call(
                &mut realm,
                &binding,
                name,
                &[
                    path.clone(),
                    Value::Boolean(false),
                    Value::Undefined,
                    Value::Boolean(false)
                ]
            )
            .unwrap(),
            Value::Undefined
        );
        let error = call(
            &mut realm,
            &binding,
            name,
            &[path.clone(), Value::Boolean(false)],
        )
        .unwrap_err()
        .0;
        assert_eq!(get(&mut realm, &error, "path"), fixture.path("invalid-�"));
        assert_eq!(get(&mut realm, &error, "syscall"), Value::str(name));
    }
    for (name, args) in [
        ("readFileUtf8", vec![path.clone()]),
        (
            "writeFileUtf8",
            vec![path.clone(), Value::str("unused"), number(0), number(0o666)],
        ),
    ] {
        let error = call(&mut realm, &binding, name, &args).unwrap_err().0;
        assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
        assert_eq!(get(&mut realm, &error, "path"), fixture.path("invalid-�"));
        assert_eq!(get(&mut realm, &error, "syscall"), Value::str("open"));
    }
    let ctor = get(&mut realm, &binding, "FSReqCallback");
    let request = ops::construct(&mut realm, &ctor, &[]).unwrap();
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let output = seen.clone();
    let callback = realm.function("capture", 1, false, move |_, this, args| {
        output.borrow_mut().push((this, args.to_vec()));
        Ok(Value::Undefined)
    });
    ops::set(
        &mut realm,
        &request,
        &Key::name("oncomplete"),
        Value::Object(callback),
    )
    .unwrap();
    call(
        &mut realm,
        &binding,
        "stat",
        &[path.clone(), Value::Boolean(false), request.clone()],
    )
    .unwrap();
    std::assert_eq!(seen.borrow().len(), 1);
    assert_eq!(seen.borrow()[0].0, request);
    let error = seen.borrow()[0].1[0].clone();
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    assert_eq!(get(&mut realm, &error, "path"), Value::Undefined);
    assert!(!realm.alive());
    realm.run_event_loop().unwrap();
    std::assert_eq!(seen.borrow().len(), 1);
    let error = call(&mut realm, &binding, "rename", &[fixture.path("old"), path])
        .unwrap_err()
        .0;
    assert_eq!(get(&mut realm, &error, "path"), fixture.path("old"));
    assert_eq!(get(&mut realm, &error, "dest"), fixture.path("invalid-�"));
}

#[test]
fn captured_private_promise_rejects_without_a_return_value_like_node_dispatch_failure() {
    let fixture = Fixture::new();
    let mut realm = Realm::new();
    let binding = crate::modules::load(&mut realm, "binding/fs").unwrap();
    let path = invalid_path(&realm, &fixture);
    let key = get(&mut realm, &binding, "kUsePromises");
    assert_eq!(
        call(
            &mut realm,
            &binding,
            "stat",
            &[path, Value::Boolean(false), key]
        )
        .unwrap(),
        Value::Undefined
    );
    let error = realm.run_event_loop().unwrap_err().0;
    assert_eq!(get(&mut realm, &error, "code"), Value::str("ENOENT"));
    assert_eq!(get(&mut realm, &error, "path"), Value::Undefined);
}
