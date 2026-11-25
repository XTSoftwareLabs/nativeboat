//! Windows named pipes for `net`, as libuv's pipe streams use them: a server
//! listening on `\\.\pipe\name` and clients connecting to it, with the same
//! handle methods as the TCP binding's connections and servers.
//!
//! A server creates the name's first instance with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a name another server holds fails with
//! `EADDRINUSE` as in libuv, and keeps one instance waiting for the next
//! client. A client opens the pipe, waiting while every instance is busy.
//! Blocking steps run on the I/O pool and report on the realm thread; reads,
//! writes and waits for clients are overlapped requests that a handle's
//! cancel event ends when it closes. A pipe closes with the last of its
//! handle and pending requests.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = usize::MAX as Handle;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const PIPE_TYPE_BYTE: u32 = 0;
const PIPE_READMODE_BYTE: u32 = 0;
const PIPE_WAIT: u32 = 0;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_PIPE_BUSY: u32 = 231;
const ERROR_NO_DATA: u32 = 232;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_IO_PENDING: u32 = 997;
const WSAEADDRINUSE: u32 = 10048;
const WSAEACCES: u32 = 10013;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = u32::MAX;
/// How long a client waits for a busy pipe, as libuv's 30 seconds.
const BUSY_WAIT_MS: u32 = 30_000;
/// The pipe's buffers and the most one read takes, as libuv's 64 KiB.
const BUFFER_SIZE: u32 = 64 * 1024;

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: Handle,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_buffer: u32,
        in_buffer: u32,
        timeout: u32,
        security: *mut c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn GetOverlappedResult(
        file: Handle,
        overlapped: *mut Overlapped,
        transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn CreateEventW(security: *mut c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: i32, timeout: u32) -> u32;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn last_error() -> u32 {
    // SAFETY: reads this thread's last Win32 error.
    unsafe { GetLastError() }
}

/// A manual-reset event, closed when dropped.
struct Event(Handle);

// SAFETY: event handles may be used from any thread.
unsafe impl Send for Event {}
// SAFETY: event operations are thread-safe.
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Self, u32> {
        // SAFETY: an unnamed manual-reset event without security attributes.
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
        if event.is_null() {
            return Err(last_error());
        }
        Ok(Self(event))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the event is owned here.
        unsafe { CloseHandle(self.0) };
    }
}

/// A pipe handle and its cancel event, shared by the JavaScript handle and
/// its pending requests; the pipe closes with the last of them. A server's
/// is the instance waiting for the next client, which `accept` replaces.
struct Shared {
    pipe: AtomicUsize,
    cancel: Event,
    /// The pipe's name, for a server's next instances.
    name: Option<Vec<u16>>,
}

// SAFETY: pipe handles may be used from any thread.
unsafe impl Send for Shared {}
// SAFETY: the handle is swapped atomically; pipe operations are
// thread-safe.
unsafe impl Sync for Shared {}

impl Shared {
    fn new(pipe: Handle, name: Option<Vec<u16>>) -> Result<Arc<Self>, u32> {
        Ok(Arc::new(Self {
            pipe: AtomicUsize::new(pipe as usize),
            cancel: Event::new()?,
            name,
        }))
    }

    fn pipe(&self) -> Handle {
        self.pipe.load(Ordering::Acquire) as Handle
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let pipe = self.pipe();
        if pipe != INVALID_HANDLE_VALUE {
            // SAFETY: no request uses the pipe any more.
            unsafe { CloseHandle(pipe) };
        }
    }
}

/// How a pending request ended.
enum Outcome<T> {
    Done(T),
    Failed(u32),
    /// The handle was closed while the request waited.
    Cancelled,
}

