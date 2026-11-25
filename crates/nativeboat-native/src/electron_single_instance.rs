//! `app.requestSingleInstanceLock`, as Electron's process singleton keeps
//! one instance per user data directory. The first instance owns a named
//! pipe whose name derives from that directory; a later instance writes its
//! command line, working directory and additional data to it and is told it
//! lost the lock, so the application can quit. The first instance reports
//! each message as the app's `second-instance` event.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Object, Property, Realm, Thrown, Value, ops};

type Handle = *mut c_void;

const INVALID_HANDLE: isize = -1;
const PIPE_ACCESS_INBOUND: u32 = 0x1;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x8;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_NO_DATA: u32 = 232;
/// The largest message a later instance may send.
const MESSAGE_LIMIT: usize = 1 << 20;

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
        security: Handle,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: Handle) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn GetNamedPipeServerProcessId(pipe: Handle, process: *mut u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        length: u32,
        read: *mut u32,
        overlapped: Handle,
    ) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: Handle,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AllowSetForegroundWindow(process: u32) -> i32;
}

/// The pipe through which instances for a user data directory meet. Names
/// are case-insensitive on Windows, like the directory.
fn pipe_name(user_data: &str) -> Vec<u16> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for unit in user_data.to_lowercase().encode_utf16() {
        for byte in unit.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    alloc::format!("\\\\.\\pipe\\nativeboat-instance-{hash:016x}")
        .encode_utf16()
        .chain([0])
        .collect()
}

/// The owned pipe and its name while this instance holds the lock.
type Held = Rc<RefCell<Option<(usize, Vec<u16>)>>>;

/// Defines the lock methods on `app`.
pub(crate) fn install(realm: &Realm, app: &Object) {
    let held: Held = Rc::new(RefCell::new(None));
    let request_held = held.clone();
    let target = app.clone();
    app.define(
        Key::name("requestSingleInstanceLock"),
        Property::data(Value::Object(realm.function(
            "requestSingleInstanceLock",
            1,
            false,
            move |realm, _, args| request(realm, &target, &request_held, args),
        ))),
    );
    let has_held = held.clone();
    app.define(
        Key::name("hasSingleInstanceLock"),
        Property::data(Value::Object(realm.function(
            "hasSingleInstanceLock",
            0,
            false,
            move |_, _, _| Ok(Value::Boolean(has_held.borrow().is_some())),
        ))),
    );
    app.define(
        Key::name("releaseSingleInstanceLock"),
        Property::data(Value::Object(realm.function(
            "releaseSingleInstanceLock",
            0,
            false,
            move |_, _, _| {
                let released = held.borrow_mut().take();
                if let Some((pipe, name)) = released {
                    // The listener waits for a connection on the pipe, and
                    // closing it under that wait blocks: connect to wake it,
                    // and it closes the pipe. Without a listener, close it.
                    if !send(&name, b"") {
                        // SAFETY: the pipe was created by this instance.
                        unsafe { CloseHandle(pipe as Handle) };
                    }
                }
                Ok(Value::Undefined)
            },
        ))),
    );
}

fn request(realm: &mut Realm, app: &Object, held: &Held, args: &[Value]) -> Result<Value, Thrown> {
    if held.borrow().is_some() {
        return Ok(Value::Boolean(true));
    }
    let user_data = match crate::electron_builtin::app_path(realm, "userData")? {
        Value::String(path) => path.to_rust_lossy(),
        _ => String::new(),
    };
    let name = pipe_name(&user_data);
    // An instance that is exiting can leave a moment where neither creating
    // nor connecting succeeds; try both a few times.
    for _ in 0..3 {
        // SAFETY: a null-terminated name and no security attributes.
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                MESSAGE_LIMIT as u32,
                0,
                core::ptr::null_mut(),
            )
        };
        if pipe as isize != INVALID_HANDLE {
            *held.borrow_mut() = Some((pipe as usize, name.clone()));
            listen(realm, app.clone(), held.clone(), pipe as usize);
            return Ok(Value::Boolean(true));
        }
        let message = message(realm, args)?;
        if send(&name, message.as_bytes()) {
            return Ok(Value::Boolean(false));
        }
    }
    // Neither owner nor client: run as an independent instance rather than
    // refusing to start.
    Ok(Value::Boolean(true))
}

