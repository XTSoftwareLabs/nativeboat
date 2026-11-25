//! Expected messages are Node 22's `internal/errors` results.

use super::*;

fn get(realm: &mut Realm, target: &Value, name: &str) -> Value {
    ops::get(realm, target, &Key::name(name)).unwrap()
}

fn message(realm: &mut Realm, result: Completion) -> String {
    let error = result.unwrap();
    let message = get(realm, &error, "message");
    ops::to_string(realm, &message).unwrap().to_rust_lossy()
}

fn shown(realm: &mut Realm, thrown: Thrown) -> String {
    ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy()
}

#[test]
fn invalid_uri_errors_keep_node_code_and_native_uri_brand() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    let class = get(&mut realm, &codes, "ERR_INVALID_URI");
    let error = ops::construct(&mut realm, &class, &[]).unwrap();
    for (name, expected) in [
        ("name", "URIError"),
        ("code", "ERR_INVALID_URI"),
        ("message", "URI malformed"),
    ] {
        assert!(crate::same_value(
            &get(&mut realm, &error, name),
            &Value::str(expected)
        ));
    }
    assert_eq!(
        shown(&mut realm, Thrown(error.clone())),
        "URIError [ERR_INVALID_URI]: URI malformed"
    );
    let global = Value::Object(realm.global.clone());
    let uri = get(&mut realm, &global, "URIError");
    assert!(ops::instance_of(&mut realm, &error, &uri).unwrap());
}

#[test]
fn native_modules_are_created_once_per_realm() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    assert!(crate::same_value(
        &errors,
        &load(&mut realm, "internal/errors").unwrap()
    ));
    let unknown = load(&mut realm, "missing").unwrap_err();
    assert_eq!(
        shown(&mut realm, unknown),
        "Error: The Node.js library module 'missing' is not compiled yet"
    );
}

#[test]
fn compiled_stylesheet_modules_register_once_and_preserve_import_order() {
    let mut realm = Realm::new();
    realm.register_native_stylesheet("nativeboat:stylesheet:first.css", "a { color: red }");
    realm.register_native_stylesheet("nativeboat:stylesheet:second.css", "b { color: blue }");
    realm.register_native_stylesheet("nativeboat:stylesheet:first.css", "a { color: green }");

    load(&mut realm, "nativeboat:stylesheet:first.css").unwrap();
    load(&mut realm, "nativeboat:stylesheet:second.css").unwrap();
    load(&mut realm, "nativeboat:stylesheet:first.css").unwrap();

    assert_eq!(
        realm.take_native_stylesheets(),
        vec!["a { color: green }", "b { color: blue }"]
    );
    assert!(realm.take_native_stylesheets().is_empty());
    assert!(load(&mut realm, "nativeboat:stylesheet:missing.css").is_err());
}

#[test]
fn watcher_async_scope_calls_the_block_synchronously_and_uses_windows_errors() {
    let mut realm = Realm::new();
    let uv = load(&mut realm, "binding/uv").unwrap();
    assert!(matches!(
        get(&mut realm, &uv, "UV_ENOSPC"),
        Value::Number(-4055.0)
    ));
    let hooks = load(&mut realm, "internal/async_hooks").unwrap();
    let symbols = get(&mut realm, &hooks, "symbols");
    assert!(matches!(
        get(&mut realm, &symbols, "owner_symbol"),
        Value::Symbol(_)
    ));
    let scope = get(&mut realm, &hooks, "defaultTriggerAsyncIdScope");
    let observed = alloc::rc::Rc::new(core::cell::Cell::new(false));
    let called = observed.clone();
    let block = realm.function("block", 2, false, move |_, this, args| {
        assert!(matches!(this, Value::Null));
        assert!(matches!(args, [Value::Number(4.0), Value::Number(5.0)]));
        called.set(true);
        Ok(Value::Number(9.0))
    });
    let result = ops::call(
        &mut realm,
        &scope,
        Value::Undefined,
        &[
            Value::Number(1.0),
            Value::Object(block),
            Value::Number(4.0),
            Value::Number(5.0),
        ],
    )
    .unwrap();
    assert!(
        observed.get(),
        "the block must run before the scope returns"
    );
    assert!(matches!(result, Value::Number(9.0)));
    let fail = realm.function("fail", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "scope"))
    });
    let error = ops::call(
        &mut realm,
        &scope,
        Value::Undefined,
        &[Value::Undefined, Value::Object(fail)],
    )
    .unwrap_err();
    assert_eq!(shown(&mut realm, error), "RangeError: scope");
}

#[test]
fn format_with_options_propagates_numeric_coercion_errors() {
    let mut realm = Realm::new();
    let module = load(&mut realm, "internal/util/inspect").unwrap();
    let format = get(&mut realm, &module, "formatWithOptions");
    let options = realm.object();
    options.define(Key::name("depth"), Property::data(Value::Number(2.0)));
    let value = realm.object();
    let fail = realm.function("valueOf", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "format"))
    });
    value.define(Key::name("valueOf"), Property::data(Value::Object(fail)));
    let error = ops::call(
        &mut realm,
        &format,
        Value::Undefined,
        &[
            Value::Object(options),
            Value::str("%d"),
            Value::Object(value),
        ],
    )
    .unwrap_err();
    assert_eq!(shown(&mut realm, error), "RangeError: format");
}

#[test]
fn node_process_builtins_expose_filesystem_constants_and_main_thread_identity() {
    let mut realm = Realm::new();
    let constants = load(&mut realm, "constants").unwrap();
    let readonly = get(&mut realm, &constants, "O_RDONLY");
    assert!(matches!(readonly, Value::Number(0.0)));
    let copyfile_exclusive = get(&mut realm, &constants, "COPYFILE_EXCL");
    assert!(matches!(copyfile_exclusive, Value::Number(1.0)));
    let has_own_property = get(&mut realm, &constants, "hasOwnProperty");
    assert!(matches!(has_own_property, Value::Object(_)));
    assert!(matches!(
        ops::call(
            &mut realm,
            &has_own_property,
            constants.clone(),
            &[Value::str("O_RDONLY")],
        )
        .unwrap(),
        Value::Boolean(true)
    ));

    let threads = load(&mut realm, "worker_threads").unwrap();
    assert!(matches!(
        get(&mut realm, &threads, "threadId"),
        Value::Number(0.0)
    ));
    assert!(matches!(
        get(&mut realm, &threads, "isMainThread"),
        Value::Boolean(true)
    ));
    assert!(crate::same_value(
        &threads,
        &load(&mut realm, "worker_threads").unwrap()
    ));

    let tty = load(&mut realm, "tty").unwrap();
    let isatty = get(&mut realm, &tty, "isatty");
    for descriptor in 0..=2 {
        let result = ops::call(
            &mut realm,
            &isatty,
            Value::Undefined,
            &[Value::Number(f64::from(descriptor))],
        )
        .unwrap();
        assert!(matches!(
            result,
            Value::Boolean(value) if value == crate::fs_win::Descriptors::default().is_tty(descriptor)
        ));
    }
    let invalid = ops::call(&mut realm, &isatty, Value::Undefined, &[]).unwrap();
    assert!(matches!(invalid, Value::Boolean(false)));
    for invalid in [
        Value::Null,
        Value::Boolean(true),
        Value::str("2"),
        Value::Object(realm.object()),
        Value::Number(-1.0),
        Value::Number(2.5),
        Value::Number(f64::NAN),
        Value::Number(f64::INFINITY),
        Value::Number(2_147_483_648.0),
        Value::Number(f64::from(i32::MAX)),
    ] {
        let result = ops::call(&mut realm, &isatty, Value::Undefined, &[invalid]).unwrap();
        assert!(matches!(result, Value::Boolean(false)));
    }
}

#[test]
fn task_queue_binding_validates_and_enqueues_microtasks_on_the_realm() {
    use std::{cell::Cell, rc::Rc};

    let mut realm = Realm::new();
    let queues = load(&mut realm, "internal/process/task_queues").unwrap();
    let enqueue = get(&mut realm, &queues, "queueMicrotask");
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let callback = Value::Object(realm.function("microtask", 0, false, move |_, _, _| {
        observed.set(observed.get() + 1);
        Ok(Value::Undefined)
    }));
    ops::call(
        &mut realm,
        &enqueue,
        Value::Undefined,
        std::slice::from_ref(&callback),
    )
    .unwrap();
    assert_eq!(calls.get(), 0);
    realm.run_jobs().unwrap();
    assert_eq!(calls.get(), 1);
    let invalid = ops::call(
        &mut realm,
        &enqueue,
        Value::Undefined,
        &[Value::str("not callable")],
    )
    .unwrap_err();
    assert!(shown(&mut realm, invalid).contains("callback"));
    realm.global.define(
        Key::name("queueMicrotask"),
        throwing_accessor(&realm, "queueMicrotask"),
    );
    let failure = ops::call(
        &mut realm,
        &enqueue,
        Value::Undefined,
        std::slice::from_ref(&callback),
    )
    .unwrap_err();
    assert!(shown(&mut realm, failure).contains("queueMicrotask"));
}

