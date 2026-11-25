//! `node-pty`, which the runtime replaces whole: `spawn(file, args,
//! options)` runs a program in a Windows pseudoconsole (ConPTY) as
//! node-pty's Windows terminal does, and gives a terminal with `pid`,
//! `cols`, `rows`, `process` (the terminal's name), `onData` and `onExit`
//! events (also `on('data' | 'exit')`), `write`, `resize`, `clear`,
//! `kill`, `pause` and `resume`.
//!
//! Output is read on the I/O pool and given as UTF-8 text, a character
//! split across reads waiting for its remaining bytes. When the program
//! exits, the pseudoconsole is closed so its last output drains, and `exit`
//! follows the last data with the exit code.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

type Handle = *mut c_void;

const DEFAULT_COLS: f64 = 80.0;
const DEFAULT_ROWS: f64 = 24.0;
const DEFAULT_NAME: &str = "Windows Shell";
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
const PSEUDOCONSOLE_INHERIT_CURSOR: u32 = 1;
const INFINITE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct StartupInfoEx {
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
    input: Handle,
    output: Handle,
    error: Handle,
    attributes: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
struct ProcessInformation {
    process: usize,
    thread: usize,
    process_id: u32,
    thread_id: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        size: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        file: Handle,
        data: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn TerminateProcess(process: *mut c_void, exit_code: u32) -> i32;
}

type CreatePipeFn = unsafe extern "system" fn(*mut Handle, *mut Handle, *mut c_void, u32) -> i32;
type CreatePseudoConsoleFn =
    unsafe extern "system" fn(Coord, Handle, Handle, u32, *mut Handle) -> i32;
type ResizePseudoConsoleFn = unsafe extern "system" fn(Handle, Coord) -> i32;
type ClosePseudoConsoleFn = unsafe extern "system" fn(Handle);
type InitializeAttributesFn = unsafe extern "system" fn(*mut c_void, u32, u32, *mut usize) -> i32;
type UpdateAttributeFn = unsafe extern "system" fn(
    *mut c_void,
    u32,
    usize,
    *const c_void,
    usize,
    *mut c_void,
    *mut usize,
) -> i32;
type DeleteAttributesFn = unsafe extern "system" fn(*mut c_void);
type CreateProcessFn = unsafe extern "system" fn(
    *const u16,
    *mut u16,
    *mut c_void,
    *mut c_void,
    i32,
    u32,
    *mut c_void,
    *const u16,
    *mut StartupInfoEx,
    *mut ProcessInformation,
) -> i32;
type SearchPathFn = unsafe extern "system" fn(
    *const u16,
    *const u16,
    *const u16,
    u32,
    *mut u16,
    *mut *mut u16,
) -> u32;

/// A kernel32 function, looked up when first used; ConPTY needs Windows 10
/// 1809 or later.
fn kernel(name: &core::ffi::CStr) -> Option<*const c_void> {
    let procedure = crate::platform::system_procedure("kernel32.dll", name);
    (!procedure.is_null()).then_some(procedure)
}

macro_rules! procedure {
    ($name:literal, $kind:ty) => {
        kernel($name).map(|procedure| {
            // SAFETY: the named kernel32 export has this signature.
            unsafe { core::mem::transmute::<*const c_void, $kind>(procedure) }
        })
    };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// node-pty's `argsToCommandLine`: arguments quoted as Windows parses them.
fn command_line(file: &str, args: &Value, arguments: &[String]) -> String {
    if let Value::String(line) = args {
        let line = line.to_rust_lossy();
        return if line.is_empty() {
            command_line(file, &Value::Undefined, &[])
        } else {
            alloc::format!("{} {line}", command_line(file, &Value::Undefined, &[]))
        };
    }
    let mut result = String::new();
    for (index, arg) in core::iter::once(file)
        .chain(arguments.iter().map(String::as_str))
        .enumerate()
    {
        if index > 0 {
            result.push(' ');
        }
        let starts = arg.starts_with('"');
        let ends = arg.ends_with('"');
        let lopsided = starts != ends;
        let unquoted = !starts && !ends;
        let quote = arg.is_empty()
            || ((arg.contains(' ') || arg.contains('\t'))
                && arg.chars().count() > 1
                && (lopsided || unquoted));
        if quote {
            result.push('"');
        }
        let mut backslashes = 0;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    result.extend(core::iter::repeat_n('\\', backslashes * 2 + 1));
                    result.push('"');
                    backslashes = 0;
                }
                other => {
                    result.extend(core::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                    result.push(other);
                }
            }
        }
        if quote {
            result.extend(core::iter::repeat_n('\\', backslashes * 2));
            result.push('"');
        } else {
            result.extend(core::iter::repeat_n('\\', backslashes));
        }
    }
    result
}

