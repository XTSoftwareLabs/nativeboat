//! Native modules: exports objects that compiled programs require in place
//! of module source, created once per realm when first required. Node's
//! internal helpers used by bundled Node library sources live here.

use crate::node_error::{invalid_arg_type, invalid_arg_value, node_error, out_of_range, received};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    Completion, ErrorKind, JsStr, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, ops,
};

/// Native addons the runtime provides, by package and binary: a package's
/// request for its `.node` binary loads the runtime's module of this name
/// (see `native_addons`). The build resolves the same names.
pub const NATIVE_BINDINGS: &[&str] = &[
    "@parcel/watcher/watcher.node",
    "@vscode/deviceid/windows.node",
    "@vscode/native-watchdog/watchdog.node",
    "@vscode/policy-watcher/vscode-policy-watcher.node",
    "@vscode/spdlog/spdlog.node",
    "@vscode/sqlite3/vscode-sqlite3.node",
    "@vscode/windows-mutex/CreateMutex.node",
    "@vscode/windows-process-tree/windows_process_tree.node",
    "@vscode/windows-registry/winregistry.node",
    "native-is-elevated/iselevated.node",
];

/// The exports of the native module `name`. `node:` names are modules of
/// the precompiled Node.js library.
pub fn load(realm: &mut Realm, name: &str) -> Completion {
    if name.starts_with("nativeboat:stylesheet:") {
        if realm.import_native_stylesheet(name).is_none() {
            return Err(realm.throw(
                ErrorKind::Error,
                &format!("native stylesheet '{name}' was not registered in this application"),
            ));
        }
        return Ok(Value::Object(realm.object()));
    }
    if let Some(module) = name.strip_prefix("node:") {
        let Some(require) = realm.node_library else {
            return Err(realm.throw(
                ErrorKind::Type,
                &format!("The Node.js library module '{module}' is not linked"),
            ));
        };
        return require(realm, module);
    }
    if let Some((_, exports)) = realm.native_modules.iter().find(|(known, _)| known == name) {
        return Ok(Value::Object(exports.clone()));
    }
    let exports = match name {
        "child_process" => crate::child_process_builtin::module(realm),
        "ced" => crate::ced::module(realm),
        "keytar" => crate::keytar::module(realm),
        "clip-filepaths" => crate::clip_filepaths::module(realm),
        "native-keymap" => crate::keyboard::module(realm),
        "bindings" => crate::native_addons::bindings(realm),
        "@vscode/windows-ca-certs" => crate::native_addons::network::ca_certificates(realm),
        "@vscode/os-proxy-resolver" => crate::native_addons::network::proxy_resolver(realm),
        "node-pty" => crate::native_addons::pty::module(realm),
        "@vscode/deviceid/windows.node" => crate::native_addons::device_id(realm),
        "@vscode/windows-mutex/CreateMutex.node" => crate::native_addons::windows_mutex(realm),
        "@vscode/policy-watcher/vscode-policy-watcher.node" => {
            crate::native_addons::policy_watcher(realm)
        }
        "@vscode/spdlog/spdlog.node" => crate::native_addons::spdlog::module(realm),
        "@vscode/sqlite3/vscode-sqlite3.node" => crate::native_addons::sqlite3::module(realm),
        "@parcel/watcher/watcher.node" => crate::native_addons::parcel_watcher::module(realm),
        "@vscode/native-watchdog/watchdog.node" => crate::native_addons::watchdog(realm),
        "native-is-elevated/iselevated.node" => crate::native_addons::is_elevated(realm),
        "@vscode/windows-process-tree/windows_process_tree.node" => {
            crate::native_addons::process_tree::module(realm)
        }
        "@vscode/windows-registry/winregistry.node" => {
            crate::native_addons::windows_registry(realm)
        }
        // Node's constants module exposes the Windows filesystem flags used
        // by graceful-fs and other unchanged Node packages.
        "constants" => crate::os_binding::fs_constants_module(realm),
        "tty" => tty(realm),
        // A packaged application runs on the main thread. Node sources use
        // this identity to make their temporary-file names process-unique.
        "worker_threads" => worker_threads(realm),
        "internal/errors" => internal_errors(realm),
        "internal/binding" => internal_binding(realm),
        "internal/options" => options(realm),
        "internal/util/inspect" => inspect(realm),
        "internal/process/task_queues" => task_queues(realm),
        "internal/async_hooks" => async_hooks(realm),
        "binding/buffer" => crate::buffer_binding::binding(realm),
        "binding/blob" => crate::blob_binding::binding(realm),
        "binding/crypto" => crate::crypto_builtin::binding(realm),
        "binding/modules" => modules_binding(realm),
        "binding/protocol" => crate::electron_protocol::binding(realm),
        "binding/fork" => crate::fork::binding(realm),
        "binding/wasm" => crate::wasm_binding::binding(realm),
        "binding/builtins" => builtins_binding(realm),
        "binding/permission" => permission_binding(realm),
        "binding/fs" => crate::fs_binding::binding(realm),
        "binding/fs_dir" => crate::fs_binding::directory_binding(realm),
        "binding/fs_event_wrap" => crate::fs_event_binding::binding(realm),
        "binding/uv" => uv_binding(realm),
        "binding/async_context_frame" => async_context_frame(realm),
        "binding/config" => config(realm),
        "binding/constants" => crate::os_binding::constants(),
        "binding/credentials" => crate::os_binding::credentials(realm),
        "binding/icu" => crate::buffer_binding::icu(realm),
        "binding/encoding_binding" => crate::url_binding::encoding(realm),
        "binding/os" => crate::os_binding::binding(realm),
        "binding/string_decoder" => crate::string_decoder::binding(realm),
        "binding/zlib" => crate::zlib_builtin::binding(realm),
        "binding/stream_wrap" => stream_wrap(realm),
        "binding/tcp" => crate::tcp_binding::binding(realm),
        "binding/pipe" => crate::pipe_binding::binding(realm),
        "binding/message_ports" => crate::realm_ports::binding(realm),
        "binding/http_client" => crate::http_binding::client_binding(realm),
        "binding/websocket" => crate::http_binding::websocket_binding(realm),
        "binding/types" => types(realm),
        "binding/util" => util(realm),
        "binding/trace_events" => trace_events(realm),
        "binding/url" => crate::url_binding::binding(realm),
        _ => {
            if let Some(binding) = name.strip_prefix("binding/") {
                return Err(realm.throw(ErrorKind::Error, &format!("No such binding: {binding}")));
            }
            // Node internal modules are private to Node, but unchanged
            // Electron applications sometimes require them through their
            // bundled Node dependencies. Resolve those names through the
            // linked Node source graph when it is present.
            if name.starts_with("internal/")
                && let Some(require) = realm.node_library
            {
                return require(realm, name);
            }
            return Err(realm.throw(
                ErrorKind::Error,
                &format!("The Node.js library module '{name}' is not compiled yet"),
            ));
        }
    };
    realm
        .native_modules
        .push((name.to_owned(), exports.clone()));
    Ok(Value::Object(exports))
}