#[test]
fn error_codes_construct_node_errors() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    let invalid_type = get(&mut realm, &codes, "ERR_INVALID_ARG_TYPE");
    let hidden = get(&mut realm, &invalid_type, "HideStackFramesError");
    assert!(crate::same_value(&invalid_type, &hidden));
    let name = get(&mut realm, &invalid_type, "name");
    assert!(crate::same_value(&name, &Value::str("NodeError")));
    let expected = Value::Object(realm.array(vec![
        Value::str("string"),
        Value::str("number"),
        Value::str("Object"),
        Value::str("Error"),
        Value::str("Buffer"),
    ]));
    let args = [Value::str("a"), expected, Value::Number(1.0)];
    let result = ops::construct(&mut realm, &invalid_type, &args);
    assert_eq!(
        message(&mut realm, result),
        "The \"a\" argument must be one of type string or number or an instance of Error, Buffer, or Object. Received type number (1)"
    );
    let args = [Value::str("a"), Value::str("Function"), Value::Number(1.0)];
    let result = ops::construct(&mut realm, &invalid_type, &args);
    assert_eq!(
        message(&mut realm, result),
        "The \"a\" argument must be of type function. Received type number (1)"
    );
    let value = get(&mut realm, &codes, "ERR_INVALID_ARG_VALUE");
    let object = Value::Object(realm.object());
    ops::set(&mut realm, &object, &Key::name("a"), Value::Number(1.0)).unwrap();
    let result = ops::construct(&mut realm, &value, &[Value::str("opt.x"), object]);
    assert_eq!(
        message(&mut realm, result),
        "The property 'opt.x' is invalid. Received { a: 1 }"
    );
    let long = Value::str(&"x".repeat(200));
    let args = [Value::str("y"), long, Value::str("must be short")];
    let result = ops::construct(&mut realm, &value, &args);
    assert_eq!(
        message(&mut realm, result),
        format!(
            "The argument 'y' must be short. Received '{}...",
            "x".repeat(127)
        )
    );
    let range = get(&mut realm, &codes, "ERR_OUT_OF_RANGE");
    let args = [
        Value::str("The thing is bad."),
        Value::str(">= 1"),
        Value::Number(2.0),
        Value::Boolean(true),
    ];
    let result = ops::construct(&mut realm, &range, &args);
    assert_eq!(
        message(&mut realm, result),
        "The thing is bad. It must be >= 1. Received 2"
    );
    let args = [Value::str("x"), Value::str(">= 1"), Value::str("str")];
    let result = ops::construct(&mut realm, &range, &args);
    assert_eq!(
        message(&mut realm, result),
        "The value of \"x\" is out of range. It must be >= 1. Received 'str'"
    );
    // Argument conversions throw instead of creating an error.
    let symbol = Value::Symbol(crate::Symbol::new(None));
    for (class, args) in [
        (&invalid_type, vec![symbol.clone()]),
        (&invalid_type, vec![Value::str("a"), symbol.clone()]),
        (
            &invalid_type,
            vec![
                Value::str("a"),
                Value::Object(realm.array(vec![symbol.clone()])),
            ],
        ),
        (&value, vec![symbol.clone()]),
        (
            &value,
            vec![Value::str("a"), Value::Undefined, symbol.clone()],
        ),
        (&range, vec![symbol.clone()]),
        (&range, vec![Value::str("a"), symbol.clone()]),
    ] {
        let thrown = ops::construct(&mut realm, class, &args).unwrap_err();
        assert!(shown(&mut realm, thrown).starts_with("TypeError: Cannot convert"));
    }
    let tagged = realm.object();
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "tag"))
    });
    tagged.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
    let args = [
        Value::str("a"),
        Value::str("b"),
        Value::Object(tagged),
        Value::Boolean(true),
    ];
    let thrown = ops::construct(&mut realm, &range, &args).unwrap_err();
    assert_eq!(shown(&mut realm, thrown), "RangeError: tag");
}

#[test]
fn directory_error_codes_match_node_22_messages() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    for (code, expected) in [
        ("ERR_DIR_CLOSED", "Directory handle was closed"),
        (
            "ERR_DIR_CONCURRENT_OPERATION",
            "Cannot do synchronous work on directory handle with concurrent asynchronous operations",
        ),
    ] {
        let constructor = get(&mut realm, &codes, code);
        let error = ops::construct(&mut realm, &constructor, &[]).unwrap();
        assert!(crate::same_value(
            &get(&mut realm, &error, "name"),
            &Value::str("Error")
        ));
        assert!(crate::same_value(
            &get(&mut realm, &error, "code"),
            &Value::str(code)
        ));
        assert!(crate::same_value(
            &get(&mut realm, &error, "message"),
            &Value::str(expected)
        ));
        assert_eq!(
            shown(&mut realm, Thrown(error)),
            format!("Error [{code}]: {expected}")
        );
    }
}

#[test]
fn hidden_stack_frames_call_through() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let hide = get(&mut realm, &errors, "hideStackFrames");
    let inner = Value::Object(realm.function("inner", 1, false, |_, this, args| {
        Ok(Value::Boolean(
            matches!(this, Value::Null) && args.len() == 2,
        ))
    }));
    let wrapped = ops::call(
        &mut realm,
        &hide,
        Value::Undefined,
        std::slice::from_ref(&inner),
    )
    .unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &wrapped, "withoutStackTrace"),
        &inner
    ));
    let result = ops::call(
        &mut realm,
        &wrapped,
        Value::Null,
        &[Value::Null, Value::Null],
    );
    assert!(crate::same_value(&result.unwrap(), &Value::Boolean(true)));
}

fn dispatcher(realm: &mut Realm) -> Result<Object, Thrown> {
    let namespace = realm.object();
    let dispatch = realm.function("dispatch", 1, false, |realm, _, args| {
        let name = ops::to_string(realm, &args[0])?.to_rust_lossy();
        Ok(Value::str(&format!("module {name}")))
    });
    namespace.define(
        Key::name("default"),
        Property::data(Value::Object(dispatch)),
    );
    Ok(namespace)
}

fn require(realm: &mut Realm, name: &str) -> Completion {
    library(realm, dispatcher, name)
}

#[test]
fn node_modules_come_from_the_linked_library() {
    let mut realm = Realm::new();
    let missing = load(&mut realm, "node:path").unwrap_err();
    assert_eq!(
        shown(&mut realm, missing),
        "TypeError: The Node.js library module 'path' is not linked"
    );
    realm.node_library = Some(require);
    let path = load(&mut realm, "node:path").unwrap();
    assert!(crate::same_value(&path, &Value::str("module path")));
    let posix = load(&mut realm, "node:path/posix").unwrap();
    assert!(crate::same_value(&posix, &Value::str("module path/posix")));
    assert_eq!(realm.native_modules.len(), 1);
    // A library without a dispatcher, or one that fails, is reported.
    let mut realm = Realm::new();
    let empty = library(&mut realm, |realm| Ok(realm.object()), "path").unwrap_err();
    assert_eq!(
        shown(&mut realm, empty),
        "TypeError: The Node.js library has no dispatcher"
    );
    let failing = library(
        &mut realm,
        |realm| Err(realm.throw(ErrorKind::Range, "library")),
        "path",
    )
    .unwrap_err();
    assert_eq!(shown(&mut realm, failing), "RangeError: library");
}

#[test]
fn direct_node_internal_requires_use_the_linked_library() {
    let mut realm = Realm::new();
    realm.node_library = Some(require);
    let directory = load(&mut realm, "internal/fs/dir").unwrap();
    assert!(crate::same_value(
        &directory,
        &Value::str("module internal/fs/dir")
    ));
}

fn text_of(realm: &mut Realm, result: Completion) -> String {
    match result {
        Ok(value) => crate::inspect::inspect(realm, &value, 2).unwrap(),
        Err(Thrown(error)) => ops::to_string(realm, &error).unwrap().to_rust_lossy(),
    }
}

fn invoke(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> String {
    let function = get(realm, target, name);
    let result = ops::call(realm, &function, Value::Undefined, args);
    text_of(realm, result)
}

fn throwing_accessor(realm: &Realm, message: &'static str) -> Property {
    let getter = realm.function("get", 0, false, move |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, message))
    });
    Property {
        slot: crate::Slot::Accessor {
            get: Some(getter),
            set: None,
        },
        enumerable: true,
        configurable: true,
    }
}

