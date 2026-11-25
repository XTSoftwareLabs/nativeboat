//! Node's `process` object for the main process of a native application:
//! identity, paths, arguments and environment from Windows, exit codes,
//! `nextTick`, warnings and the standard output streams, plus Electron's
//! `type` and `resourcesPath`. When the Node.js library is linked, `process`
//! is also an EventEmitter (see `process_events`).

use crate::node_error::{invalid_arg_type, node_error, out_of_range};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    Completion, ErrorKind, JsStr, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, ops,
    platform,
};
use alloc::boxed::Box;

/// The Node release whose behavior the runtime follows.
pub const NODE_VERSION: &str = "22.15.0";

/// The largest exit code magnitude `process.exitCode` accepts.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Installs `globalThis.process`: an accessor creating the object on first
/// read, whose setter replaces it with a data property, like Node's.
pub(crate) fn install(realm: &Realm) {
    let get = realm.function("get", 0, false, |realm, _, _| process(realm));
    let set = realm.function("set", 1, false, |realm, this, args| {
        let Value::Object(target) = this else {
            return Err(realm.throw(
                ErrorKind::Type,
                "Object.defineProperty called on non-object",
            ));
        };
        let value = args.first().cloned().unwrap_or_default();
        target.define(
            Key::name("process"),
            Property {
                slot: Slot::Data {
                    value,
                    writable: true,
                },
                enumerable: false,
                configurable: true,
            },
        );
        Ok(Value::Undefined)
    });
    realm.global.define(
        Key::name("process"),
        Property {
            slot: Slot::Accessor {
                get: Some(get),
                set: Some(set),
            },
            enumerable: false,
            configurable: true,
        },
    );
}

/// This realm's `process` object, created when first needed.
pub fn process(realm: &mut Realm) -> Completion {
    let process = match realm.process.clone() {
        Some(process) => process,
        None => create(realm)?,
    };
    if !realm.process_emitter && realm.node_library.is_some() {
        crate::process_events::emitter(realm, &process)?;
    }
    Ok(Value::Object(process))
}

/// `process.release`, as Node 22 reports it.
fn release(realm: &mut Realm) -> Object {
    let release = realm.object();
    let base = format!("https://nodejs.org/download/release/v{NODE_VERSION}/");
    for (name, value) in [
        ("name", "node".to_owned()),
        ("lts", "Jod".to_owned()),
        ("sourceUrl", format!("{base}node-v{NODE_VERSION}.tar.gz")),
        (
            "headersUrl",
            format!("{base}node-v{NODE_VERSION}-headers.tar.gz"),
        ),
        ("libUrl", format!("{base}win-x64/node.lib")),
    ] {
        release.define(Key::name(name), Property::data(Value::str(&value)));
    }
    release
}

/// `process.config`: the build configuration packages read to pick native
/// binaries, frozen as in Node. Only the variables those packages read are
/// given.
fn config(realm: &mut Realm) -> Object {
    let empty = |realm: &Realm| Value::Object(realm.array(Vec::new()));
    let defaults = realm.object();
    for name in ["cflags", "defines", "include_dirs", "libraries"] {
        defaults.define(Key::name(name), Property::data(empty(realm)));
    }
    defaults.define(
        Key::name("configurations"),
        Property::data(Value::Object(realm.object())),
    );
    defaults.define(
        Key::name("default_configuration"),
        Property::data(Value::str("Release")),
    );
    let variables = realm.object();
    for (name, value) in [
        ("asan", Value::Number(0.0)),
        ("host_arch", Value::str("x64")),
        ("napi_build_version", Value::str("10")),
        ("node_module_version", Value::Number(127.0)),
        ("node_shared", Value::Boolean(false)),
        ("target_arch", Value::str("x64")),
    ] {
        variables.define(Key::name(name), Property::data(value));
    }
    let config = realm.object();
    config.define(
        Key::name("target_defaults"),
        Property::data(Value::Object(defaults.clone())),
    );
    config.define(
        Key::name("variables"),
        Property::data(Value::Object(variables.clone())),
    );
    for object in [&defaults, &variables, &config] {
        crate::realm::freeze(object);
    }
    config
}