/// `worker_threads` of the main thread: no parent port or worker data, and
/// the message port classes of the Node library, loaded when first read.
fn worker_threads(realm: &mut Realm) -> Object {
    let exports = realm.object();
    exports.define(
        Key::name("isMainThread"),
        Property::data(Value::Boolean(true)),
    );
    exports.define(Key::name("threadId"), Property::data(Value::Number(0.0)));
    exports.define(Key::name("parentPort"), Property::data(Value::Null));
    exports.define(Key::name("workerData"), Property::data(Value::Null));
    for name in [
        "BroadcastChannel",
        "MessageChannel",
        "MessagePort",
        "receiveMessageOnPort",
    ] {
        let target = exports.downgrade();
        let get = realm.function("get", 0, false, move |realm, _, _| {
            if realm.node_library.is_none() {
                return Ok(Value::Undefined);
            }
            let library = load(realm, "node:internal/worker/io")?;
            let value = ops::get(realm, &library, &Key::name(name))?;
            if let Some(target) = target.upgrade() {
                target.define(Key::name(name), Property::data(value.clone()));
            }
            Ok(value)
        });
        exports.define(
            Key::name(name),
            Property {
                slot: crate::Slot::Accessor {
                    get: Some(get),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    exports
}

fn tty(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let isatty = realm.function("isatty", 1, false, |realm, _, args| {
        let descriptor = match args.first() {
            Some(Value::Number(value))
                if (0.0..=f64::from(i32::MAX)).contains(value)
                    && *value == f64::from(*value as i32) =>
            {
                *value as i32
            }
            _ => return Ok(Value::Boolean(false)),
        };
        Ok(Value::Boolean(crate::fs_binding::is_tty(realm, descriptor)))
    });
    exports.define(Key::name("isatty"), Property::data(Value::Object(isatty)));
    exports
}

/// The precompiled Node.js library's `require`: `library` evaluates the
/// library's dispatching module once per realm, whose default export maps a
/// module name to that module's exports.
pub fn library(
    realm: &mut Realm,
    library: fn(&mut Realm) -> Result<Object, Thrown>,
    name: &str,
) -> Completion {
    const KEY: &str = "node:*";
    let dispatch = match realm.native_modules.iter().find(|(known, _)| known == KEY) {
        Some((_, dispatch)) => dispatch.clone(),
        None => {
            let namespace = Value::Object(library(realm)?);
            let Value::Object(dispatch) = ops::get_named(realm, &namespace, "default")? else {
                return Err(realm.throw(ErrorKind::Type, "The Node.js library has no dispatcher"));
            };
            realm
                .native_modules
                .push((KEY.to_owned(), dispatch.clone()));
            dispatch
        }
    };
    ops::call(
        realm,
        &Value::Object(dispatch),
        Value::Undefined,
        &[Value::str(name)],
    )
}

/// `internalBinding('builtins')`: `load(name)` gives the exports of a
/// built-in module a `require` made at run time asks for by name: the
/// runtime's native modules, or a module of the Node.js library the program
/// links. Others are not in the program, and fail as Node's missing modules
/// do.
fn builtins_binding(realm: &mut Realm) -> Object {
    let binding = realm.object();
    let load_builtin = realm.function("load", 1, false, |realm, _, args| {
        let requested = text(realm, &arg(args, 0))?;
        let name = requested
            .strip_prefix("node:")
            .unwrap_or(&requested)
            .to_owned();
        let native = matches!(
            name.as_str(),
            "child_process" | "constants" | "tty" | "worker_threads" | "electron"
        );
        let loaded = if native {
            load(realm, &name)
        } else {
            load(realm, &format!("node:{name}"))
        };
        match loaded {
            Ok(Value::Object(exports)) => Ok(Value::Object(exports)),
            _ => {
                let error = realm.error(
                    ErrorKind::Error,
                    &format!(
                        "Cannot find module '{requested}': it is not compiled into this program"
                    ),
                );
                error.define(
                    Key::name("code"),
                    Property::data(Value::str("MODULE_NOT_FOUND")),
                );
                Err(Thrown(Value::Object(error)))
            }
        }
    });
    binding.define(
        Key::name("load"),
        Property::data(Value::Object(load_builtin)),
    );
    binding
}

/// `internalBinding(name)`, which Node's internal modules receive from
/// their module wrapper.
fn internal_binding(realm: &mut Realm) -> Object {
    realm.function("internalBinding", 1, false, |realm, _, args| {
        let name = text(realm, &arg(args, 0))?;
        load(realm, &format!("binding/{name}"))
    })
}

/// `internal/options`: command-line options have their defaults, as a
/// packaged application has no Node.js options.
fn options(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let get = realm.function("getOptionValue", 1, false, |_, _, _| {
        Ok(Value::Boolean(false))
    });
    exports.define(
        Key::name("getOptionValue"),
        Property::data(Value::Object(get)),
    );
    exports
}

/// `internalBinding('async_context_frame')`: reads and replaces the async
/// context frame, with the names of V8's embedder API Node uses.
fn async_context_frame(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let get = realm.function(
        "getContinuationPreservedEmbedderData",
        0,
        false,
        |realm, _, _| Ok(realm.async_context.clone()),
    );
    let set = realm.function(
        "setContinuationPreservedEmbedderData",
        1,
        false,
        |realm, _, args| {
            realm.async_context = arg(args, 0);
            Ok(Value::Undefined)
        },
    );
    for (name, function) in [
        ("getContinuationPreservedEmbedderData", get),
        ("setContinuationPreservedEmbedderData", set),
    ] {
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

/// `internalBinding('uv')`: libuv's error codes as `UV_` constants, with
/// `errname` and `getErrorMessage`, as packages such as execa read them.
fn uv_binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    for (code, name, _) in crate::uv_error::ERRORS {
        exports.define(
            Key::name(&format!("UV_{name}")),
            Property::data(Value::Number(f64::from(*code))),
        );
    }
    for (name, message) in [("errname", false), ("getErrorMessage", true)] {
        let function = realm.function(name, 1, false, move |realm, _, args| {
            let code = ops::to_int32(realm, &arg(args, 0))?;
            let (name, text) = crate::uv_error::describe(code);
            Ok(Value::str(if message { &text } else { &name }))
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

fn async_hooks(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let symbols = realm.object();
    symbols.define(
        Key::name("owner_symbol"),
        Property::data(Value::Symbol(crate::Symbol::new(Some(JsStr::from(
            "owner_symbol",
        ))))),
    );
    exports.define(Key::name("symbols"), Property::data(Value::Object(symbols)));
    let scope = realm.function("defaultTriggerAsyncIdScope", 2, false, |realm, _, args| {
        // Node calls the block immediately. In particular, fs watcher close
        // passes process.nextTick as the block; adding a tick here would
        // reorder close/stop delivery relative to other pending callbacks.
        ops::call(
            realm,
            &arg(args, 1),
            Value::Null,
            args.get(2..).unwrap_or_default(),
        )
    });
    exports.define(
        Key::name("defaultTriggerAsyncIdScope"),
        Property::data(Value::Object(scope)),
    );
    exports
}

/// With the packaged runtime's permission option disabled, every supported
/// filesystem permission check succeeds, as Node does without --permission.
fn permission_binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let has = realm.function("has", 2, false, |_, _, _| Ok(Value::Boolean(true)));
    exports.define(Key::name("has"), Property::data(Value::Object(has)));
    exports
}

/// `internal/util/inspect`: `inspect` is the runtime's inspection; options
/// other than `depth` are not supported.
fn inspect(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let inspect = realm.function("inspect", 2, false, |realm, _, args| {
        let options = arg(args, 1);
        let depth = match &options {
            Value::Object(_) => ops::get_named(realm, &options, "depth")?,
            _ => Value::Undefined,
        };
        let text = match depth {
            // Infinity saturates to the largest depth.
            Value::Number(depth) if depth >= 0.0 => {
                crate::inspect::inspect(realm, &arg(args, 0), depth as usize)?
            }
            Value::Number(depth) if depth < 0.0 => {
                crate::inspect::inspect_shallow(realm, &arg(args, 0))?
            }
            _ => crate::inspect::inspect(realm, &arg(args, 0), crate::inspect::DEPTH)?,
        };
        Ok(Value::str(&text))
    });
    inspect.define(
        Key::name("custom"),
        Property::data(Value::Symbol(crate::symbol_builtin::intern(
            realm,
            crate::JsStr::from("nodejs.util.inspect.custom"),
        ))),
    );
    exports.define(Key::name("inspect"), Property::data(Value::Object(inspect)));
    let format = realm.function("format", 0, false, |realm, _, args| {
        Ok(Value::str(&crate::inspect::format(realm, args)?))
    });
    exports.define(Key::name("format"), Property::data(Value::Object(format)));
    let format_with_options = realm.function("formatWithOptions", 1, false, |realm, _, args| {
        let options = arg(args, 0);
        let depth = match options {
            Value::Object(_) => ops::get_named(realm, &options, "depth")?,
            _ => Value::Undefined,
        };
        let text = match depth {
            Value::Number(depth) if depth.is_finite() && depth >= 0.0 => {
                crate::inspect::format_with_depth(
                    realm,
                    &args[1.min(args.len())..],
                    depth as usize,
                )?
            }
            _ => crate::inspect::format(realm, &args[1.min(args.len())..])?,
        };
        Ok(Value::str(&text))
    });
    exports.define(
        Key::name("formatWithOptions"),
        Property::data(Value::Object(format_with_options)),
    );
    let strip_vt = realm.function("stripVTControlCharacters", 1, false, |realm, _, args| {
        let text = ops::to_string(realm, &arg(args, 0))?;
        Ok(Value::String(crate::inspect::strip_vt_control_characters(
            &text,
        )))
    });
    exports.define(
        Key::name("stripVTControlCharacters"),
        Property::data(Value::Object(strip_vt)),
    );
    let range = realm.function("identicalSequenceRange", 2, false, identical_sequence_range);
    exports.define(
        Key::name("identicalSequenceRange"),
        Property::data(Value::Object(range)),
    );
    exports
}

/// Node's debuglog formatter uses trace events only when its trace-category
/// byte is enabled. Packaged nativeboat applications do not accept Node's
/// tracing command-line options, so categories stay disabled; the log path
/// itself remains available through the unchanged Node debuglog source.
fn trace_events(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let enabled = realm.function("getCategoryEnabledBuffer", 1, false, |realm, _, _| {
        Ok(Value::Object(realm.array(alloc::vec![Value::Number(0.0)])))
    });
    let trace = realm.function("trace", 4, false, |_, _, _| Ok(Value::Undefined));
    exports.define(
        Key::name("getCategoryEnabledBuffer"),
        Property::data(Value::Object(enabled)),
    );
    exports.define(Key::name("trace"), Property::data(Value::Object(trace)));
    exports
}

/// The web-streams queuing strategies use only Node's custom inspection
/// formatter from `internal/webstreams/util`.
/// `internalBinding('stream_wrap')`: the stream base state slots Node's web
/// stream adapters name. Applications have no stream base handles.
fn stream_wrap(realm: &mut Realm) -> Object {
    let exports = realm.object();
    for (index, name) in [
        "kReadBytesOrError",
        "kArrayBufferOffset",
        "kBytesWritten",
        "kLastWriteWasAsync",
    ]
    .into_iter()
    .enumerate()
    {
        exports.define(Key::name(name), Property::data(Value::Number(index as f64)));
    }
    let state = crate::typed_array_builtin::new_bytes(realm, alloc::vec![0; 5 * 4], None);
    exports.define(
        Key::name("streamBaseState"),
        Property::data(Value::Object(state)),
    );
    exports
}

/// The filesystem Blob source uses only the native microtask enqueue API
/// from Node's task-queue module. Realm checkpoints drain this queue directly.
fn task_queues(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let queue = realm.function("queueMicrotask", 1, false, |realm, _, args| {
        let callback = arg(args, 0);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(invalid_arg_type(
                realm,
                "callback",
                &["Function"],
                &callback,
            ));
        }
        let native = ops::get(
            realm,
            &Value::Object(realm.global.clone()),
            &Key::name("queueMicrotask"),
        )?;
        ops::call(realm, &native, Value::Undefined, &[callback])
    });
    exports.define(
        Key::name("queueMicrotask"),
        Property::data(Value::Object(queue)),
    );
    exports
}

/// Node's `identicalSequenceRange(a, b)`: the first run of more than three
/// equal consecutive entries of `a` that also appear consecutively in `b`.
fn identical_sequence_range(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let a = crate::iter::collect(realm, &arg(args, 0))?;
    let b = crate::iter::collect(realm, &arg(args, 1))?;
    let (found_length, found_offset) = (0..a.len().saturating_sub(3))
        .find_map(|i| {
            let position = b
                .iter()
                .position(|value| ops::strict_equals(value, &a[i]))?;
            let rest = b.len() - position;
            // `i` leaves at least four entries of `a`, so `maximum > 1`.
            let maximum = (a.len() - i).min(rest);
            let length = (1..maximum)
                .find(|&k| !ops::strict_equals(&a[i + k], &b[position + k]))
                .unwrap_or(maximum);
            (rest > 3 && length > 3).then_some((length, i))
        })
        .unwrap_or((0, 0));
    let result = realm.object();
    result.define(
        Key::name("len"),
        Property::data(Value::Number(found_length as f64)),
    );
    result.define(
        Key::name("offset"),
        Property::data(Value::Number(found_offset as f64)),
    );
    Ok(Value::Object(result))
}

/// `internalBinding('config')`: the `icu` binding provides `transcode`
/// for the encodings Node's buffers use; no other ICU data is bundled.
fn config(realm: &mut Realm) -> Object {
    let exports = realm.object();
    exports.define(Key::name("hasIntl"), Property::data(Value::Boolean(true)));
    exports
}

/// `internalBinding('util')`: property filters, own non-index keys and the
/// private symbols the library uses.
fn util(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let protected = realm.function("getProtectedRegExp", 0, false, |realm, _, _| {
        Ok(Value::Object(
            realm
                .protected_regexp_constructor
                .as_ref()
                .expect("protected RegExp is installed at bootstrap")
                .clone(),
        ))
    });
    exports.define(
        Key::name("getProtectedRegExp"),
        Property::data(Value::Object(protected)),
    );
    let constants = realm.object();
    for (name, value) in [
        ("ALL_PROPERTIES", 0.0),
        ("ONLY_WRITABLE", 1.0),
        ("ONLY_ENUMERABLE", 2.0),
        ("ONLY_CONFIGURABLE", 4.0),
        ("SKIP_STRINGS", 8.0),
        ("SKIP_SYMBOLS", 16.0),
        ("kDisallowCloneAndTransfer", 0.0),
        ("kTransferable", 1.0),
        ("kCloneable", 2.0),
        ("kPending", 0.0),
        ("kFulfilled", 1.0),
        ("kRejected", 2.0),
    ] {
        constants.define(Key::name(name), Property::data(Value::Number(value)));
    }
    exports.define(
        Key::name("constants"),
        Property::data(Value::Object(constants)),
    );
    // A promise's state and result: [kPending], or [kFulfilled or kRejected,
    // result]; undefined for other values.
    let details = realm.function("getPromiseDetails", 1, false, |realm, _, args| {
        let Some(object) = args.first().and_then(Value::as_object) else {
            return Ok(Value::Undefined);
        };
        let state = match &object.data().kind {
            Kind::Promise(state) => state.borrow().state.clone(),
            _ => return Ok(Value::Undefined),
        };
        let entries = match state {
            crate::promise_builtin::State::Pending => vec![Value::Number(0.0)],
            crate::promise_builtin::State::Fulfilled(value) => vec![Value::Number(1.0), value],
            crate::promise_builtin::State::Rejected(value) => vec![Value::Number(2.0), value],
        };
        Ok(Value::Object(realm.array(entries)))
    });
    exports.define(
        Key::name("getPromiseDetails"),
        Property::data(Value::Object(details)),
    );
    let keys = realm.function("getOwnNonIndexProperties", 2, false, |realm, _, args| {
        let Value::Object(object) = arg(args, 0) else {
            return Ok(Value::Object(realm.array(Vec::new())));
        };
        let filter = ops::to_uint32(realm, &arg(args, 1))?;
        let mut names = Vec::new();
        for key in object.own_keys() {
            let enumerable = object
                .own_property(&key)
                .is_some_and(|property| property.enumerable);
            let symbol = matches!(key, Key::Symbol(_));
            let skipped = matches!(key, Key::Index(_))
                || (filter & 2 != 0 && !enumerable)
                || (filter & 8 != 0 && !symbol)
                || (filter & 16 != 0 && symbol);
            if !skipped {
                names.push(match key {
                    Key::Symbol(symbol) => Value::Symbol(symbol),
                    key => Value::String(key.to_js()),
                });
            }
        }
        Ok(Value::Object(realm.array(names)))
    });
    exports.define(
        Key::name("getOwnNonIndexProperties"),
        Property::data(Value::Object(keys)),
    );
    let lazy = realm.function("defineLazyProperties", 4, false, define_lazy_properties);
    exports.define(
        Key::name("defineLazyProperties"),
        Property::data(Value::Object(lazy)),
    );
    // Call sites cannot be inspected without stacks: deprecation warnings
    // meant for application code are treated as coming from dependencies.
    let inside = realm.function("isInsideNodeModules", 2, false, |_, _, _| {
        Ok(Value::Boolean(true))
    });
    exports.define(
        Key::name("isInsideNodeModules"),
        Property::data(Value::Object(inside)),
    );
    let symbols = realm.object();
    for (name, description) in [
        (
            "untransferable_object_private_symbol",
            "node:untransferableObject",
        ),
        ("transfer_mode_private_symbol", "node:transfer_mode"),
    ] {
        symbols.define(
            Key::name(name),
            Property::data(Value::Symbol(crate::symbol_builtin::private_intern(
                realm,
                description,
            ))),
        );
    }
    exports.define(
        Key::name("privateSymbols"),
        Property::data(Value::Object(symbols)),
    );
    exports
}

/// `defineLazyProperties(target, id, keys, enumerable = true)`: properties
/// whose first read loads module `id` and replaces them with its export.
fn define_lazy_properties(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let Value::Object(target) = arg(args, 0) else {
        return Ok(Value::Undefined);
    };
    let id = text(realm, &arg(args, 1))?;
    let enumerable = !matches!(arg(args, 3), Value::Boolean(false));
    for key in crate::iter::collect(realm, &arg(args, 2))? {
        let key = ops::to_key(realm, &key)?;
        let (load_id, load_key) = (id.clone(), key.clone());
        let get = realm.function("get", 0, false, move |realm, this, _| {
            let module = load(realm, &load_id)?;
            let value = ops::get(realm, &module, &load_key)?;
            if let Value::Object(this) = this {
                this.define(load_key.clone(), Property::data(value.clone()));
            }
            Ok(value)
        });
        let set_key = key.clone();
        let set = realm.function("set", 1, false, move |_, this, args| {
            if let Value::Object(this) = this {
                this.define(set_key.clone(), Property::data(arg(args, 0)));
            }
            Ok(Value::Undefined)
        });
        target.define(
            key,
            Property {
                slot: crate::Slot::Accessor {
                    get: Some(get),
                    set: Some(set),
                },
                enumerable,
                configurable: true,
            },
        );
    }
    Ok(Value::Undefined)
}

/// Node's globals that come from library modules: `Buffer` (an accessor,
/// as in Node), `atob`, `btoa`, `URL`, `Blob`, `DOMException`, the Fetch API,
/// the web stream classes and the message port classes. Each loads its module when first read and
/// becomes a data property; without the library they are undefined.
pub(crate) fn install(realm: &Realm) {
    const STREAMS: &str = "node:stream/web";
    const PORTS: &str = "node:internal/worker/io";
    for (name, module, enumerable) in [
        ("Buffer", "node:buffer", false),
        ("atob", "node:buffer", true),
        ("btoa", "node:buffer", true),
        ("Blob", "node:buffer", false),
        ("File", "node:buffer", false),
        ("URLSearchParams", "node:url", false),
        ("URL", "node:url", false),
        ("fetch", "node:internal/fetch", true),
        ("FormData", "node:internal/fetch", false),
        ("Headers", "node:internal/fetch", false),
        ("Request", "node:internal/fetch", false),
        ("Response", "node:internal/fetch", false),
        ("WebAssembly", "node:internal/webassembly", false),
        ("WebSocket", "node:internal/websocket", false),
        ("navigator", "node:internal/navigator", false),
        (
            "DOMException",
            "node:internal/per_context/domexception",
            false,
        ),
        ("ReadableStream", STREAMS, false),
        ("ReadableStreamDefaultReader", STREAMS, false),
        ("ReadableStreamBYOBReader", STREAMS, false),
        ("ReadableStreamBYOBRequest", STREAMS, false),
        ("ReadableByteStreamController", STREAMS, false),
        ("ReadableStreamDefaultController", STREAMS, false),
        ("TransformStream", STREAMS, false),
        ("TransformStreamDefaultController", STREAMS, false),
        ("WritableStream", STREAMS, false),
        ("WritableStreamDefaultWriter", STREAMS, false),
        ("WritableStreamDefaultController", STREAMS, false),
        ("ByteLengthQueuingStrategy", STREAMS, false),
        ("CountQueuingStrategy", STREAMS, false),
        ("TextEncoderStream", STREAMS, false),
        ("TextDecoderStream", STREAMS, false),
        ("CompressionStream", STREAMS, false),
        ("DecompressionStream", STREAMS, false),
        ("MessageChannel", PORTS, false),
        ("MessagePort", PORTS, false),
        ("MessageEvent", PORTS, false),
        ("BroadcastChannel", PORTS, false),
    ] {
        let get = realm.function("get", 0, false, move |realm, _, _| {
            if realm.node_library.is_none() {
                return Ok(Value::Undefined);
            }
            let exports = load(realm, module)?;
            let value = ops::get(realm, &exports, &Key::name(name))?;
            realm.global.define(
                Key::name(name),
                Property {
                    slot: crate::Slot::Data {
                        value: value.clone(),
                        writable: true,
                    },
                    enumerable,
                    configurable: true,
                },
            );
            Ok(value)
        });
        let set = realm.function("set", 1, false, move |realm, _, args| {
            realm.global.define(
                Key::name(name),
                Property {
                    slot: crate::Slot::Data {
                        value: arg(args, 0),
                        writable: true,
                    },
                    enumerable,
                    configurable: true,
                },
            );
            Ok(Value::Undefined)
        });
        realm.global.define(
            Key::name(name),
            Property {
                slot: crate::Slot::Accessor {
                    get: Some(get),
                    set: Some(set),
                },
                enumerable,
                configurable: true,
            },
        );
    }
}

/// A global that comes from a library module, as compiled code reads it;
/// the module name links the library into the application.
pub fn global(realm: &mut Realm, module: &str, name: &str) -> Completion {
    let _ = module;
    let global = Value::Object(realm.global.clone());
    ops::get(realm, &global, &Key::name(name))
}

/// A brand check on an object's kind.
type Check = fn(&Kind) -> bool;

/// `internalBinding('types')`: brand checks on native objects.
fn types(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let checks: [(&str, Check); 20] = [
        ("isArgumentsObject", |kind| matches!(kind, Kind::Arguments)),
        ("isArrayBuffer", |kind| matches!(kind, Kind::ArrayBuffer(_))),
        ("isAnyArrayBuffer", |kind| {
            matches!(kind, Kind::ArrayBuffer(_))
        }),
        ("isBooleanObject", |kind| {
            matches!(kind, Kind::Primitive(crate::object::Wrapped::Boolean(_)))
        }),
        ("isBoxedPrimitive", |kind| {
            matches!(kind, Kind::Primitive(_) | Kind::SymbolObject(_))
        }),
        ("isDataView", |kind| matches!(kind, Kind::DataView(_))),
        ("isDate", |kind| matches!(kind, Kind::Date(_))),
        ("isGeneratorObject", |kind| {
            matches!(kind, Kind::Generator(_))
        }),
        (
            "isMap",
            |kind| matches!(kind, Kind::Collection(collection) if !collection.set),
        ),
        (
            "isMapIterator",
            |kind| matches!(kind, Kind::CollectionIterator(iterator) if !iterator.set),
        ),
        ("isNativeError", |kind| matches!(kind, Kind::Error)),
        ("isNumberObject", |kind| {
            matches!(kind, Kind::Primitive(crate::object::Wrapped::Number(_)))
        }),
        ("isPromise", |kind| matches!(kind, Kind::Promise(_))),
        ("isRegExp", |kind| matches!(kind, Kind::RegExp(_))),
        (
            "isSet",
            |kind| matches!(kind, Kind::Collection(collection) if collection.set),
        ),
        (
            "isSetIterator",
            |kind| matches!(kind, Kind::CollectionIterator(iterator) if iterator.set),
        ),
        ("isStringObject", |kind| {
            matches!(kind, Kind::Primitive(crate::object::Wrapped::String(_)))
        }),
        ("isSymbolObject", |kind| {
            matches!(kind, Kind::SymbolObject(_))
        }),
        (
            "isWeakMap",
            |kind| matches!(kind, Kind::WeakCollection(collection) if !collection.set),
        ),
        (
            "isWeakSet",
            |kind| matches!(kind, Kind::WeakCollection(collection) if collection.set),
        ),
    ];
    for (name, check) in checks {
        let function = realm.function(name, 1, false, move |_, _, args| {
            Ok(Value::Boolean(
                matches!(arg(args, 0), Value::Object(object) if check(&object.data().kind)),
            ))
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    // Objects these runtime types never create.
    for name in [
        "isAsyncFunction",
        "isBigIntObject",
        "isExternal",
        "isGeneratorFunction",
        "isModuleNamespaceObject",
        "isProxy",
        "isSharedArrayBuffer",
    ] {
        let function = realm.function(name, 1, false, |_, _, _| Ok(Value::Boolean(false)));
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn text(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    Ok(ops::to_string(realm, value)?.to_rust_lossy())
}

/// `internal/errors`: `codes` with Node's error classes for argument
/// validation, and `hideStackFrames`, which has no stack to trim here.
fn internal_errors(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let codes = realm.object();
    let classes: [(&str, Build); 24] = [
        ("ERR_BUFFER_OUT_OF_BOUNDS", out_of_bounds),
        ("ERR_INVALID_ARG_TYPE", invalid_type),
        ("ERR_INVALID_ARG_VALUE", invalid_value),
        ("ERR_INVALID_BUFFER_SIZE", |realm, args| {
            let size = formatted(realm, &arg(args, 0))?;
            let message = format!("Buffer size must be a multiple of {size}");
            Ok(node_error(
                realm,
                ErrorKind::Range,
                "ERR_INVALID_BUFFER_SIZE",
                &message,
            ))
        }),
        ("ERR_ARG_NOT_ITERABLE", |realm, args| {
            let name = formatted(realm, &arg(args, 0))?;
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_ARG_NOT_ITERABLE",
                &format!("{name} must be iterable"),
            ))
        }),
        ("ERR_INVALID_TUPLE", |realm, args| {
            let name = formatted(realm, &arg(args, 0))?;
            let tuple = formatted(realm, &arg(args, 1))?;
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INVALID_TUPLE",
                &format!("{name} must be an iterable {tuple} tuple"),
            ))
        }),
        ("ERR_INVALID_THIS", |realm, args| {
            let kind = formatted(realm, &arg(args, 0))?;
            let message = format!("Value of \"this\" must be of type {kind}");
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INVALID_THIS",
                &message,
            ))
        }),
        ("ERR_INVALID_URL", |realm, args| {
            let object =
                crate::node_error::object(realm, ErrorKind::Type, "ERR_INVALID_URL", "Invalid URL");
            object.define(Key::name("input"), Property::data(arg(args, 0)));
            let base = arg(args, 1);
            if !matches!(base, Value::Undefined | Value::Null) {
                object.define(Key::name("base"), Property::data(base));
            }
            Ok(Thrown(Value::Object(object)))
        }),
        ("ERR_INVALID_FILE_URL_HOST", |realm, args| {
            let platform = formatted(realm, &arg(args, 0))?;
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INVALID_FILE_URL_HOST",
                &format!("File URL host must be \"localhost\" or empty on {platform}"),
            ))
        }),
        ("ERR_INVALID_FILE_URL_PATH", |realm, args| {
            let reason = formatted(realm, &arg(args, 0))?;
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INVALID_FILE_URL_PATH",
                &format!("File URL path {reason}"),
            ))
        }),
        ("ERR_INVALID_URL_SCHEME", |realm, args| {
            let expected = arg(args, 0);
            let expected = if matches!(expected, Value::String(_)) {
                vec![expected]
            } else {
                crate::iter::collect(realm, &expected)?
            };
            if expected.len() > 2 {
                return Err(node_error(
                    realm,
                    ErrorKind::Error,
                    "ERR_INTERNAL_ASSERTION",
                    "This is caused by either a bug in Node.js or incorrect usage of Node.js internals.\nPlease open an issue with this stack trace at https://github.com/nodejs/node/issues\n",
                ));
            }
            let first = text(realm, &arg(&expected, 0))?;
            let suffix = if expected.len() == 2 {
                format!("one of scheme {first} or {}", text(realm, &expected[1])?)
            } else {
                format!("of scheme {first}")
            };
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INVALID_URL_SCHEME",
                &format!("The URL must be {suffix}"),
            ))
        }),
        ("ERR_NO_CRYPTO", |realm, _| {
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_NO_CRYPTO",
                "Node.js is not compiled with OpenSSL crypto support",
            ))
        }),
        ("ERR_INTERNAL_ASSERTION", |realm, args| {
            let suffix = "This is caused by either a bug in Node.js or incorrect usage of Node.js internals.\nPlease open an issue with this stack trace at https://github.com/nodejs/node/issues\n";
            let message = match arg(args, 0) {
                Value::Undefined => suffix.to_owned(),
                value => format!("{}\n{suffix}", text(realm, &value)?),
            };
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_INTERNAL_ASSERTION",
                &message,
            ))
        }),
        ("ERR_FS_EISDIR", |realm, args| {
            system_error_kind(realm, args, "ERR_FS_EISDIR", "Path is a directory")
        }),
        ("ERR_FS_INVALID_SYMLINK_TYPE", |realm, args| {
            let kind = formatted(realm, &arg(args, 0))?;
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_FS_INVALID_SYMLINK_TYPE",
                &format!(
                    "Symlink type must be one of \"dir\", \"file\", or \"junction\". Received \"{kind}\""
                ),
            ))
        }),
        ("ERR_INCOMPATIBLE_OPTION_PAIR", |realm, args| {
            let first = formatted(realm, &arg(args, 0))?;
            let second = formatted(realm, &arg(args, 1))?;
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_INCOMPATIBLE_OPTION_PAIR",
                &format!(
                    "Option \"{first}\" cannot be used in combination with option \"{second}\""
                ),
            ))
        }),
        ("ERR_MISSING_ARGS", missing_args),
        ("ERR_DIR_CLOSED", |realm, _| {
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_DIR_CLOSED",
                "Directory handle was closed",
            ))
        }),
        ("ERR_DIR_CONCURRENT_OPERATION", |realm, _| {
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_DIR_CONCURRENT_OPERATION",
                "Cannot do synchronous work on directory handle with concurrent asynchronous operations",
            ))
        }),
        ("ERR_INVALID_URI", |realm, _| {
            Ok(node_error(
                realm,
                ErrorKind::Uri,
                "ERR_INVALID_URI",
                "URI malformed",
            ))
        }),
        ("ERR_OUT_OF_RANGE", range),
        ("ERR_SYSTEM_ERROR", system_error),
        ("ERR_UNHANDLED_ERROR", |realm, args| {
            let message = match arg(args, 0) {
                Value::Undefined => "Unhandled error.".to_owned(),
                error => format!("Unhandled error. ({})", text(realm, &error)?),
            };
            Ok(node_error(
                realm,
                ErrorKind::Error,
                "ERR_UNHANDLED_ERROR",
                &message,
            ))
        }),
        ("ERR_UNKNOWN_ENCODING", |realm, args| {
            let encoding = formatted(realm, &arg(args, 0))?;
            let message = format!("Unknown encoding: {encoding}");
            Ok(node_error(
                realm,
                ErrorKind::Type,
                "ERR_UNKNOWN_ENCODING",
                &message,
            ))
        }),
    ];
    for (code, build) in classes {
        // Construction returns the new error; argument conversions may throw.
        let required = match code {
            "ERR_INTERNAL_ASSERTION" | "ERR_FS_INVALID_SYMLINK_TYPE" => 1,
            "ERR_INCOMPATIBLE_OPTION_PAIR" => 2,
            _ => 0,
        };
        let class = realm.function("NodeError", 0, true, move |realm, _, args| {
            if args.len() < required {
                return Err(node_error(realm, ErrorKind::Error, "ERR_INTERNAL_ASSERTION",
                    &format!("Code: {code}; The provided arguments length ({}) does not match the required ones ({required}).\nThis is caused by either a bug in Node.js or incorrect usage of Node.js internals.\nPlease open an issue with this stack trace at https://github.com/nodejs/node/issues\n",args.len())));
            }
            build(realm, args).map(|Thrown(error)| error)
        });
        // Validators throw the variant that hides their stack frames.
        class.define(
            Key::name("HideStackFramesError"),
            Property::data(Value::Object(class.clone())),
        );
        codes.define(Key::name(code), Property::data(Value::Object(class)));
    }
    crate::node_error_table::install(realm, &codes);
    exports.define(Key::name("codes"), Property::data(Value::Object(codes)));
    let hide = realm.function("hideStackFrames", 1, false, |realm, _, args| {
        let function = arg(args, 0);
        let wrapper = {
            let function = function.clone();
            realm.function("wrappedFn", 0, false, move |realm, this, args| {
                ops::call(realm, &function, this, args)
            })
        };
        wrapper.define(Key::name("withoutStackTrace"), Property::data(function));
        Ok(Value::Object(wrapper))
    });
    exports.define(
        Key::name("hideStackFrames"),
        Property::data(Value::Object(hide)),
    );
    exports.define(
        Key::name("AbortError"),
        Property::data(Value::Object(abort_error(realm))),
    );
    let uv_map = uv_error_map(realm);
    let uv = uv_exception(realm, uv_map.clone());
    exports.define(
        Key::name("uvErrmapGet"),
        Property::data(Value::Object(uv_map)),
    );
    exports.define(Key::name("UVException"), Property::data(Value::Object(uv)));
    let enhance = crate::Symbol::new(Some(crate::JsStr::from("kEnhanceStackBeforeInspector")));
    exports.define(
        Key::name("kEnhanceStackBeforeInspector"),
        Property::data(Value::Symbol(enhance)),
    );
    let generic = realm.function("genericNodeError", 2, false, |realm, _, args| {
        let message = text(realm, &arg(args, 0))?;
        let error = Value::Object(realm.error(ErrorKind::Error, &message));
        let properties = arg(args, 1);
        if let Value::Object(object) = &properties {
            for key in object.own_keys() {
                if object
                    .own_property(&key)
                    .is_some_and(|property| property.enumerable)
                {
                    let value = ops::get(realm, &properties, &key)?;
                    ops::set(realm, &error, &key, value)?;
                }
            }
        }
        Ok(error)
    });
    exports.define(
        Key::name("genericNodeError"),
        Property::data(Value::Object(generic)),
    );
    exports
}