#[test]
fn buffer_error_codes_and_generic_errors() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    let symbol = Value::Symbol(crate::Symbol::new(Some(crate::JsStr::from("e"))));
    let array = Value::Object(realm.array(vec![Value::str("a"), Value::str("b")]));
    for (code, args, expected) in [
        (
            "ERR_BUFFER_OUT_OF_BOUNDS",
            vec![Value::str("offset")],
            "\"offset\" is outside of buffer bounds",
        ),
        (
            "ERR_BUFFER_OUT_OF_BOUNDS",
            vec![],
            "Attempt to access memory outside buffer bounds",
        ),
        (
            "ERR_INVALID_BUFFER_SIZE",
            vec![Value::str("16-bits")],
            "Buffer size must be a multiple of 16-bits",
        ),
        (
            "ERR_MISSING_ARGS",
            vec![Value::str("input")],
            "The \"input\" argument must be specified",
        ),
        (
            "ERR_MISSING_ARGS",
            vec![array, Value::str("c")],
            "The \"a\" or \"b\" and \"c\" arguments must be specified",
        ),
        (
            "ERR_UNKNOWN_ENCODING",
            vec![symbol.clone()],
            "Unknown encoding: Symbol(e)",
        ),
        (
            "ERR_UNKNOWN_ENCODING",
            vec![Value::Number(5.0)],
            "Unknown encoding: 5",
        ),
    ] {
        let class = get(&mut realm, &codes, code);
        let error = ops::construct(&mut realm, &class, &args).unwrap();
        let message = get(&mut realm, &error, "message");
        let message = ops::to_string(&mut realm, &message)
            .unwrap()
            .to_rust_lossy();
        assert_eq!(message, expected, "{code}");
    }
    let symbols = Value::Object(realm.array(vec![symbol.clone()]));
    for (code, args) in [
        ("ERR_BUFFER_OUT_OF_BOUNDS", vec![symbol.clone()]),
        ("ERR_INVALID_BUFFER_SIZE", vec![Value::Undefined]),
        ("ERR_MISSING_ARGS", vec![symbol.clone()]),
        ("ERR_MISSING_ARGS", vec![symbols]),
    ] {
        let class = get(&mut realm, &codes, code);
        let result = ops::construct(&mut realm, &class, &args);
        let shown = text_of(&mut realm, result);
        assert!(
            shown.starts_with("TypeError") || shown.contains("undefined"),
            "{code}: {shown}"
        );
    }
    let properties = realm.object();
    properties.define(Key::name("code"), Property::data(Value::str("X")));
    properties.define(Key::name("hidden"), Property::hidden(Value::Number(1.0)));
    let generic = get(&mut realm, &errors, "genericNodeError");
    let args = [Value::str("boom"), Value::Object(properties)];
    let error = ops::call(&mut realm, &generic, Value::Undefined, &args).unwrap();
    let code = get(&mut realm, &error, "code");
    assert_eq!(text_of(&mut realm, Ok(code)), "'X'");
    assert!(matches!(
        get(&mut realm, &error, "hidden"),
        Value::Undefined
    ));
    let plain = ops::call(
        &mut realm,
        &generic,
        Value::Undefined,
        &[Value::str("boom")],
    )
    .unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &plain).unwrap().to_rust_lossy(),
        "Error: boom"
    );
    let failing = realm.object();
    failing.define(Key::name("x"), throwing_accessor(&realm, "prop"));
    let args = [Value::str("boom"), Value::Object(failing)];
    let result = ops::call(&mut realm, &generic, Value::Undefined, &args);
    assert_eq!(text_of(&mut realm, result), "RangeError: prop");
    let result = ops::call(&mut realm, &generic, Value::Undefined, &[symbol]);
    assert!(text_of(&mut realm, result).starts_with("TypeError"));
}

#[test]
fn bindings_options_and_inspection() {
    let mut realm = Realm::new();
    let binding = load(&mut realm, "internal/binding").unwrap();
    for name in [
        "buffer",
        "config",
        "fs_dir",
        "fs_event_wrap",
        "icu",
        "string_decoder",
        "trace_events",
        "types",
        "util",
    ] {
        let result = ops::call(&mut realm, &binding, Value::Undefined, &[Value::str(name)]);
        assert!(matches!(result, Ok(Value::Object(_))), "{name}");
    }
    let missing = ops::call(
        &mut realm,
        &binding,
        Value::Undefined,
        &[Value::str("nope")],
    );
    assert_eq!(text_of(&mut realm, missing), "Error: No such binding: nope");
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let invalid = ops::call(&mut realm, &binding, Value::Undefined, &[symbol]);
    assert!(text_of(&mut realm, invalid).starts_with("TypeError"));
    let missing = load(&mut realm, "internal/blob");
    assert_eq!(
        text_of(&mut realm, missing),
        "Error: The Node.js library module 'internal/blob' is not compiled yet"
    );
    let options = load(&mut realm, "internal/options").unwrap();
    assert_eq!(
        invoke(&mut realm, &options, "getOptionValue", &[Value::str("--x")]),
        "false"
    );
    let inspect = load(&mut realm, "internal/util/inspect").unwrap();
    let inner = Value::Object(realm.array(vec![Value::Number(1.0)]));
    let middle = Value::Object(realm.array(vec![inner]));
    let nested = Value::Object(realm.array(vec![middle]));
    let depth = realm.object();
    depth.define(Key::name("depth"), Property::data(Value::Number(0.0)));
    let infinite = realm.object();
    infinite.define(
        Key::name("depth"),
        Property::data(Value::Number(f64::INFINITY)),
    );
    for (options, expected) in [
        (Value::Undefined, "'[ [ [ 1 ] ] ]'"),
        (Value::Object(depth), "'[ [Array] ]'"),
        (Value::Object(infinite), "'[ [ [ 1 ] ] ]'"),
    ] {
        let shown = invoke(&mut realm, &inspect, "inspect", &[nested.clone(), options]);
        assert_eq!(shown, expected);
    }
    let failing = realm.object();
    failing.define(Key::name("depth"), throwing_accessor(&realm, "depth"));
    let shown = invoke(
        &mut realm,
        &inspect,
        "inspect",
        &[nested, Value::Object(failing)],
    );
    assert_eq!(shown, "RangeError: depth");
    let tagged = realm.object();
    let tag = Key::Symbol(realm.symbols.to_string_tag.clone());
    tagged.define(tag, throwing_accessor(&realm, "tag"));
    let shown = invoke(&mut realm, &inspect, "inspect", &[Value::Object(tagged)]);
    assert_eq!(shown, "RangeError: tag");
    let config = load(&mut realm, "binding/config").unwrap();
    assert!(matches!(
        get(&mut realm, &config, "hasIntl"),
        Value::Boolean(true)
    ));
    let decoder = load(&mut realm, "binding/string_decoder").unwrap();
    let encodings = get(&mut realm, &decoder, "encodings");
    let join = get(&mut realm, &encodings, "join");
    let joined = ops::call(&mut realm, &join, encodings, &[]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &joined).unwrap().to_rust_lossy(),
        "ascii,utf8,base64,utf16le,latin1,hex,buffer,base64url"
    );
}

#[test]
fn node_inspect_module_formats_options_and_strips_vt_sequences() {
    let mut realm = Realm::new();
    let inspect = load(&mut realm, "internal/util/inspect").unwrap();
    assert_eq!(
        invoke(
            &mut realm,
            &inspect,
            "format",
            &[
                Value::str("%s %d"),
                Value::str("value"),
                Value::Number(42.0)
            ],
        ),
        "'value 42'"
    );
    let formatted = invoke(
        &mut realm,
        &inspect,
        "formatWithOptions",
        &[
            Value::Undefined,
            Value::str("%s %d"),
            Value::str("value"),
            Value::Number(42.0),
        ],
    );
    assert_eq!(formatted, "'value 42'");
    let options = realm.object();
    options.define(Key::name("depth"), Property::data(Value::Number(0.0)));
    let nested =
        Value::Object(realm.array(vec![Value::Object(realm.array(vec![Value::Number(1.0)]))]));
    assert_eq!(
        invoke(
            &mut realm,
            &inspect,
            "formatWithOptions",
            &[Value::Object(options), Value::str("%O"), nested],
        ),
        "'[ [Array] ]'"
    );
    assert_eq!(
        invoke(
            &mut realm,
            &inspect,
            "stripVTControlCharacters",
            &[Value::str("漢\u{1b}[31m字\u{1b}[0m\u{1b}]0;title\u{7}!")],
        ),
        "'漢字!'"
    );
    let trace = load(&mut realm, "binding/trace_events").unwrap();
    let enabled = invoke(
        &mut realm,
        &trace,
        "getCategoryEnabledBuffer",
        &[Value::str("util")],
    );
    assert_eq!(enabled, "[ 0 ]");
    let trace_function = get(&mut realm, &trace, "trace");
    assert!(matches!(
        ops::call(
            &mut realm,
            &trace_function,
            trace,
            &[
                Value::Number(1.0),
                Value::str("util"),
                Value::str("label"),
                Value::Number(0.0)
            ],
        ),
        Ok(Value::Undefined)
    ));
}