/// The program a file names: itself when it has a directory, else the first
/// match on the search path, with `.exe` when it has no extension.
fn program_path(file: &str) -> String {
    if file.contains(['\\', '/']) {
        return file.into();
    }
    let Some(search) = procedure!(c"SearchPathW", SearchPathFn) else {
        return file.into();
    };
    let name = wide(file);
    let extension = wide(".exe");
    let mut buffer = alloc::vec![0_u16; 1024];
    // SAFETY: the names are nul-terminated and the buffer holds its length.
    let length = unsafe {
        search(
            core::ptr::null(),
            name.as_ptr(),
            extension.as_ptr(),
            buffer.len() as u32,
            buffer.as_mut_ptr(),
            core::ptr::null_mut(),
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return file.into();
    }
    String::from_utf16_lossy(&buffer[..length])
}

/// The environment block of `NAME=value` strings, sorted by name without
/// case, as Windows expects.
fn environment_block(variables: &[(String, String)]) -> Vec<u16> {
    let mut sorted: Vec<&(String, String)> = variables.iter().collect();
    sorted.sort_by_key(|(name, _)| name.to_ascii_uppercase());
    let mut block = Vec::new();
    for (name, value) in sorted {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

struct Terminal {
    console: Cell<usize>,
    input: Cell<usize>,
    output: Cell<usize>,
    process: Cell<usize>,
    pid: u32,
    cols: Cell<f64>,
    rows: Cell<f64>,
    paused: Cell<bool>,
    reading: Cell<bool>,
    /// Bytes of a character split across reads.
    partial: RefCell<Vec<u8>>,
    output_done: Cell<bool>,
    exit_code: Cell<Option<u32>>,
    exited: Cell<bool>,
    data_listeners: RefCell<Vec<Value>>,
    exit_listeners: RefCell<Vec<Value>>,
}

impl Terminal {
    fn close_console(&self) {
        let console = self.console.replace(0);
        if console != 0
            && let Some(close) = procedure!(c"ClosePseudoConsole", ClosePseudoConsoleFn)
        {
            // SAFETY: the pseudoconsole was created by `spawn` and is closed once.
            unsafe { close(console as Handle) };
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close_console();
        for handle in [self.input.get(), self.output.get(), self.process.get()] {
            if handle != 0 {
                // SAFETY: each handle is owned by the terminal and closed once.
                unsafe { CloseHandle(handle as Handle) };
            }
        }
    }
}

/// UTF-8 text of whole characters in `bytes`, keeping a trailing partial
/// character for the next read.
fn decode(partial: &RefCell<Vec<u8>>, bytes: &[u8]) -> String {
    let mut pending = partial.borrow_mut();
    pending.extend_from_slice(bytes);
    let mut end = pending.len();
    // A lead byte within the last three bytes whose sequence is incomplete.
    for back in 1..=pending.len().min(3) {
        let byte = pending[pending.len() - back];
        if byte & 0xC0 == 0x80 {
            continue;
        }
        let needed = match byte {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if needed > back {
            end = pending.len() - back;
        }
        break;
    }
    let text = String::from_utf8_lossy(&pending[..end]).into_owned();
    pending.drain(..end);
    text
}

fn emit(realm: &mut Realm, listeners: &RefCell<Vec<Value>>, value: Value) -> Result<(), Thrown> {
    let listeners = listeners.borrow().clone();
    for listener in listeners {
        ops::call(
            realm,
            &listener,
            Value::Undefined,
            core::slice::from_ref(&value),
        )?;
    }
    Ok(())
}

/// Emits `exit` once both the output has ended and the exit code is known.
fn finish(realm: &mut Realm, terminal: &Rc<Terminal>) -> Result<(), Thrown> {
    let Some(code) = terminal.exit_code.get() else {
        return Ok(());
    };
    if !terminal.output_done.get() || terminal.exited.replace(true) {
        return Ok(());
    }
    let event = realm.object();
    event.define(
        Key::name("exitCode"),
        Property::data(Value::Number(f64::from(code))),
    );
    event.define(Key::name("signal"), Property::data(Value::Undefined));
    emit(realm, &terminal.exit_listeners, Value::Object(event))
}

fn read(realm: &mut Realm, terminal: Rc<Terminal>) -> Result<(), Thrown> {
    if terminal.paused.get() || terminal.reading.get() || terminal.output_done.get() {
        return Ok(());
    }
    terminal.reading.set(true);
    let output = terminal.output.get();
    let reader = terminal.clone();
    let submitted = realm.submit_io(
        move || {
            let mut buffer = alloc::vec![0_u8; 64 * 1024];
            let mut read = 0;
            // SAFETY: the pipe stays open until the terminal drops, which
            // waits for this read; the buffer holds its length.
            let ok = unsafe {
                ReadFile(
                    output as Handle,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    &mut read,
                    core::ptr::null_mut(),
                )
            };
            if ok == 0 || read == 0 {
                return None;
            }
            buffer.truncate(read as usize);
            Some(buffer)
        },
        move |realm, chunk| {
            reader.reading.set(false);
            match chunk {
                Some(bytes) => {
                    let text = decode(&reader.partial, &bytes);
                    if !text.is_empty() {
                        emit(realm, &reader.data_listeners, Value::str(&text))?;
                    }
                    read(realm, reader)
                }
                None => {
                    reader.output_done.set(true);
                    finish(realm, &reader)
                }
            }
        },
    );
    if submitted.is_err() {
        terminal.reading.set(false);
        terminal.output_done.set(true);
    }
    Ok(())
}

/// Waits for the program to exit, then closes the pseudoconsole so its
/// output drains and ends.
fn wait(realm: &mut Realm, terminal: Rc<Terminal>) {
    let process = terminal.process.get();
    let waiter = terminal.clone();
    let _ = realm.submit_io(
        move || {
            let mut code = 0;
            // SAFETY: the process handle stays open until the terminal drops.
            unsafe {
                WaitForSingleObject(process as Handle, INFINITE);
                GetExitCodeProcess(process as Handle, &mut code);
            }
            code
        },
        move |realm, code| {
            waiter.exit_code.set(Some(code));
            let console = waiter.console.replace(0);
            if console == 0 {
                return finish(realm, &waiter);
            }
            // Closing can wait for the output to be read: it runs on the
            // pool while the reader drains it.
            let closer = waiter.clone();
            let _ = realm.submit_io(
                move || {
                    if let Some(close) = procedure!(c"ClosePseudoConsole", ClosePseudoConsoleFn) {
                        // SAFETY: the pseudoconsole was taken from the
                        // terminal and is closed once.
                        unsafe { close(console as Handle) };
                    }
                },
                move |realm, ()| finish(realm, &closer),
            );
            Ok(())
        },
    );
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn spawn_error(realm: &mut Realm, message: &str) -> Thrown {
    Thrown(Value::Object(realm.error(ErrorKind::Error, message)))
}

fn number_option(
    realm: &mut Realm,
    options: &Value,
    name: &'static str,
    default: f64,
) -> Result<f64, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(default);
    }
    Ok(match ops::get_named(realm, options, name)? {
        Value::Number(value) if value > 0.0 => value,
        _ => default,
    })
}

fn string_option(
    realm: &mut Realm,
    options: &Value,
    name: &'static str,
) -> Result<Option<String>, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(None);
    }
    Ok(match ops::get_named(realm, options, name)? {
        Value::String(value) => Some(value.to_rust_lossy()),
        _ => None,
    })
}

/// The `env` option, or the process's environment, as name and value pairs.
fn environment(realm: &mut Realm, options: &Value) -> Result<Vec<(String, String)>, Thrown> {
    let source = match options {
        Value::Object(_) => ops::get_named(realm, options, "env")?,
        _ => Value::Undefined,
    };
    let source = match source {
        Value::Object(_) => source,
        _ => {
            let global = Value::Object(realm.global_object());
            let process = ops::get_named(realm, &global, "process")?;
            ops::get_named(realm, &process, "env")?
        }
    };
    let Value::Object(object) = &source else {
        return Ok(Vec::new());
    };
    let mut variables = Vec::new();
    for key in ops::own_property_keys(realm, object)? {
        let Key::String(name) = &key else {
            continue;
        };
        let value = ops::get(realm, &source, &key)?;
        if matches!(value, Value::Undefined) {
            continue;
        }
        let value = ops::to_string(realm, &value)?.to_rust_lossy();
        variables.push((name.to_rust_lossy(), value));
    }
    Ok(variables)
}

fn spawn(realm: &mut Realm, args: &[Value]) -> crate::Completion {
    let file = match arg(args, 0) {
        Value::String(file) if !file.to_rust_lossy().is_empty() => file.to_rust_lossy(),
        _ => "cmd.exe".into(),
    };
    let arguments_value = arg(args, 1);
    let mut arguments = Vec::new();
    if let Value::Object(list) = &arguments_value
        && list.is_array()
    {
        let length = match ops::get_named(realm, &arguments_value, "length")? {
            Value::Number(length) => length as u32,
            _ => 0,
        };
        for index in 0..length {
            let item = ops::get(realm, &arguments_value, &Key::Index(index))?;
            arguments.push(ops::to_string(realm, &item)?.to_rust_lossy());
        }
    }
    let options = arg(args, 2);
    let cols = number_option(realm, &options, "cols", DEFAULT_COLS)?;
    let rows = number_option(realm, &options, "rows", DEFAULT_ROWS)?;
    let variables = environment(realm, &options)?;
    let name = match string_option(realm, &options, "name")? {
        Some(name) => name,
        None => variables
            .iter()
            .find(|(variable, _)| variable == "TERM")
            .map_or_else(|| DEFAULT_NAME.into(), |(_, value)| value.clone()),
    };
    let cwd = match string_option(realm, &options, "cwd")? {
        Some(cwd) => cwd,
        None => crate::platform::current_directory()
            .map(|directory| directory.to_rust_lossy())
            .unwrap_or_default(),
    };
    let inherit_cursor = matches!(&options, Value::Object(_))
        && matches!(
            ops::get_named(realm, &options, "conptyInheritCursor")?,
            Value::Boolean(true)
        );

    let (
        Some(create_pipe),
        Some(create_console),
        Some(initialize),
        Some(update),
        Some(delete),
        Some(create_process),
    ) = (
        procedure!(c"CreatePipe", CreatePipeFn),
        procedure!(c"CreatePseudoConsole", CreatePseudoConsoleFn),
        procedure!(c"InitializeProcThreadAttributeList", InitializeAttributesFn),
        procedure!(c"UpdateProcThreadAttribute", UpdateAttributeFn),
        procedure!(c"DeleteProcThreadAttributeList", DeleteAttributesFn),
        procedure!(c"CreateProcessW", CreateProcessFn),
    )
    else {
        return Err(spawn_error(
            realm,
            "Pseudoconsoles need Windows 10 1809 or later",
        ));
    };
    let mut input_read: Handle = core::ptr::null_mut();
    let mut input_write: Handle = core::ptr::null_mut();
    let mut output_read: Handle = core::ptr::null_mut();
    let mut output_write: Handle = core::ptr::null_mut();
    // SAFETY: the out-pointers are writable; failures are checked.
    let piped = unsafe {
        create_pipe(&mut input_read, &mut input_write, core::ptr::null_mut(), 0) != 0
            && create_pipe(
                &mut output_read,
                &mut output_write,
                core::ptr::null_mut(),
                0,
            ) != 0
    };
    if !piped {
        return Err(spawn_error(
            realm,
            "Could not create the pseudoconsole's pipes",
        ));
    }
    let size = Coord {
        x: cols as i16,
        y: rows as i16,
    };
    let mut console: Handle = core::ptr::null_mut();
    let flags = if inherit_cursor {
        PSEUDOCONSOLE_INHERIT_CURSOR
    } else {
        0
    };
    // SAFETY: the pipe ends are open; the console out-pointer is writable.
    let status = unsafe { create_console(size, input_read, output_write, flags, &mut console) };
    // SAFETY: the pseudoconsole holds its own references to these ends.
    unsafe {
        CloseHandle(input_read);
        CloseHandle(output_write);
    }
    if status < 0 {
        // SAFETY: the remaining ends are open and closed once.
        unsafe {
            CloseHandle(input_write);
            CloseHandle(output_read);
        }
        return Err(spawn_error(
            realm,
            &alloc::format!("CreatePseudoConsole failed with {status:#x}"),
        ));
    }
    let mut attributes_size = 0;
    // SAFETY: a null list asks for the size one attribute needs.
    unsafe { initialize(core::ptr::null_mut(), 1, 0, &mut attributes_size) };
    let mut attributes = alloc::vec![0_u8; attributes_size];
    let list = attributes.as_mut_ptr().cast::<c_void>();
    let mut console_value = console;
    // SAFETY: the list is sized as Windows asked, and the attribute's value
    // outlives the process creation below.
    let listed = unsafe {
        initialize(list, 1, 0, &mut attributes_size) != 0
            && update(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                console_value,
                core::mem::size_of::<Handle>(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            ) != 0
    };
    let _ = &mut console_value;
    let program = program_path(&file);
    let mut line = wide(&command_line(&program, &arguments_value, &arguments));
    let mut block = environment_block(&variables);
    let directory = wide(&cwd);
    // SAFETY: every field is an integer or nullable pointer.
    let mut startup: StartupInfoEx = unsafe { core::mem::zeroed() };
    startup.cb = core::mem::size_of::<StartupInfoEx>() as u32;
    startup.flags = STARTF_USESTDHANDLES;
    startup.attributes = list;
    let mut information = ProcessInformation::default();
    // SAFETY: the command line, environment and directory are nul-terminated
    // and writable where Windows writes; the startup information is extended.
    let created = listed
        && unsafe {
            create_process(
                core::ptr::null(),
                line.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                block.as_mut_ptr().cast(),
                directory.as_ptr(),
                &mut startup,
                &mut information,
            )
        } != 0;
    // SAFETY: the list was initialized above.
    unsafe { delete(list) };
    let terminal = Rc::new(Terminal {
        console: Cell::new(console as usize),
        input: Cell::new(input_write as usize),
        output: Cell::new(output_read as usize),
        process: Cell::new(information.process),
        pid: information.process_id,
        cols: Cell::new(cols),
        rows: Cell::new(rows),
        paused: Cell::new(false),
        reading: Cell::new(false),
        partial: RefCell::new(Vec::new()),
        output_done: Cell::new(false),
        exit_code: Cell::new(None),
        exited: Cell::new(false),
        data_listeners: RefCell::new(Vec::new()),
        exit_listeners: RefCell::new(Vec::new()),
    });
    if !created {
        return Err(spawn_error(
            realm,
            &alloc::format!("File not found: {file}"),
        ));
    }
    if information.thread != 0 {
        // SAFETY: the thread handle is not needed.
        unsafe { CloseHandle(information.thread as Handle) };
    }
    let object = terminal_object(realm, &terminal, name);
    read(realm, terminal.clone())?;
    wait(realm, terminal);
    Ok(Value::Object(object))
}

fn method(
    realm: &Realm,
    target: &Object,
    name: &'static str,
    body: impl Fn(&mut Realm, Value, &[Value]) -> crate::Completion + 'static,
) {
    let function = realm.function(name, 0, false, body);
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

/// An event subscription function: `event(listener)` adds the listener and
/// gives `{ dispose() }`.
fn event(realm: &Realm, terminal: &Rc<Terminal>, data: bool) -> Value {
    let terminal = terminal.clone();
    Value::Object(realm.function("event", 1, false, move |realm, _, args| {
        let listener = arg(args, 0);
        let listeners = if data {
            &terminal.data_listeners
        } else {
            &terminal.exit_listeners
        };
        listeners.borrow_mut().push(listener.clone());
        let disposable = realm.object();
        let owner = terminal.clone();
        method(realm, &disposable, "dispose", move |_, _, _| {
            let listeners = if data {
                &owner.data_listeners
            } else {
                &owner.exit_listeners
            };
            let mut known = listeners.borrow_mut();
            if let Some(at) = known
                .iter()
                .position(|item| crate::same_value(item, &listener))
            {
                known.remove(at);
            }
            Ok(Value::Undefined)
        });
        Ok(Value::Object(disposable))
    }))
}

fn terminal_object(realm: &mut Realm, terminal: &Rc<Terminal>, name: String) -> Object {
    let object = realm.object();
    object.define(
        Key::name("pid"),
        Property::data(Value::Number(f64::from(terminal.pid))),
    );
    object.define(Key::name("process"), Property::data(Value::str(&name)));
    object.define(
        Key::name("handleFlowControl"),
        Property::data(Value::Boolean(false)),
    );
    for (name, data) in [("onData", true), ("onExit", false)] {
        object.define(
            Key::name(name),
            Property::data(event(realm, terminal, data)),
        );
    }
    for (name, read_cols) in [("cols", true), ("rows", false)] {
        let owner = terminal.clone();
        let getter = realm.function(name, 0, false, move |_, _, _| {
            Ok(Value::Number(if read_cols {
                owner.cols.get()
            } else {
                owner.rows.get()
            }))
        });
        ops::define_accessor(&object, Key::name(name), Some(getter), None);
    }
    // EventEmitter-style listeners for `data` and `exit`.
    for name in ["on", "addListener", "once"] {
        let owner = terminal.clone();
        method(realm, &object, name, move |realm, this, args| {
            let event = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
            let listener = arg(args, 1);
            match event.as_str() {
                "data" => owner.data_listeners.borrow_mut().push(listener),
                // node-pty emits `exit` with the code, not an event object.
                "exit" => {
                    let wrapped = realm.function("exit", 1, false, move |realm, _, args| {
                        let code = match arg(args, 0) {
                            Value::Object(_) => ops::get_named(realm, &arg(args, 0), "exitCode")?,
                            other => other,
                        };
                        ops::call(realm, &listener, Value::Undefined, &[code])
                    });
                    owner
                        .exit_listeners
                        .borrow_mut()
                        .push(Value::Object(wrapped));
                }
                _ => {}
            }
            Ok(this)
        });
    }
    let writer = terminal.clone();
    method(realm, &object, "write", move |realm, _, args| {
        let bytes = match arg(args, 0) {
            Value::String(text) => text.to_rust_lossy().into_bytes(),
            Value::Object(buffer) => {
                crate::typed_array_builtin::view_bytes(&Value::Object(buffer)).unwrap_or_default()
            }
            other => ops::to_string(realm, &other)?.to_rust_lossy().into_bytes(),
        };
        let input = writer.input.get();
        if input != 0 && !bytes.is_empty() {
            let mut written = 0;
            // SAFETY: the input pipe is open until the terminal drops.
            unsafe {
                WriteFile(
                    input as Handle,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut written,
                    core::ptr::null_mut(),
                )
            };
        }
        Ok(Value::Undefined)
    });
    let resizer = terminal.clone();
    method(realm, &object, "resize", move |realm, _, args| {
        let (Value::Number(cols), Value::Number(rows)) = (arg(args, 0), arg(args, 1)) else {
            return Err(spawn_error(
                realm,
                "resizing must be done using positive cols and rows",
            ));
        };
        if !(cols > 0.0 && rows > 0.0 && cols.is_finite() && rows.is_finite()) {
            return Err(spawn_error(
                realm,
                "resizing must be done using positive cols and rows",
            ));
        }
        resizer.cols.set(cols);
        resizer.rows.set(rows);
        let console = resizer.console.get();
        if console != 0
            && let Some(resize) = procedure!(c"ResizePseudoConsole", ResizePseudoConsoleFn)
        {
            let size = Coord {
                x: cols as i16,
                y: rows as i16,
            };
            // SAFETY: the pseudoconsole is open.
            unsafe { resize(console as Handle, size) };
        }
        Ok(Value::Undefined)
    });
    method(realm, &object, "clear", |_, _, _| Ok(Value::Undefined));
    let killer = terminal.clone();
    method(realm, &object, "kill", move |_, _, _| {
        let process = killer.process.get();
        if process != 0 && killer.exit_code.get().is_none() {
            // SAFETY: the process handle is open until the terminal drops.
            unsafe { TerminateProcess(process as Handle, 1) };
        }
        Ok(Value::Undefined)
    });
    let pauser = terminal.clone();
    method(realm, &object, "pause", move |_, _, _| {
        pauser.paused.set(true);
        Ok(Value::Undefined)
    });
    let resumer = terminal.clone();
    method(realm, &object, "resume", move |realm, _, _| {
        resumer.paused.set(false);
        read(realm, resumer.clone())?;
        Ok(Value::Undefined)
    });
    object
}

pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let spawn_function = realm.function("spawn", 3, false, |realm, _, args| spawn(realm, args));
    exports.define(
        Key::name("spawn"),
        Property::data(Value::Object(spawn_function.clone())),
    );
    // node-pty 0.x names.
    exports.define(
        Key::name("fork"),
        Property::data(Value::Object(spawn_function.clone())),
    );
    exports.define(
        Key::name("createTerminal"),
        Property::data(Value::Object(spawn_function)),
    );
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_quote_as_node_pty_does() {
        let args = |list: &[&str]| {
            list.iter()
                .map(|item| String::from(*item))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            command_line("cmd.exe", &Value::Undefined, &args(&["/c", "echo hi"])),
            "cmd.exe /c \"echo hi\""
        );
        assert_eq!(
            command_line("a b.exe", &Value::Undefined, &args(&[""])),
            "\"a b.exe\" \"\""
        );
        assert_eq!(
            command_line("x", &Value::Undefined, &args(&["say \"hi\""])),
            "x \"say \\\"hi\\\"\""
        );
        assert_eq!(
            command_line("x", &Value::Undefined, &args(&["C:\\dir\\"])),
            "x C:\\dir\\"
        );
        assert_eq!(
            command_line("x", &Value::Undefined, &args(&["C:\\a b\\"])),
            "x \"C:\\a b\\\\\""
        );
        assert_eq!(command_line("x", &Value::str("-l"), &[]), "x -l");
    }

    #[test]
    fn split_characters_wait_for_their_remaining_bytes() {
        let partial = RefCell::new(Vec::new());
        let euro = "€".as_bytes();
        assert_eq!(decode(&partial, &[b'a', euro[0]]), "a");
        assert_eq!(decode(&partial, &euro[1..]), "€");
        assert_eq!(decode(&partial, b"z"), "z");
    }
}