/// Node's lazy error map retains row identity and observable row mutations.
fn uv_error_map(realm: &Realm) -> Object {
    let rows = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::<(i32, Object)>::new()));
    realm.function("uvErrmapGet", 1, false, move |realm, _, args| {
        let Value::Number(number) = arg(args, 0) else {
            return Ok(Value::Undefined);
        };
        let Some((name, message)) = crate::uv_error::mapped(number) else {
            return Ok(Value::Undefined);
        };
        let code = number as i32;
        if let Some((_, row)) = rows.borrow().iter().find(|(known, _)| *known == code) {
            return Ok(Value::Object(row.clone()));
        }
        let row = realm.array(vec![Value::str(name), Value::str(message)]);
        rows.borrow_mut().push((code, row.clone()));
        Ok(Value::Object(row))
    })
}

fn uv_path(realm: &mut Realm, context: &Value, name: &str) -> Result<Option<Value>, Thrown> {
    if !ops::to_boolean(&ops::get(realm, context, &Key::name(name))?) {
        return Ok(None);
    }
    // The upstream class reads the property twice and calls its toString
    // method; this differs from an abstract string conversion.
    let path = ops::get(realm, context, &Key::name(name))?;
    let method = ops::get_named(realm, &path, "toString")?;
    ops::call(realm, &method, path, &[]).map(Some)
}

