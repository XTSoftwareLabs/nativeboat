use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn get(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn invalid_request_urls_fail_before_opening_a_winhttp_session() {
    let request = Request {
        url: "not a URL".into(),
        method: "POST".into(),
        headers: Vec::new(),
        body: Vec::new(),
        agent: None,
        insecure: false,
        upgrade: false,
    };
    let error = perform(&request, true).unwrap_err();
    assert_eq!(error.code, "ERR_INVALID_URL");
    assert_eq!(error.message, "Invalid URL");

    let hostless = Request {
        url: "mailto:user@example.com".into(),
        method: "GET".into(),
        headers: Vec::new(),
        body: Vec::new(),
        agent: None,
        insecure: false,
        upgrade: false,
    };
    let error = perform(&hostless, true).unwrap_err();
    assert_eq!(error.code, "ERR_INVALID_URL");
}

#[test]
fn response_headers_normalize_names_and_combine_duplicates_except_cookies() {
    let (status, headers) = parse_headers(
        "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Test: one\r\nX-Test: two\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n\r\n",
    );
    assert_eq!(status, "Created");
    assert_eq!(
        headers,
        [
            ("content-type".into(), "application/json".into()),
            ("x-test".into(), "one, two".into()),
            ("set-cookie".into(), "a=1".into()),
            ("set-cookie".into(), "b=2".into()),
        ]
    );
}

#[test]
fn winhttp_failures_keep_node_network_error_codes() {
    for (native, expected) in [
        (12002, "ETIMEDOUT"),
        (12007, "ENOTFOUND"),
        (12029, "ECONNREFUSED"),
        (12030, "ECONNRESET"),
        (12152, "ECONNRESET"),
        (12037, "CERT_INVALID"),
        (12175, "CERT_INVALID"),
        (5, "EIO"),
    ] {
        let failure = win_failure_code("send", native);
        assert_eq!(failure.code, expected);
        assert!(failure.message.starts_with("send: "));
        assert!(
            failure
                .message
                .ends_with(&format!("(WinHTTP error {native})"))
        );
    }
}

#[test]
fn winhttp_client_sends_a_request_and_reads_status_headers_and_body() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let read = socket.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).contains("POST /sample?x=1 HTTP/1.1"));
        socket
            .write_all(
                b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\nX-Reply: native\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\nok",
            )
            .unwrap();
    });
    let response = perform(
        &Request {
            url: format!("http://{address}/sample?x=1"),
            method: "POST".into(),
            headers: vec![("Content-Length".into(), "0".into())],
            body: Vec::new(),
            agent: None,
            insecure: false,
            upgrade: false,
        },
        true,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.status_text, "Created");
    assert_eq!(response.body, b"ok");
    for expected in [
        ("content-length", "2"),
        ("x-reply", "native"),
        ("set-cookie", "a=1"),
        ("set-cookie", "b=2"),
        ("connection", "close"),
    ] {
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| { name == expected.0 && value == expected.1 })
        );
    }
}

/// The request line and headers of a request `perform` sends.
fn sent(agent: Option<&str>, headers: Vec<(String, String)>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let read = socket.read(&mut request).unwrap();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let request = Request {
        url: format!("http://{address}/"),
        method: "GET".into(),
        headers,
        body: Vec::new(),
        agent: agent.map(Into::into),
        insecure: false,
        upgrade: false,
    };
    perform(&request, true).unwrap();
    server.join().unwrap()
}

#[test]
fn requests_send_a_user_agent_only_when_given_one() {
    // Node's `https` sends none of its own.
    assert!(
        !sent(None, Vec::new())
            .to_ascii_lowercase()
            .contains("user-agent")
    );
    let request = sent(Some("Test/1"), Vec::new());
    assert!(request.contains("User-Agent: Test/1\r\n"), "{request}");
    // A user agent in the headers replaces the given one.
    let request = sent(
        Some("Test/1"),
        vec![("User-Agent".into(), "axios/1.13.2".into())],
    );
    assert!(
        request.contains("User-Agent: axios/1.13.2\r\n"),
        "{request}"
    );
    assert!(!request.contains("Test/1"), "{request}");
}