/// Runs an overlapped request on `pipe` and waits for it or for `shared`'s
/// cancellation. `start` returns whether the request completed at once, or
/// the error it failed with.
fn overlapped(
    shared: &Shared,
    pipe: Handle,
    start: impl FnOnce(*mut Overlapped) -> Result<bool, u32>,
) -> Outcome<u32> {
    let event = match Event::new() {
        Ok(event) => event,
        Err(error) => return Outcome::Failed(error),
    };
    let mut request = Overlapped {
        event: event.0,
        ..Overlapped::default()
    };
    match start(&mut request) {
        Ok(_) => {}
        Err(ERROR_IO_PENDING) => {}
        Err(error) => return Outcome::Failed(error),
    }
    let handles = [event.0, shared.cancel.0];
    // SAFETY: both events stay open for the wait.
    let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
    if woke != WAIT_OBJECT_0 {
        // SAFETY: the request belongs to this pipe; it must finish before
        // its OVERLAPPED and buffers are released.
        unsafe {
            CancelIoEx(pipe, &mut request);
            WaitForSingleObject(event.0, INFINITE);
        }
    }
    let mut transferred = 0;
    // SAFETY: the request completed on this pipe.
    let done = unsafe { GetOverlappedResult(pipe, &mut request, &mut transferred, 0) };
    if woke != WAIT_OBJECT_0 {
        return Outcome::Cancelled;
    }
    if done == 0 {
        return Outcome::Failed(last_error());
    }
    Outcome::Done(transferred)
}

/// Whether a call that returned `ok` completed, or the error it reported.
fn started(ok: i32) -> Result<bool, u32> {
    if ok != 0 { Ok(true) } else { Err(last_error()) }
}

/// Reads what arrives next; empty at the end of the stream, when the other
/// end has closed.
fn read(shared: &Shared) -> Outcome<Vec<u8>> {
    let pipe = shared.pipe();
    let mut buffer = alloc::vec![0u8; BUFFER_SIZE as usize];
    let pointer = buffer.as_mut_ptr();
    // SAFETY: the buffer outlives the request, which `overlapped` awaits.
    let outcome = overlapped(shared, pipe, |request| unsafe {
        started(ReadFile(
            pipe,
            pointer,
            BUFFER_SIZE,
            core::ptr::null_mut(),
            request.cast(),
        ))
    });
    match outcome {
        Outcome::Done(count) => {
            buffer.truncate(count as usize);
            Outcome::Done(buffer)
        }
        Outcome::Failed(ERROR_BROKEN_PIPE | ERROR_NO_DATA) => Outcome::Done(Vec::new()),
        Outcome::Failed(error) => Outcome::Failed(error),
        Outcome::Cancelled => Outcome::Cancelled,
    }
}

fn write(shared: &Shared, bytes: &[u8]) -> Outcome<()> {
    let pipe = shared.pipe();
    let mut offset = 0;
    while offset < bytes.len() {
        let length = (bytes.len() - offset).min(u32::MAX as usize) as u32;
        let pointer = bytes[offset..].as_ptr();
        // SAFETY: the bytes outlive the request, which `overlapped` awaits.
        let outcome = overlapped(shared, pipe, |request| unsafe {
            started(WriteFile(
                pipe,
                pointer,
                length,
                core::ptr::null_mut(),
                request.cast(),
            ))
        });
        match outcome {
            Outcome::Done(0) => return Outcome::Failed(ERROR_NO_DATA),
            Outcome::Done(count) => offset += count as usize,
            Outcome::Failed(error) => return Outcome::Failed(error),
            Outcome::Cancelled => return Outcome::Cancelled,
        }
    }
    Outcome::Done(())
}

/// A failed operation and the call that failed, for the error's message.
#[derive(Debug)]
struct Failure {
    error: u32,
    syscall: &'static str,
}