#[test]
fn util_binding_properties_and_lazy_definitions() {
    let mut realm = Realm::new();
    let util = load(&mut realm, "binding/util").unwrap();
    let constants = get(&mut realm, &util, "constants");
    let enumerable = get(&mut realm, &constants, "ONLY_ENUMERABLE");
    assert_eq!(text_of(&mut realm, Ok(enumerable)), "2");
    let object = Value::Object(realm.array(vec![Value::Number(1.0)]));
    let tag = Key::Symbol(realm.symbols.iterator.clone());
    let array = object.as_object().unwrap().clone();
    array.define(Key::name("extra"), Property::data(Value::Number(2.0)));
    array.define(tag, Property::data(Value::Number(3.0)));
    for (filter, expected) in [
        (0.0, "[ 'length', 'extra', Symbol(Symbol.iterator) ]"),
        (2.0, "[ 'extra', Symbol(Symbol.iterator) ]"),
        (8.0, "[ Symbol(Symbol.iterator) ]"),
        (16.0, "[ 'length', 'extra' ]"),
    ] {
        let args = [object.clone(), Value::Number(filter)];
        assert_eq!(
            invoke(&mut realm, &util, "getOwnNonIndexProperties", &args),
            expected
        );
    }
    let shown = invoke(
        &mut realm,
        &util,
        "getOwnNonIndexProperties",
        &[Value::Number(1.0)],
    );
    assert_eq!(shown, "[]");
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let args = [object.clone(), symbol.clone()];
    let shown = invoke(&mut realm, &util, "getOwnNonIndexProperties", &args);
    assert!(shown.starts_with("TypeError"));
    assert_eq!(
        invoke(&mut realm, &util, "isInsideNodeModules", &[]),
        "true"
    );
    let symbols = get(&mut realm, &util, "privateSymbols");
    let private = get(&mut realm, &symbols, "untransferable_object_private_symbol");
    assert!(matches!(private, Value::Symbol(_)));
    // Lazy properties load their module when first read, then hold data.
    let target = Value::Object(realm.object());
    let object = target.as_object().unwrap().clone();
    let keys = Value::Object(realm.array(vec![Value::str("codes"), Value::str("missing")]));
    let args = [target.clone(), Value::str("internal/errors"), keys];
    invoke(&mut realm, &util, "defineLazyProperties", &args);
    assert!(object.own_property(&Key::name("codes")).unwrap().enumerable);
    assert!(matches!(
        get(&mut realm, &target, "codes"),
        Value::Object(_)
    ));
    let codes = object.own_property(&Key::name("codes")).unwrap();
    assert!(matches!(codes.slot, crate::Slot::Data { .. }));
    ops::set(
        &mut realm,
        &target,
        &Key::name("missing"),
        Value::Number(4.0),
    )
    .unwrap();
    let missing = get(&mut realm, &target, "missing");
    assert_eq!(text_of(&mut realm, Ok(missing)), "4");
    let hidden = Value::Object(realm.array(vec![Value::str("Blob")]));
    let args = [
        target.clone(),
        Value::str("internal/blob"),
        hidden,
        Value::Boolean(false),
    ];
    invoke(&mut realm, &util, "defineLazyProperties", &args);
    let blob = object.own_property(&Key::name("Blob")).unwrap();
    assert!(!blob.enumerable);
    let crate::Slot::Accessor {
        get: Some(getter),
        set: Some(setter),
    } = blob.slot
    else {
        panic!("Blob is lazy");
    };
    let result = ops::get(&mut realm, &target, &Key::name("Blob"));
    assert_eq!(
        text_of(&mut realm, result),
        "Error: The Node.js library module 'internal/blob' is not compiled yet"
    );
    // Accessors called on primitives define nothing.
    let result = ops::call(&mut realm, &Value::Object(setter), Value::Number(1.0), &[]);
    assert!(result.is_ok());
    let keys = Value::Object(realm.array(vec![Value::str("codes")]));
    let args = [target.clone(), Value::str("internal/errors"), keys];
    invoke(&mut realm, &util, "defineLazyProperties", &args);
    let codes = object.own_property(&Key::name("codes")).unwrap();
    let crate::Slot::Accessor {
        get: Some(codes_getter),
        ..
    } = codes.slot
    else {
        panic!("codes is lazy again");
    };
    let result = ops::call(
        &mut realm,
        &Value::Object(codes_getter),
        Value::Number(1.0),
        &[],
    );
    assert!(result.is_ok());
    let _ = getter;
    let shown = invoke(
        &mut realm,
        &util,
        "defineLazyProperties",
        &[Value::Number(1.0)],
    );
    assert_eq!(shown, "undefined");
    let invalid_key = Value::Object(realm.array(vec![symbol.clone()]));
    let failing_key = realm.object();
    let to_string = realm.function("toString", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "key"))
    });
    failing_key.define(
        Key::name("toString"),
        Property::data(Value::Object(to_string)),
    );
    let failing_keys = Value::Object(realm.array(vec![Value::Object(failing_key)]));
    for (args, expected) in [
        (
            vec![target.clone(), symbol.clone(), Value::Undefined],
            "TypeError",
        ),
        (
            vec![target.clone(), Value::str("x"), Value::Number(1.0)],
            "TypeError",
        ),
        (
            vec![target.clone(), Value::str("x"), failing_keys],
            "RangeError",
        ),
        (vec![target, Value::str("x"), invalid_key], "undefined"),
    ] {
        let shown = invoke(&mut realm, &util, "defineLazyProperties", &args);
        assert!(shown.starts_with(expected), "{shown}");
    }
}

#[test]
fn disabled_runtime_permissions_allow_native_filesystem_access() {
    let mut realm = Realm::new();
    let permission = load(&mut realm, "binding/permission").unwrap();
    for args in [
        vec![Value::str("fs.read"), Value::str("C:\\fixture.txt")],
        vec![Value::str("fs.write"), Value::Undefined],
        vec![],
    ] {
        assert_eq!(invoke(&mut realm, &permission, "has", &args), "true");
    }
}

#[test]
fn blob_binding_copies_slices_reads_concatenates_and_stores_native_bytes() {
    use std::{cell::RefCell, rc::Rc};

    let mut realm = Realm::new();
    let binding = load(&mut realm, "binding/blob").unwrap();
    let part = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        vec![0, 1, 2, 3],
        None,
    ));
    let sources = Value::Object(realm.array(vec![part.clone()]));
    let create = get(&mut realm, &binding, "createBlob");
    let blob = ops::call(
        &mut realm,
        &create,
        Value::Undefined,
        &[sources, Value::Undefined],
    )
    .unwrap();
    let invalid_sources = Value::Object(realm.array(vec![Value::str("not an ArrayBufferView")]));
    assert!(
        ops::call(
            &mut realm,
            &create,
            Value::Undefined,
            &[invalid_sources, Value::Undefined],
        )
        .is_err()
    );

    let slice = get(&mut realm, &blob, "slice");
    let sliced = ops::call(
        &mut realm,
        &slice,
        blob.clone(),
        &[Value::Number(-2.0), Value::Undefined],
    )
    .unwrap();
    assert!(
        ops::call(
            &mut realm,
            &slice,
            blob.clone(),
            &[Value::Number(0.0), Value::Number(2.0)],
        )
        .is_ok()
    );
    let get_reader = get(&mut realm, &sliced, "getReader");
    let reader = ops::call(&mut realm, &get_reader, sliced.clone(), &[]).unwrap();
    let counts = Rc::new(RefCell::new(Vec::new()));
    let callback_counts = counts.clone();
    let callback = Value::Object(realm.function("pullCallback", 2, false, move |_, _, args| {
        let count = match args.first() {
            Some(Value::Number(count)) => *count as usize,
            _ => usize::MAX,
        };
        callback_counts.borrow_mut().push(count);
        Ok(Value::Undefined)
    }));
    let pull = get(&mut realm, &reader, "pull");
    for _ in 0..2 {
        ops::call(
            &mut realm,
            &pull,
            reader.clone(),
            std::slice::from_ref(&callback),
        )
        .unwrap();
    }
    assert_eq!(*counts.borrow(), [2, 0]);

    let buffer = get(&mut realm, &part, "buffer");
    let buffers = Value::Object(realm.array(vec![buffer.clone(), buffer]));
    let concat = get(&mut realm, &binding, "concat");
    let joined = ops::call(&mut realm, &concat, Value::Undefined, &[buffers]).unwrap();
    assert_eq!(
        crate::typed_array_builtin::buffer_of(&joined)
            .unwrap()
            .borrow()
            .as_slice(),
        &[0, 1, 2, 3, 0, 1, 2, 3]
    );

    let store = get(&mut realm, &binding, "storeDataObject");
    ops::call(
        &mut realm,
        &store,
        Value::Undefined,
        &[
            Value::str("blob:test"),
            blob,
            Value::Number(4.0),
            Value::str("text/plain"),
        ],
    )
    .unwrap();
    let get_stored = get(&mut realm, &binding, "getDataObject");
    let stored = ops::call(
        &mut realm,
        &get_stored,
        Value::Undefined,
        &[Value::str("blob:test")],
    )
    .unwrap();
    assert!(matches!(
        get(&mut realm, &stored, "length"),
        Value::Number(3.0)
    ));
    let revoke = get(&mut realm, &binding, "revokeObjectURL");
    ops::call(
        &mut realm,
        &revoke,
        Value::Undefined,
        &[Value::str("blob:test")],
    )
    .unwrap();
    assert!(matches!(
        ops::call(
            &mut realm,
            &get_stored,
            Value::Undefined,
            &[Value::str("blob:test")]
        )
        .unwrap(),
        Value::Undefined
    ));
}

#[test]
fn blob_binding_reads_file_paths_and_rejects_invalid_native_handles() {
    let mut realm = Realm::new();
    let binding = load(&mut realm, "binding/blob").unwrap();
    let path = std::env::temp_dir().join(format!("nativeboat-blob-{}.txt", std::process::id()));
    std::fs::write(&path, b"native blob").unwrap();
    let create_from_path = get(&mut realm, &binding, "createBlobFromFilePath");
    let result = ops::call(
        &mut realm,
        &create_from_path,
        Value::Undefined,
        &[Value::str(path.to_str().unwrap())],
    )
    .unwrap();
    let Value::Object(tuple) = result else {
        panic!("file-backed Blob tuple expected");
    };
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(tuple), &Key::Index(1)).unwrap(),
        Value::Number(11.0)
    ));
    std::fs::remove_file(&path).unwrap();

    let invalid = ops::call(
        &mut realm,
        &create_from_path,
        Value::Undefined,
        &[Value::str("N:\\nativeboat-missing-blob")],
    )
    .unwrap();
    assert!(matches!(invalid, Value::Undefined));
}