/// Node 22's UVException, preserving context getter/setter and conversion order.
fn uv_exception(realm: &Realm, error_map: Object) -> Object {
    let prototype = Object::new(Some(realm.error_prototype.clone()), Kind::Ordinary);
    let error_constructor = realm
        .error_constructor
        .clone()
        .expect("errors are installed");
    let function = realm.builtin_constructor(
        "UVException",
        1,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Class constructor UVException cannot be invoked without 'new'",
            ))
        },
        move |realm, target, args| {
            let context = arg(args, 0);
            let errno = ops::get_named(realm, &context, "errno")?;
            let row = ops::call(
                realm,
                &Value::Object(error_map.clone()),
                Value::Undefined,
                &[errno],
            )?;
            let (code, uv_message) = if row.is_nullish() {
                (Value::str("UNKNOWN"), Value::str("unknown error"))
            } else {
                (
                    ops::get(realm, &row, &Key::Index(0))?,
                    ops::get(realm, &row, &Key::Index(1))?,
                )
            };
            let code_text = ops::to_string(realm, &code)?;
            let provided = ops::get_named(realm, &context, "message")?;
            let detail = if ops::to_boolean(&provided) {
                provided
            } else {
                uv_message
            };
            let detail = ops::to_string(realm, &detail)?;
            let syscall = ops::get_named(realm, &context, "syscall")?;
            let syscall = ops::to_string(realm, &syscall)?;
            let mut message = code_text
                .concat(&crate::JsStr::from(": "))
                .concat(&detail)
                .concat(&crate::JsStr::from(", "))
                .concat(&syscall);
            let path = uv_path(realm, &context, "path")?;
            if let Some(path) = &path {
                message = message
                    .concat(&crate::JsStr::from(" '"))
                    .concat(&ops::to_string(realm, path)?)
                    .concat(&crate::JsStr::from("'"));
            }
            let dest = uv_path(realm, &context, "dest")?;
            if let Some(dest) = &dest {
                message = message
                    .concat(&crate::JsStr::from(" -> '"))
                    .concat(&ops::to_string(realm, dest)?)
                    .concat(&crate::JsStr::from("'"));
            }
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => realm.error_prototype.clone(),
            };
            let error = Object::new(Some(prototype), Kind::Error);
            crate::errors::capture_stack(realm, &error);
            error.define(
                Key::name("message"),
                Property::hidden(Value::String(message)),
            );
            let this = Value::Object(error.clone());
            let object = ops::to_object(realm, &context)?;
            let keys = object
                .own_keys()
                .into_iter()
                .filter(|key| {
                    !matches!(key, Key::Symbol(_))
                        && object
                            .own_property(key)
                            .is_some_and(|property| property.enumerable)
                })
                .collect::<Vec<_>>();
            for key in keys {
                if ["message", "path", "dest"]
                    .iter()
                    .any(|name| key == Key::name(name))
                {
                    continue;
                }
                let value = ops::get(realm, &context, &key)?;
                ops::set(realm, &this, &key, value)?;
            }
            ops::set(realm, &this, &Key::name("code"), code)?;
            for (name, value) in [("path", path), ("dest", dest)] {
                if let Some(value) = value
                    && ops::to_boolean(&value)
                {
                    ops::set(realm, &this, &Key::name(name), value)?;
                }
            }
            Ok(error)
        },
    );
    function.data_mut().prototype = Some(error_constructor.clone());
    let getter = realm.function("get constructor", 0, false, move |_, _, _| {
        Ok(Value::Object(error_constructor.clone()))
    });
    prototype.define(
        Key::name("constructor"),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    function
}