/// Creates an instance of the named pipe, the first only if no server holds
/// the name.
fn instance(name: &[u16], first: bool) -> Result<Handle, u32> {
    let flags = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    // SAFETY: the name is NUL-terminated; no security attributes.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER_SIZE,
            BUFFER_SIZE,
            0,
            core::ptr::null_mut(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    Ok(pipe)
}

/// A server on the named pipe, as libuv binds one: a name in use is
/// `EADDRINUSE`, and an invalid one `EACCES`.
fn server(path: &str) -> Result<Arc<Shared>, Failure> {
    let name = wide(path);
    let pipe = instance(&name, true).map_err(|error| Failure {
        error: match error {
            ERROR_ACCESS_DENIED => WSAEADDRINUSE,
            ERROR_PATH_NOT_FOUND | ERROR_INVALID_NAME => WSAEACCES,
            error => error,
        },
        syscall: "listen",
    })?;
    Shared::new(pipe, Some(name)).map_err(|error| {
        // SAFETY: the instance was just created and is owned here.
        unsafe { CloseHandle(pipe) };
        Failure {
            error,
            syscall: "listen",
        }
    })
}

/// Waits for a client on the server's waiting instance, which becomes the
/// connection; a new instance then waits for the next client.
fn accept_next(shared: &Shared) -> Outcome<usize> {
    let pipe = shared.pipe();
    if pipe == INVALID_HANDLE_VALUE {
        return Outcome::Cancelled;
    }
    // SAFETY: the instance is open while `shared` lives.
    let outcome = overlapped(shared, pipe, |request| unsafe {
        match started(ConnectNamedPipe(pipe, request.cast())) {
            // A client that connected before the wait began.
            Err(ERROR_PIPE_CONNECTED) => Ok(true),
            other => other,
        }
    });
    match outcome {
        Outcome::Done(_) => {}
        Outcome::Failed(error) => return Outcome::Failed(error),
        Outcome::Cancelled => return Outcome::Cancelled,
    }
    let name = shared.name.as_deref().unwrap_or(&[0]);
    let next = instance(name, false).unwrap_or(INVALID_HANDLE_VALUE);
    shared.pipe.store(next as usize, Ordering::Release);
    Outcome::Done(pipe as usize)
}

/// Opens the named pipe, waiting while every instance is busy.
fn connect_to(path: &str) -> Result<Handle, Failure> {
    let name = wide(path);
    loop {
        // SAFETY: the name is NUL-terminated; no security attributes or
        // template.
        let pipe = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                core::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                core::ptr::null_mut(),
            )
        };
        if pipe != INVALID_HANDLE_VALUE {
            return Ok(pipe);
        }
        let error = last_error();
        // SAFETY: the name is NUL-terminated.
        if error != ERROR_PIPE_BUSY || unsafe { WaitNamedPipeW(name.as_ptr(), BUSY_WAIT_MS) } == 0 {
            return Err(Failure {
                error: if error == ERROR_PIPE_BUSY {
                    last_error()
                } else {
                    error
                },
                syscall: "connect",
            });
        }
    }
}

struct HandleState {
    shared: Option<Arc<Shared>>,
    referenced: Rc<Cell<bool>>,
    reading: bool,
}

type State = Rc<RefCell<HandleState>>;

/// An Error as Node's `exceptionWithHostPort` makes them for pipes:
/// `syscall CODE[: description] path`, with `errno`, `code`, `syscall` and
/// `address` properties.
fn pipe_error(realm: &mut Realm, failure: &Failure, path: Option<&str>, uv_message: bool) -> Value {
    let errno = crate::uv_error::translate(failure.error);
    let (code, description) = crate::uv_error::describe(errno);
    let mut message = alloc::format!("{} {code}", failure.syscall);
    if uv_message && !description.is_empty() {
        message.push_str(": ");
        message.push_str(&description);
    }
    if let Some(path) = path {
        message.push(' ');
        message.push_str(path);
    }
    let error = realm.error(ErrorKind::Error, &message);
    error.define(
        Key::name("errno"),
        Property::data(Value::Number(f64::from(errno))),
    );
    error.define(Key::name("code"), Property::data(Value::str(&code)));
    error.define(
        Key::name("syscall"),
        Property::data(Value::str(failure.syscall)),
    );
    if let Some(path) = path {
        error.define(Key::name("address"), Property::data(Value::str(path)));
    }
    Value::Object(error)
}

fn text_arg(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, Thrown> {
    match args.get(index) {
        None | Some(Value::Undefined | Value::Null) => Ok(String::new()),
        Some(value) => Ok(ops::to_string(realm, value)?.to_rust_lossy()),
    }
}

fn callback_arg(realm: &Realm, args: &[Value], index: usize) -> Result<Value, Thrown> {
    let callback = args.get(index).cloned().unwrap_or(Value::Undefined);
    if !crate::array_builtin::is_callable(&callback) {
        return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
    }
    Ok(callback)
}

fn define_function(
    realm: &Realm,
    object: &Object,
    name: &str,
    length: u32,
    body: impl Fn(&mut Realm, &[Value]) -> Result<Value, Thrown> + 'static,
) {
    let function = realm.function(name, length, false, move |realm, _, args| body(realm, args));
    object.define(Key::name(name), Property::data(Value::Object(function)));
}

fn queue_error(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Error, "could not queue a pipe request")
}