#[test]
fn type_checks_and_library_globals() {
    let mut realm = Realm::new();
    let types = load(&mut realm, "binding/types").unwrap();
    let constructor = {
        let global = Value::Object(realm.global.clone());
        get(&mut realm, &global, "Map")
    };
    let map = ops::construct(&mut realm, &constructor, &[]).unwrap();
    assert_eq!(
        invoke(&mut realm, &types, "isMap", std::slice::from_ref(&map)),
        "true"
    );
    assert_eq!(
        invoke(&mut realm, &types, "isSet", std::slice::from_ref(&map)),
        "false"
    );
    assert_eq!(
        invoke(&mut realm, &types, "isMap", &[Value::Number(1.0)]),
        "false"
    );
    // Every check answers for every kind.
    let names: Vec<String> = types
        .as_object()
        .unwrap()
        .own_keys()
        .iter()
        .map(|key| key.to_js().to_rust_lossy())
        .collect();
    for name in &names {
        let shown = invoke(&mut realm, &types, name, std::slice::from_ref(&map));
        let expected = if name == "isMap" { "true" } else { "false" };
        assert_eq!(shown, expected, "{name}");
    }
    assert_eq!(names.len(), 27);
    let entries = get(&mut realm, &map, "entries");
    let iterator = ops::call(&mut realm, &entries, map.clone(), &[]).unwrap();
    for (name, expected) in [("isMapIterator", "true"), ("isSetIterator", "false")] {
        let shown = invoke(&mut realm, &types, name, std::slice::from_ref(&iterator));
        assert_eq!(shown, expected, "{name}");
    }
    // Library globals are undefined without the library and load from it.
    assert!(matches!(
        global(&mut realm, "node:buffer", "Buffer"),
        Ok(Value::Undefined)
    ));
    fn library(realm: &mut Realm, name: &str) -> Completion {
        let exports = realm.object();
        exports.define(Key::name("Buffer"), Property::data(Value::str(name)));
        Ok(Value::Object(exports))
    }
    realm.node_library = Some(library);
    let buffer = global(&mut realm, "node:buffer", "Buffer");
    assert_eq!(text_of(&mut realm, buffer), "'buffer'");
    let property = realm.global.own_property(&Key::name("Buffer")).unwrap();
    assert!(!property.enumerable && matches!(property.slot, crate::Slot::Data { .. }));
    let global_object = Value::Object(realm.global.clone());
    ops::set(
        &mut realm,
        &global_object,
        &Key::name("btoa"),
        Value::Number(9.0),
    )
    .unwrap();
    assert!(
        realm
            .global
            .own_property(&Key::name("btoa"))
            .unwrap()
            .enumerable
    );
    let btoa = global(&mut realm, "node:buffer", "btoa");
    assert_eq!(text_of(&mut realm, btoa), "9");
    fn failing(realm: &mut Realm, _: &str) -> Completion {
        Err(realm.throw(ErrorKind::Range, "library"))
    }
    let mut realm = Realm::new();
    realm.node_library = Some(failing);
    let result = global(&mut realm, "node:buffer", "atob");
    assert_eq!(text_of(&mut realm, result), "RangeError: library");
    fn missing_exports(realm: &mut Realm, _: &str) -> Completion {
        let exports = realm.object();
        exports.define(Key::name("atob"), throwing_accessor(realm, "export"));
        Ok(Value::Object(exports))
    }
    let mut realm = Realm::new();
    realm.node_library = Some(missing_exports);
    let result = global(&mut realm, "node:buffer", "atob");
    assert_eq!(text_of(&mut realm, result), "RangeError: export");
}

#[test]
fn abort_errors_and_event_error_codes() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let abort = get(&mut realm, &errors, "AbortError");
    let options = realm.object();
    options.define(Key::name("cause"), Property::data(Value::str("why")));
    for (args, expected) in [
        (vec![], "AbortError: The operation was aborted"),
        (
            vec![Value::str("stop"), Value::Object(options)],
            "AbortError: stop",
        ),
        (vec![Value::Number(1.0), Value::Null], "AbortError: 1"),
    ] {
        let error = ops::construct(&mut realm, &abort, &args).unwrap();
        assert_eq!(
            ops::to_string(&mut realm, &error).unwrap().to_rust_lossy(),
            expected
        );
        let code = get(&mut realm, &error, "code");
        assert_eq!(text_of(&mut realm, Ok(code)), "'ABORT_ERR'");
    }
    let cause_options = realm.object();
    cause_options.define(Key::name("cause"), Property::data(Value::Number(7.0)));
    let args = [Value::str("x"), Value::Object(cause_options)];
    let with_cause = ops::construct(&mut realm, &abort, &args).unwrap();
    let cause = get(&mut realm, &with_cause, "cause");
    assert_eq!(text_of(&mut realm, Ok(cause)), "7");
    let invalid = ops::construct(&mut realm, &abort, &[Value::str("x"), Value::Number(1.0)]);
    assert!(text_of(&mut realm, invalid).starts_with("TypeError [ERR_INVALID_ARG_TYPE]"));
    let called = ops::call(&mut realm, &abort, Value::Undefined, &[]);
    assert_eq!(
        text_of(&mut realm, called),
        "TypeError: Class constructor AbortError cannot be invoked without 'new'"
    );
    // Subclasses choose the prototype; a non-object prototype falls back.
    let target = Value::Object(realm.function("Sub", 0, true, |_, this, _| Ok(this)));
    let prototype = realm.object();
    ops::set(
        &mut realm,
        &target,
        &Key::name("prototype"),
        Value::Object(prototype.clone()),
    )
    .unwrap();
    let derived = ops::construct_target(&mut realm, &abort, &[], &target).unwrap();
    assert_eq!(
        derived.as_object().unwrap().data().prototype,
        Some(prototype)
    );
    ops::set(
        &mut realm,
        &target,
        &Key::name("prototype"),
        Value::Number(1.0),
    )
    .unwrap();
    let fallback = ops::construct_target(&mut realm, &abort, &[], &target).unwrap();
    let abort_prototype = get(&mut realm, &abort, "prototype");
    assert_eq!(
        fallback.as_object().unwrap().data().prototype,
        abort_prototype.as_object().cloned()
    );
    let symbol = Value::Symbol(crate::Symbol::new(None));
    let failing = ops::construct(&mut realm, &abort, std::slice::from_ref(&symbol));
    assert!(text_of(&mut realm, failing).starts_with("TypeError"));
    let throwing = realm.object();
    throwing.define(Key::name("cause"), throwing_accessor(&realm, "cause"));
    let failing = ops::construct(
        &mut realm,
        &abort,
        &[Value::str("x"), Value::Object(throwing)],
    );
    assert_eq!(text_of(&mut realm, failing), "RangeError: cause");
    let codes = get(&mut realm, &errors, "codes");
    for (code, args, expected) in [
        (
            "ERR_ARG_NOT_ITERABLE",
            vec![Value::str("Query pairs")],
            "TypeError [ERR_ARG_NOT_ITERABLE]: Query pairs must be iterable",
        ),
        (
            "ERR_INVALID_TUPLE",
            vec![Value::str("Each query pair"), Value::str("[name, value]")],
            "TypeError [ERR_INVALID_TUPLE]: Each query pair must be an iterable [name, value] tuple",
        ),
        (
            "ERR_INVALID_THIS",
            vec![Value::str("EventTarget")],
            "TypeError [ERR_INVALID_THIS]: Value of \"this\" must be of type EventTarget",
        ),
        (
            "ERR_UNHANDLED_ERROR",
            vec![],
            "Error [ERR_UNHANDLED_ERROR]: Unhandled error.",
        ),
        (
            "ERR_UNHANDLED_ERROR",
            vec![Value::str("'x'")],
            "Error [ERR_UNHANDLED_ERROR]: Unhandled error. ('x')",
        ),
    ] {
        let class = get(&mut realm, &codes, code);
        let error = ops::construct(&mut realm, &class, &args).unwrap();
        assert_eq!(
            ops::to_string(&mut realm, &error).unwrap().to_rust_lossy(),
            expected
        );
    }
    for code in ["ERR_INVALID_THIS", "ERR_UNHANDLED_ERROR"] {
        let class = get(&mut realm, &codes, code);
        let result = ops::construct(&mut realm, &class, std::slice::from_ref(&symbol));
        let shown = text_of(&mut realm, result);
        assert!(
            shown.starts_with("TypeError") || shown.contains("Symbol()"),
            "{code}: {shown}"
        );
    }
    assert!(matches!(
        get(&mut realm, &errors, "kEnhanceStackBeforeInspector"),
        Value::Symbol(_)
    ));
}