#[test]
fn electron_net_fetch_exposes_response_headers_and_a_readable_body() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let read = socket.read(&mut request).unwrap();
        // Electron's network service sends the application's user agent.
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.contains("User-Agent: Mozilla/5.0 ("), "{request}");
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!",
            )
            .unwrap();
    });
    let mut realm = Realm::new();
    let options = realm.object();
    options.define(Key::name("redirect"), Property::data(Value::str("error")));
    let promise = fetch(
        &mut realm,
        &[
            Value::str(&format!("http://{address}/diagram")),
            Value::Object(options),
        ],
    )
    .unwrap();
    let observed = Rc::new(RefCell::new(None));
    let observed_done = Rc::new(RefCell::new(false));
    let observed_result = observed.clone();
    let observed_done_result = observed_done.clone();
    let on_response = realm.function("response", 1, false, move |realm, _, args| {
        let response = args.first().cloned().unwrap_or(Value::Undefined);
        let status = ops::get(realm, &response, &Key::name("status"))?;
        let headers = ops::get(realm, &response, &Key::name("headers"))?;
        let get_header = ops::get(realm, &headers, &Key::name("get"))?;
        let mime = ops::call(realm, &get_header, headers, &[Value::str("Content-Type")])?;
        let body = ops::get(realm, &response, &Key::name("body"))?;
        let get_reader = ops::get(realm, &body, &Key::name("getReader"))?;
        let reader = ops::call(realm, &get_reader, body, &[])?;
        let read = ops::get(realm, &reader, &Key::name("read"))?;
        let chunk_promise = ops::call(realm, &read, reader.clone(), &[])?;
        let then = ops::get(realm, &chunk_promise, &Key::name("then"))?;
        let done_promise = ops::call(realm, &read, reader.clone(), &[])?;
        let then_done = ops::get(realm, &done_promise, &Key::name("then"))?;
        let cancel = ops::get(realm, &reader, &Key::name("cancel"))?;
        ops::call(realm, &cancel, reader, &[])?;
        let observed_done = observed_done_result.clone();
        let on_done = realm.function("done", 1, false, move |realm, _, args| {
            let result = args.first().cloned().unwrap_or(Value::Undefined);
            *observed_done.borrow_mut() = matches!(
                ops::get(realm, &result, &Key::name("done"))?,
                Value::Boolean(true)
            );
            Ok(Value::Undefined)
        });
        ops::call(realm, &then_done, done_promise, &[Value::Object(on_done)])?;
        let observed_chunk = observed_result.clone();
        let on_chunk = realm.function("chunk", 1, false, move |realm, _, args| {
            let chunk = args.first().cloned().unwrap_or(Value::Undefined);
            let value = ops::get(realm, &chunk, &Key::name("value"))?;
            *observed_chunk.borrow_mut() = Some((
                status.clone(),
                ops::to_string(realm, &mime)?.to_rust_lossy(),
                crate::typed_array_builtin::view_bytes(&value)
                    .unwrap_or_default()
                    .to_vec(),
            ));
            Ok(Value::Undefined)
        });
        ops::call(realm, &then, chunk_promise, &[Value::Object(on_chunk)])?;
        Ok(Value::Undefined)
    });
    let then = ops::get(&mut realm, &promise, &Key::name("then")).unwrap();
    ops::call(&mut realm, &then, promise, &[Value::Object(on_response)]).unwrap();
    realm.run_event_loop().unwrap();
    server.join().unwrap();
    let observed = observed.borrow();
    let (status, mime, bytes) = observed.as_ref().unwrap();
    assert!(crate::same_value(status, &Value::Number(200.0)));
    assert_eq!(mime, "image/png");
    assert_eq!(bytes, b"PNG!");
    assert!(*observed_done.borrow());
}