fn new_state(shared: Arc<Shared>) -> State {
    Rc::new(RefCell::new(HandleState {
        shared: Some(shared),
        referenced: Rc::new(Cell::new(true)),
        reading: false,
    }))
}

/// The methods shared by connections and servers: `close()`, `ref()`,
/// `unref()` and `hasRef()`.
fn handle_object(realm: &Realm, state: &State) -> Object {
    let handle = realm.object();
    let closing = state.clone();
    define_function(realm, &handle, "close", 0, move |_, _| {
        if let Some(shared) = closing.borrow_mut().shared.take() {
            // SAFETY: the event is open while `shared` lives.
            unsafe { SetEvent(shared.cancel.0) };
        }
        Ok(Value::Undefined)
    });
    for (name, value) in [("ref", true), ("unref", false)] {
        let state = state.clone();
        define_function(realm, &handle, name, 0, move |_, _| {
            state.borrow().referenced.set(value);
            Ok(Value::Undefined)
        });
    }
    let state = state.clone();
    define_function(realm, &handle, "hasRef", 0, move |_, _| {
        Ok(Value::Boolean(state.borrow().referenced.get()))
    });
    handle
}

/// A connection's handle, as the TCP binding's: `readStart(onread)`,
/// `readStop()`, `write(bytes, callback)` and `shutdown()`. Pipes have no
/// socket options or addresses, so `setNoDelay` and `setKeepAlive` do
/// nothing and the addresses are undefined.
fn connection_object(realm: &Realm, shared: Arc<Shared>) -> Object {
    let state = new_state(shared);
    let handle = handle_object(realm, &state);
    let reader = state.clone();
    define_function(realm, &handle, "readStart", 1, move |realm, args| {
        let callback = callback_arg(realm, args, 0)?;
        if reader.borrow().reading {
            return Ok(Value::Undefined);
        }
        reader.borrow_mut().reading = true;
        queue_read(realm, reader.clone(), callback)?;
        Ok(Value::Undefined)
    });
    let stopper = state.clone();
    define_function(realm, &handle, "readStop", 0, move |_, _| {
        stopper.borrow_mut().reading = false;
        Ok(Value::Undefined)
    });
    let writer = state.clone();
    define_function(realm, &handle, "write", 2, move |realm, args| {
        let callback = callback_arg(realm, args, 1)?;
        let Some(bytes) = args
            .first()
            .and_then(crate::typed_array_builtin::view_bytes)
        else {
            return Err(realm.throw(ErrorKind::Type, "data must be a Buffer"));
        };
        let Some(shared) = writer.borrow().shared.clone() else {
            let failure = Failure {
                error: ERROR_BROKEN_PIPE,
                syscall: "write",
            };
            let error = pipe_error(realm, &failure, None, false);
            ops::call(realm, &callback, Value::Undefined, &[error])?;
            return Ok(Value::Undefined);
        };
        let referenced = writer.borrow().referenced.clone();
        realm
            .submit_io_with_ref(
                move || write(&shared, &bytes),
                referenced,
                move |realm, outcome| {
                    let error = match outcome {
                        Outcome::Done(()) | Outcome::Cancelled => Value::Null,
                        Outcome::Failed(error) => pipe_error(
                            realm,
                            &Failure {
                                error,
                                syscall: "write",
                            },
                            None,
                            false,
                        ),
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    // libuv's pipe shutdown waits for the writes, which complete in order;
    // the other end sees the end of the stream when this one closes.
    for name in ["shutdown", "setNoDelay", "setKeepAlive"] {
        define_function(realm, &handle, name, 0, |_, _| Ok(Value::Undefined));
    }
    for name in ["localAddress", "remoteAddress"] {
        define_function(realm, &handle, name, 0, |_, _| Ok(Value::Undefined));
    }
    handle
}

fn queue_read(realm: &mut Realm, state: State, callback: Value) -> Result<(), Thrown> {
    let Some(shared) = state.borrow().shared.clone() else {
        state.borrow_mut().reading = false;
        return Ok(());
    };
    let referenced = state.borrow().referenced.clone();
    realm
        .submit_io_with_ref(
            move || read(&shared),
            referenced,
            move |realm, outcome| {
                let (error, chunk) = match outcome {
                    Outcome::Cancelled => return Ok(()),
                    Outcome::Done(bytes) if bytes.is_empty() => (Value::Null, Value::Null),
                    Outcome::Done(bytes) => (
                        Value::Null,
                        Value::Object(crate::typed_array_builtin::new_bytes(realm, bytes, None)),
                    ),
                    Outcome::Failed(error) => (
                        pipe_error(
                            realm,
                            &Failure {
                                error,
                                syscall: "read",
                            },
                            None,
                            false,
                        ),
                        Value::Null,
                    ),
                };
                let more = matches!(chunk, Value::Object(_));
                if !more {
                    state.borrow_mut().reading = false;
                }
                ops::call(realm, &callback, Value::Undefined, &[error, chunk])?;
                // The callback may have stopped reading or closed the handle.
                if more && state.borrow().reading {
                    queue_read(realm, state, callback)?;
                }
                Ok(())
            },
        )
        .map_err(|_| queue_error(realm))
}

/// A server's handle: `accept(callback)` waits for one client and calls
/// `callback(error, connection)`.
fn server_object(realm: &Realm, shared: Arc<Shared>) -> Object {
    let state = new_state(shared);
    let handle = handle_object(realm, &state);
    let acceptor = state.clone();
    define_function(realm, &handle, "accept", 1, move |realm, args| {
        let callback = callback_arg(realm, args, 0)?;
        let Some(shared) = acceptor.borrow().shared.clone() else {
            return Ok(Value::Undefined);
        };
        let referenced = acceptor.borrow().referenced.clone();
        realm
            .submit_io_with_ref(
                move || accept_next(&shared),
                referenced,
                move |realm, outcome| {
                    let (error, connection) = match outcome {
                        Outcome::Cancelled => return Ok(()),
                        Outcome::Done(pipe) => match Shared::new(pipe as Handle, None) {
                            Ok(shared) => {
                                (Value::Null, Value::Object(connection_object(realm, shared)))
                            }
                            Err(error) => {
                                // SAFETY: the connected instance is owned here.
                                unsafe { CloseHandle(pipe as Handle) };
                                let failure = Failure {
                                    error,
                                    syscall: "accept",
                                };
                                (pipe_error(realm, &failure, None, false), Value::Null)
                            }
                        },
                        Outcome::Failed(error) => {
                            let failure = Failure {
                                error,
                                syscall: "accept",
                            };
                            (pipe_error(realm, &failure, None, false), Value::Null)
                        }
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error, connection])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    handle
}

/// `internalBinding('pipe')`: `connect(path, callback(error, connection))`
/// and `listen(path)`, a server or a thrown error.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    define_function(realm, &exports, "connect", 2, |realm, args| {
        let path = text_arg(realm, args, 0)?;
        let callback = callback_arg(realm, args, 1)?;
        let target = path.clone();
        realm
            .submit_io(
                move || {
                    connect_to(&target).and_then(|pipe| {
                        Shared::new(pipe, None).map_err(|error| {
                            // SAFETY: the opened pipe is owned here.
                            unsafe { CloseHandle(pipe) };
                            Failure {
                                error,
                                syscall: "connect",
                            }
                        })
                    })
                },
                move |realm, result| {
                    let (error, connection) = match result {
                        Ok(shared) => {
                            (Value::Null, Value::Object(connection_object(realm, shared)))
                        }
                        Err(failure) => {
                            (pipe_error(realm, &failure, Some(&path), false), Value::Null)
                        }
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error, connection])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    define_function(realm, &exports, "listen", 1, |realm, args| {
        let path = text_arg(realm, args, 0)?;
        match server(&path) {
            Ok(shared) => Ok(Value::Object(server_object(realm, shared))),
            Err(failure) => Err(Thrown(pipe_error(realm, &failure, Some(&path), true))),
        }
    });
    exports
}

#[cfg(test)]
mod tests;