#[test]
fn identical_sequences_and_shallow_inspection() {
    let mut realm = Realm::new();
    let inspect = load(&mut realm, "internal/util/inspect").unwrap();
    let numbers = |realm: &Realm, values: &[f64]| {
        Value::Object(realm.array(values.iter().map(|value| Value::Number(*value)).collect()))
    };
    for (a, b, expected) in [
        (
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            vec![9.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            "{ len: 5, offset: 1 }",
        ),
        (
            vec![1.0, 2.0, 3.0, 4.0, 5.0],
            vec![1.0, 2.0, 3.0, 9.0, 9.0],
            "{ len: 0, offset: 0 }",
        ),
        (
            vec![1.0, 2.0, 3.0, 4.0, 5.0],
            vec![7.0],
            "{ len: 0, offset: 0 }",
        ),
        (
            vec![1.0, 2.0, 3.0, 4.0, 5.0],
            vec![8.0, 8.0, 8.0, 3.0, 4.0],
            "{ len: 0, offset: 0 }",
        ),
        (vec![1.0], vec![1.0], "{ len: 0, offset: 0 }"),
    ] {
        let args = [numbers(&realm, &a), numbers(&realm, &b)];
        assert_eq!(
            invoke(&mut realm, &inspect, "identicalSequenceRange", &args),
            expected
        );
    }
    let shown = invoke(
        &mut realm,
        &inspect,
        "identicalSequenceRange",
        &[Value::Number(1.0)],
    );
    assert!(shown.starts_with("TypeError"));
    let args = [numbers(&realm, &[1.0]), Value::Number(1.0)];
    let shown = invoke(&mut realm, &inspect, "identicalSequenceRange", &args);
    assert!(shown.starts_with("TypeError"));
    let shallow = realm.object();
    shallow.define(Key::name("depth"), Property::data(Value::Number(-1.0)));
    let full = realm.object();
    full.define(Key::name("a"), Property::data(Value::Number(1.0)));
    let shown = invoke(
        &mut realm,
        &inspect,
        "inspect",
        &[Value::Object(full), Value::Object(shallow)],
    );
    assert_eq!(shown, "'[Object]'");
}

#[test]
fn system_errors_describe_the_failed_call() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    let class = get(&mut realm, &codes, "ERR_SYSTEM_ERROR");
    let hidden = get(&mut realm, &class, "HideStackFramesError");
    assert!(crate::same_value(&class, &hidden));
    let context = realm.object();
    for (key, value) in [
        ("errno", Value::Number(-4040.0)),
        ("code", Value::str("ESRCH")),
        ("message", Value::str("no such process")),
        ("syscall", Value::str("uv_os_getpriority")),
    ] {
        context.define(Key::name(key), Property::data(value));
    }
    let context = Value::Object(context);
    let error = ops::construct(&mut realm, &class, std::slice::from_ref(&context)).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &error).unwrap().to_rust_lossy(),
        "SystemError [ERR_SYSTEM_ERROR]: A system error occurred: uv_os_getpriority returned ESRCH (no such process)"
    );
    let keys: Vec<String> = error
        .as_object()
        .unwrap()
        .own_keys()
        .into_iter()
        .filter(|key| {
            error
                .as_object()
                .unwrap()
                .own_property(key)
                .is_some_and(|property| property.enumerable)
        })
        .map(|key| format!("{key:?}"))
        .collect();
    assert_eq!(keys.len(), 4, "{keys:?}");
    let errno = get(&mut realm, &error, "errno");
    assert_eq!(text_of(&mut realm, Ok(errno)), "-4040");
    ops::set(&mut realm, &error, &Key::name("errno"), Value::Number(5.0)).unwrap();
    let changed = get(&mut realm, &context, "errno");
    assert_eq!(text_of(&mut realm, Ok(changed)), "5");
    let info = get(&mut realm, &error, "info");
    assert!(crate::same_value(&info, &context));
    // Paths and destinations extend the message and are exposed too.
    for (key, value) in [("path", "/a"), ("dest", "/b")] {
        let target = context.as_object().unwrap();
        target.define(Key::name(key), Property::data(Value::str(value)));
    }
    let error = ops::construct(&mut realm, &class, std::slice::from_ref(&context)).unwrap();
    let message = get(&mut realm, &error, "message");
    assert_eq!(
        text_of(&mut realm, Ok(message)),
        "'A system error occurred: uv_os_getpriority returned ESRCH (no such process) /a => /b'"
    );
    let dest = get(&mut realm, &error, "dest");
    assert_eq!(text_of(&mut realm, Ok(dest)), "'/b'");
    let throwing = realm.object();
    throwing.define(Key::name("syscall"), throwing_accessor(&realm, "syscall"));
    let failed = ops::construct(&mut realm, &class, &[Value::Object(throwing)]);
    assert_eq!(text_of(&mut realm, failed), "RangeError: syscall");
}

#[test]
fn url_search_params_globals_and_inspect_symbols_use_native_registry_identity() {
    let mut realm = Realm::new();
    assert!(matches!(
        global(&mut realm, "node:url", "URLSearchParams"),
        Ok(Value::Undefined)
    ));
    let module = load(&mut realm, "internal/util/inspect").unwrap();
    let inspect = get(&mut realm, &module, "inspect");
    let custom = get(&mut realm, &inspect, "custom");
    let global_object = Value::Object(realm.global.clone());
    let symbol = get(&mut realm, &global_object, "Symbol");
    let symbol_for = get(&mut realm, &symbol, "for");
    let expected = ops::call(
        &mut realm,
        &symbol_for,
        symbol,
        &[Value::str("nodejs.util.inspect.custom")],
    )
    .unwrap();
    assert!(crate::same_value(&custom, &expected));
    fn library(realm: &mut Realm, name: &str) -> Completion {
        assert_eq!(name, "url");
        let exports = realm.object();
        exports.define(
            Key::name("URLSearchParams"),
            Property::data(Value::Number(42.0)),
        );
        Ok(Value::Object(exports))
    }
    realm.node_library = Some(library);
    let value = global(&mut realm, "node:url", "URLSearchParams").unwrap();
    assert!(crate::same_value(&value, &Value::Number(42.0)));
    let property = realm
        .global
        .own_property(&Key::name("URLSearchParams"))
        .unwrap();
    assert!(!property.enumerable && property.configurable);
    ops::set(
        &mut realm,
        &global_object,
        &Key::name("URLSearchParams"),
        Value::Number(9.0),
    )
    .unwrap();
    let value = global(&mut realm, "node:url", "URLSearchParams").unwrap();
    assert!(crate::same_value(&value, &Value::Number(9.0)));
}

#[test]
fn node_private_symbols_share_only_the_agent_and_stay_out_of_symbol_for() {
    fn key(realm: &mut Realm) -> Value {
        let util = load(realm, "binding/util").unwrap();
        let private = get(realm, &util, "privateSymbols");
        get(realm, &private, "transfer_mode_private_symbol")
    }
    let mut first = Realm::new();
    let private = key(&mut first);
    let mut shared = first.sibling();
    assert!(crate::same_value(&private, &key(&mut shared)));
    let mut unrelated = Realm::new();
    assert!(!crate::same_value(&private, &key(&mut unrelated)));
    let public = Value::Symbol(crate::symbol_builtin::intern(
        &first,
        crate::JsStr::from("node:transfer_mode"),
    ));
    assert!(!crate::same_value(&private, &public));
    let Value::Symbol(symbol) = private else {
        panic!()
    };
    assert!(!crate::symbol_builtin::is_registered(&first, &symbol));
}

#[test]
fn url_coded_errors_keep_fields_messages_and_scheme_validation() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    let invalid = get(&mut realm, &codes, "ERR_INVALID_URL");
    for args in [
        vec![Value::str("bad")],
        vec![Value::str("bad"), Value::Null],
        vec![Value::str("bad"), Value::str("base")],
    ] {
        let error = ops::construct(&mut realm, &invalid, &args).unwrap();
        assert!(crate::same_value(
            &get(&mut realm, &error, "input"),
            &Value::str("bad")
        ));
        assert!(crate::same_value(
            &get(&mut realm, &error, "message"),
            &Value::str("Invalid URL")
        ));
        let expected = if args.len() == 2 && !matches!(args[1], Value::Null) {
            Value::str("base")
        } else {
            Value::Undefined
        };
        assert!(crate::same_value(
            &get(&mut realm, &error, "base"),
            &expected
        ));
    }
    for (code, args, expected) in [
        (
            "ERR_INVALID_FILE_URL_HOST",
            vec![Value::str("linux")],
            "File URL host must be \"localhost\" or empty on linux",
        ),
        (
            "ERR_INVALID_FILE_URL_PATH",
            vec![Value::str("must be absolute")],
            "File URL path must be absolute",
        ),
        (
            "ERR_NO_CRYPTO",
            vec![],
            "Node.js is not compiled with OpenSSL crypto support",
        ),
        (
            "ERR_INVALID_URL_SCHEME",
            vec![Value::str("file")],
            "The URL must be of scheme file",
        ),
    ] {
        let constructor = get(&mut realm, &codes, code);
        let result = ops::construct(&mut realm, &constructor, &args);
        assert_eq!(message(&mut realm, result), expected);
    }
    let scheme = get(&mut realm, &codes, "ERR_INVALID_URL_SCHEME");
    for (values, expected) in [
        (vec![], "The URL must be of scheme undefined"),
        (vec![Value::str("a")], "The URL must be of scheme a"),
        (
            vec![Value::str("a"), Value::str("b")],
            "The URL must be one of scheme a or b",
        ),
    ] {
        let array = Value::Object(realm.array(values));
        let result = ops::construct(&mut realm, &scheme, &[array]);
        assert_eq!(message(&mut realm, result), expected);
    }
    let array = Value::Object(realm.array(vec![Value::str("a"), Value::str("b"), Value::str("c")]));
    let error = ops::construct(&mut realm, &scheme, &[array]).unwrap_err().0;
    assert!(crate::same_value(
        &get(&mut realm, &error, "code"),
        &Value::str("ERR_INTERNAL_ASSERTION")
    ));
    assert!(ops::construct(&mut realm, &scheme, &[Value::Undefined]).is_err());
    let array = Value::Object(realm.array(vec![Value::Symbol(crate::Symbol::new(None))]));
    assert!(ops::construct(&mut realm, &scheme, &[array]).is_err());
}