/// Creates the `process` object.
fn create(realm: &mut Realm) -> Result<Object, Thrown> {
    let exec_path = exec_path_text(realm)?;
    let argv = argv(realm)?;
    let prototype = realm.object();
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        fixed(Value::str("process"), false),
    );
    let process = Object::new(Some(prototype), Kind::Ordinary);
    let versions = realm.object();
    versions.define(Key::name("node"), fixed(Value::str(NODE_VERSION), true));
    if let Some(electron) = realm.electron_version {
        versions.define(Key::name("electron"), fixed(Value::str(electron), true));
    }
    let version = format!("v{NODE_VERSION}");
    for (name, value) in [
        ("version", Value::str(&version)),
        ("versions", Value::Object(versions)),
        ("arch", Value::str("x64")),
        ("platform", Value::str("win32")),
    ] {
        process.define(Key::name(name), fixed(value, true));
    }
    let exit_code_get = realm.function("get", 0, false, |realm, _, _| Ok(realm.exit_code.clone()));
    let exit_code_set = realm.function("set", 1, false, |realm, _, args| {
        set_exit_code(realm, args.first().unwrap_or(&Value::Undefined))?;
        Ok(Value::Undefined)
    });
    process.define(
        Key::name("exitCode"),
        Property {
            slot: Slot::Accessor {
                get: Some(exit_code_get),
                set: Some(exit_code_set),
            },
            enumerable: true,
            configurable: false,
        },
    );
    let exit = realm.function("exit", 1, false, exit);
    let chdir = realm.function("chdir", 1, false, chdir);
    let binding = realm.function("binding", 1, false, process_binding);
    let emit_warning = realm.function("emitWarning", 4, false, emit_warning);
    let next_tick = realm.function("nextTick", 1, false, next_tick);
    for (name, function) in [
        ("exit", exit),
        ("chdir", chdir),
        ("binding", binding),
        ("emitWarning", emit_warning),
        ("nextTick", next_tick),
    ] {
        process.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let kill = realm.function("kill", 2, false, kill);
    process.define(Key::name("kill"), Property::data(Value::Object(kill)));
    for (name, function) in [
        ("memoryUsage", crate::process_usage::memory_usage(realm)),
        ("resourceUsage", crate::process_usage::resource_usage(realm)),
        ("cpuUsage", crate::process_usage::cpu_usage(realm)),
        (
            "getProcessMemoryInfo",
            crate::process_usage::process_memory_info(realm),
        ),
        ("uptime", crate::process_usage::uptime(realm)),
        (
            "availableMemory",
            crate::process_usage::available_memory(realm),
        ),
        (
            "constrainedMemory",
            crate::process_usage::constrained_memory(realm),
        ),
    ] {
        process.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let release = release(realm);
    process.define(Key::name("release"), Property::data(Value::Object(release)));
    let config = config(realm);
    process.define(Key::name("config"), Property::data(Value::Object(config)));
    let hrtime = realm.function("hrtime", 1, false, hrtime);
    let hrtime_bigint = realm.function("bigint", 0, false, |_, _, _| {
        Ok(Value::BigInt(alloc::rc::Rc::new(
            crate::bigint::BigInt::from_u64(platform::hrtime_ns()),
        )))
    });
    hrtime.define(
        Key::name("bigint"),
        Property::data(Value::Object(hrtime_bigint)),
    );
    process.define(Key::name("hrtime"), Property::data(Value::Object(hrtime)));
    for (name, descriptor) in [("stdin", 0), ("stdout", 1), ("stderr", 2)] {
        let get = realm.function("get", 0, false, move |realm, _, _| {
            stream(realm, descriptor)
        });
        process.define(
            Key::name(name),
            Property {
                slot: Slot::Accessor {
                    get: Some(get),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    let cwd = cwd(realm);
    let environment = crate::environment_builtin::environment(realm);
    let exec_argv = Value::Object(realm.array(Vec::new()));
    for (name, value) in [
        ("cwd", cwd),
        ("env", environment),
        ("argv", argv),
        ("execArgv", exec_argv),
    ] {
        process.define(Key::name(name), Property::data(value));
    }
    let pid = Value::Number(f64::from(platform::process_id()));
    process.define(Key::name("pid"), fixed(pid, true));
    let ppid = Value::Number(f64::from(platform::parent_process_id()));
    process.define(Key::name("ppid"), fixed(ppid, true));
    let umask = realm.function("umask", 1, false, umask);
    process.define(Key::name("umask"), Property::data(Value::Object(umask)));
    let report = report(realm);
    process.define(Key::name("report"), Property::data(Value::Object(report)));
    process.define(
        Key::name("execPath"),
        Property::data(Value::String(exec_path.clone())),
    );
    let argv0 = platform::command_arguments()
        .into_iter()
        .next()
        .map_or(Value::str(""), Value::String);
    process.define(
        Key::name("argv0"),
        Property {
            slot: Slot::Data {
                value: argv0,
                writable: false,
            },
            enumerable: true,
            configurable: false,
        },
    );
    // Electron's main-process members.
    process.define(Key::name("type"), Property::data(Value::str("browser")));
    process.define(
        Key::name("resourcesPath"),
        Property::data(Value::String(resources_path(&exec_path))),
    );
    process.define(Key::name("_exiting"), Property::data(Value::Boolean(false)));
    realm.process = Some(process.clone());
    Ok(process)
}

/// `process.hrtime([previous])`, backed by Windows' monotonic performance
/// counter. The returned nanoseconds are normalized to `[0, 1e9)` even when
/// the requested interval is negative, matching Node's tuple convention.
fn hrtime(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let current = i128::from(platform::hrtime_ns());
    let previous = match args.first() {
        None | Some(Value::Undefined) => 0,
        Some(Value::Object(array)) if array.is_array() => {
            let length = ops::get_named(realm, &Value::Object(array.clone()), "length")?;
            if ops::to_number(realm, &length)? != 2.0 {
                return Err(invalid_arg_type(
                    realm,
                    "time",
                    &["an integer array of length 2"],
                    &Value::Object(array.clone()),
                ));
            }
            let seconds = ops::get(realm, &Value::Object(array.clone()), &Key::Index(0))?;
            let nanos = ops::get(realm, &Value::Object(array.clone()), &Key::Index(1))?;
            let seconds = ops::to_number(realm, &seconds)?;
            let nanos = ops::to_number(realm, &nanos)?;
            if !seconds.is_finite()
                || !nanos.is_finite()
                || crate::float::fract(seconds) != 0.0
                || crate::float::fract(nanos) != 0.0
                || seconds.abs() > MAX_SAFE_INTEGER
                || nanos.abs() > MAX_SAFE_INTEGER
            {
                return Err(invalid_arg_type(
                    realm,
                    "time",
                    &["an integer array of length 2"],
                    &Value::Object(array.clone()),
                ));
            }
            (seconds as i128) * 1_000_000_000 + nanos as i128
        }
        Some(value) => {
            return Err(invalid_arg_type(
                realm,
                "time",
                &["an integer array of length 2"],
                value,
            ));
        }
    };
    let elapsed = current - previous;
    let seconds = elapsed.div_euclid(1_000_000_000);
    let nanos = elapsed.rem_euclid(1_000_000_000);
    Ok(Value::Object(realm.array(vec![
        Value::Number(seconds as f64),
        Value::Number(nanos as f64),
    ])))
}

/// A read-only data property.
fn fixed(value: Value, enumerable: bool) -> Property {
    Property {
        slot: Slot::Data {
            value,
            writable: false,
        },
        enumerable,
        configurable: true,
    }
}

/// Electron's `resources` directory beside the executable.
fn resources_path(exec_path: &JsStr) -> JsStr {
    let units = exec_path.units();
    let end = units
        .iter()
        .rposition(|unit| *unit == u16::from(b'\\'))
        .map_or(0, |index| index + 1);
    JsStr::from_units(&units[..end]).concat(&JsStr::from("resources"))
}

impl Realm {
    /// The exit code for a program that ends normally: `process.exitCode`
    /// as a 32-bit integer, or 0.
    pub fn exit_status(&self) -> i32 {
        match self.exit_code {
            Value::Number(code) => ops::wrap32(code) as i32,
            _ => 0,
        }
    }
}

/// `process.exitCode = value`: undefined, null, or a safe integer, which
/// may be given as an integer string.
fn set_exit_code(realm: &mut Realm, value: &Value) -> Result<(), Thrown> {
    let integer = |number: f64| number.is_finite() && crate::float::fract(number) == 0.0;
    let code = match value {
        Value::Undefined | Value::Null => Value::Undefined,
        _ => {
            let number = match value {
                Value::Number(number) => *number,
                Value::String(text) if !text.is_empty() && integer(crate::number::parse(text)) => {
                    crate::number::parse(text)
                }
                _ => return Err(invalid_arg_type(realm, "code", &["number"], value)),
            };
            if !integer(number) {
                return Err(out_of_range(
                    realm,
                    "code",
                    "an integer",
                    &Value::Number(number),
                ));
            }
            if number.abs() > MAX_SAFE_INTEGER {
                return Err(out_of_range(
                    realm,
                    "code",
                    ">= -9007199254740991 && <= 9007199254740991",
                    &Value::Number(number),
                ));
            }
            Value::Number(number)
        }
    };
    realm.exit_code = code;
    Ok(())
}

/// `process.exit(code)`: records the code, emits `exit`, then ends the
/// process at once.
fn exit(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let code = args.first().cloned().unwrap_or_default();
    if !matches!(code, Value::Undefined) {
        set_exit_code(realm, &code)?;
    }
    crate::process_events::exit(realm)?;
    crate::trace::exit(&format!("process.exit({}) called", realm.exit_status()));
    platform::exit_process(realm.exit_status() as u32);
    Ok(Value::Undefined)
}

/// `process.kill(pid, signal)`: on Windows every supported nonzero signal
/// terminates the target; signal zero only checks that it is still running.
fn kill(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let pid_value = args.first().cloned().unwrap_or_default();
    let Value::Number(pid_number) = pid_value else {
        return Err(invalid_arg_type(realm, "pid", &["number"], &pid_value));
    };
    if !pid_number.is_finite()
        || crate::float::fract(pid_number) != 0.0
        || pid_number <= 0.0
        || pid_number > f64::from(u32::MAX)
    {
        return Err(out_of_range(
            realm,
            "pid",
            "a positive 32-bit integer",
            &Value::Number(pid_number),
        ));
    }
    let pid = pid_number as u32;
    let signal_value = args.get(1).cloned().unwrap_or_default();
    let signal = match signal_value {
        Value::Undefined => 15,
        Value::Number(number) if number.is_finite() && crate::float::fract(number) == 0.0 => {
            let number = number as i32;
            if number == 0 || crate::os_binding::signal_number_by_value(number).is_some() {
                number
            } else {
                return Err(unknown_signal(
                    realm,
                    &crate::number::to_string(number as f64),
                ));
            }
        }
        Value::String(ref name) => {
            let name = name.to_rust_lossy();
            let Some(signal) = crate::os_binding::signal_number(&name) else {
                return Err(unknown_signal(realm, &name));
            };
            signal
        }
        ref value => {
            return Err(invalid_arg_type(
                realm,
                "signal",
                &["string", "number"],
                value,
            ));
        }
    };
    match platform::kill(pid, signal) {
        Ok(()) => Ok(Value::Boolean(true)),
        Err(error) => {
            let errno = if error == 87 {
                -4040 // A stale Windows process handle is UV_ESRCH.
            } else {
                crate::uv_error::translate(error)
            };
            let (code, message) = crate::uv_error::describe(errno);
            let error = node_error(
                realm,
                ErrorKind::Error,
                &code,
                &format!("{message}, kill '{pid}'"),
            );
            if let Value::Object(object) = &error.0 {
                object.define(
                    Key::name("errno"),
                    Property::data(Value::Number(errno as f64)),
                );
                object.define(Key::name("syscall"), Property::data(Value::str("kill")));
                object.define(Key::name("pid"), Property::data(Value::Number(pid as f64)));
            }
            Err(error)
        }
    }
}

/// `process.chdir(path)`: change the process directory and preserve Node's
/// path validation and libuv error details.
fn chdir(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let path = args.first().cloned().unwrap_or_default();
    let Value::String(path) = path else {
        return Err(invalid_arg_type(realm, "path", &["string"], &path));
    };
    match platform::set_current_directory(&path) {
        Ok(()) => Ok(Value::Undefined),
        Err(error) => {
            let errno = crate::uv_error::translate(error);
            let (code, message) = crate::uv_error::describe(errno);
            let error = node_error(
                realm,
                ErrorKind::Error,
                &code,
                &format!("{message}, chdir '{}'", path.to_rust_lossy()),
            );
            if let Value::Object(object) = &error.0 {
                object.define(
                    Key::name("errno"),
                    Property::data(Value::Number(f64::from(errno))),
                );
                object.define(Key::name("syscall"), Property::data(Value::str("chdir")));
                object.define(Key::name("path"), Property::data(Value::String(path)));
            }
            Err(error)
        }
    }
}

/// `process.binding(name)`: expose the native buffer binding used by
/// `safer-buffer`; other private bindings remain unavailable to applications.
fn process_binding(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let name = args.first().cloned().unwrap_or_default();
    let Value::String(name) = name else {
        return Err(invalid_arg_type(realm, "name", &["string"], &name));
    };
    let name = name.to_rust_lossy();
    // The legacy bindings Node still lets `process.binding` reach, of those
    // the runtime has.
    if matches!(
        name.as_str(),
        "buffer" | "config" | "constants" | "fs" | "icu" | "os" | "url" | "util" | "uv" | "zlib"
    ) {
        crate::modules::load(realm, &format!("binding/{name}"))
    } else {
        Err(realm.throw(ErrorKind::Error, &format!("No such binding: {name}")))
    }
}

fn unknown_signal(realm: &mut Realm, signal: &str) -> Thrown {
    node_error(
        realm,
        ErrorKind::Type,
        "ERR_UNKNOWN_SIGNAL",
        &format!("Unknown signal: {signal}"),
    )
}

/// `process.nextTick(callback, ...args)`.
fn next_tick(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let callback = args.first().cloned().unwrap_or_default();
    if !crate::array_builtin::is_callable(&callback) {
        return Err(invalid_arg_type(
            realm,
            "callback",
            &["function"],
            &callback,
        ));
    }
    let args = args[1..].to_vec();
    realm.enqueue_tick(Box::new(move |realm| {
        ops::call(realm, &callback, Value::Undefined, &args).map(drop)
    }));
    Ok(Value::Undefined)
}

/// `process.emitWarning(warning, type | options, code, ctor)`: the warning
/// is emitted on a later tick, and Node's default listener prints it.
fn emit_warning(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let arg = |index: usize| args.get(index).cloned().unwrap_or_default();
    let (warning, mut kind, mut code) = (arg(0), arg(1), arg(2));
    let mut detail = None;
    if matches!(&kind, Value::Object(object)
        if !matches!(object.data().kind, Kind::Array(_)) && ops::type_of(&kind) == "object")
    {
        ops::get_named(realm, &kind, "ctor")?;
        code = ops::get_named(realm, &kind, "code")?;
        if let Value::String(text) = ops::get_named(realm, &kind, "detail")? {
            detail = Some(text);
        }
        let named = ops::get_named(realm, &kind, "type")?;
        kind = if ops::to_boolean(&named) {
            named
        } else {
            Value::str("Warning")
        };
    } else if ops::type_of(&kind) == "function" {
        code = Value::Undefined;
        kind = Value::str("Warning");
    }
    if !matches!(kind, Value::Undefined | Value::String(_)) {
        return Err(invalid_arg_type(realm, "type", &["string"], &kind));
    }
    if ops::type_of(&code) == "function" {
        code = Value::Undefined;
    } else if !matches!(code, Value::Undefined | Value::String(_)) {
        return Err(invalid_arg_type(realm, "code", &["string"], &code));
    }
    let warning = match warning {
        Value::String(text) => {
            let error = Value::Object(realm.error(ErrorKind::Error, &text.to_rust_lossy()));
            let name = if ops::to_boolean(&kind) {
                kind
            } else {
                Value::str("Warning")
            };
            ops::set(realm, &error, &Key::name("name"), name)?;
            if !matches!(code, Value::Undefined) {
                ops::set(realm, &error, &Key::name("code"), code)?;
            }
            if let Some(detail) = detail {
                ops::set(realm, &error, &Key::name("detail"), Value::String(detail))?;
            }
            error
        }
        Value::Object(ref object) if is_error(realm, object) => warning,
        _ => {
            return Err(invalid_arg_type(
                realm,
                "warning",
                &["Error", "string"],
                &warning,
            ));
        }
    };
    let name = ops::get_named(realm, &warning, "name")?;
    let deprecation =
        matches!(&name, Value::String(name) if *name == JsStr::from("DeprecationWarning"));
    if deprecation {
        let process = process(realm)?;
        if ops::to_boolean(&ops::get_named(realm, &process, "noDeprecation")?) {
            return Ok(Value::Undefined);
        }
        if ops::to_boolean(&ops::get_named(realm, &process, "throwDeprecation")?) {
            // Thrown on a later tick, after earlier warnings are printed.
            realm.enqueue_tick(Box::new(move |_| Err(Thrown(warning))));
            return Ok(Value::Undefined);
        }
    }
    realm.enqueue_tick(Box::new(move |realm| {
        match crate::process_events::emit(realm, "warning", core::slice::from_ref(&warning))? {
            Some(_) => Ok(()),
            None => on_warning(realm, &warning),
        }
    }));
    Ok(Value::Undefined)
}

/// `value instanceof Error` for the realm's intrinsic `Error`.
fn is_error(realm: &Realm, object: &Object) -> bool {
    let mut prototype = object.data().prototype.clone();
    while let Some(current) = prototype {
        if current == realm.error_prototype {
            return true;
        }
        prototype = current.data().prototype.clone();
    }
    false
}

/// Node's default `warning` listener, which prints warning errors.
pub(crate) fn on_warning(realm: &mut Realm, warning: &Value) -> Result<(), Thrown> {
    if !warning
        .as_object()
        .is_some_and(|object| is_error(realm, object))
    {
        return Ok(());
    }
    let name = ops::get_named(realm, warning, "name")?;
    let deprecation =
        matches!(&name, Value::String(name) if *name == JsStr::from("DeprecationWarning"));
    let mut text = format!("(node:{}) ", platform::process_id());
    let code = ops::get_named(realm, warning, "code")?;
    if ops::to_boolean(&code) {
        let code = ops::to_string(realm, &code)?;
        text.push_str(&format!("[{code}] "));
    }
    let to_string = ops::get_named(realm, warning, "toString")?;
    let shown = if crate::array_builtin::is_callable(&to_string) {
        ops::call(realm, &to_string, warning.clone(), &[])?
    } else {
        crate::builtins::error_to_string(realm, warning.clone(), &[])?
    };
    text.push_str(&ops::to_string(realm, &shown)?.to_rust_lossy());
    if let Value::String(detail) = ops::get_named(realm, warning, "detail")? {
        text.push('\n');
        text.push_str(&detail.to_rust_lossy());
    }
    if !realm.warning_hint_shown {
        let process = process(realm)?;
        let argv0 = ops::get_named(realm, &process, "argv0")?;
        let argv0 = if ops::to_boolean(&argv0) {
            ops::to_string(realm, &argv0)?.to_rust_lossy()
        } else {
            "node".to_owned()
        };
        let flag = if deprecation {
            "--trace-deprecation"
        } else {
            "--trace-warnings"
        };
        let program = basename(&argv0);
        text.push_str(&format!(
            "\n(Use `{program} {flag} ...` to show where the warning was created)"
        ));
        realm.warning_hint_shown = true;
    }
    text.push('\n');
    platform::write_stderr(&text);
    Ok(())
}

/// A Windows path's last component without a `.exe` extension.
fn basename(path: &str) -> &str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.strip_suffix(".exe") {
        Some(stem) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// `process.umask([mask])`: the mask files are created with. Windows has
/// none; as in Node there, it starts at 0, and setting it returns the old one.
fn umask(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    static MASK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let old = MASK.load(core::sync::atomic::Ordering::Relaxed);
    match args.first() {
        None | Some(Value::Undefined) => {}
        Some(Value::String(text)) => {
            let text = text.to_rust_lossy();
            let Ok(mask) = u32::from_str_radix(&text, 8) else {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "The argument 'mask' must be a 32-bit unsigned integer or an octal string",
                ));
            };
            MASK.store(mask & 0o777, core::sync::atomic::Ordering::Relaxed);
        }
        Some(value) => {
            let mask = ops::to_number(realm, value)?;
            if !(0.0..=4_294_967_295.0).contains(&mask) || f64::from(mask as u32) != mask {
                return Err(realm.throw(
                    ErrorKind::Range,
                    "The value of 'mask' is out of range. It must be >= 0 && <= 4294967295",
                ));
            }
            MASK.store(mask as u32 & 0o777, core::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(Value::Number(f64::from(old)))
}

/// `process.report`: `getReport()` describes the process as Node's
/// diagnostic report header does; reports are not written to files.
fn report(realm: &mut Realm) -> Object {
    let report = realm.object();
    let get_report = realm.function("getReport", 1, false, |realm, _, _| {
        let header = realm.object();
        let process = realm.process.clone();
        let read = |realm: &mut Realm, name: &'static str| match &process {
            Some(process) => ops::get_named(realm, &Value::Object(process.clone()), name),
            None => Ok(Value::Undefined),
        };
        let cwd = cwd(realm);
        let command_line = read(realm, "argv")?;
        let versions = read(realm, "versions")?;
        for (name, value) in [
            ("reportVersion", Value::Number(3.0)),
            ("event", Value::str("JavaScript API")),
            ("trigger", Value::str("GetReport")),
            ("filename", Value::Null),
            (
                "processId",
                Value::Number(f64::from(platform::process_id())),
            ),
            ("cwd", cwd),
            ("commandLine", command_line),
            ("nodejsVersion", Value::str(&format!("v{NODE_VERSION}"))),
            ("wordSize", Value::Number(64.0)),
            ("arch", Value::str("x64")),
            ("platform", Value::str("win32")),
            ("componentVersions", versions),
            ("osName", Value::str("Windows_NT")),
        ] {
            header.define(Key::name(name), Property::data(value));
        }
        let result = realm.object();
        result.define(Key::name("header"), Property::data(Value::Object(header)));
        Ok(Value::Object(result))
    });
    report.define(
        Key::name("getReport"),
        Property::data(Value::Object(get_report)),
    );
    for (name, value) in [
        ("compact", Value::Boolean(false)),
        ("directory", Value::str("")),
        ("filename", Value::str("")),
        ("reportOnFatalError", Value::Boolean(false)),
        ("reportOnSignal", Value::Boolean(false)),
        ("reportOnUncaughtException", Value::Boolean(false)),
        ("excludeNetwork", Value::Boolean(false)),
        ("excludeEnv", Value::Boolean(false)),
        ("signal", Value::str("SIGUSR2")),
    ] {
        report.define(Key::name(name), Property::data(value));
    }
    report
}

/// `process.stdin` (descriptor 0), `process.stdout` (1) or `process.stderr`
/// (2). A windowed application's input has no data to read.
fn stream(realm: &mut Realm, descriptor: u32) -> Completion {
    let index = descriptor as usize;
    if let Some(stream) = &realm.process_streams[index] {
        return Ok(Value::Object(stream.clone()));
    }
    let stream = realm.object();
    if realm.node_library.is_some() {
        // Node's Writable streams inherit from EventEmitter. MarkText installs
        // `error` listeners on both process streams to handle a closed output
        // pipe, so preserve the real EventEmitter behavior on these objects.
        let emitter = crate::modules::load(realm, "node:events")?;
        let prototype = ops::get_named(realm, &emitter, "prototype")?;
        stream.set_prototype(prototype.as_object().cloned());
        ops::call(realm, &emitter, Value::Object(stream.clone()), &[])?;
    }
    stream.define(
        Key::name("fd"),
        Property::data(Value::Number(f64::from(descriptor))),
    );
    stream.define(
        Key::name("destroyed"),
        Property::data(Value::Boolean(false)),
    );
    if descriptor != 0 {
        stream.define(Key::name("writable"), Property::data(Value::Boolean(true)));
    }
    if platform::is_console(descriptor) {
        stream.define(Key::name("isTTY"), Property::data(Value::Boolean(true)));
    }
    if descriptor == 0 {
        // Reading never starts, so these only return the stream.
        for name in [
            "pause",
            "resume",
            "setEncoding",
            "setRawMode",
            "ref",
            "unref",
        ] {
            let method = realm.function(name, 1, false, |_, this, _| Ok(this));
            stream.define(Key::name(name), Property::hidden(Value::Object(method)));
        }
    } else {
        let write = realm.function("write", 3, false, move |realm, _, args| {
            write(realm, descriptor, args)
        });
        stream.define(Key::name("write"), Property::hidden(Value::Object(write)));
    }
    realm.process_streams[index] = Some(stream.clone());
    Ok(Value::Object(stream))
}

/// `stream.write(chunk, encoding, callback)`: a synchronous write whose
/// callback runs on a later tick.
fn write(realm: &mut Realm, descriptor: u32, args: &[Value]) -> Completion {
    let arg = |index: usize| args.get(index).cloned().unwrap_or_default();
    let (chunk, encoding, mut callback) = (arg(0), arg(1), arg(2));
    let mut selected = crate::encoding::Encoding::Utf8;
    if crate::array_builtin::is_callable(&encoding) {
        callback = encoding;
    } else if ops::to_boolean(&encoding) {
        // "buffer" passes the stream's check, but strings cannot be
        // converted from it.
        let known = match &encoding {
            Value::String(name) => match crate::encoding::named(name.units()) {
                Some(named) => {
                    selected = named;
                    true
                }
                None => *name == JsStr::from("buffer") && !matches!(chunk, Value::String(_)),
            },
            _ => false,
        };
        if !known {
            let name = match encoding {
                Value::Symbol(_) => crate::inspect::inspect(realm, &encoding, 0)?,
                _ => ops::to_string(realm, &encoding)?.to_rust_lossy(),
            };
            return Err(node_error(
                realm,
                ErrorKind::Type,
                "ERR_UNKNOWN_ENCODING",
                &format!("Unknown encoding: {name}"),
            ));
        }
    }
    let bytes = match &chunk {
        Value::Null => {
            return Err(node_error(
                realm,
                ErrorKind::Type,
                "ERR_STREAM_NULL_VALUES",
                "May not write null values to stream",
            ));
        }
        Value::String(text) => crate::encoding::encode(text.units(), selected),
        _ => match crate::typed_array_builtin::view_bytes(&chunk) {
            Some(bytes) => bytes,
            None => {
                return Err(invalid_arg_type(
                    realm,
                    "chunk",
                    &["string", "Buffer", "TypedArray", "DataView"],
                    &chunk,
                ));
            }
        },
    };
    platform::write_standard(descriptor, &bytes);
    if crate::array_builtin::is_callable(&callback) {
        realm.enqueue_tick(Box::new(move |realm| {
            ops::call(realm, &callback, Value::Undefined, &[Value::Null]).map(drop)
        }));
    }
    Ok(Value::Boolean(true))
}

/// The same current-directory wrapper function on each read in a realm.
/// Receivers and arguments are ignored, like Node's current-directory function.
pub fn cwd(realm: &mut Realm) -> Value {
    if let Some(function) = &realm.process_cwd {
        return Value::Object(function.clone());
    }
    let function = realm.function("wrappedCwd", 0, true, |realm, _, _| {
        path_value(realm, "uv_cwd", platform::current_directory())
    });
    realm.process_cwd = Some(function.clone());
    Value::Object(function)
}

/// Read the path of the native executable at run time.
pub fn exec_path(realm: &Realm) -> Completion {
    exec_path_text(realm).map(Value::String)
}

/// A path inside the application directory, from one with `/` separators.
pub fn app_relative(realm: &Realm, relative: &str) -> Completion {
    let Value::String(directory) = app_path(realm)? else {
        unreachable!("the application path is a string")
    };
    let relative = relative.replace('/', "\\");
    let path = directory.concat(&JsStr::from(relative.trim_end_matches('\\')));
    let units = path.units();
    // The directory itself, without its trailing separator.
    Ok(Value::String(match units.last() {
        Some(&last) if last == u16::from(b'\\') && units.len() > 1 => {
            JsStr::from_units(&units[..units.len() - 1])
        }
        _ => path,
    }))
}

/// The `file:` URL of a path in the application folder, encoded as Node's
/// `url.pathToFileURL` encodes it: `import.meta.url` of an application
/// module.
pub fn app_relative_url(realm: &Realm, relative: &str) -> Completion {
    let Value::String(path) = app_relative(realm, relative)? else {
        unreachable!("application paths are strings")
    };
    let path = path.to_rust_lossy().replace('\\', "/");
    let unc = path.starts_with("//");
    let mut url = String::from(if unc { "file:" } else { "file:///" });
    for (index, byte) in path.bytes().enumerate() {
        let drive_colon = index == 1 && byte == b':';
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/')
            || drive_colon
        {
            url.push(char::from(byte));
        } else {
            url.push_str(&alloc::format!("%{byte:02X}"));
        }
    }
    Ok(Value::str(&url))
}

/// The application directory, which is the directory containing the native
/// executable for a packaged application.
pub(crate) fn app_path(realm: &Realm) -> Completion {
    let Value::String(executable) = exec_path(realm)? else {
        unreachable!("the executable path is a string")
    };
    let units = executable.units();
    let Some(separator) = units
        .iter()
        .rposition(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
    else {
        return Ok(Value::str("."));
    };
    Ok(Value::String(JsStr::from_units(&units[..=separator])))
}

fn exec_path_text(realm: &Realm) -> Result<JsStr, Thrown> {
    path_text(realm, "uv_exepath", platform::executable_path())
}

/// Packaged native application arguments: executable plus application values,
/// with stable array identity and no JavaScript entry-file element.
pub fn argv(realm: &mut Realm) -> Completion {
    if let Some(arguments) = &realm.process_argv {
        return Ok(Value::Object(arguments.clone()));
    }
    let executable = exec_path(realm)?;
    let values = match &realm.page_arguments {
        Some(page) => core::iter::once(executable)
            .chain(page.iter().map(|argument| Value::str(argument)))
            .collect(),
        None => core::iter::once(executable)
            .chain(
                platform::command_arguments()
                    .into_iter()
                    .skip(1)
                    .map(Value::String),
            )
            .collect(),
    };
    let arguments = realm.array(values);
    realm.process_argv = Some(arguments.clone());
    Ok(Value::Object(arguments))
}

fn path_value(realm: &Realm, syscall: &str, path: Result<JsStr, u32>) -> Completion {
    path_text(realm, syscall, path).map(Value::String)
}

fn path_text(realm: &Realm, syscall: &str, path: Result<JsStr, u32>) -> Result<JsStr, Thrown> {
    match path {
        Ok(path) => Ok(path),
        Err(code) => {
            // Preserve the OS failure as a catchable native Error. Node/libuv
            // errno/code translation is separate platform work.
            let error = realm.error(
                ErrorKind::Error,
                &format!("Windows error {code}, {syscall}"),
            );
            error.define(
                Key::name("win32Code"),
                Property::data(Value::Number(f64::from(code))),
            );
            error.define(Key::name("syscall"), Property::data(Value::str(syscall)));
            Err(Thrown(Value::Object(error)))
        }
    }
}

#[cfg(test)]
mod tests;