/// The second instance's message: its command line, working directory and
/// additional data, as JSON.
fn message(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    let process = crate::process_builtin::process(realm)?;
    let argv = ops::get_named(realm, &process, "argv")?;
    let cwd = crate::process_builtin::cwd(realm);
    let cwd = ops::call(realm, &cwd, Value::Undefined, &[])?;
    let payload = realm.object();
    ops::define_data(&payload, Key::name("argv"), argv);
    ops::define_data(&payload, Key::name("cwd"), cwd);
    ops::define_data(
        &payload,
        Key::name("data"),
        args.first().cloned().unwrap_or(Value::Undefined),
    );
    let text = crate::json_builtin::stringify(realm, Value::Undefined, &[Value::Object(payload)])?;
    Ok(match text {
        Value::String(text) => text.to_rust_lossy(),
        _ => String::from("{}"),
    })
}

/// Writes a message to the instance that owns the pipe and lets it bring
/// its window to the front. Returns whether the owner received it.
fn send(name: &[u16], message: &[u8]) -> bool {
    // SAFETY: a null-terminated name; the handle is closed below.
    unsafe {
        if WaitNamedPipeW(name.as_ptr(), 2000) == 0 {
            return false;
        }
        let pipe = CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        );
        if pipe as isize == INVALID_HANDLE {
            return false;
        }
        let mut owner = 0;
        if GetNamedPipeServerProcessId(pipe, &mut owner) != 0 {
            AllowSetForegroundWindow(owner);
        }
        let mut sent = 0;
        let mut complete = true;
        while sent < message.len() {
            let mut written = 0;
            let chunk = (message.len() - sent).min(65_536) as u32;
            if WriteFile(
                pipe,
                message[sent..].as_ptr(),
                chunk,
                &mut written,
                core::ptr::null_mut(),
            ) == 0
                || written == 0
            {
                complete = false;
                break;
            }
            sent += written as usize;
        }
        CloseHandle(pipe);
        complete
    }
}

/// Waits on the pool for the next instance's message, reports it and waits
/// again. The wait does not keep the event loop alive.
fn listen(realm: &mut Realm, app: Object, held: Held, pipe: usize) {
    // Without a pool thread the instance still runs; it just hears from no
    // later instance.
    let _ = realm.submit_io_with_ref(
        move || receive(pipe),
        Rc::new(Cell::new(false)),
        move |realm, received| {
            if held.borrow().as_ref().map(|(owned, _)| *owned) != Some(pipe) {
                // Released: the wake-up connection ended the wait.
                // SAFETY: the pipe was created by this instance and nothing
                // uses it any more.
                unsafe { CloseHandle(pipe as Handle) };
                return Ok(());
            }
            let Some(bytes) = received else {
                return Ok(());
            };
            listen(realm, app.clone(), held, pipe);
            second_instance(realm, &app, &bytes)
        },
    );
}

/// One instance's message, or `None` once the pipe fails.
fn receive(pipe: usize) -> Option<Vec<u8>> {
    let pipe = pipe as Handle;
    // SAFETY: the pipe stays open while this instance holds the lock; once
    // released, these calls fail and the listener stops.
    unsafe {
        // A client may have connected, or even written and gone, before
        // the wait began; what it wrote is still there to read.
        if ConnectNamedPipe(pipe, core::ptr::null_mut()) == 0
            && !matches!(GetLastError(), ERROR_PIPE_CONNECTED | ERROR_NO_DATA)
        {
            return None;
        }
        let mut message = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut read = 0;
            let ok = ReadFile(
                pipe,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut read,
                core::ptr::null_mut(),
            );
            if ok == 0 || read == 0 {
                break;
            }
            if message.len() < MESSAGE_LIMIT {
                message.extend_from_slice(&buffer[..read as usize]);
            }
        }
        DisconnectNamedPipe(pipe);
        Some(message)
    }
}

/// Emits `second-instance` with the message's command line, working
/// directory and additional data.
fn second_instance(realm: &mut Realm, app: &Object, bytes: &[u8]) -> Result<(), Thrown> {
    let text = String::from_utf8_lossy(bytes);
    let parsed = crate::json_builtin::parse(realm, Value::Undefined, &[Value::str(&text)]);
    let Ok(Value::Object(payload)) = parsed else {
        return Ok(());
    };
    let payload = Value::Object(payload);
    let argv = ops::get_named(realm, &payload, "argv")?;
    let cwd = ops::get_named(realm, &payload, "cwd")?;
    let data = ops::get_named(realm, &payload, "data")?;
    let argv = match argv {
        Value::Object(_) => argv,
        _ => Value::Object(realm.array(Vec::new())),
    };
    let data = if matches!(data, Value::Undefined) {
        Value::Null
    } else {
        data
    };
    let event = Value::Object(realm.object());
    crate::electron_window::emit(realm, app, "second-instance", &[event, argv, cwd, data]).map(drop)
}

#[cfg(test)]
mod tests;