#[test]
fn filesystem_prerequisite_errors_and_internal_assertions_keep_node_messages() {
    let mut realm = Realm::new();
    let errors = load(&mut realm, "internal/errors").unwrap();
    let codes = get(&mut realm, &errors, "codes");
    for (code, args, expected) in [
        (
            "ERR_FS_INVALID_SYMLINK_TYPE",
            vec![Value::str("invalid")],
            "Symlink type must be one of \"dir\", \"file\", or \"junction\". Received \"invalid\"",
        ),
        (
            "ERR_INCOMPATIBLE_OPTION_PAIR",
            vec![Value::str("a"), Value::str("b")],
            "Option \"a\" cannot be used in combination with option \"b\"",
        ),
        (
            "ERR_INTERNAL_ASSERTION",
            vec![Value::str("failure")],
            "failure\nThis is caused by either a bug in Node.js or incorrect usage of Node.js internals.\nPlease open an issue with this stack trace at https://github.com/nodejs/node/issues\n",
        ),
        (
            "ERR_INTERNAL_ASSERTION",
            vec![Value::Undefined],
            "This is caused by either a bug in Node.js or incorrect usage of Node.js internals.\nPlease open an issue with this stack trace at https://github.com/nodejs/node/issues\n",
        ),
    ] {
        let ctor = get(&mut realm, &codes, code);
        let result = ops::construct(&mut realm, &ctor, &args);
        assert_eq!(message(&mut realm, result), expected);
    }
    let ctor = get(&mut realm, &codes, "ERR_INTERNAL_ASSERTION");
    let missing = ops::construct(&mut realm, &ctor, &[]).unwrap_err();
    assert!(
        shown(&mut realm, missing)
            .contains("The provided arguments length (0) does not match the required ones (1)")
    );
    let context = realm.object();
    for (key, value) in [
        ("errno", Value::Number(-4068.0)),
        ("code", Value::str("EISDIR")),
        ("message", Value::str("illegal operation on a directory")),
        ("syscall", Value::str("read")),
        ("path", Value::str("/dir")),
    ] {
        context.define(Key::name(key), Property::data(value));
    }
    let ctor = get(&mut realm, &codes, "ERR_FS_EISDIR");
    let result = ops::construct(&mut realm, &ctor, &[Value::Object(context)]);
    assert_eq!(
        message(&mut realm, result),
        "Path is a directory: read returned EISDIR (illegal operation on a directory) /dir"
    );
}

fn uv_context(realm: &Realm, errno: Value) -> Value {
    let object = realm.object();
    for (key, value) in [("errno", errno), ("syscall", Value::str("open"))] {
        object.define(Key::name(key), Property::data(value));
    }
    Value::Object(object)
}

#[test]
fn uv_exception_propagates_call_limit_and_restores_constructor_state() {
    let mut realm = Realm::new();
    let exports = load(&mut realm, "internal/errors").unwrap();
    let ctor = get(&mut realm, &exports, "UVException");
    let context = uv_context(&realm, Value::Number(-4058.0));
    realm.max_depth = 1;
    let error = ops::construct(&mut realm, &ctor, core::slice::from_ref(&context)).unwrap_err();
    realm.max_depth = 4_000;
    assert!(shown(&mut realm, error).contains("Maximum call stack size exceeded"));
    assert!(matches!(realm.new_target(), Value::Undefined));
    let result = ops::construct(&mut realm, &ctor, &[context]);
    assert_eq!(
        message(&mut realm, result),
        "ENOENT: no such file or directory, open"
    );
}

#[test]
fn uv_error_constructor_brands_fallbacks_and_shared_rows_match_node() {
    let mut realm = Realm::new();
    let exports = load(&mut realm, "internal/errors").unwrap();
    let ctor = get(&mut realm, &exports, "UVException");
    let map = get(&mut realm, &exports, "uvErrmapGet");
    let row = ops::call(
        &mut realm,
        &map,
        Value::Undefined,
        &[Value::Number(-4058.0)],
    )
    .unwrap();
    let same = ops::call(
        &mut realm,
        &map,
        Value::Undefined,
        &[Value::Number(-4058.0)],
    )
    .unwrap();
    assert!(ops::strict_equals(&row, &same));
    for key in [
        Value::Number(0.0),
        Value::Number(f64::NAN),
        Value::str("-4058"),
    ] {
        assert!(matches!(
            ops::call(&mut realm, &map, Value::Undefined, &[key]).unwrap(),
            Value::Undefined
        ));
    }
    let context = uv_context(&realm, Value::Number(-4058.0));
    let error = ops::construct(&mut realm, &ctor, core::slice::from_ref(&context)).unwrap();
    assert!(matches!(
        error.as_object().unwrap().data().kind,
        Kind::Error
    ));
    assert!(ops::instance_of(&mut realm, &error, &ctor).unwrap());
    let original = Value::Object(realm.error_constructor.clone().unwrap());
    assert!(crate::same_value(
        &get(&mut realm, &error, "constructor"),
        &original
    ));
    let inherited = Value::Object(ctor.as_object().unwrap().data().prototype.clone().unwrap());
    assert!(crate::same_value(&inherited, &original));
    assert_eq!(
        message(&mut realm, Ok(error)),
        "ENOENT: no such file or directory, open"
    );
    ops::set(&mut realm, &row, &Key::Index(0), Value::str("CHANGED")).unwrap();
    ops::set(
        &mut realm,
        &row,
        &Key::Index(1),
        Value::str("changed message"),
    )
    .unwrap();
    let result = ops::construct(&mut realm, &ctor, core::slice::from_ref(&context));
    assert_eq!(
        message(&mut realm, result),
        "CHANGED: changed message, open"
    );
    let unknown = uv_context(&realm, Value::Number(12345.0));
    let result = ops::construct(&mut realm, &ctor, &[unknown]);
    assert_eq!(message(&mut realm, result), "UNKNOWN: unknown error, open");
    let called = ops::call(
        &mut realm,
        &ctor,
        Value::Undefined,
        core::slice::from_ref(&context),
    )
    .unwrap_err();
    assert!(shown(&mut realm, called).contains("cannot be invoked without 'new'"));
    let custom = Value::Object(realm.function("Target", 0, true, |_, _, _| Ok(Value::Undefined)));
    let result =
        ops::construct_target(&mut realm, &ctor, core::slice::from_ref(&context), &custom).unwrap();
    assert!(ops::instance_of(&mut realm, &result, &custom).unwrap());
    assert!(!ops::instance_of(&mut realm, &result, &ctor).unwrap());
    ops::set(
        &mut realm,
        &custom,
        &Key::name("prototype"),
        Value::Number(3.0),
    )
    .unwrap();
    let result = ops::construct_target(&mut realm, &ctor, &[context], &custom).unwrap();
    assert_eq!(
        result.as_object().unwrap().data().prototype,
        Some(realm.error_prototype.clone())
    );
}

#[test]
fn uv_exception_paths_enumeration_and_failure_order_preserve_context_observations() {
    let mut realm = Realm::new();
    let exports = load(&mut realm, "internal/errors").unwrap();
    let ctor = get(&mut realm, &exports, "UVException");
    let context = uv_context(&realm, Value::Number(-4058.0));
    let object = context.as_object().unwrap().clone();
    for (key, value) in [
        ("message", Value::str("custom")),
        ("path", Value::str("ü")),
        ("dest", Value::str("中")),
        ("extra", Value::Number(3.0)),
    ] {
        object.define(Key::name(key), Property::data(value));
    }
    object.define(Key::name("hidden"), Property::hidden(Value::Boolean(true)));
    let symbol = Key::Symbol(crate::Symbol::new(None));
    object.define(symbol.clone(), Property::data(Value::Boolean(true)));
    let result = ops::construct(&mut realm, &ctor, core::slice::from_ref(&context)).unwrap();
    assert_eq!(
        message(&mut realm, Ok(result.clone())),
        "ENOENT: custom, open 'ü' -> '中'"
    );
    assert!(!result.as_object().unwrap().own_property(&symbol).is_some());
    assert!(
        !result
            .as_object()
            .unwrap()
            .own_property(&Key::name("hidden"))
            .is_some()
    );
    assert!(crate::same_value(
        &get(&mut realm, &result, "extra"),
        &Value::Number(3.0)
    ));
    for (name, returned) in [("path", Value::Undefined), ("dest", Value::Number(0.0))] {
        let path = realm.object();
        let conversion = realm.function("toString", 0, false, move |_, _, _| Ok(returned.clone()));
        path.define(
            Key::name("toString"),
            Property::data(Value::Object(conversion)),
        );
        object.define(Key::name(name), Property::data(Value::Object(path)));
    }
    let result = ops::construct(&mut realm, &ctor, core::slice::from_ref(&context)).unwrap();
    assert_eq!(
        message(&mut realm, Ok(result.clone())),
        "ENOENT: custom, open 'undefined' -> '0'"
    );
    assert!(
        !result
            .as_object()
            .unwrap()
            .own_property(&Key::name("path"))
            .is_some()
    );
    assert!(
        !result
            .as_object()
            .unwrap()
            .own_property(&Key::name("dest"))
            .is_some()
    );
    for name in ["errno", "message", "syscall", "path", "dest", "extra"] {
        let ctx = uv_context(&realm, Value::Number(-4058.0));
        ctx.as_object()
            .unwrap()
            .define(Key::name(name), throwing_accessor(&realm, name));
        let failed = ops::construct(&mut realm, &ctor, &[ctx]);
        assert_eq!(text_of(&mut realm, failed), format!("RangeError: {name}"));
    }
    let plain = Value::Object(realm.object());
    let result = ops::construct(&mut realm, &ctor, &[plain]);
    assert_eq!(
        message(&mut realm, result),
        "UNKNOWN: unknown error, undefined"
    );
    let result = ops::construct(&mut realm, &ctor, &[Value::Null]);
    assert!(result.is_err());
}

