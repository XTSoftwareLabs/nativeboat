use super::*;

fn call(
    realm: &mut Realm,
    target: &Object,
    name: &'static str,
    args: &[Value],
) -> Result<Value, Thrown> {
    let function = ops::get_named(realm, &Value::Object(target.clone()), name)?;
    ops::call(realm, &function, Value::Object(target.clone()), args)
}

/// A handler that calls back at once with what `answer` makes of the URL.
fn file_handler(
    realm: &mut Realm,
    answer: impl Fn(&mut Realm, String) -> Value + 'static,
) -> Value {
    Value::Object(realm.function("handler", 2, false, move |realm, _, args| {
        let url = ops::get_named(realm, &args[0], "url")?;
        let url = ops::to_string(realm, &url)?.to_rust_lossy();
        let value = answer(realm, url);
        ops::call(realm, &args[1], Value::Undefined, &[value])?;
        Ok(Value::Undefined)
    }))
}

fn data_object(realm: &mut Realm, key: &str, value: Value) -> Value {
    let object = realm.object();
    object.define(Key::name(key), Property::data(value));
    Value::Object(object)
}

/// A `Response`-like object: `ok`, and `arrayBuffer()` giving a promise of
/// the body's buffer.
fn response(realm: &mut Realm, ok: bool, body: &[u8]) -> Value {
    let object = realm.object();
    object.define(Key::name("ok"), Property::data(Value::Boolean(ok)));
    let status = if ok { 200.0 } else { 404.0 };
    object.define(Key::name("status"), Property::data(Value::Number(status)));
    let body = body.to_vec();
    let read = realm.function("arrayBuffer", 0, false, move |realm, _, _| {
        let view = crate::typed_array_builtin::new_bytes(realm, body.clone(), None);
        let buffer = ops::get_named(realm, &Value::Object(view), "buffer")?;
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[buffer])?;
        Ok(promise)
    });
    object.define(
        Key::name("arrayBuffer"),
        Property::data(Value::Object(read)),
    );
    Value::Object(object)
}