#[test]
fn electron_net_fetch_rejects_redirects_when_requested() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let read = socket.read(&mut request).unwrap();
        // Electron's network service sends the application's user agent.
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.contains("User-Agent: Mozilla/5.0 ("), "{request}");
        socket
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    let mut realm = Realm::new();
    let options = realm.object();
    options.define(Key::name("redirect"), Property::data(Value::str("error")));
    let promise = fetch(
        &mut realm,
        &[
            Value::str(&format!("http://{address}/diagram")),
            Value::Object(options),
        ],
    )
    .unwrap();
    let observed = Rc::new(RefCell::new(None));
    let observed_error = observed.clone();
    let on_error = realm.function("redirect error", 1, false, move |realm, _, args| {
        let error = args.first().cloned().unwrap_or(Value::Undefined);
        let message = ops::get(realm, &error, &Key::name("message"))?;
        *observed_error.borrow_mut() = Some(ops::to_string(realm, &message)?.to_rust_lossy());
        Ok(Value::Undefined)
    });
    let catch = ops::get(&mut realm, &promise, &Key::name("catch")).unwrap();
    ops::call(&mut realm, &catch, promise, &[Value::Object(on_error)]).unwrap();
    realm.run_event_loop().unwrap();
    server.join().unwrap();
    assert_eq!(
        observed.borrow().as_deref(),
        Some("fetch redirect was rejected")
    );
}

#[test]
fn electron_net_fetch_rejects_invalid_urls_on_the_realm_event_loop() {
    let mut realm = Realm::new();
    let promise = fetch(&mut realm, &[Value::str("not a URL")]).unwrap();
    let observed = Rc::new(RefCell::new(None));
    let observed_error = observed.clone();
    let on_error = realm.function("fetch error", 1, false, move |realm, _, args| {
        let error = args.first().cloned().unwrap_or(Value::Undefined);
        let code = ops::get(realm, &error, &Key::name("code"))?;
        *observed_error.borrow_mut() = Some(ops::to_string(realm, &code)?.to_rust_lossy());
        Ok(Value::Undefined)
    });
    let catch = ops::get(&mut realm, &promise, &Key::name("catch")).unwrap();
    ops::call(&mut realm, &catch, promise, &[Value::Object(on_error)]).unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(observed.borrow().as_deref(), Some("ERR_INVALID_URL"));
}

#[test]
fn electron_net_fetch_array_buffer_method_returns_an_array_buffer() {
    let mut realm = Realm::new();
    let response = fetch_response(
        &mut realm,
        Response {
            status: 200,
            status_text: "OK".into(),
            headers: Vec::new(),
            body: b"body".to_vec(),
        },
    )
    .unwrap();
    let array_buffer = get(&mut realm, &response, "arrayBuffer");
    let promise = ops::call(&mut realm, &array_buffer, Value::Object(response), &[]).unwrap();
    let observed = Rc::new(RefCell::new(false));
    let observed_buffer = observed.clone();
    let callback = realm.function("array buffer", 1, false, move |_, _, args| {
        let buffer = args.first().cloned().unwrap_or(Value::Undefined);
        *observed_buffer.borrow_mut() = crate::typed_array_builtin::buffer_of(&buffer).is_some();
        Ok(Value::Undefined)
    });
    let then = ops::get(&mut realm, &promise, &Key::name("then")).unwrap();
    ops::call(&mut realm, &then, promise, &[Value::Object(callback)]).unwrap();
    realm.run_jobs().unwrap();
    assert!(*observed.borrow());
}

fn client_request(realm: &mut Realm) -> Value {
    let binding = client_binding(realm);
    get(realm, &binding, "request")
}

fn bytes(realm: &Realm, value: &[u8]) -> Value {
    Value::Object(crate::typed_array_builtin::new_bytes(
        realm,
        value.to_vec(),
        None,
    ))
}