#[test]
fn uv_exception_uses_original_error_constructor_after_global_and_prototype_edits() {
    let mut realm = Realm::new();
    let original = Value::Object(realm.error_constructor.clone().unwrap());
    let poison = Value::Object(realm.function("poison", 0, true, |_, _, _| {
        panic!("poisoned Error must not be called")
    }));
    let global = Value::Object(realm.global.clone());
    ops::set(&mut realm, &global, &Key::name("Error"), poison.clone()).unwrap();
    realm
        .error_prototype
        .define(Key::name("constructor"), Property::data(poison));
    let exports = load(&mut realm, "internal/errors").unwrap();
    let ctor = get(&mut realm, &exports, "UVException");
    let context = uv_context(&realm, Value::Number(-4058.0));
    let error = ops::construct(&mut realm, &ctor, &[context]).unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &error, "constructor"),
        &original
    ));
    assert!(ops::instance_of(&mut realm, &error, &original).unwrap());
}

#[test]
fn uv_exception_messages_preserve_unpaired_utf16_units() {
    let mut realm = Realm::new();
    let exports = load(&mut realm, "internal/errors").unwrap();
    let ctor = get(&mut realm, &exports, "UVException");
    let context = uv_context(&realm, Value::Number(-4058.0));
    let object = context.as_object().unwrap();
    for name in ["message", "syscall", "path", "dest"] {
        object.define(
            Key::name(name),
            Property::data(Value::String(crate::JsStr::from_units(&[0xd800]))),
        );
    }
    let error = ops::construct(&mut realm, &ctor, &[context]).unwrap();
    let message = get(&mut realm, &error, "message");
    let Value::String(message) = message else {
        panic!("messages are strings")
    };
    let expected = "ENOENT: "
        .encode_utf16()
        .chain([0xd800])
        .chain(", ".encode_utf16())
        .chain([0xd800])
        .chain(" '".encode_utf16())
        .chain([0xd800])
        .chain("' -> '".encode_utf16())
        .chain([0xd800])
        .chain("'".encode_utf16())
        .collect::<Vec<_>>();
    assert_eq!(message.units(), expected);
}

#[test]
fn bindings_serve_the_web_streams_encoding_and_blob_sources() {
    let mut realm = Realm::new();
    let constants = load(&mut realm, "binding/constants").unwrap();
    let zlib = get(&mut realm, &constants, "zlib");
    assert!(crate::same_value(
        &get(&mut realm, &zlib, "Z_SYNC_FLUSH"),
        &Value::Number(2.0)
    ));
    assert!(crate::same_value(
        &get(&mut realm, &zlib, "Z_MAX_CHUNK"),
        &Value::Number(f64::INFINITY)
    ));

    // Promise details: [state] or [state, result].
    let util = load(&mut realm, "binding/util").unwrap();
    let details = get(&mut realm, &util, "getPromiseDetails");
    let (pending, _, _) = crate::promise_builtin::capability(&realm);
    let state = ops::call(&mut realm, &details, Value::Undefined, &[pending]).unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &state, "length"),
        &Value::Number(1.0)
    ));
    assert!(crate::same_value(
        &get(&mut realm, &state, "0"),
        &Value::Number(0.0)
    ));
    let plain = ops::call(
        &mut realm,
        &details,
        Value::Undefined,
        &[Value::Number(1.0)],
    )
    .unwrap();
    assert!(matches!(plain, Value::Undefined));

    // Copies between array buffers, overlapping or not.
    let buffer = load(&mut realm, "binding/buffer").unwrap();
    let copy = get(&mut realm, &buffer, "copyArrayBuffer");
    let source = crate::typed_array_builtin::new_bytes(&realm, alloc::vec![1, 2, 3, 4], None);
    let target = crate::typed_array_builtin::new_bytes(&realm, alloc::vec![0; 4], None);
    let source_buffer = get(&mut realm, &Value::Object(source), "buffer");
    let target_buffer = get(&mut realm, &Value::Object(target.clone()), "buffer");
    ops::call(
        &mut realm,
        &copy,
        Value::Undefined,
        &[
            target_buffer.clone(),
            Value::Number(1.0),
            source_buffer.clone(),
            Value::Number(0.0),
            Value::Number(3.0),
        ],
    )
    .unwrap();
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&Value::Object(target)).unwrap(),
        [0, 1, 2, 3]
    );

    // Node's TextEncoder and TextDecoder natives.
    let encoding = load(&mut realm, "binding/encoding_binding").unwrap();
    let encode = get(&mut realm, &encoding, "encodeUtf8String");
    let bytes = ops::call(
        &mut realm,
        &encode,
        Value::Undefined,
        &[Value::str("h\u{e9}")],
    )
    .unwrap();
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&bytes).unwrap(),
        [b'h', 0xc3, 0xa9]
    );
    let into = get(&mut realm, &encoding, "encodeInto");
    let results = get(&mut realm, &encoding, "encodeIntoResults");
    let small = crate::typed_array_builtin::new_bytes(&realm, alloc::vec![0; 2], None);
    ops::call(
        &mut realm,
        &into,
        Value::Undefined,
        &[Value::str("h\u{e9}"), Value::Object(small)],
    )
    .unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &results, "0"),
        &Value::Number(1.0)
    ));
    assert!(crate::same_value(
        &get(&mut realm, &results, "1"),
        &Value::Number(1.0)
    ));
    let decode = get(&mut realm, &encoding, "decodeUTF8");
    let marked =
        crate::typed_array_builtin::new_bytes(&realm, alloc::vec![0xef, 0xbb, 0xbf, b'x'], None);
    let text = ops::call(
        &mut realm,
        &decode,
        Value::Undefined,
        &[
            Value::Object(marked),
            Value::Boolean(false),
            Value::Boolean(false),
        ],
    )
    .unwrap();
    assert!(crate::same_value(&text, &Value::str("x")));

    // Blob parts may be other Blobs' handles.
    let blob = load(&mut realm, "binding/blob").unwrap();
    let create = get(&mut realm, &blob, "createBlob");
    let part = crate::typed_array_builtin::new_bytes(&realm, alloc::vec![7, 8], None);
    let parts = Value::Object(realm.array(alloc::vec![Value::Object(part)]));
    let inner = ops::call(
        &mut realm,
        &create,
        Value::Undefined,
        &[parts, Value::Number(2.0)],
    )
    .unwrap();
    let tail = crate::typed_array_builtin::new_bytes(&realm, alloc::vec![9], None);
    let parts = Value::Object(realm.array(alloc::vec![inner, Value::Object(tail)]));
    assert!(
        ops::call(
            &mut realm,
            &create,
            Value::Undefined,
            &[parts, Value::Number(3.0)]
        )
        .is_ok()
    );

    let wrap = load(&mut realm, "binding/stream_wrap").unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &wrap, "kLastWriteWasAsync"),
        &Value::Number(3.0)
    ));
}

#[test]
fn uv_binding_names_libuv_errors() {
    let mut realm = Realm::new();
    let uv = load(&mut realm, "binding/uv").unwrap();
    assert!(crate::same_value(
        &get(&mut realm, &uv, "UV_ENOENT"),
        &Value::Number(-4058.0)
    ));
    let errname = get(&mut realm, &uv, "errname");
    let name = ops::call(
        &mut realm,
        &errname,
        Value::Undefined,
        &[Value::Number(-4058.0)],
    )
    .unwrap();
    assert!(crate::same_value(&name, &Value::str("ENOENT")));
    let message = get(&mut realm, &uv, "getErrorMessage");
    let text = ops::call(
        &mut realm,
        &message,
        Value::Undefined,
        &[Value::Number(-4058.0)],
    )
    .unwrap();
    assert!(crate::same_value(
        &text,
        &Value::str("no such file or directory")
    ));
}