/// Node's `AbortError` class: an Error subclass with `code` `ABORT_ERR` and
/// `name` `AbortError`, whose options may give a `cause`.
fn abort_error(realm: &mut Realm) -> Object {
    let prototype = Object::new(Some(realm.error_prototype.clone()), Kind::Ordinary);
    let fallback = prototype.clone();

    realm.builtin_constructor(
        "AbortError",
        0,
        &prototype,
        |realm, _, _| {
            Err(realm.throw(
                ErrorKind::Type,
                "Class constructor AbortError cannot be invoked without 'new'",
            ))
        },
        move |realm, target, args| {
            let options = arg(args, 1);
            if !matches!(options, Value::Undefined) && ops::type_of(&options) != "object" {
                return Err(invalid_arg_type(realm, "options", &["Object"], &options));
            }
            let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
                Value::Object(prototype) => prototype,
                _ => fallback.clone(),
            };
            let error = Object::new(Some(prototype), Kind::Error);
            crate::errors::capture_stack(realm, &error);
            let message = match arg(args, 0) {
                Value::Undefined => Value::str("The operation was aborted"),
                message => Value::String(ops::to_string(realm, &message)?),
            };
            error.define(Key::name("message"), Property::hidden(message));
            if let Value::Object(_) = options
                && ops::has_property(realm, &Key::name("cause"), &options)?
            {
                let cause = ops::get_named(realm, &options, "cause")?;
                error.define(Key::name("cause"), Property::hidden(cause));
            }
            error.define(Key::name("code"), Property::data(Value::str("ABORT_ERR")));
            error.define(Key::name("name"), Property::data(Value::str("AbortError")));
            Ok(error)
        },
    )
}