#[test]
fn client_binding_sends_the_request_and_reports_status_raw_headers_and_body() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !request.ends_with(b"body\x01\x02") {
            let read = socket.read(&mut chunk).unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&chunk[..read]);
        }
        let text = String::from_utf8_lossy(&request).to_string();
        socket
            .write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 3\r\nX-Reply: native\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\nyes",
            )
            .unwrap();
        text
    });
    let mut realm = Realm::new();
    let request = client_request(&mut realm);
    let headers = realm.array(vec![Value::str("X-Request"), Value::str("test")]);
    let seen = Rc::new(RefCell::new(None));
    let seen_response = seen.clone();
    let callback = realm.function("done", 5, false, move |realm, _, args| {
        let raw = args[3].clone();
        let count = crate::array_builtin::length_of(realm, &raw)?;
        let mut names = Vec::new();
        for index in (0..count).step_by(2) {
            let name = ops::get(realm, &raw, &Key::Index(index as u32))?;
            names.push(ops::to_string(realm, &name)?.to_rust_lossy());
        }
        *seen_response.borrow_mut() = Some((
            args[0].clone(),
            args[1].clone(),
            args[2].clone(),
            names,
            args[4].clone(),
        ));
        Ok(Value::Undefined)
    });
    let body = bytes(&realm, b"body\x01\x02");
    ops::call(
        &mut realm,
        &request,
        Value::Undefined,
        &[
            Value::str(&format!("http://127.0.0.1:{}/callback", address.port())),
            Value::str("POST"),
            Value::Object(headers),
            body,
            Value::Object(callback),
        ],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    let sent = server.join().unwrap();
    assert!(sent.starts_with("POST /callback HTTP/1.1\r\n"), "{sent}");
    assert!(sent.contains("X-Request: test\r\n"), "{sent}");
    let (error, status, status_text, names, reader) = seen.borrow_mut().take().unwrap();
    let body = read_all(&mut realm, &reader);
    let (error, status, status_text, names) = (&error, &status, &status_text, &names);
    assert!(matches!(error, Value::Null));
    assert!(crate::same_value(status, &Value::Number(202.0)));
    assert!(crate::same_value(status_text, &Value::str("Accepted")));
    assert_eq!(
        names
            .iter()
            .filter(|name| name.as_str() == "set-cookie")
            .count(),
        2
    );
    assert!(names.iter().any(|name| name == "x-reply"));
    assert_eq!(body, b"yes");
}

#[test]
fn client_binding_reports_invalid_urls_and_requires_a_callback() {
    let mut realm = Realm::new();
    let request = client_request(&mut realm);
    let code = Rc::new(RefCell::new(String::new()));
    let seen = code.clone();
    let callback = realm.function("done", 1, false, move |realm, _, args| {
        let value = ops::get(realm, &args[0], &Key::name("code"))?;
        *seen.borrow_mut() = ops::to_string(realm, &value)?.to_rust_lossy();
        Ok(Value::Undefined)
    });
    let empty = realm.array(Vec::new());
    ops::call(
        &mut realm,
        &request,
        Value::Undefined,
        &[
            Value::str("not a url"),
            Value::str("GET"),
            Value::Object(empty.clone()),
            Value::Undefined,
            Value::Object(callback),
        ],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    assert_eq!(code.borrow().as_str(), "ERR_INVALID_URL");
    assert!(
        ops::call(
            &mut realm,
            &request,
            Value::Undefined,
            &[
                Value::str("http://127.0.0.1/"),
                Value::str("GET"),
                Value::Object(empty),
            ],
        )
        .is_err()
    );
}

/// Reads the next part of a response body: `Ok(bytes)`, empty at the end,
/// or the error's code.
fn read_part(realm: &mut Realm, reader: &Value) -> Result<Vec<u8>, String> {
    let read = ops::get(realm, reader, &Key::name("read")).unwrap();
    let part = Rc::new(RefCell::new(None));
    let seen = part.clone();
    let callback = realm.function("part", 2, false, move |realm, _, args| {
        *seen.borrow_mut() = Some(match &args[0] {
            Value::Null => Ok(crate::typed_array_builtin::view_bytes(&args[1]).unwrap()),
            error => {
                let code = ops::get(realm, error, &Key::name("code"))?;
                Err(ops::to_string(realm, &code)?.to_rust_lossy())
            }
        });
        Ok(Value::Undefined)
    });
    ops::call(realm, &read, reader.clone(), &[Value::Object(callback)]).unwrap();
    realm.run_event_loop().unwrap();
    part.borrow_mut().take().unwrap()
}

fn read_all(realm: &mut Realm, reader: &Value) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let part = read_part(realm, reader).unwrap();
        if part.is_empty() {
            return body;
        }
        body.extend_from_slice(&part);
    }
}

/// Sends a GET through the binding and returns the response's body reader.
fn open_body(realm: &mut Realm, url: &str, insecure: bool) -> Result<Value, String> {
    let request = client_request(realm);
    let result = Rc::new(RefCell::new(None));
    let seen = result.clone();
    let callback = realm.function("done", 5, false, move |realm, _, args| {
        *seen.borrow_mut() = Some(match &args[0] {
            Value::Null => Ok(args[4].clone()),
            error => {
                let code = ops::get(realm, error, &Key::name("code"))?;
                Err(ops::to_string(realm, &code)?.to_rust_lossy())
            }
        });
        Ok(Value::Undefined)
    });
    let empty = realm.array(Vec::new());
    ops::call(
        realm,
        &request,
        Value::Undefined,
        &[
            Value::str(url),
            Value::str("GET"),
            Value::Object(empty),
            Value::Undefined,
            Value::Object(callback),
            Value::Boolean(insecure),
        ],
    )
    .unwrap();
    realm.run_event_loop().unwrap();
    result.borrow_mut().take().unwrap()
}

