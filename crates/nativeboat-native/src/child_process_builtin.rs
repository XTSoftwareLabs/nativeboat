//! Synchronous Windows child-process operations used by Node's
//! `child_process` module. The generated application launches processes with
//! Win32 directly; it does not use the host C runtime or a JavaScript engine.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

use crate::prelude::*;
use crate::{Completion, ErrorKind, Key, Object, Property, Realm, Slot, Value, ops, platform};

type Handle = *mut c_void;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count: u32,
    y_count: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_data: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        attrs: *mut SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attrs: *mut c_void,
        thread_attrs: *mut c_void,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        cwd: *const u16,
        startup: *mut StartupInfo,
        info: *mut ProcessInformation,
    ) -> i32;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(
        file: Handle,
        data: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn PeekNamedPipe(
        pipe: Handle,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        left: *mut u32,
    ) -> i32;
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        size: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

const STARTF_USESTDHANDLES: u32 = 0x100;
const HANDLE_FLAG_INHERIT: u32 = 1;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const WAIT_TIMEOUT: u32 = 258;
const STD_INPUT_HANDLE: u32 = -10i32 as u32;

#[derive(Default)]
struct Captured {
    pid: u32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: u32,
}

struct ChildHandles {
    process: Handle,
    thread: Handle,
    stdout: Handle,
    stderr: Handle,
}

struct ChildChunk {
    handles: ChildHandles,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    status: Option<u32>,
}

#[derive(Clone)]
struct CaptureEvents {
    pid: u32,
    child: Object,
    stdout: Object,
    stderr: Object,
    stdin: Rc<Cell<Handle>>,
    stdout_pending: Rc<RefCell<Vec<u8>>>,
    stderr_pending: Rc<RefCell<Vec<u8>>>,
    /// Whether the child keeps the event loop alive: `ref()` and `unref()`.
    referenced: Rc<Cell<bool>>,
}

// SAFETY: CreateProcess handles can be consumed by the Windows thread-pool
// callback; ownership moves there exactly once.
unsafe impl Send for ChildHandles {}

#[derive(Clone)]
struct Listener {
    event: String,
    callback: Value,
    once: bool,
}

/// Creates the native `child_process` subset used by MarkText.
pub fn module(realm: &Realm) -> Object {
    let exports = realm.object();
    for (name, shell) in [("execFileSync", false), ("execSync", true)] {
        let function = realm.function(name, 3, false, move |realm, _, args| {
            let command = string_arg(realm, args, 0)?;
            let argv = if shell {
                Vec::new()
            } else {
                string_array(realm, args, 1)?
            };
            let option_index = if shell { 1 } else { 2 };
            let options = args.get(option_index).and_then(Value::as_object).cloned();
            let shell_mode = shell || option_bool(&options, "shell");
            let command = if shell_mode {
                let command_line = if shell {
                    command
                } else {
                    core::iter::once(command.as_str())
                        .chain(argv.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                run("cmd.exe", &[command_line], &options, true)
            } else {
                run(&command, &argv, &options, false)
            };
            match command {
                Ok(captured) if captured.status == 0 => {
                    Ok(Value::str(&String::from_utf8_lossy(&captured.stdout)))
                }
                Ok(captured) => Err(realm.throw(
                    ErrorKind::Error,
                    &format!(
                        "Command failed: exit code {}\n{}",
                        captured.status,
                        String::from_utf8_lossy(&captured.stderr)
                    ),
                )),
                Err(code) => Err(realm.throw(
                    ErrorKind::Error,
                    &format!("Could not start child process (Windows error {code})"),
                )),
            }
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let spawn_sync = realm.function("spawnSync", 3, false, spawn_sync);
    exports.define(
        Key::name("spawnSync"),
        Property::data(Value::Object(spawn_sync)),
    );
    let spawn = realm.function("spawn", 3, false, spawn);
    let spawn_value = Value::Object(spawn.clone());
    exports.define(Key::name("spawn"), Property::data(Value::Object(spawn)));
    let exec = realm.function("exec", 3, false, {
        let spawn = spawn_value.clone();
        move |realm, _, args| exec_async(realm, &spawn, args, false)
    });
    promisified(realm, &exec, &spawn_value, false);
    exports.define(Key::name("exec"), Property::data(Value::Object(exec)));
    let exec_file = realm.function("execFile", 4, false, {
        let spawn = spawn_value.clone();
        move |realm, _, args| exec_async(realm, &spawn, args, true)
    });
    promisified(realm, &exec_file, &spawn_value, true);
    exports.define(
        Key::name("execFile"),
        Property::data(Value::Object(exec_file)),
    );
    // fork(modulePath[, args][, options]): the program's own executable,
    // running the module as the compiler linked it, with Node's IPC
    // channel. The Node library's utility process module implements it;
    // the compiler links that module when a program imports `fork`.
    let fork = realm.function("fork", 3, false, |realm, _, args| {
        let module = crate::modules::load(realm, "node:internal/electron/utility_process")?;
        let fork = ops::get_named(realm, &module, "fork")?;
        ops::call(realm, &fork, Value::Undefined, args)
    });
    exports.define(Key::name("fork"), Property::data(Value::Object(fork)));
    exports
}

/// `util.promisify` of `exec` and `execFile`, as Node defines it: a promise
/// of `{ stdout, stderr }`, rejected with the command's error, whose `child`
/// is the process.
fn promisified(realm: &Realm, function: &Object, spawn: &Value, exec_file: bool) {
    let spawn = spawn.clone();
    let custom = realm.function("promisified", 0, false, move |realm, _, args| {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        let done = realm.function("", 3, false, move |realm, _, args| {
            let error = args.first().cloned().unwrap_or(Value::Undefined);
            if !matches!(error, Value::Null | Value::Undefined) {
                return ops::call(realm, &reject, Value::Undefined, &[error]);
            }
            let result = realm.object();
            for (index, name) in [(1, "stdout"), (2, "stderr")] {
                let output = args.get(index).cloned().unwrap_or(Value::Undefined);
                result.define(Key::name(name), Property::data(output));
            }
            ops::call(realm, &resolve, Value::Undefined, &[Value::Object(result)])
        });
        let mut arguments = args.to_vec();
        arguments.push(Value::Object(done));
        let child = exec_async(realm, &spawn, &arguments, exec_file)?;
        if let Value::Object(object) = &promise {
            object.define(Key::name("child"), Property::data(child));
        }
        Ok(promise)
    });
    let key = Key::Symbol(crate::symbol_builtin::intern(
        realm,
        "nodejs.util.promisify.custom".into(),
    ));
    function.define(key, Property::hidden(Value::Object(custom)));
}

fn exec_async(realm: &mut Realm, spawn: &Value, args: &[Value], exec_file: bool) -> Completion {
    let command = string_arg(realm, args, 0)?;
    let mut argv = Vec::new();
    let mut options = None;
    let mut callback = None;
    if exec_file {
        let mut next = 1;
        if let Some(value) = args.get(1) {
            match value {
                Value::Object(object) if object.is_array() => {
                    argv = string_array(realm, args, 1)?;
                    next = 2;
                }
                value if crate::array_builtin::is_callable(value) => {
                    callback = Some(value.clone());
                    next = 2;
                }
                Value::Object(object) => {
                    options = Some(object.clone());
                    next = 2;
                }
                Value::Null | Value::Undefined => next = 2,
                _ => {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "The args argument must be an array or an options object",
                    ));
                }
            }
        }
        for value in args.iter().skip(next) {
            if crate::array_builtin::is_callable(value) {
                callback = Some(value.clone());
            } else if let Value::Object(object) = value
                && !object.is_array()
            {
                options = Some(object.clone());
            } else if !matches!(value, Value::Null | Value::Undefined) {
                return Err(realm.throw(ErrorKind::Type, "The options argument must be an object"));
            }
        }
    } else {
        for value in args.iter().skip(1) {
            if crate::array_builtin::is_callable(value) {
                callback = Some(value.clone());
            } else if let Value::Object(object) = value {
                options = Some(object.clone());
            } else if !matches!(value, Value::Null | Value::Undefined) {
                return Err(realm.throw(ErrorKind::Type, "The options argument must be an object"));
            }
        }
    }
    let options_value = if exec_file {
        options.map(Value::Object).unwrap_or(Value::Undefined)
    } else {
        let shell_options = realm.object();
        shell_options.define(Key::name("shell"), Property::data(Value::Boolean(true)));
        if let Some(cwd) = option_string(&options, "cwd") {
            shell_options.define(Key::name("cwd"), Property::data(Value::str(&cwd)));
        }
        Value::Object(shell_options)
    };
    // Errors name the command as Node does: the file and its arguments.
    let display = if exec_file && !argv.is_empty() {
        format!("{command} {}", argv.join(" "))
    } else {
        command.clone()
    };
    let argv = Value::Object(realm.array(argv.into_iter().map(|arg| Value::str(&arg)).collect()));
    let child = ops::call(
        realm,
        spawn,
        Value::Undefined,
        &[Value::str(&command), argv, options_value],
    )?;
    let Some(callback) = callback else {
        return Ok(child);
    };
    let Value::Object(child_object) = &child else {
        return Ok(child);
    };
    let stdout = ops::get_named(realm, &child, "stdout")?;
    let stderr = ops::get_named(realm, &child, "stderr")?;
    let stdout_text = Rc::new(RefCell::new(String::new()));
    let stderr_text = Rc::new(RefCell::new(String::new()));
    for stream in [&stdout, &stderr] {
        if stream.as_object().is_some() {
            let set_encoding = ops::get_named(realm, stream, "setEncoding")?;
            ops::call(realm, &set_encoding, stream.clone(), &[Value::str("utf8")])?;
        }
    }
    if let Some(stdout_stream) = stdout.as_object() {
        let output = stdout_text.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(chunk)) = args.first() {
                output.borrow_mut().push_str(&chunk.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        add_listener(realm, stdout_stream, "data", &data)?;
    }
    if let Some(stderr_stream) = stderr.as_object() {
        let output = stderr_text.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(chunk)) = args.first() {
                output.borrow_mut().push_str(&chunk.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        add_listener(realm, stderr_stream, "data", &data)?;
    }
    let completed = Rc::new(Cell::new(false));
    let close_completed = completed.clone();
    let close_callback = callback.clone();
    let close_stdout = stdout_text.clone();
    let close_stderr = stderr_text.clone();
    let close = realm.function("close", 2, false, move |realm, _, args| {
        if close_completed.replace(true) {
            return Ok(Value::Undefined);
        }
        let status = args.first().and_then(|value| match value {
            Value::Number(value) => Some(*value),
            _ => None,
        });
        let stdout = Value::str(&close_stdout.borrow());
        let stderr = Value::str(&close_stderr.borrow());
        let error = if status == Some(0.0) {
            Value::Null
        } else {
            let error = realm.error(
                ErrorKind::Error,
                &format!("Command failed: {display}\n{}", close_stderr.borrow()),
            );
            error.define(
                Key::name("code"),
                Property::data(Value::Number(status.unwrap_or(-1.0))),
            );
            error.define(Key::name("cmd"), Property::data(Value::str(&display)));
            error.define(Key::name("stdout"), Property::data(stdout.clone()));
            error.define(Key::name("stderr"), Property::data(stderr.clone()));
            Value::Object(error)
        };
        ops::call(
            realm,
            &close_callback,
            Value::Undefined,
            &[error, stdout, stderr],
        )
    });
    add_listener(realm, child_object, "close", &close)?;

    let error_completed = completed;
    let error_callback = callback;
    let error_stdout = stdout_text;
    let error_stderr = stderr_text;
    let on_error = realm.function("error", 1, false, move |realm, _, args| {
        if error_completed.replace(true) {
            return Ok(Value::Undefined);
        }
        let error = args.first().cloned().unwrap_or(Value::Undefined);
        ops::call(
            realm,
            &error_callback,
            Value::Undefined,
            &[
                error,
                Value::str(&error_stdout.borrow()),
                Value::str(&error_stderr.borrow()),
            ],
        )
    });
    add_listener(realm, child_object, "error", &on_error)?;
    Ok(child)
}

fn add_listener(realm: &mut Realm, emitter: &Object, event: &str, callback: &Object) -> Completion {
    let on = ops::get_named(realm, &Value::Object(emitter.clone()), "on")?;
    ops::call(
        realm,
        &on,
        Value::Object(emitter.clone()),
        &[Value::str(event), Value::Object(callback.clone())],
    )
}

fn emitter(realm: &Realm) -> Object {
    let object = realm.object();
    let listeners = Rc::new(RefCell::new(Vec::<Listener>::new()));
    for (method, once) in [("on", false), ("addListener", false), ("once", true)] {
        let listeners = listeners.clone();
        let function = realm.function(method, 2, false, move |realm, this, args| {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            let event = ops::to_string(realm, &event)?.to_rust_lossy();
            let Some(callback) = args.get(1).cloned() else {
                return Err(
                    realm.throw(ErrorKind::Type, "The listener argument must be a function")
                );
            };
            if !crate::array_builtin::is_callable(&callback) {
                return Err(
                    realm.throw(ErrorKind::Type, "The listener argument must be a function")
                );
            }
            listeners.borrow_mut().push(Listener {
                event,
                callback,
                once,
            });
            Ok(this)
        });
        object.define(Key::name(method), Property::data(Value::Object(function)));
    }
    for method in ["removeListener", "off"] {
        let listeners = listeners.clone();
        let function = realm.function(method, 2, false, move |realm, this, args| {
            let event = args.first().cloned().unwrap_or(Value::Undefined);
            let event = ops::to_string(realm, &event)?.to_rust_lossy();
            if let Some(callback) = args.get(1) {
                listeners.borrow_mut().retain(|listener| {
                    listener.event != event || !crate::same_value(&listener.callback, callback)
                });
            }
            Ok(this)
        });
        object.define(Key::name(method), Property::data(Value::Object(function)));
    }
    let for_emit = listeners.clone();
    let emit = realm.function("emit", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        let event = ops::to_string(realm, &event)?.to_rust_lossy();
        let callbacks = {
            let mut listeners = for_emit.borrow_mut();
            let callbacks = listeners
                .iter()
                .filter(|listener| listener.event == event)
                .map(|listener| listener.callback.clone())
                .collect::<Vec<_>>();
            listeners.retain(|listener| listener.event != event || !listener.once);
            callbacks
        };
        if event == "error" && callbacks.is_empty() {
            let error = args.get(1).cloned().unwrap_or(Value::Undefined);
            let message = ops::get_named(realm, &error, "message")
                .ok()
                .map(|message| ops::to_string(realm, &message).map(|value| value.to_rust_lossy()))
                .transpose()?
                .unwrap_or_else(|| "Unhandled child process error".to_owned());
            return Err(realm.throw(ErrorKind::Error, &message));
        }
        for callback in &callbacks {
            ops::call(
                realm,
                callback,
                Value::Undefined,
                args.get(1..).unwrap_or_default(),
            )?;
        }
        Ok(Value::Boolean(!callbacks.is_empty()))
    });
    object.define(Key::name("emit"), Property::data(Value::Object(emit)));
    let count_listeners = listeners;
    let count = realm.function("listenerCount", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        let event = ops::to_string(realm, &event)?.to_rust_lossy();
        let count = count_listeners
            .borrow()
            .iter()
            .filter(|listener| listener.event == event)
            .count();
        Ok(Value::Number(count as f64))
    });
    object.define(
        Key::name("listenerCount"),
        Property::data(Value::Object(count)),
    );
    object
}

fn emit_event(realm: &mut Realm, emitter: &Object, event: &str, args: &[Value]) -> Completion {
    let emit = ops::get_named(realm, &Value::Object(emitter.clone()), "emit")?;
    let mut values = Vec::with_capacity(args.len() + 1);
    values.push(Value::str(event));
    values.extend_from_slice(args);
    ops::call(realm, &emit, Value::Object(emitter.clone()), &values)
}

fn stream(realm: &Realm, input: bool, input_pipe: Option<Rc<Cell<Handle>>>) -> Object {
    let stream = emitter(realm);
    // Data arrives as Buffers until `setEncoding` names an encoding, as
    // Node's readable streams give it; `readableEncoding` says which.
    stream.define(Key::name("readableEncoding"), Property::data(Value::Null));
    let set_encoding = realm.function("setEncoding", 1, false, |realm, this, args| {
        let encoding = match args.first() {
            None | Some(Value::Undefined | Value::Null) => Value::str("utf8"),
            Some(value) => Value::String(ops::to_string(realm, value)?),
        };
        ops::set(realm, &this, &Key::name("readableEncoding"), encoding)?;
        Ok(this)
    });
    stream.define(
        Key::name("setEncoding"),
        Property::data(Value::Object(set_encoding)),
    );
    if input && let Some(pipe) = input_pipe {
        let write_pipe = pipe.clone();
        let write = realm.function("write", 3, false, move |realm, _, args| {
            let value = args.first().cloned().unwrap_or(Value::Undefined);
            let bytes = input_bytes(realm, &value)?;
            Ok(Value::Boolean(write_input(write_pipe.get(), &bytes)))
        });
        stream.define(Key::name("write"), Property::data(Value::Object(write)));
        let end_pipe = pipe.clone();
        let end = realm.function("end", 3, false, move |realm, this, args| {
            if let Some(value) = args.first() {
                let bytes = input_bytes(realm, value)?;
                let _ = write_input(end_pipe.get(), &bytes);
            }
            close_input(&end_pipe);
            Ok(this)
        });
        stream.define(Key::name("end"), Property::data(Value::Object(end)));
    }
    stream
}

/// The bytes a child's input is given: a Buffer's or typed array's own,
/// else the UTF-8 of the value as a string.
fn input_bytes(realm: &mut Realm, value: &Value) -> Result<Vec<u8>, crate::Thrown> {
    if let Some(bytes) = crate::typed_array_builtin::view_bytes(value) {
        return Ok(bytes);
    }
    Ok(ops::to_string(realm, value)?.to_rust_lossy().into_bytes())
}

fn write_input(pipe: Handle, mut bytes: &[u8]) -> bool {
    if pipe.is_null() {
        return false;
    }
    while !bytes.is_empty() {
        let mut written = 0;
        // SAFETY: bytes remains live and writable count is an output pointer.
        if unsafe {
            WriteFile(
                pipe,
                bytes.as_ptr().cast_mut(),
                bytes.len().min(u32::MAX as usize) as u32,
                &mut written,
                core::ptr::null_mut(),
            )
        } == 0
            || written == 0
        {
            return false;
        }
        bytes = &bytes[written as usize..];
    }
    true
}

fn close_input(pipe: &Rc<Cell<Handle>>) {
    let handle = pipe.replace(core::ptr::null_mut());
    if !handle.is_null() {
        // SAFETY: close_input swaps away and closes the sole owned parent pipe handle.
        unsafe { CloseHandle(handle) };
    }
}

/// `spawnSync`, as Node returns it: the process's `pid`, `output`, `stdout`
/// and `stderr`, as `Buffer`s or in the `encoding` the options name, and its
/// `status`; a process that cannot start gives its `error` instead of
/// throwing.
fn spawn_sync(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let command = string_arg(realm, args, 0)?;
    // The arguments are optional: `spawnSync(command, options)`.
    let (argv, options) = match args.get(1) {
        Some(Value::Object(object)) if !object.is_array() => (Vec::new(), Some(object.clone())),
        _ => (
            string_array(realm, args, 1)?,
            args.get(2).and_then(Value::as_object).cloned(),
        ),
    };
    let result = if option_bool(&options, "shell") {
        let command_line = core::iter::once(command.as_str())
            .chain(argv.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        run("cmd.exe", &[command_line], &options, true)
    } else {
        run(&command, &argv, &options, false)
    };
    let text = option_string(&options, "encoding").filter(|encoding| encoding != "buffer");
    let object = realm.object();
    match result {
        Ok(captured) => {
            let mut output = |bytes: Vec<u8>| match text {
                Some(_) => Ok(Value::str(&String::from_utf8_lossy(&bytes))),
                None => crate::electron_clipboard::buffer_value(realm, bytes),
            };
            let stdout = output(captured.stdout)?;
            let stderr = output(captured.stderr)?;
            let output = realm.array(vec![Value::Null, stdout.clone(), stderr.clone()]);
            for (name, value) in [
                ("pid", Value::Number(f64::from(captured.pid))),
                ("output", Value::Object(output)),
                ("stdout", stdout),
                ("stderr", stderr),
                ("status", Value::Number(f64::from(captured.status))),
                ("signal", Value::Null),
            ] {
                object.define(Key::name(name), Property::data(value));
            }
        }
        Err(code) => {
            let error = realm.error(
                ErrorKind::Error,
                &format!("Could not start child process (Windows error {code})"),
            );
            // Node names the failure as libuv maps Windows' errors.
            let name = match code {
                2 | 3 => "ENOENT",
                5 => "EACCES",
                _ => "UNKNOWN",
            };
            for (key, value) in [
                ("code", Value::str(name)),
                ("syscall", Value::str(&format!("spawnSync {command}"))),
                ("path", Value::str(&command)),
            ] {
                error.define(Key::name(key), Property::data(value));
            }
            for (name, value) in [
                ("error", Value::Object(error)),
                ("pid", Value::Number(0.0)),
                ("output", Value::Null),
                ("stdout", Value::Null),
                ("stderr", Value::Null),
                ("status", Value::Null),
                ("signal", Value::Null),
            ] {
                object.define(Key::name(name), Property::data(value));
            }
        }
    }
    Ok(Value::Object(object))
}

fn spawn(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let command = string_arg(realm, args, 0)?;
    let argv = string_array(realm, args, 1)?;
    let options = args.get(2).and_then(Value::as_object).cloned();
    let shell = option_bool(&options, "shell");
    let child = emitter(realm);
    let stdin_handle = Rc::new(Cell::new(core::ptr::null_mut()));
    let stdin = stream(realm, true, Some(stdin_handle.clone()));
    let stdout = stream(realm, false, None);
    let stderr = stream(realm, false, None);
    for (name, value) in [
        ("stdin", Value::Object(stdin.clone())),
        ("stdout", Value::Object(stdout.clone())),
        ("stderr", Value::Object(stderr.clone())),
    ] {
        child.define(Key::name(name), Property::data(value));
    }
    let killed = Rc::new(Cell::new(false));
    let killed_state = killed.clone();
    let referenced = Rc::new(Cell::new(true));
    for (name, state) in [("ref", true), ("unref", false)] {
        let referenced = referenced.clone();
        let method = realm.function(name, 0, false, move |_, _, _| {
            referenced.set(state);
            Ok(Value::Undefined)
        });
        child.define(Key::name(name), Property::data(Value::Object(method)));
    }
    match create_child(&command, &argv, &options, shell) {
        Ok((pid, handles, input)) => {
            stdin_handle.set(input);
            child.define(
                Key::name("pid"),
                Property::data(Value::Number(f64::from(pid))),
            );
            let kill = realm.function("kill", 1, false, move |_, _, args| {
                let signal = match args.first() {
                    None | Some(Value::Undefined) => 15,
                    Some(Value::Number(value)) if *value == 0.0 => 0,
                    Some(_) => 15,
                };
                let success = platform::kill(pid, signal).is_ok();
                if success && signal != 0 {
                    killed_state.set(true);
                }
                Ok(Value::Boolean(success))
            });
            child.define(Key::name("kill"), Property::data(Value::Object(kill)));

            let spawned_child = child.clone();
            let stdout_pending = Rc::new(RefCell::new(Vec::new()));
            let stderr_pending = Rc::new(RefCell::new(Vec::new()));
            submit_capture(
                realm,
                handles,
                CaptureEvents {
                    pid,
                    child: child.clone(),
                    stdout,
                    stderr,
                    stdin: stdin_handle.clone(),
                    stdout_pending,
                    stderr_pending,
                    referenced,
                },
            )
            .map_err(|error| {
                let _ = platform::kill(pid, 9);
                close_input(&stdin_handle);
                realm.throw(
                    ErrorKind::Error,
                    &format!("Could not queue child process I/O (Windows error {error})"),
                )
            })?;
            realm.enqueue(Box::new(move |realm| {
                emit_event(realm, &spawned_child, "spawn", &[])?;
                Ok(())
            }));
        }
        Err(error) => {
            child.define(Key::name("pid"), Property::data(Value::Undefined));
            child.define(
                Key::name("kill"),
                Property::data(Value::Object(realm.function(
                    "kill",
                    1,
                    false,
                    |_, _, _| Ok(Value::Boolean(false)),
                ))),
            );
            let event_child = child.clone();
            realm.enqueue(Box::new(move |realm| {
                let error = realm.error(
                    ErrorKind::Error,
                    &format!("Could not start child process (Windows error {error})"),
                );
                emit_event(realm, &event_child, "error", &[Value::Object(error)])?;
                emit_event(realm, &event_child, "close", &[Value::Null, Value::Null])?;
                Ok(())
            }));
        }
    }
    let killed_get = realm.function("get killed", 0, false, move |_, _, _| {
        Ok(Value::Boolean(killed.get()))
    });
    child.define(
        Key::name("killed"),
        Property {
            slot: Slot::Accessor {
                get: Some(killed_get),
                set: None,
            },
            enumerable: true,
            configurable: false,
        },
    );
    Ok(Value::Object(child))
}

fn create_child(
    command: &str,
    args: &[String],
    options: &Option<Object>,
    shell: bool,
) -> Result<(u32, ChildHandles, Handle), u32> {
    let mut security = SecurityAttributes {
        length: core::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: core::ptr::null_mut(),
        inherit: 1,
    };
    let (mut stdin_read, mut stdin_write) = (core::ptr::null_mut(), core::ptr::null_mut());
    let (mut stdout_read, mut stdout_write) = (core::ptr::null_mut(), core::ptr::null_mut());
    let (mut stderr_read, mut stderr_write) = (core::ptr::null_mut(), core::ptr::null_mut());
    // SAFETY: each pipe call writes valid output handles and reads the live security descriptor.
    if unsafe { CreatePipe(&mut stdin_read, &mut stdin_write, &mut security, 0) } == 0 {
        // SAFETY: retrieves this thread's last Win32 error immediately after failure.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: handles and security descriptor remain valid for each pipe creation.
    if unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, &mut security, 0) } == 0 {
        close_handles(&[stdin_read, stdin_write]);
        // SAFETY: retrieves this thread's last Win32 error immediately after failure.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: handles and security descriptor remain valid for each pipe creation.
    if unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, &mut security, 0) } == 0 {
        close_handles(&[stdin_read, stdin_write, stdout_read, stdout_write]);
        // SAFETY: retrieves this thread's last Win32 error immediately after failure.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: parent-owned ends must not be inherited; child ends remain inheritable.
    unsafe {
        SetHandleInformation(stdin_write, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(stdout_read, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(stderr_read, HANDLE_FLAG_INHERIT, 0);
    }
    // SAFETY: every field is an integer or pointer and zero is valid for initialization.
    let mut startup: StartupInfo = unsafe { core::mem::zeroed() };
    startup.cb = core::mem::size_of::<StartupInfo>() as u32;
    startup.flags = STARTF_USESTDHANDLES;
    startup.stdin = stdin_read;
    startup.stdout = stdout_write;
    startup.stderr = stderr_write;
    let command_line = if shell {
        let joined = core::iter::once(command)
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{} /d /s /c \"{}\"", quote("cmd.exe"), joined)
    } else {
        let mut text = quote(command);
        for arg in args {
            text.push(' ');
            text.push_str(&quote(arg));
        }
        text
    };
    let mut command_line: Vec<u16> = command_line
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let cwd: Vec<u16> = option_string(options, "cwd")
        .map(|value| value.encode_utf16().chain(core::iter::once(0)).collect())
        .unwrap_or_default();
    let mut environment = environment_block(options);
    // SAFETY: every field is a handle or integer and zero is valid for initialization.
    let mut info: ProcessInformation = unsafe { core::mem::zeroed() };
    // SAFETY: the mutable command line, cwd, startup structure and output structure remain live.
    let created = unsafe {
        CreateProcessW(
            core::ptr::null(),
            command_line.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment
                .as_mut()
                .map_or(core::ptr::null_mut(), |block| block.as_mut_ptr().cast()),
            if cwd.is_empty() {
                core::ptr::null()
            } else {
                cwd.as_ptr()
            },
            &mut startup,
            &mut info,
        )
    };
    close_handles(&[stdin_read, stdout_write, stderr_write]);
    if created == 0 {
        // SAFETY: retrieves the CreateProcessW error before any cleanup OS call.
        let error = unsafe { GetLastError() };
        close_handles(&[stdin_write, stdout_read, stderr_read]);
        return Err(error);
    }
    Ok((
        info.process_id,
        ChildHandles {
            process: info.process,
            thread: info.thread,
            stdout: stdout_read,
            stderr: stderr_read,
        },
        stdin_write,
    ))
}

fn close_handles(handles: &[Handle]) {
    for handle in handles.iter().copied().filter(|handle| !handle.is_null()) {
        // SAFETY: callers pass handles they own and have not yet closed.
        unsafe { CloseHandle(handle) };
    }
}

fn submit_capture(
    realm: &mut Realm,
    handles: ChildHandles,
    events: CaptureEvents,
) -> Result<(), u32> {
    let owned_handles = [
        handles.process,
        handles.thread,
        handles.stdout,
        handles.stderr,
    ];
    let referenced = events.referenced.clone();
    let result = realm.submit_io_with_ref(
        move || capture_child_chunk(handles),
        referenced,
        move |realm, result| {
            match result {
                Ok(chunk) => {
                    if let Some(bytes) = &chunk.stdout {
                        emit_stream_data(
                            realm,
                            &events.stdout,
                            &events.stdout_pending,
                            bytes,
                            false,
                        )?;
                    }
                    if let Some(bytes) = &chunk.stderr {
                        emit_stream_data(
                            realm,
                            &events.stderr,
                            &events.stderr_pending,
                            bytes,
                            false,
                        )?;
                    }
                    if let Some(status) = chunk.status {
                        close_input(&events.stdin);
                        emit_stream_data(realm, &events.stdout, &events.stdout_pending, &[], true)?;
                        emit_stream_data(realm, &events.stderr, &events.stderr_pending, &[], true)?;
                        emit_event(realm, &events.stdout, "end", &[])?;
                        emit_event(realm, &events.stderr, "end", &[])?;
                        let code = Value::Number(f64::from(status));
                        emit_event(realm, &events.child, "exit", &[code.clone(), Value::Null])?;
                        emit_event(realm, &events.child, "close", &[code, Value::Null])?;
                    } else if let Err(error) = submit_capture(realm, chunk.handles, events.clone())
                    {
                        let _ = platform::kill(events.pid, 9);
                        close_input(&events.stdin);
                        let error = realm.error(
                            ErrorKind::Error,
                            &format!("Could not queue child process I/O (Windows error {error})"),
                        );
                        emit_event(realm, &events.child, "error", &[Value::Object(error)])?;
                        emit_event(realm, &events.child, "close", &[Value::Null, Value::Null])?;
                    }
                }
                Err(error) => {
                    close_input(&events.stdin);
                    let error = realm.error(
                        ErrorKind::Error,
                        &format!("Child process failed (Windows error {error})"),
                    );
                    emit_event(realm, &events.child, "error", &[Value::Object(error)])?;
                    emit_event(realm, &events.child, "close", &[Value::Null, Value::Null])?;
                }
            }
            Ok(())
        },
    );
    if result.is_err() {
        // The pool rejected the callback, so its closure did not take ownership.
        close_handles(&owned_handles);
    }
    result
}

fn emit_stream_data(
    realm: &mut Realm,
    stream: &Object,
    pending: &Rc<RefCell<Vec<u8>>>,
    bytes: &[u8],
    final_chunk: bool,
) -> Completion {
    let encoding = ops::get_named(realm, &Value::Object(stream.clone()), "readableEncoding")?;
    if matches!(encoding, Value::Null | Value::Undefined) {
        if bytes.is_empty() {
            return Ok(Value::Undefined);
        }
        if realm.buffer_prototype.is_none()
            && let Some(require) = realm.node_library
        {
            let _ = require(realm, "buffer")?;
        }
        let prototype = realm.buffer_prototype.clone();
        let chunk = crate::typed_array_builtin::new_bytes(realm, bytes.to_vec(), prototype);
        emit_event(realm, stream, "data", &[Value::Object(chunk)])?;
        return Ok(Value::Undefined);
    }
    let text = {
        let mut pending = pending.borrow_mut();
        pending.extend_from_slice(bytes);
        match core::str::from_utf8(&pending) {
            Ok(text) => {
                let text = text.to_owned();
                pending.clear();
                Some(text)
            }
            Err(error) if error.error_len().is_none() && !final_chunk => {
                let valid = error.valid_up_to();
                let text = core::str::from_utf8(&pending[..valid])
                    .expect("valid_up_to identifies UTF-8")
                    .to_owned();
                let remainder = pending[valid..].to_vec();
                *pending = remainder;
                (!text.is_empty()).then_some(text)
            }
            Err(_) => {
                let text = String::from_utf8_lossy(&pending).into_owned();
                pending.clear();
                (!text.is_empty()).then_some(text)
            }
        }
    };
    if let Some(text) = text {
        emit_event(realm, stream, "data", &[Value::str(&text)])?;
    }
    Ok(Value::Undefined)
}

fn capture_child_chunk(mut handles: ChildHandles) -> Result<ChildChunk, u32> {
    loop {
        let stdout = match read_pipe_chunk(handles.stdout) {
            Ok(Some(bytes)) if !bytes.is_empty() => Some(bytes),
            Ok(Some(_)) => None,
            Ok(None) => {
                close_handles(&[handles.stdout]);
                handles.stdout = core::ptr::null_mut();
                None
            }
            Err(error) => return close_child_after_error(handles, error),
        };
        let stderr = match read_pipe_chunk(handles.stderr) {
            Ok(Some(bytes)) if !bytes.is_empty() => Some(bytes),
            Ok(Some(_)) => None,
            Ok(None) => {
                close_handles(&[handles.stderr]);
                handles.stderr = core::ptr::null_mut();
                None
            }
            Err(error) => return close_child_after_error(handles, error),
        };
        // SAFETY: this worker owns the valid process handle until it returns.
        let wait = unsafe { WaitForSingleObject(handles.process, 0) };
        if wait != 0 && wait != WAIT_TIMEOUT {
            // SAFETY: immediately retrieves the failed wait's thread-local error.
            let error = unsafe { GetLastError() };
            return close_child_after_error(handles, error);
        }
        if stdout.is_some() || stderr.is_some() {
            return Ok(ChildChunk {
                handles,
                stdout,
                stderr,
                status: None,
            });
        }
        if wait == 0 && handles.stdout.is_null() && handles.stderr.is_null() {
            let mut status = 0;
            // SAFETY: process handle and writable status pointer remain valid.
            if unsafe { GetExitCodeProcess(handles.process, &mut status) } == 0 {
                // SAFETY: immediately retrieves the failed query's thread-local error.
                let error = unsafe { GetLastError() };
                return close_child_after_error(handles, error);
            }
            close_handles(&[handles.thread, handles.process]);
            return Ok(ChildChunk {
                handles: ChildHandles {
                    thread: core::ptr::null_mut(),
                    process: core::ptr::null_mut(),
                    ..handles
                },
                stdout: None,
                stderr: None,
                status: Some(status),
            });
        }
        // SAFETY: this worker owns the valid process handle until it returns.
        if unsafe { WaitForSingleObject(handles.process, 10) } == u32::MAX {
            // SAFETY: immediately retrieves the failed wait's thread-local error.
            let error = unsafe { GetLastError() };
            return close_child_after_error(handles, error);
        }
    }
}

fn close_child_after_error(handles: ChildHandles, error: u32) -> Result<ChildChunk, u32> {
    close_handles(&[
        handles.thread,
        handles.process,
        handles.stdout,
        handles.stderr,
    ]);
    Err(error)
}

fn read_pipe_chunk(pipe: Handle) -> Result<Option<Vec<u8>>, u32> {
    if pipe.is_null() {
        return Ok(None);
    }
    let mut available = 0;
    // SAFETY: the pipe is valid and the output count pointer remains live.
    if unsafe {
        PeekNamedPipe(
            pipe,
            core::ptr::null_mut(),
            0,
            core::ptr::null_mut(),
            &mut available,
            core::ptr::null_mut(),
        )
    } == 0
    {
        // SAFETY: immediately retrieves the failed pipe query's thread-local error.
        let error = unsafe { GetLastError() };
        return if error == 109 { Ok(None) } else { Err(error) };
    }
    if available == 0 {
        return Ok(Some(Vec::new()));
    }
    let mut buffer = vec![0; available.min(65_536) as usize];
    let mut read = 0;
    // SAFETY: buffer and count pointer remain writable for the requested size.
    if unsafe {
        ReadFile(
            pipe,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut read,
            core::ptr::null_mut(),
        )
    } == 0
    {
        // SAFETY: immediately retrieves the failed read's thread-local error.
        let error = unsafe { GetLastError() };
        return if error == 109 { Ok(None) } else { Err(error) };
    }
    buffer.truncate(read as usize);
    Ok(Some(buffer))
}

fn string_arg(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, crate::Thrown> {
    let value = args.get(index).cloned().unwrap_or(Value::Undefined);
    if value.is_nullish() {
        return Err(realm.throw(ErrorKind::Type, "The command argument must be a string"));
    }
    Ok(ops::to_string(realm, &value)?.to_rust_lossy())
}

fn string_array(
    realm: &mut Realm,
    args: &[Value],
    index: usize,
) -> Result<Vec<String>, crate::Thrown> {
    let Some(value) = args.get(index) else {
        return Ok(Vec::new());
    };
    if matches!(value, Value::Undefined) {
        return Ok(Vec::new());
    }
    let Value::Object(array) = value else {
        return Err(realm.throw(ErrorKind::Type, "The args argument must be an array"));
    };
    let length = ops::get_named(realm, value, "length")?;
    let length = ops::to_number(realm, &length)?.clamp(0.0, 65_536.0) as usize;
    (0..length)
        .map(|index| {
            let value = ops::get(
                realm,
                &Value::Object(array.clone()),
                &Key::Index(index as u32),
            )?;
            if value.is_nullish() {
                return Err(
                    realm.throw(ErrorKind::Type, "The args array must contain only strings")
                );
            }
            Ok(ops::to_string(realm, &value)?.to_rust_lossy())
        })
        .collect()
}

fn option_string(options: &Option<Object>, name: &str) -> Option<String> {
    let options = options.as_ref()?;
    let property = options.own_property(&Key::name(name))?;
    match property.slot {
        crate::Slot::Data {
            value: Value::String(value),
            ..
        } => Some(value.to_rust_lossy()),
        _ => None,
    }
}

/// The `env` option as a Windows environment block: `name=value` strings
/// of the object's own string-keyed data properties, sorted by name without
/// case as Windows expects, each ended by a nul and the block by another.
/// Values convert to strings as Node converts them; `undefined` ones are
/// left out. No `env` inherits the parent's environment.
fn environment_block(options: &Option<Object>) -> Option<Vec<u16>> {
    let env = match options.as_ref()?.own_property(&Key::name("env"))?.slot {
        crate::Slot::Data {
            value: Value::Object(env),
            ..
        } => env,
        _ => return None,
    };
    let mut variables: Vec<(String, String)> = Vec::new();
    for key in env.own_keys() {
        let name = match &key {
            Key::String(name) => name.to_rust_lossy(),
            Key::Index(index) => alloc::format!("{index}"),
            Key::Symbol(_) => continue,
        };
        let Some(property) = env.own_property(&key) else {
            continue;
        };
        if !property.enumerable {
            continue;
        }
        let value = match property.slot {
            crate::Slot::Data { value, .. } => match value {
                Value::String(value) => value.to_rust_lossy(),
                Value::Number(value) => crate::number::to_string(value),
                Value::Boolean(value) => String::from(if value { "true" } else { "false" }),
                Value::Null => String::from("null"),
                _ => continue,
            },
            _ => continue,
        };
        if name.is_empty() || name.chars().skip(1).any(|c| c == '=') {
            continue;
        }
        variables.push((name, value));
    }
    variables.sort_by_key(|(name, _)| name.to_ascii_uppercase());
    let mut block = Vec::new();
    for (name, value) in variables {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Some(block)
}

fn option_bool(options: &Option<Object>, name: &str) -> bool {
    options
        .as_ref()
        .and_then(|options| options.own_property(&Key::name(name)))
        .is_some_and(|property| {
            matches!(
                property.slot,
                crate::Slot::Data {
                    value: Value::Boolean(true),
                    ..
                }
            )
        })
}

fn run(
    command: &str,
    args: &[String],
    options: &Option<Object>,
    shell: bool,
) -> Result<Captured, u32> {
    let mut stdout_read = core::ptr::null_mut();
    let mut stdout_write = core::ptr::null_mut();
    let mut stderr_read = core::ptr::null_mut();
    let mut stderr_write = core::ptr::null_mut();
    let mut security = SecurityAttributes {
        length: core::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: core::ptr::null_mut(),
        inherit: 1,
    };
    // SAFETY: the output pointers and security descriptor remain live through process creation.
    if unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, &mut security, 0) } == 0 {
        // SAFETY: retrieves the failed CreatePipe result before another OS call.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: same contract as the first pipe; close the first pair on failure.
    if unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, &mut security, 0) } == 0 {
        // SAFETY: both handles were returned by the first successful CreatePipe.
        unsafe {
            CloseHandle(stdout_read);
            CloseHandle(stdout_write);
        }
        // SAFETY: retrieves the failed CreatePipe result before cleanup changes it.
        return Err(unsafe { GetLastError() });
    }
    // Parent read ends must not be inherited by the child.
    // SAFETY: both valid pipe handles are parent-owned and accept handle flags.
    unsafe {
        SetHandleInformation(stdout_read, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(stderr_read, HANDLE_FLAG_INHERIT, 0);
    }
    // SAFETY: every field is an integer or pointer, for which zero is valid.
    let mut startup: StartupInfo = unsafe { core::mem::zeroed() };
    startup.cb = core::mem::size_of::<StartupInfo>() as u32;
    startup.flags = STARTF_USESTDHANDLES;
    // SAFETY: GetStdHandle has no pointer preconditions.
    startup.stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    startup.stdout = stdout_write;
    startup.stderr = stderr_write;
    let mut command_line = quote(command);
    if shell {
        command_line.push_str(" /d /s /c \"");
        if let Some(command) = args.first() {
            command_line.push_str(command);
        }
        command_line.push('"');
    } else {
        for arg in args {
            command_line.push(' ');
            command_line.push_str(&quote(arg));
        }
    }
    let mut command_line: Vec<u16> = command_line
        .encode_utf16()
        .chain(core::iter::once(0))
        .collect();
    let cwd: Vec<u16> = option_string(options, "cwd")
        .map(|s| s.encode_utf16().chain(core::iter::once(0)).collect())
        .unwrap_or_default();
    let mut environment = environment_block(options);
    // SAFETY: every field is a handle or integer, for which zero is valid.
    let mut info: ProcessInformation = unsafe { core::mem::zeroed() };
    // SAFETY: mutable, nul-terminated command line and startup handles are valid for the call.
    let created = unsafe {
        CreateProcessW(
            core::ptr::null(),
            command_line.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment
                .as_mut()
                .map_or(core::ptr::null_mut(), |block| block.as_mut_ptr().cast()),
            if cwd.is_empty() {
                core::ptr::null()
            } else {
                cwd.as_ptr()
            },
            &mut startup,
            &mut info,
        )
    };
    // SAFETY: the parent owns these two pipe write handles after CreatePipe.
    unsafe {
        CloseHandle(stdout_write);
        CloseHandle(stderr_write);
    }
    if created == 0 {
        // SAFETY: retrieves the CreateProcessW failure before closing pipe handles.
        let error = unsafe { GetLastError() };
        // SAFETY: both read handles were returned by successful CreatePipe calls.
        unsafe {
            CloseHandle(stdout_read);
            CloseHandle(stderr_read);
        }
        return Err(error);
    }
    let result = capture(info.process, stdout_read, stderr_read).map(|captured| Captured {
        pid: info.process_id,
        ..captured
    });
    // SAFETY: ProcessInformation owns these handles and capture has finished.
    unsafe {
        CloseHandle(info.thread);
        CloseHandle(info.process);
        CloseHandle(stdout_read);
        CloseHandle(stderr_read);
    }
    result
}

fn capture(process: Handle, stdout: Handle, stderr: Handle) -> Result<Captured, u32> {
    let mut result = Captured::default();
    let mut exited = false;
    loop {
        drain(stdout, &mut result.stdout)?;
        drain(stderr, &mut result.stderr)?;
        if exited {
            break;
        }
        // SAFETY: capture holds the valid process handle until it returns.
        match unsafe { WaitForSingleObject(process, 10) } {
            0 => exited = true,
            WAIT_TIMEOUT => {}
            // SAFETY: immediately reads the failed wait's thread-local error.
            _ => return Err(unsafe { GetLastError() }),
        }
    }
    // Drain bytes written between the last poll and process exit.
    drain(stdout, &mut result.stdout)?;
    drain(stderr, &mut result.stderr)?;
    // SAFETY: the process handle and output code pointer remain valid.
    if unsafe { GetExitCodeProcess(process, &mut result.status) } == 0 {
        // SAFETY: immediately reads the failed GetExitCodeProcess error.
        return Err(unsafe { GetLastError() });
    }
    Ok(result)
}

fn drain(pipe: Handle, output: &mut Vec<u8>) -> Result<(), u32> {
    loop {
        let mut available = 0;
        // SAFETY: only the byte count output is requested; the pipe stays open while polled.
        if unsafe {
            PeekNamedPipe(
                pipe,
                core::ptr::null_mut(),
                0,
                core::ptr::null_mut(),
                &mut available,
                core::ptr::null_mut(),
            )
        } == 0
        {
            return Ok(()); // A child closing its writer produces a broken pipe.
        }
        if available == 0 {
            return Ok(());
        }
        let mut buffer = vec![0; available.min(65_536) as usize];
        let mut read = 0;
        // SAFETY: buffer is writable for its reported size.
        if unsafe {
            ReadFile(
                pipe,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut read,
                core::ptr::null_mut(),
            )
        } == 0
        {
            return Ok(());
        }
        output.extend_from_slice(&buffer[..read as usize]);
    }
}

/// An argument on a command line as libuv quotes it: bare unless it is
/// empty or holds spaces, tabs or quotes; quoted, it follows the rules of the
/// Windows C runtime and CommandLineToArgvW, where backslashes before quotes
/// and a closing quote double.
fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.into();
    }
    let mut result = String::from("\"");
    let mut slashes = 0;
    for character in argument.chars() {
        if character == '\\' {
            slashes += 1;
            continue;
        }
        if character == '"' {
            result.extend(core::iter::repeat_n('\\', slashes * 2 + 1));
            result.push('"');
        } else {
            result.extend(core::iter::repeat_n('\\', slashes));
            result.push(character);
        }
        slashes = 0;
    }
    result.extend(core::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};

    use super::{module, quote};
    use crate::{Key, Object, Property, Realm, Value, ops};

    fn call(
        realm: &mut Realm,
        exports: &Object,
        name: &str,
        args: &[Value],
    ) -> Result<Value, crate::Thrown> {
        let function = ops::get(realm, &Value::Object(exports.clone()), &Key::name(name))?;
        ops::call(realm, &function, Value::Undefined, args)
    }

    fn strings(realm: &Realm, values: &[&str]) -> Value {
        Value::Object(realm.array(values.iter().map(|value| Value::str(value)).collect()))
    }

    fn listen(realm: &mut Realm, emitter: &Object, event: &str, callback: &Object) {
        let on = ops::get_named(realm, &Value::Object(emitter.clone()), "on").unwrap();
        ops::call(
            realm,
            &on,
            Value::Object(emitter.clone()),
            &[Value::str(event), Value::Object(callback.clone())],
        )
        .unwrap();
    }

    fn call_method(
        realm: &mut Realm,
        receiver: &Object,
        method: &str,
        args: &[Value],
    ) -> Result<Value, crate::Thrown> {
        let function = ops::get(realm, &Value::Object(receiver.clone()), &Key::name(method))?;
        ops::call(realm, &function, Value::Object(receiver.clone()), args)
    }

    #[test]
    fn windows_command_line_arguments_preserve_spaces_quotes_and_trailing_slashes() {
        for (argument, expected) in [
            ("plain", "plain"),
            ("", "\"\""),
            ("tail\\", "tail\\"),
            ("two words", "\"two words\""),
            ("a\"b", "\"a\\\"b\""),
            ("tab\tbed\\", "\"tab\tbed\\\\\""),
            ("x\\\"y", "\"x\\\\\\\"y\""),
        ] {
            assert_eq!(quote(argument), expected);
        }
    }

    #[test]
    fn synchronous_direct_and_shell_processes_capture_output_and_honor_options() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let options = realm.object();
        options.define(
            Key::name("cwd"),
            Property::data(Value::str(r"C:\Windows\System32")),
        );
        options.define(Key::name("shell"), Property::data(Value::Boolean(true)));
        let direct_args = [
            Value::str("where.exe"),
            strings(&realm, &["cmd.exe"]),
            Value::Object(options),
        ];
        let direct = call(&mut realm, &exports, "execFileSync", &direct_args).unwrap();
        let Value::String(direct) = direct else {
            panic!("execFileSync should return captured text")
        };
        assert!(direct.to_rust_lossy().to_lowercase().contains("cmd.exe"));

        let non_string_cwd = realm.object();
        non_string_cwd.define(Key::name("cwd"), Property::data(Value::Boolean(true)));
        let default_cwd_args = [
            Value::str("where.exe"),
            strings(&realm, &["cmd.exe"]),
            Value::Object(non_string_cwd),
        ];
        assert!(call(&mut realm, &exports, "execFileSync", &default_cwd_args).is_ok());

        let shell = call(
            &mut realm,
            &exports,
            "execSync",
            &[Value::str("echo native child process")],
        )
        .unwrap();
        let Value::String(shell) = shell else {
            panic!("execSync should return captured text")
        };
        assert_eq!(shell.to_rust_lossy(), "native child process\r\n");
    }

    #[test]
    fn spawn_sync_reports_output_status_and_start_failures_as_node_does() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let get = |realm: &mut Realm, object: &Value, name: &'static str| {
            ops::get_named(realm, object, name).unwrap()
        };
        let text = |value: Value| match value {
            Value::String(value) => value.to_rust_lossy(),
            other => panic!("not text: {other:?}"),
        };
        // The options may stand in for the arguments.
        let options = realm.object();
        options.define(Key::name("shell"), Property::data(Value::Boolean(true)));
        options.define(Key::name("encoding"), Property::data(Value::str("utf8")));
        let result = call(
            &mut realm,
            &exports,
            "spawnSync",
            &[
                Value::str("echo out& echo err 1>&2& exit 3"),
                Value::Object(options),
            ],
        )
        .unwrap();
        assert_eq!(text(get(&mut realm, &result, "stdout")), "out\r\n");
        assert_eq!(text(get(&mut realm, &result, "stderr")), "err \r\n");
        assert!(matches!(
            get(&mut realm, &result, "status"),
            Value::Number(3.0)
        ));
        assert!(matches!(get(&mut realm, &result, "signal"), Value::Null));
        assert!(matches!(
            get(&mut realm, &result, "error"),
            Value::Undefined
        ));
        assert!(matches!(get(&mut realm, &result, "pid"), Value::Number(pid) if pid > 0.0));
        let output = get(&mut realm, &result, "output");
        assert!(matches!(get(&mut realm, &output, "0"), Value::Null));
        assert_eq!(text(get(&mut realm, &output, "1")), "out\r\n");

        // Without an encoding, or with "buffer", the output is bytes.
        for encoding in [None, Some("buffer")] {
            let options = realm.object();
            if let Some(encoding) = encoding {
                options.define(Key::name("encoding"), Property::data(Value::str(encoding)));
            }
            let args = [
                Value::str("where.exe"),
                strings(&realm, &["where.exe"]),
                Value::Object(options),
            ];
            let result = call(&mut realm, &exports, "spawnSync", &args).unwrap();
            let stdout = get(&mut realm, &result, "stdout");
            assert!(matches!(stdout, Value::Object(_)), "{stdout:?}");
            let length = get(&mut realm, &stdout, "length");
            assert!(matches!(length, Value::Number(length) if length > 10.0));
            assert!(matches!(
                get(&mut realm, &result, "status"),
                Value::Number(0.0)
            ));
        }

        // A program that cannot start gives its error, named as Node names it.
        for (program, code) in [
            ("nativeboat-no-such-program.exe", "ENOENT"),
            (r"C:\Windows", "EACCES"),
            (r"C:\Windows\win.ini", "UNKNOWN"),
        ] {
            let result = call(&mut realm, &exports, "spawnSync", &[Value::str(program)]).unwrap();
            let error = get(&mut realm, &result, "error");
            assert_eq!(text(get(&mut realm, &error, "code")), code, "{program}");
            assert_eq!(
                text(get(&mut realm, &error, "syscall")),
                format!("spawnSync {program}")
            );
            assert_eq!(text(get(&mut realm, &error, "path")), program);
            for name in ["status", "stdout", "stderr", "output", "signal"] {
                assert!(
                    matches!(get(&mut realm, &result, name), Value::Null),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn synchronous_process_errors_cover_invalid_arguments_and_exit_status() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        assert!(call(&mut realm, &exports, "execSync", &[]).is_err());
        assert!(
            call(
                &mut realm,
                &exports,
                "execFileSync",
                &[Value::str("where.exe")]
            )
            .is_err()
        );
        assert!(
            call(
                &mut realm,
                &exports,
                "execFileSync",
                &[Value::str("where.exe"), Value::Undefined]
            )
            .is_err()
        );
        assert!(
            call(
                &mut realm,
                &exports,
                "execFileSync",
                &[Value::str("where.exe"), Value::Number(1.0)]
            )
            .is_err()
        );
        let null_array = Value::Object(realm.array(vec![Value::Null]));
        assert!(
            call(
                &mut realm,
                &exports,
                "execFileSync",
                &[Value::str("where.exe"), null_array]
            )
            .is_err()
        );

        let missing_command = strings(&realm, &["nativeboat-command-that-does-not-exist.exe"]);
        let result = call(
            &mut realm,
            &exports,
            "execFileSync",
            &[Value::str("where.exe"), missing_command],
        );
        assert!(result.is_err());

        let no_args = strings(&realm, &[]);
        let launch_error = call(
            &mut realm,
            &exports,
            "execFileSync",
            &[Value::str("nativeboat-missing-executable.exe"), no_args],
        );
        assert!(launch_error.is_err());
    }

    #[test]
    fn asynchronous_spawn_emits_spawn_output_end_exit_and_close_in_order() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [Value::str("where.exe"), strings(&realm, &["cmd.exe"])];
        let child_value = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child_value else {
            panic!("spawn should return a child object")
        };
        let events = Rc::new(RefCell::new(Vec::<String>::new()));
        for event in ["spawn", "exit", "close"] {
            let events = events.clone();
            let callback = realm.function(event, 2, false, move |_, _, _| {
                events.borrow_mut().push(event.to_owned());
                Ok(Value::Undefined)
            });
            listen(&mut realm, &child, event, &callback);
        }
        let stdout = ops::get(
            &mut realm,
            &Value::Object(child.clone()),
            &Key::name("stdout"),
        )
        .unwrap();
        let Value::Object(stdout) = stdout else {
            panic!("stdout should be an object")
        };
        call_method(&mut realm, &stdout, "setEncoding", &[Value::str("utf8")]).unwrap();
        let events_for_data = events.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            let text = match args.first() {
                Some(Value::String(text)) => text.to_rust_lossy(),
                _ => String::new(),
            };
            events_for_data.borrow_mut().push(format!("data:{text}"));
            Ok(Value::Undefined)
        });
        listen(&mut realm, &stdout, "data", &data);
        let events_for_end = events.clone();
        let end = realm.function("end", 0, false, move |_, _, _| {
            events_for_end.borrow_mut().push("end".to_owned());
            Ok(Value::Undefined)
        });
        listen(&mut realm, &stdout, "end", &end);

        realm.run_event_loop().unwrap();
        let events = events.borrow();
        assert_eq!(events.first().map(String::as_str), Some("spawn"));
        assert!(events.iter().any(|event| {
            event.starts_with("data:") && event.to_lowercase().contains("cmd.exe")
        }));
        assert!(events.iter().any(|event| event == "end"));
        assert_eq!(
            events.get(events.len() - 2).map(String::as_str),
            Some("exit")
        );
        assert_eq!(events.last().map(String::as_str), Some("close"));
    }

    #[test]
    fn asynchronous_spawn_streams_bytes_until_an_encoding_is_set() {
        // Without `setEncoding`, output arrives as bytes, as Node's Buffers;
        // input written as bytes reaches the child unchanged.
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [Value::str("sort.exe"), strings(&realm, &[])];
        let child_value = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child_value else {
            panic!("spawn should return a child object")
        };
        let stdout = ops::get(
            &mut realm,
            &Value::Object(child.clone()),
            &Key::name("stdout"),
        )
        .unwrap();
        let Value::Object(stdout) = stdout else {
            panic!("stdout should be an object")
        };
        let encoding = ops::get(
            &mut realm,
            &Value::Object(stdout.clone()),
            &Key::name("readableEncoding"),
        )
        .unwrap();
        assert!(matches!(encoding, Value::Null));
        let received = Rc::new(RefCell::new(Vec::<u8>::new()));
        let strings_seen = Rc::new(Cell::new(false));
        let (bytes, seen) = (received.clone(), strings_seen.clone());
        let data = realm.function("data", 1, false, move |_, _, args| {
            match args.first() {
                Some(Value::String(_)) => seen.set(true),
                Some(value) => bytes
                    .borrow_mut()
                    .extend(crate::typed_array_builtin::view_bytes(value).unwrap_or_default()),
                None => {}
            }
            Ok(Value::Undefined)
        });
        listen(&mut realm, &stdout, "data", &data);
        let stdin = ops::get(
            &mut realm,
            &Value::Object(child.clone()),
            &Key::name("stdin"),
        )
        .unwrap();
        let Value::Object(stdin) = stdin else {
            panic!("stdin should be an object")
        };
        let input =
            crate::typed_array_builtin::new_bytes(&realm, b"pear\r\napple\r\n".to_vec(), None);
        call_method(&mut realm, &stdin, "end", &[Value::Object(input)]).unwrap();
        realm.run_event_loop().unwrap();
        assert!(!strings_seen.get());
        let text = String::from_utf8(received.borrow().clone()).unwrap();
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            ["apple", "pear"]
        );
    }

    #[test]
    fn asynchronous_spawn_reports_missing_executables_through_error_and_close() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [
            Value::str("nativeboat-missing-executable.exe"),
            strings(&realm, &[]),
        ];
        let child_value = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child_value else {
            panic!("spawn should return a child object")
        };
        let events = Rc::new(RefCell::new(Vec::<String>::new()));
        for event in ["error", "close"] {
            let events = events.clone();
            let callback = realm.function(event, 2, false, move |_, _, _| {
                events.borrow_mut().push(event.to_owned());
                Ok(Value::Undefined)
            });
            listen(&mut realm, &child, event, &callback);
        }
        realm.run_event_loop().unwrap();
        assert_eq!(&*events.borrow(), &["error", "close"]);
    }

    #[test]
    fn asynchronous_spawn_can_be_terminated_and_reports_killed_state() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [
            Value::str("ping.exe"),
            strings(&realm, &["-n", "5", "127.0.0.1"]),
        ];
        let child_value = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child_value else {
            panic!("spawn should return a child object")
        };
        assert!(matches!(
            call_method(&mut realm, &child, "kill", &[Value::Number(0.0)]).unwrap(),
            Value::Boolean(true)
        ));
        assert!(matches!(
            call_method(&mut realm, &child, "kill", &[]).unwrap(),
            Value::Boolean(true)
        ));
        assert!(matches!(
            call_method(&mut realm, &child, "kill", &[Value::str("SIGTERM")]).unwrap(),
            Value::Boolean(false)
        ));
        assert!(matches!(
            ops::get(&mut realm, &Value::Object(child), &Key::name("killed")).unwrap(),
            Value::Boolean(true)
        ));
        realm.run_event_loop().unwrap();
    }

    #[test]
    fn unreferenced_children_leave_the_event_loop_free() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [
            Value::str("ping.exe"),
            strings(&realm, &["-n", "30", "127.0.0.1"]),
        ];
        let Value::Object(child) = call(&mut realm, &exports, "spawn", &spawn_args).unwrap() else {
            panic!("spawn should return a child object")
        };
        assert!(matches!(
            call_method(&mut realm, &child, "unref", &[]).unwrap(),
            Value::Undefined
        ));
        let started = std::time::Instant::now();
        realm.run_event_loop().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        // A referenced child keeps the loop until it exits.
        call_method(&mut realm, &child, "ref", &[]).unwrap();
        call_method(&mut realm, &child, "kill", &[]).unwrap();
        realm.run_event_loop().unwrap();
    }

    #[test]
    fn asynchronous_spawn_shell_mode_runs_the_command_through_cmd() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let options = realm.object();
        options.define(Key::name("shell"), Property::data(Value::Boolean(true)));
        let spawn_args = [
            Value::str("echo"),
            strings(&realm, &["native async shell"]),
            Value::Object(options),
        ];
        let child = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child else {
            panic!("spawn should return a child object")
        };
        let stdout = ops::get(
            &mut realm,
            &Value::Object(child.clone()),
            &Key::name("stdout"),
        )
        .unwrap();
        let Value::Object(stdout) = stdout else {
            panic!("stdout should be an object")
        };
        let output = Rc::new(RefCell::new(String::new()));
        let output_for_callback = output.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(value)) = args.first() {
                output_for_callback
                    .borrow_mut()
                    .push_str(&value.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        call_method(&mut realm, &stdout, "setEncoding", &[Value::str("utf8")]).unwrap();
        listen(&mut realm, &stdout, "data", &data);
        realm.run_event_loop().unwrap();
        assert_eq!(&*output.borrow(), "native async shell\r\n");
    }

    #[test]
    fn asynchronous_spawn_delivers_stderr_data_before_close() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let options = realm.object();
        options.define(Key::name("shell"), Property::data(Value::Boolean(true)));
        let spawn_args = [
            Value::str("echo"),
            strings(&realm, &["native async stderr 1>&2"]),
            Value::Object(options),
        ];
        let child = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child else {
            panic!("spawn should return a child object")
        };
        let stderr = ops::get(&mut realm, &Value::Object(child), &Key::name("stderr")).unwrap();
        let Value::Object(stderr) = stderr else {
            panic!("stderr should be an object")
        };
        let output = Rc::new(RefCell::new(String::new()));
        let output_for_callback = output.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(value)) = args.first() {
                output_for_callback
                    .borrow_mut()
                    .push_str(&value.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        call_method(&mut realm, &stderr, "setEncoding", &[Value::str("utf8")]).unwrap();
        listen(&mut realm, &stderr, "data", &data);
        realm.run_event_loop().unwrap();
        assert_eq!(&*output.borrow(), "native async stderr \r\n");
    }

    #[test]
    fn asynchronous_stdin_write_and_end_deliver_all_input_then_close_the_pipe() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let spawn_args = [Value::str("findstr.exe"), strings(&realm, &["/c:needle"])];
        let child = call(&mut realm, &exports, "spawn", &spawn_args).unwrap();
        let Value::Object(child) = child else {
            panic!("spawn should return a child object")
        };
        let stdin = ops::get(
            &mut realm,
            &Value::Object(child.clone()),
            &Key::name("stdin"),
        )
        .unwrap();
        let Value::Object(stdin) = stdin else {
            panic!("stdin should be an object")
        };
        assert!(matches!(
            call_method(&mut realm, &stdin, "write", &[Value::str("needle from ")]).unwrap(),
            Value::Boolean(true)
        ));
        call_method(&mut realm, &stdin, "end", &[Value::str("native stdin\r\n")]).unwrap();
        call_method(&mut realm, &stdin, "end", &[]).unwrap();
        assert!(matches!(
            call_method(&mut realm, &stdin, "write", &[Value::str("after close")]).unwrap(),
            Value::Boolean(false)
        ));
        let stdout = ops::get(&mut realm, &Value::Object(child), &Key::name("stdout")).unwrap();
        let Value::Object(stdout) = stdout else {
            panic!("stdout should be an object")
        };
        let output = Rc::new(RefCell::new(String::new()));
        let output_for_callback = output.clone();
        let data = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(value)) = args.first() {
                output_for_callback
                    .borrow_mut()
                    .push_str(&value.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        call_method(&mut realm, &stdout, "setEncoding", &[Value::str("utf8")]).unwrap();
        listen(&mut realm, &stdout, "data", &data);
        realm.run_event_loop().unwrap();
        assert_eq!(&*output.borrow(), "needle from native stdin\r\n");
    }

    #[test]
    fn exec_and_exec_file_callbacks_collect_output_and_report_errors() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let results = Rc::new(RefCell::new(Vec::<String>::new()));
        for (name, command, argv) in [
            ("exec", "echo native exec", Vec::new()),
            ("execFile", "where.exe", vec!["cmd.exe"]),
            (
                "execFile",
                "where.exe",
                vec!["nativeboat-missing-command.exe"],
            ),
            ("execFile", "nativeboat-missing-executable.exe", Vec::new()),
        ] {
            let results = results.clone();
            let callback = realm.function(name, 3, false, move |_, _, args| {
                let failed = !matches!(args.first(), Some(Value::Null));
                let output = match args.get(1) {
                    Some(Value::String(output)) => output.to_rust_lossy(),
                    _ => String::new(),
                };
                let code = args
                    .first()
                    .and_then(Value::as_object)
                    .and_then(|error| error.own_property(&Key::name("code")))
                    .and_then(|property| match property.slot {
                        crate::Slot::Data {
                            value: Value::Number(code),
                            ..
                        } => Some(code.to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "missing".to_owned());
                let message = args
                    .first()
                    .and_then(Value::as_object)
                    .and_then(|error| error.own_property(&Key::name("message")))
                    .and_then(|property| match property.slot {
                        crate::Slot::Data {
                            value: Value::String(message),
                            ..
                        } => Some(message.to_rust_lossy()),
                        _ => None,
                    })
                    .unwrap_or_default();
                results
                    .borrow_mut()
                    .push(format!("{name}:{failed}:{code}:{message}:{output}"));
                Ok(Value::Undefined)
            });
            let argv = strings(&realm, &argv);
            let mut call_args = vec![Value::str(command)];
            if name == "exec" {
                call_args.push(Value::Object(callback));
            } else {
                call_args.push(argv);
                call_args.push(Value::Object(callback));
            }
            call(&mut realm, &exports, name, &call_args).unwrap();
        }
        realm.run_event_loop().unwrap();
        let mut results = results.borrow().clone();
        results.sort();
        assert_eq!(results.len(), 4);
        assert!(
            results
                .iter()
                .any(|value| { value.starts_with("exec:false:missing::native exec") }),
            "{results:?}"
        );
        assert!(
            results.iter().any(|value| {
                value.starts_with("execFile:false:missing::")
                    && value.to_lowercase().contains("cmd.exe")
            }),
            "{results:?}"
        );
        assert!(
            results.iter().any(|value| value.starts_with(
                "execFile:true:1:Command failed: where.exe nativeboat-missing-command.exe\n"
            )),
            "{results:?}"
        );
        assert!(
            results.iter().any(|value| {
                value.starts_with(
                    "execFile:true:missing:Could not start child process (Windows error ",
                )
            }),
            "{results:?}"
        );
    }

    #[test]
    fn exec_functions_reject_invalid_argument_types_before_spawning() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        assert!(
            call(
                &mut realm,
                &exports,
                "execFile",
                &[Value::str("where.exe"), Value::str("cmd.exe")],
            )
            .is_err()
        );
        assert!(
            call(
                &mut realm,
                &exports,
                "exec",
                &[Value::str("echo native"), Value::Number(1.0)],
            )
            .is_err()
        );
    }

    #[test]
    fn exec_file_accepts_callback_without_an_argument_array() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let output = Rc::new(RefCell::new(String::new()));
        let callback_output = output.clone();
        let callback = realm.function("callback", 3, false, move |_, _, args| {
            if matches!(args.first(), Some(Value::Null)) {
                if let Some(Value::String(stdout)) = args.get(1) {
                    *callback_output.borrow_mut() = stdout.to_rust_lossy();
                }
            } else {
                *callback_output.borrow_mut() = "callback received an error".to_owned();
            }
            Ok(Value::Undefined)
        });
        call(
            &mut realm,
            &exports,
            "execFile",
            &[Value::str("whoami.exe"), Value::Object(callback)],
        )
        .unwrap();
        realm.run_event_loop().unwrap();
        assert!(!output.borrow().trim().is_empty());
    }

    #[test]
    fn exec_and_exec_file_forward_working_directory_options() {
        let mut realm = Realm::new();
        let exports = module(&realm);
        let output = Rc::new(RefCell::new(Vec::<(&'static str, String)>::new()));
        let exec_output = output.clone();
        let exec_callback = realm.function("execCallback", 3, false, move |_, _, args| {
            if let Some(Value::String(stdout)) = args.get(1) {
                exec_output
                    .borrow_mut()
                    .push(("exec", stdout.to_rust_lossy()));
            }
            Ok(Value::Undefined)
        });
        let cwd = Value::str("C:\\Windows");
        let exec_options = realm.object();
        exec_options.define(Key::name("cwd"), Property::data(cwd.clone()));
        call(
            &mut realm,
            &exports,
            "exec",
            &[
                Value::str("cd"),
                Value::Object(exec_options),
                Value::Object(exec_callback),
            ],
        )
        .unwrap();

        let file_output = output.clone();
        let file_callback = realm.function("execFileCallback", 3, false, move |_, _, args| {
            if let Some(Value::String(stdout)) = args.get(1) {
                file_output
                    .borrow_mut()
                    .push(("execFile", stdout.to_rust_lossy()));
            }
            Ok(Value::Undefined)
        });
        let file_options = realm.object();
        file_options.define(Key::name("cwd"), Property::data(cwd));
        call(
            &mut realm,
            &exports,
            "execFile",
            &[
                Value::str("whoami.exe"),
                Value::Undefined,
                Value::Object(file_options),
                Value::Object(file_callback),
            ],
        )
        .unwrap();
        realm.run_event_loop().unwrap();

        let output = output.borrow();
        assert_eq!(output.len(), 2, "{output:?}");
        let exec = output.iter().find(|(kind, _)| *kind == "exec").unwrap();
        let exec_file = output.iter().find(|(kind, _)| *kind == "execFile").unwrap();
        assert!(exec.1.to_lowercase().contains("windows"), "{output:?}");
        assert!(!exec_file.1.trim().is_empty(), "{output:?}");
    }

    #[test]
    fn event_emitter_supports_once_off_and_rejects_unhandled_errors() {
        let mut realm = Realm::new();
        let events = super::emitter(&realm);
        let called = Rc::new(Cell::new(0));
        let once_called = called.clone();
        let once = realm.function("once", 0, false, move |_, _, _| {
            once_called.set(once_called.get() + 1);
            Ok(Value::Undefined)
        });
        call_method(
            &mut realm,
            &events,
            "once",
            &[Value::str("change"), Value::Object(once)],
        )
        .unwrap();
        call_method(&mut realm, &events, "emit", &[Value::str("change")]).unwrap();
        call_method(&mut realm, &events, "emit", &[Value::str("change")]).unwrap();
        assert_eq!(called.get(), 1);

        let removed = Rc::new(Cell::new(0));
        let removed_called = removed.clone();
        let callback = realm.function("listener", 0, false, move |_, _, _| {
            removed_called.set(removed_called.get() + 1);
            Ok(Value::Undefined)
        });
        call_method(
            &mut realm,
            &events,
            "addListener",
            &[Value::str("change"), Value::Object(callback.clone())],
        )
        .unwrap();
        call_method(
            &mut realm,
            &events,
            "off",
            &[Value::str("change"), Value::Object(callback)],
        )
        .unwrap();
        let removed_again = realm.function("listener", 0, false, |_, _, _| Ok(Value::Undefined));
        call_method(
            &mut realm,
            &events,
            "addListener",
            &[Value::str("change"), Value::Object(removed_again.clone())],
        )
        .unwrap();
        call_method(
            &mut realm,
            &events,
            "removeListener",
            &[Value::str("change"), Value::Object(removed_again)],
        )
        .unwrap();
        assert!(matches!(
            call_method(
                &mut realm,
                &events,
                "listenerCount",
                &[Value::str("change")]
            )
            .unwrap(),
            Value::Number(0.0)
        ));
        assert!(matches!(
            call_method(&mut realm, &events, "emit", &[Value::str("change")]).unwrap(),
            Value::Boolean(false)
        ));
        assert_eq!(removed.get(), 0);
        assert!(
            call_method(
                &mut realm,
                &events,
                "emit",
                &[Value::str("error"), Value::str("unhandled")]
            )
            .is_err()
        );
        assert!(
            call_method(
                &mut realm,
                &events,
                "on",
                &[Value::str("invalid"), Value::str("not a function")],
            )
            .is_err()
        );
    }

    #[test]
    fn stream_data_preserves_utf8_sequences_across_pipe_chunks() {
        let mut realm = Realm::new();
        let stream = super::emitter(&realm);
        let chunks = Rc::new(RefCell::new(Vec::<String>::new()));
        let chunks_for_callback = chunks.clone();
        let callback = realm.function("data", 1, false, move |_, _, args| {
            if let Some(Value::String(value)) = args.first() {
                chunks_for_callback.borrow_mut().push(value.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        stream.define(
            Key::name("readableEncoding"),
            Property::data(Value::str("utf8")),
        );
        listen(&mut realm, &stream, "data", &callback);
        let pending = Rc::new(RefCell::new(Vec::new()));
        super::emit_stream_data(&mut realm, &stream, &pending, &[0xe2, 0x82], false).unwrap();
        assert!(chunks.borrow().is_empty());
        super::emit_stream_data(&mut realm, &stream, &pending, &[0xac], false).unwrap();
        super::emit_stream_data(&mut realm, &stream, &pending, &[0xf0, 0x9f], false).unwrap();
        super::emit_stream_data(&mut realm, &stream, &pending, &[], true).unwrap();
        super::emit_stream_data(&mut realm, &stream, &pending, &[0xff], false).unwrap();
        assert_eq!(&*chunks.borrow(), &["€", "�", "�"]);
    }
}