/// A value as `util.format`'s `%s` shows it: strings unchanged, symbols
/// and objects inspected.
pub(crate) fn formatted(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    match value {
        Value::Symbol(_) | Value::Object(_) => crate::inspect::inspect(realm, value, 0),
        _ => text(realm, value),
    }
}

/// `new ERR_SYSTEM_ERROR(context)`: Node's `SystemError` for a failed
/// system call, whose `errno` and `syscall` read and write the context.
fn system_error(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    system_error_kind(realm, args, "ERR_SYSTEM_ERROR", "A system error occurred")
}

fn system_error_kind(
    realm: &mut Realm,
    args: &[Value],
    error_code: &str,
    prefix: &str,
) -> Result<Thrown, Thrown> {
    let context = arg(args, 0);
    let field = |realm: &mut Realm, key: &str| ops::get(realm, &context, &Key::name(key));
    let (syscall, code, message) = (
        field(realm, "syscall")?,
        field(realm, "code")?,
        field(realm, "message")?,
    );
    let (path, dest) = (field(realm, "path")?, field(realm, "dest")?);
    let mut text = format!(
        "{prefix}: {} returned {} ({})",
        text(realm, &syscall)?,
        text(realm, &code)?,
        text(realm, &message)?
    );
    if !matches!(path, Value::Undefined) {
        text.push_str(&format!(" {}", self::text(realm, &path)?));
    }
    if !matches!(dest, Value::Undefined) {
        text.push_str(&format!(" => {}", self::text(realm, &dest)?));
    }
    let Thrown(error) = node_error(realm, ErrorKind::Error, error_code, &text);
    let object = error.as_object().expect("node errors are objects");
    let writable_hidden = Property {
        slot: Slot::Data {
            value: Value::str("SystemError"),
            writable: true,
        },
        enumerable: false,
        configurable: true,
    };
    object.define(Key::name("name"), writable_hidden);
    object.define(
        Key::name("info"),
        Property {
            slot: Slot::Data {
                value: context.clone(),
                writable: false,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let mut names = vec!["errno", "syscall"];
    for (key, value) in [("path", &path), ("dest", &dest)] {
        if !matches!(value, Value::Undefined) {
            names.push(key);
        }
    }
    for key in names {
        let (target, source) = (context.clone(), context.clone());
        let get = realm.function("get", 0, false, move |realm, _, _| {
            ops::get(realm, &source, &Key::name(key))
        });
        let set = realm.function("set", 1, false, move |realm, _, args| {
            ops::set(realm, &target, &Key::name(key), arg(args, 0))?;
            Ok(Value::Undefined)
        });
        object.define(
            Key::name(key),
            Property {
                slot: Slot::Accessor {
                    get: Some(get),
                    set: Some(set),
                },
                enumerable: true,
                configurable: true,
            },
        );
    }
    Ok(Thrown(error))
}

/// `new ERR_BUFFER_OUT_OF_BOUNDS(name)`.
fn out_of_bounds(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    let name = arg(args, 0);
    let message = if ops::to_boolean(&name) {
        format!("\"{}\" is outside of buffer bounds", text(realm, &name)?)
    } else {
        "Attempt to access memory outside buffer bounds".to_owned()
    };
    Ok(node_error(
        realm,
        ErrorKind::Range,
        "ERR_BUFFER_OUT_OF_BOUNDS",
        &message,
    ))
}

/// `new ERR_MISSING_ARGS(...names)`; an array lists alternatives.
fn missing_args(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    let mut names = Vec::new();
    for value in args {
        let alternatives = if matches!(value, Value::Object(object) if matches!(object.data().kind, Kind::Array(_)))
        {
            crate::iter::collect(realm, value)?
        } else {
            vec![value.clone()]
        };
        let mut wrapped = Vec::new();
        for alternative in alternatives {
            wrapped.push(format!("\"{}\"", text(realm, &alternative)?));
        }
        names.push(wrapped.join(" or "));
    }
    let plural = if args.len() > 1 { "s" } else { "" };
    let message = format!(
        "The {} argument{plural} must be specified",
        crate::node_error::list(&names, "and")
    );
    Ok(node_error(
        realm,
        ErrorKind::Type,
        "ERR_MISSING_ARGS",
        &message,
    ))
}

/// Creates a coded error from constructor arguments.
type Build = fn(&mut Realm, &[Value]) -> Result<Thrown, Thrown>;

/// `new ERR_INVALID_ARG_TYPE(name, expected, actual)`.
fn invalid_type(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    let name = text(realm, &arg(args, 0))?;
    let expected = arg(args, 1);
    let mut names = Vec::new();
    if matches!(&expected, Value::Object(object) if matches!(object.data().kind, Kind::Array(_))) {
        for value in crate::iter::collect(realm, &expected)? {
            names.push(text(realm, &value)?);
        }
    } else {
        names.push(text(realm, &expected)?);
    }
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    Ok(invalid_arg_type(realm, &name, &names, &arg(args, 2)))
}

/// `new ERR_INVALID_ARG_VALUE(name, value, reason = 'is invalid')`.
fn invalid_value(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    let name = text(realm, &arg(args, 0))?;
    let reason = match arg(args, 2) {
        Value::Undefined => "is invalid".to_owned(),
        reason => text(realm, &reason)?,
    };
    Ok(invalid_arg_value(realm, &name, &arg(args, 1), &reason))
}

/// `new ERR_OUT_OF_RANGE(name, range, input, replaceDefaultBoolean)`; with
/// the flag set, `name` is the whole first sentence.
fn range(realm: &mut Realm, args: &[Value]) -> Result<Thrown, Thrown> {
    let name = text(realm, &arg(args, 0))?;
    let range = text(realm, &arg(args, 1))?;
    if ops::to_boolean(&arg(args, 3)) {
        let received = received(realm, &arg(args, 2))?;
        let message = format!("{name} It must be {range}. Received {received}");
        return Ok(node_error(
            realm,
            ErrorKind::Range,
            "ERR_OUT_OF_RANGE",
            &message,
        ));
    }
    Ok(out_of_range(realm, &name, &range, &arg(args, 2)))
}

/// `require(request)` for a request the build did not resolve: a computed
/// path, or a name only the running program can serve. As Node routes every
/// `require` through `Module._load`, a program that links the `module`
/// library has its `Module._load` called with a parent module whose
/// `filename` is `parent`, so a loader the program installs there serves
/// the request; the library's own `_load` loads compiled entries by path.
/// Without the library, compiled entries are loaded directly.
pub fn require_unlinked(realm: &mut Realm, request: &Value, parent: &Value) -> Completion {
    let request = Value::String(ops::to_string(realm, request)?);
    let module = match realm.node_library {
        Some(require) => require(realm, "module")?,
        None => Value::Undefined,
    };
    if matches!(module, Value::Object(_)) {
        let load = ops::get_named(realm, &module, "_load")?;
        let parent_module = realm.object();
        for name in ["id", "filename"] {
            parent_module.define(Key::name(name), Property::data(parent.clone()));
        }
        let paths = realm.array(Vec::new());
        parent_module.define(Key::name("paths"), Property::data(Value::Object(paths)));
        parent_module.define(Key::name("loaded"), Property::data(Value::Boolean(true)));
        return ops::call(
            realm,
            &load,
            module,
            &[request, Value::Object(parent_module), Value::Boolean(false)],
        );
    }
    match load_compiled(realm, &request, parent)? {
        Some(exports) => Ok(exports),
        None => Err(module_not_found(realm, &request)),
    }
}

/// Gives a compiled CommonJS module's object the properties of Node's
/// besides `exports` and `filename`: `id`, `path`, `require`, `loaded`
/// (true once the module has run), `children` and `paths`.
pub fn define_commonjs_module(
    realm: &mut Realm,
    module: &Value,
    dirname: Value,
    require: &Value,
) -> Result<(), Thrown> {
    let filename = ops::get_named(realm, module, "filename")?;
    ops::set(realm, module, &Key::name("id"), filename)?;
    ops::set(realm, module, &Key::name("path"), dirname)?;
    ops::set(realm, module, &Key::name("require"), require.clone())?;
    ops::set(realm, module, &Key::name("loaded"), Value::Boolean(false))?;
    for name in ["children", "paths"] {
        let list = Value::Object(realm.array(Vec::new()));
        ops::set(realm, module, &Key::name(name), list)?;
    }
    Ok(())
}

/// The compiled entry a request names from the module at `parent`: an
/// absolute path, or a relative one from the parent's folder, matched with
/// or without its extension against the paths the program registered. The
/// entry's `module.exports` export when it has one, else its namespace;
/// `None` when no compiled entry matches.
pub fn load_compiled(
    realm: &mut Realm,
    request: &Value,
    parent: &Value,
) -> Result<Option<Value>, Thrown> {
    let request = ops::to_string(realm, request)?.to_rust_lossy();
    let parent = match parent {
        Value::String(parent) => parent.to_rust_lossy(),
        _ => String::new(),
    };
    let Some(path) = request_path(&request, &parent) else {
        return Ok(None);
    };
    let Some(entry) = realm.script_entry(&path) else {
        return Ok(None);
    };
    let namespace = realm.load_entry_now(entry)?;
    if let Value::Object(object) = &namespace
        && object.own_property(&Key::name("module.exports")).is_some()
    {
        return ops::get_named(realm, &namespace, "module.exports").map(Some);
    }
    Ok(Some(namespace))
}

/// The absolute path a request names from a parent file, with `/`
/// separators and `.` and `..` segments resolved; `None` for bare names.
fn request_path(request: &str, parent: &str) -> Option<String> {
    let request = request.replace('\\', "/");
    let absolute = request.starts_with('/') || request.as_bytes().get(1) == Some(&b':');
    let joined = if absolute {
        request
    } else if request == "."
        || request == ".."
        || request.starts_with("./")
        || request.starts_with("../")
    {
        let parent = parent.replace('\\', "/");
        let folder = parent.rsplit_once('/').map_or("", |(folder, _)| folder);
        format!("{folder}/{request}")
    } else {
        return None;
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            "" if !parts.is_empty() => {}
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn module_not_found(realm: &mut Realm, request: &Value) -> Thrown {
    let text = match request {
        Value::String(text) => text.to_rust_lossy(),
        _ => String::new(),
    };
    let error = realm.error(ErrorKind::Error, &format!("Cannot find module '{text}'"));
    error.define(
        Key::name("code"),
        Property::data(Value::str("MODULE_NOT_FOUND")),
    );
    Thrown(Value::Object(error))
}

/// `internalBinding('modules')`: `loadCompiled(request, parentFilename)`
/// gives the exports of the compiled entry a request names, or undefined.
fn modules_binding(realm: &mut Realm) -> Object {
    let binding = realm.object();
    realm.method(&binding, "loadCompiled", 2, |realm, _, args| {
        let request = arg(args, 0);
        let parent = arg(args, 1);
        Ok(load_compiled(realm, &request, &parent)?.unwrap_or(Value::Undefined))
    });
    binding
}

#[cfg(test)]
mod tests;