/// Accepts one request and returns its socket once the request has arrived.
fn accept_request(listener: &TcpListener) -> std::net::TcpStream {
    let (mut socket, _) = listener.accept().unwrap();
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !request.ends_with(b"\r\n\r\n") {
        let read = socket.read(&mut chunk).unwrap();
        assert_ne!(read, 0);
        request.extend_from_slice(&chunk[..read]);
    }
    socket
}

#[test]
fn client_binding_streams_response_bodies_as_they_arrive() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let (first_read, read_first) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let mut socket = accept_request(&listener);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\none")
            .unwrap();
        // The rest waits until the client has read the first part.
        let waited = read_first
            .recv_timeout(std::time::Duration::from_secs(10))
            .is_ok();
        socket.write_all(b"two").unwrap();
        waited
    });
    let mut realm = Realm::new();
    let reader = open_body(&mut realm, &format!("http://{address}/"), false).unwrap();
    assert_eq!(read_part(&mut realm, &reader).unwrap(), b"one");
    first_read.send(()).unwrap();
    assert_eq!(read_all(&mut realm, &reader), b"two");
    assert!(server.join().unwrap(), "the first part arrived alone");
    // Ended bodies keep reading as ended.
    assert_eq!(read_part(&mut realm, &reader).unwrap(), b"");
}

#[test]
fn closed_bodies_read_as_ended_and_cut_bodies_fail() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let mut socket = accept_request(&listener);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc")
                .unwrap();
        }
    });
    let mut realm = Realm::new();
    let url = format!("http://{address}/");
    // A body cut short by the server fails once its bytes are read.
    let reader = open_body(&mut realm, &url, false).unwrap();
    let mut body = Vec::new();
    let failure = loop {
        match read_part(&mut realm, &reader) {
            Ok(part) => {
                assert!(!part.is_empty(), "the body ended early: {body:?}");
                body.extend_from_slice(&part);
            }
            Err(code) => break code,
        }
    };
    assert_eq!(body, b"abc");
    assert_eq!(failure, "ECONNRESET");
    // A closed body reads as ended without touching the network, and a read
    // needs a callback.
    let reader = open_body(&mut realm, &url, false).unwrap();
    let close = ops::get(&mut realm, &reader, &Key::name("close")).unwrap();
    ops::call(&mut realm, &close, reader.clone(), &[]).unwrap();
    ops::call(&mut realm, &close, reader.clone(), &[]).unwrap();
    assert_eq!(read_part(&mut realm, &reader).unwrap(), b"");
    let read = ops::get(&mut realm, &reader, &Key::name("read")).unwrap();
    assert!(ops::call(&mut realm, &read, reader.clone(), &[]).is_err());
    server.join().unwrap();
}

#[test]
fn cancelled_exchanges_stop_reading() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut socket = accept_request(&listener);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc")
            .unwrap();
        socket
    });
    let request = Request {
        url: format!("http://{address}/"),
        method: "GET".into(),
        headers: Vec::new(),
        body: Vec::new(),
        agent: None,
        insecure: false,
        upgrade: false,
    };
    let opened = open(&request, false).unwrap();
    assert_eq!(opened.status, 200);
    opened.exchange.cancel();
    opened.exchange.cancel();
    assert_eq!(opened.exchange.read().unwrap_err().code, "EIO");
    drop(opened);
    drop(server.join().unwrap());
}

#[test]
fn untrusted_certificates_are_accepted_only_when_asked() {
    // Nothing listens on port 1: the request is set up and its connection
    // refused, whether or not certificates are checked.
    let mut realm = Realm::new();
    for insecure in [false, true] {
        let failure = open_body(&mut realm, "https://127.0.0.1:1/", insecure).unwrap_err();
        assert_eq!(failure, "ECONNREFUSED");
    }
}