/// The registry is the process's, so one test covers it: each kind of
/// handler, scheme matching, registration rules and refusals.
#[test]
fn registered_schemes_are_served_by_their_handlers() {
    let mut realm = Realm::new();
    let realm = &mut realm;
    let protocol = protocol(realm);
    assert!(!serves("app://host/a.css"));
    assert_eq!(serve("app://host/a.css"), None);

    // registerFileProtocol: a path, or `{ path }`, or `{ error }`.
    let handler = file_handler(realm, |realm, url| {
        if url.ends_with("/missing.css") {
            let error = data_object(realm, "error", Value::Number(-6.0));
            return error;
        }
        let path = alloc::format!("C:\\app\\{}", url.trim_start_matches("app://host/"));
        if url.ends_with(".png") {
            data_object(realm, "path", Value::str(&path))
        } else {
            Value::str(&path)
        }
    });
    let registered = call(
        realm,
        &protocol,
        "registerFileProtocol",
        &[Value::str("App"), handler.clone()],
    );
    assert!(matches!(registered, Ok(Value::Boolean(true))));
    assert!(matches!(
        call(
            realm,
            &protocol,
            "registerFileProtocol",
            &[Value::str("app"), handler.clone()]
        ),
        Ok(Value::Boolean(false))
    ));
    assert!(matches!(
        call(
            realm,
            &protocol,
            "isProtocolRegistered",
            &[Value::str("app")]
        ),
        Ok(Value::Boolean(true))
    ));
    assert!(serves("APP://host/a.css"));
    assert!(!serves("other://host/a.css"));
    assert!(!serves("no scheme"));
    assert_eq!(
        serve("app://host/a.css"),
        Some(Served::File("C:\\app\\a.css".into()))
    );
    assert_eq!(
        serve(" app://host/b.png "),
        Some(Served::File("C:\\app\\b.png".into()))
    );
    assert_eq!(serve("app://host/missing.css"), None);
    assert!(
        call(
            realm,
            &protocol,
            "registerFileProtocol",
            &[Value::Number(1.0), handler.clone()]
        )
        .is_err()
    );
    assert!(
        call(
            realm,
            &protocol,
            "registerFileProtocol",
            &[Value::str("x"), Value::Null]
        )
        .is_err()
    );

    // registerBufferProtocol: bytes, or `{ data }`; registerStringProtocol:
    // text, or `{ data }`.
    let buffer = file_handler(realm, |realm, url| {
        let bytes = crate::typed_array_builtin::new_bytes(realm, url.clone().into_bytes(), None);
        if url.contains("wrapped") {
            data_object(realm, "data", Value::Object(bytes))
        } else {
            Value::Object(bytes)
        }
    });
    call(
        realm,
        &protocol,
        "registerBufferProtocol",
        &[Value::str("bytes"), buffer],
    )
    .unwrap();
    assert_eq!(serve("bytes:a"), Some(Served::Bytes(b"bytes:a".to_vec())));
    assert_eq!(
        serve("bytes:wrapped"),
        Some(Served::Bytes(b"bytes:wrapped".to_vec()))
    );
    let text = file_handler(realm, |realm, url| {
        if url.contains("wrapped") {
            data_object(realm, "data", Value::str("wrapped text"))
        } else {
            Value::str("text")
        }
    });
    call(
        realm,
        &protocol,
        "registerStringProtocol",
        &[Value::str("text"), text],
    )
    .unwrap();
    assert_eq!(serve("text:a"), Some(Served::Bytes(b"text".to_vec())));
    assert_eq!(
        serve("text:wrapped"),
        Some(Served::Bytes(b"wrapped text".to_vec()))
    );

    // A handler that never calls back serves nothing.
    let silent = Value::Object(realm.function("silent", 2, false, |_, _, _| Ok(Value::Undefined)));
    call(
        realm,
        &protocol,
        "registerFileProtocol",
        &[Value::str("silent"), silent],
    )
    .unwrap();
    assert_eq!(serve("silent:x"), None);

    // handle: a Response or a promise of one; one that is not ok serves
    // nothing. It is consulted before the scheme's older registration.
    let handle = Value::Object(realm.function("handle", 1, false, |realm, _, args| {
        let url = ops::get_named(realm, &args[0], "url")?;
        let url = ops::to_string(realm, &url)?.to_rust_lossy();
        let answer = response(realm, !url.ends_with("/refused"), url.as_bytes());
        if url.contains("later") {
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            ops::call(realm, &resolve, Value::Undefined, &[answer])?;
            Ok(promise)
        } else {
            Ok(answer)
        }
    }));
    call(
        realm,
        &protocol,
        "handle",
        &[Value::str("app"), handle.clone()],
    )
    .unwrap();
    assert!(
        call(
            realm,
            &protocol,
            "handle",
            &[Value::str("app"), handle.clone()]
        )
        .is_err()
    );
    assert!(matches!(
        call(realm, &protocol, "isProtocolHandled", &[Value::str("APP")]),
        Ok(Value::Boolean(true))
    ));
    assert_eq!(
        serve("app://host/now"),
        Some(Served::Bytes(b"app://host/now".to_vec()))
    );
    assert_eq!(
        serve("app://host/later"),
        Some(Served::Bytes(b"app://host/later".to_vec()))
    );
    assert_eq!(serve("app://host/refused"), None);
    // A renderer's fetch sees a failing status, which a page load ignores.
    assert_eq!(
        reply("app://host/refused").map(|reply| reply.status),
        Some(404)
    );

    // Removing a handled scheme leaves its registration; removing that ends
    // serving.
    call(realm, &protocol, "unhandle", &[Value::str("app")]).unwrap();
    assert!(matches!(
        call(realm, &protocol, "isProtocolHandled", &[Value::str("app")]),
        Ok(Value::Boolean(false))
    ));
    assert_eq!(
        serve("app://host/a.css"),
        Some(Served::File("C:\\app\\a.css".into()))
    );
    assert!(matches!(
        call(realm, &protocol, "unregisterProtocol", &[Value::str("app")]),
        Ok(Value::Boolean(true))
    ));
    assert!(matches!(
        call(realm, &protocol, "unregisterProtocol", &[Value::str("app")]),
        Ok(Value::Boolean(false))
    ));
    assert!(!serves("app://host/a.css"));

    // The renderer binding reads served files and bytes.
    let binding = binding(realm);
    let served = call(realm, &binding, "serve", &[Value::str("text:a")]).unwrap();
    let body = ops::get_named(realm, &served, "body").unwrap();
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&body),
        Some(b"text".to_vec())
    );
    assert!(matches!(
        ops::get_named(realm, &served, "status"),
        Ok(Value::Number(status)) if status == 200.0
    ));
    assert!(matches!(
        call(realm, &binding, "serve", &[Value::str("app://host/a.css")]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        call(realm, &binding, "serves", &[Value::str("bytes:x")]),
        Ok(Value::Boolean(true))
    ));
}
