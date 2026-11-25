//! HTTP requests backed by WinHTTP, which brings TLS and the system proxy:
//! the `http_client` binding behind the `https` library module, Electron's
//! `net.fetch`, and the `websocket` binding behind the `WebSocket` global.
//!
//! Requests run on the Windows thread pool and report completion on the
//! owning realm thread.

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

type Handle = *mut c_void;
type Bool = i32;

#[link(name = "winhttp", kind = "raw-dylib")]
unsafe extern "system" {
    fn WinHttpOpen(
        agent: *const u16,
        access: u32,
        proxy: *const u16,
        bypass: *const u16,
        flags: u32,
    ) -> Handle;
    fn WinHttpConnect(session: Handle, server: *const u16, port: u16, reserved: u32) -> Handle;
    fn WinHttpOpenRequest(
        connection: Handle,
        verb: *const u16,
        object: *const u16,
        version: *const u16,
        referrer: *const u16,
        accept: *const *const u16,
        flags: u32,
    ) -> Handle;
    fn WinHttpSendRequest(
        request: Handle,
        headers: *const u16,
        headers_length: u32,
        optional: *const c_void,
        optional_length: u32,
        total_length: u32,
        context: usize,
    ) -> Bool;
    fn WinHttpReceiveResponse(request: Handle, reserved: *mut c_void) -> Bool;
    fn WinHttpQueryHeaders(
        request: Handle,
        level: u32,
        name: *const u16,
        buffer: *mut c_void,
        size: *mut u32,
        index: *mut u32,
    ) -> Bool;
    fn WinHttpReadData(request: Handle, buffer: *mut c_void, size: u32, read: *mut u32) -> Bool;
    fn WinHttpQueryDataAvailable(request: Handle, available: *mut u32) -> Bool;
    fn WinHttpCloseHandle(handle: Handle) -> Bool;
    fn WinHttpSetOption(handle: Handle, option: u32, buffer: *const c_void, size: u32) -> Bool;
    fn WinHttpWebSocketCompleteUpgrade(request: Handle, context: usize) -> Handle;
    fn WinHttpWebSocketSend(socket: Handle, kind: u32, buffer: *const c_void, length: u32) -> u32;
    fn WinHttpWebSocketReceive(
        socket: Handle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        kind: *mut u32,
    ) -> u32;
    fn WinHttpWebSocketShutdown(
        socket: Handle,
        status: u16,
        reason: *const c_void,
        length: u32,
    ) -> u32;
    fn WinHttpWebSocketQueryCloseStatus(
        socket: Handle,
        status: *mut u16,
        reason: *mut c_void,
        length: u32,
        consumed: *mut u32,
    ) -> u32;
    fn GetLastError() -> u32;
}

const AUTO_PROXY: u32 = 4;
const NO_PROXY: u32 = 1;
const SECURE: u32 = 0x0080_0000;
const OPTION_DISABLE_FEATURE: u32 = 63;
const DISABLE_REDIRECTS: u32 = 2;
const QUERY_STATUS: u32 = 19 | 0x2000_0000;
const QUERY_RAW_HEADERS: u32 = 22;
const OPTION_SECURITY_FLAGS: u32 = 31;
/// Unknown authorities, expired or misnamed certificates and wrong usages.
const IGNORE_CERTIFICATE_ERRORS: u32 = 0x0100 | 0x0200 | 0x1000 | 0x2000;
static DIRECT_PROXY: AtomicBool = AtomicBool::new(false);
const OPTION_UPGRADE_TO_WEB_SOCKET: u32 = 114;
const BINARY_MESSAGE: u32 = 0;
const BINARY_FRAGMENT: u32 = 1;
const UTF8_MESSAGE: u32 = 2;
const UTF8_FRAGMENT: u32 = 3;
/// The longest close reason a close frame carries.
const CLOSE_REASON_BYTES: usize = 123;

#[derive(Clone)]
struct Request {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The `User-Agent` sent when the headers give none; without one, none
    /// is sent, as Node sends none.
    agent: Option<String>,
    /// Whether a server certificate the system does not trust is accepted:
    /// Node's `rejectUnauthorized: false`.
    insecure: bool,
    /// Whether the request asks to switch to the WebSocket protocol.
    upgrade: bool,
}

#[derive(Debug)]
struct Response {
    status: u32,
    status_text: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
}

/// The binding behind `https` requests: `request(url, method, headers,
/// body, callback)` sends one request without following redirects, as
/// Node does, and calls `callback(error, status, statusText, rawHeaders,
/// body)` when the response headers arrive. `headers` alternates names and
/// values; a true sixth argument accepts untrusted server certificates.
/// `body` reads the response as it arrives: `body.read(callback)` calls
/// `callback(error, chunk)` with the next part, empty at the end, and
/// `body.close()` ends the request.
pub(crate) fn client_binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let request = realm.function("request", 5, false, |realm, _, args| {
        let text = |realm: &mut Realm, index: usize| -> Result<String, Thrown> {
            let value = args.get(index).cloned().unwrap_or(Value::Undefined);
            Ok(ops::to_string(realm, &value)?.to_rust_lossy())
        };
        let url = text(realm, 0)?;
        let method = text(realm, 1)?;
        let list = args.get(2).cloned().unwrap_or(Value::Undefined);
        let mut headers = Vec::new();
        if list.as_object().is_some() {
            let count = crate::array_builtin::length_of(realm, &list)?;
            for index in (0..count.saturating_sub(1)).step_by(2) {
                let name = ops::get(realm, &list, &Key::Index(index as u32))?;
                let value = ops::get(realm, &list, &Key::Index(index as u32 + 1))?;
                headers.push((
                    ops::to_string(realm, &name)?.to_rust_lossy(),
                    ops::to_string(realm, &value)?.to_rust_lossy(),
                ));
            }
        }
        let body = args
            .get(3)
            .and_then(crate::typed_array_builtin::view_bytes)
            .unwrap_or_default();
        let callback = args.get(4).cloned().unwrap_or(Value::Undefined);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
        }
        let insecure = args.get(5).is_some_and(ops::to_boolean);
        let request = Request {
            url,
            method,
            headers,
            body,
            agent: None,
            insecure,
            upgrade: false,
        };
        realm
            .submit_io(
                move || open(&request, false),
                move |realm, result| {
                    let arguments = match result {
                        Ok(opened) => {
                            let mut raw = Vec::with_capacity(opened.headers.len() * 2);
                            for (name, value) in &opened.headers {
                                raw.push(Value::str(name));
                                raw.push(Value::str(value));
                            }
                            let body = body_reader(realm, Arc::new(opened.exchange));
                            alloc::vec![
                                Value::Null,
                                Value::Number(f64::from(opened.status)),
                                Value::str(&opened.status_text),
                                Value::Object(realm.array(raw)),
                                Value::Object(body),
                            ]
                        }
                        Err(failure) => alloc::vec![Value::Object(failure_error(realm, failure))],
                    };
                    ops::call(realm, &callback, Value::Undefined, &arguments)?;
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(ErrorKind::Error, "could not queue native HTTP request"))?;
        Ok(Value::Undefined)
    });
    exports.define(Key::name("request"), Property::data(Value::Object(request)));
    exports
}

/// A Node network error for a failed request.
fn failure_error(realm: &mut Realm, failure: Failure) -> Object {
    let error = realm.error(ErrorKind::Error, &failure.message);
    error.define(Key::name("code"), Property::data(Value::str(failure.code)));
    error
}

/// The `body` of a response: `read(callback)` queues a read of its next
/// part on the pool, and `close()` ends the request. A closed body reads
/// as ended.
fn body_reader(realm: &mut Realm, exchange: Arc<Exchange>) -> Object {
    let reader = realm.object();
    let open = Rc::new(RefCell::new(Some(exchange)));
    let current = open.clone();
    let read = realm.function("read", 1, false, move |realm, _, args| {
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
        }
        let exchange = current.borrow().clone();
        let complete = move |realm: &mut Realm, result: Result<Vec<u8>, Failure>| {
            let arguments = match result {
                Ok(chunk) => {
                    let chunk = crate::typed_array_builtin::new_bytes(realm, chunk, None);
                    alloc::vec![Value::Null, Value::Object(chunk)]
                }
                Err(failure) => alloc::vec![Value::Object(failure_error(realm, failure))],
            };
            ops::call(realm, &callback, Value::Undefined, &arguments)?;
            Ok(())
        };
        let queued = match exchange {
            Some(exchange) => realm.submit_io(move || exchange.read(), complete),
            None => realm.submit_io(|| Ok(Vec::new()), complete),
        };
        queued.map_err(|_| realm.throw(ErrorKind::Error, "could not queue native HTTP read"))?;
        Ok(Value::Undefined)
    });
    reader.define(Key::name("read"), Property::data(Value::Object(read)));
    let close = realm.function("close", 0, false, move |_, _, _| {
        if let Some(exchange) = open.borrow_mut().take() {
            exchange.cancel();
        }
        Ok(Value::Undefined)
    });
    reader.define(Key::name("close"), Property::data(Value::Object(close)));
    reader
}

/// Creates the Electron `net.fetch` entry point using the shared WinHTTP
/// transport. The buffered response is exposed as a one-chunk web reader.
pub fn fetch(realm: &mut Realm, args: &[Value]) -> Result<Value, Thrown> {
    let url = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
    let options = args.get(1).cloned().unwrap_or(Value::Undefined);
    let method = option_string(realm, &options, "method")?.unwrap_or_else(|| "GET".into());
    let headers = option_headers(realm, &options)?;
    let redirect_error = option_string(realm, &options, "redirect")?.as_deref() == Some("error");
    // Electron's network service sends the application's user agent.
    let request = Request {
        url,
        method,
        headers,
        body: Vec::new(),
        agent: Some(crate::electron_builtin::user_agent(realm)),
        insecure: false,
        upgrade: false,
    };
    let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
    realm
        .submit_io(
            move || perform(&request, !redirect_error),
            move |realm, result| {
                match result {
                    Ok(response) if redirect_error && (300..400).contains(&response.status) => {
                        let error = realm.error(ErrorKind::Type, "fetch redirect was rejected");
                        ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
                    }
                    Ok(response) => match fetch_response(realm, response) {
                        Ok(response) => {
                            ops::call(
                                realm,
                                &resolve,
                                Value::Undefined,
                                &[Value::Object(response)],
                            )?;
                        }
                        Err(error) => {
                            ops::call(realm, &reject, Value::Undefined, &[error.0])?;
                        }
                    },
                    Err(failure) => {
                        let error = realm.error(ErrorKind::Error, &failure.message);
                        error.define(Key::name("code"), Property::data(Value::str(failure.code)));
                        ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
                    }
                }
                Ok(())
            },
        )
        .map_err(|_| realm.throw(ErrorKind::Error, "could not queue native fetch"))?;
    Ok(promise)
}

/// Creates the minimal Response surface used by Electron's net.fetch callers.
fn fetch_response(realm: &mut Realm, response: Response) -> Result<Object, Thrown> {
    let result = realm.object();
    result.define(
        Key::name("status"),
        Property::data(Value::Number(response.status as f64)),
    );
    result.define(
        Key::name("statusText"),
        Property::data(Value::str(&response.status_text)),
    );
    result.define(
        Key::name("ok"),
        Property::data(Value::Boolean((200..300).contains(&response.status))),
    );

    let headers = realm.object();
    let values = response.headers;
    let get = realm.function("get", 1, false, move |realm, _, args| {
        let name = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?
            .to_rust_lossy()
            .to_ascii_lowercase();
        let value = values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| Value::str(value))
            .unwrap_or(Value::Null);
        Ok(value)
    });
    headers.define(Key::name("get"), Property::data(Value::Object(get)));
    result.define(Key::name("headers"), Property::data(Value::Object(headers)));
    result.define(
        Key::name("body"),
        Property::data(Value::Object(fetch_body(realm, response.body.clone()))),
    );

    let bytes = response.body;
    let array_buffer = realm.function("arrayBuffer", 0, false, move |realm, _, _| {
        let view = crate::typed_array_builtin::new_bytes(realm, bytes.clone(), None);
        let buffer = ops::get_named(realm, &Value::Object(view), "buffer")?;
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[buffer])?;
        Ok(promise)
    });
    result.define(
        Key::name("arrayBuffer"),
        Property::data(Value::Object(array_buffer)),
    );
    Ok(result)
}

fn fetch_body(realm: &Realm, bytes: Vec<u8>) -> Object {
    let body = realm.object();
    let get_reader = realm.function("getReader", 0, false, move |realm, _, _| {
        let state = Rc::new(RefCell::new(Some(bytes.clone())));
        let reader = realm.object();
        let state_for_read = state.clone();
        let read = realm.function("read", 0, false, move |realm, _, _| {
            let chunk = state_for_read.borrow_mut().take();
            let result = realm.object();
            match chunk {
                Some(bytes) => {
                    result.define(Key::name("done"), Property::data(Value::Boolean(false)));
                    let value = crate::typed_array_builtin::new_bytes(realm, bytes, None);
                    result.define(Key::name("value"), Property::data(Value::Object(value)));
                }
                None => {
                    result.define(Key::name("done"), Property::data(Value::Boolean(true)));
                    result.define(Key::name("value"), Property::data(Value::Undefined));
                }
            }
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            ops::call(realm, &resolve, Value::Undefined, &[Value::Object(result)])?;
            Ok(promise)
        });
        reader.define(Key::name("read"), Property::data(Value::Object(read)));
        let state_for_cancel = state;
        let cancel = realm.function("cancel", 0, false, move |realm, _, _| {
            state_for_cancel.borrow_mut().take();
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
            Ok(promise)
        });
        reader.define(Key::name("cancel"), Property::data(Value::Object(cancel)));
        Ok(Value::Object(reader))
    });
    body.define(
        Key::name("getReader"),
        Property::data(Value::Object(get_reader)),
    );
    body
}

/// The body of a successful GET of a URL, following redirects: what a page
/// shows from the network, such as an image, asked for with the page's user
/// agent. `None` when the request fails or the server answers with an error
/// status.
#[cfg(feature = "dom")]
pub(crate) fn download(url: &str, agent: String) -> Option<Vec<u8>> {
    let request = Request {
        url: url.into(),
        method: "GET".into(),
        headers: Vec::new(),
        body: Vec::new(),
        agent: Some(agent),
        insecure: false,
        upgrade: false,
    };
    perform(&request, true)
        .ok()
        .filter(|response| (200..300).contains(&response.status))
        .map(|response| response.body)
}

/// Changes the process-wide proxy mode used by native WinHTTP sessions.
pub fn set_direct_proxy(enabled: bool) {
    DIRECT_PROXY.store(enabled, Ordering::Relaxed);
}

/// Whether `session.setProxy` asked for direct connections.
pub(crate) fn direct_proxy() -> bool {
    DIRECT_PROXY.load(Ordering::Relaxed)
}

fn option_string(realm: &mut Realm, object: &Value, key: &str) -> Result<Option<String>, Thrown> {
    if matches!(object, Value::Undefined | Value::Null) {
        return Ok(None);
    }
    let value = ops::get(realm, object, &Key::name(key))?;
    if matches!(value, Value::Undefined | Value::Null) {
        Ok(None)
    } else {
        Ok(Some(ops::to_string(realm, &value)?.to_rust_lossy()))
    }
}

fn option_headers(realm: &mut Realm, options: &Value) -> Result<Vec<(String, String)>, Thrown> {
    if matches!(options, Value::Undefined | Value::Null) {
        return Ok(Vec::new());
    }
    let value = ops::get_named(realm, options, "headers")?;
    let Value::Object(headers) = value else {
        return Ok(Vec::new());
    };
    let keys = headers.own_keys();
    keys.into_iter()
        .filter_map(|key| match key {
            Key::String(name) => Some(name.to_rust_lossy()),
            Key::Index(_) | Key::Symbol(_) => None,
        })
        .map(|name| {
            let value = ops::get(realm, &Value::Object(headers.clone()), &Key::name(&name))?;
            Ok((name, ops::to_string(realm, &value)?.to_rust_lossy()))
        })
        .collect()
}

/// An open WinHTTP request: its session, connection and request handles,
/// closed together when the last owner drops it. Reads run on pool
/// threads while the realm keeps its own reference.
struct Exchange {
    session: Handle,
    connection: Handle,
    request: Handle,
    cancelled: AtomicBool,
    /// The body's `Content-Length`, when the response gives one.
    expected: Option<u64>,
    /// The body bytes read so far.
    received: AtomicU64,
}

// SAFETY: WinHTTP handles may be used and closed from any thread; the
// request handle is closed once, by whichever of `cancel` and `drop` runs
// first.
unsafe impl Send for Exchange {}
// SAFETY: as above; shared readers only read through WinHTTP.
unsafe impl Sync for Exchange {}

impl Exchange {
    /// Closes the request, which ends a read waiting on it on another
    /// thread with an error.
    fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) && !self.request.is_null() {
            // SAFETY: the request handle is open until this first close.
            unsafe { WinHttpCloseHandle(self.request) };
        }
    }

    /// The next part of the response body as soon as some arrives, empty
    /// at its end. A connection that ends before the `Content-Length` the
    /// response gave fails, as Node reports it.
    fn read(&self) -> Result<Vec<u8>, Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(win_failure_code("read", 12017));
        }
        let mut available = 0_u32;
        // SAFETY: `available` is writable; the call waits for data or the
        // end, and fails for a request closed meanwhile.
        if unsafe { WinHttpQueryDataAvailable(self.request, &mut available) } == 0 {
            return Err(win_failure("read"));
        }
        let mut chunk = alloc::vec![0_u8; (available as usize).min(64 * 1024)];
        let mut read = 0_u32;
        let failed = !chunk.is_empty() && {
            // SAFETY: `chunk` is writable for its full length during the
            // call; a request closed meanwhile makes the call fail.
            (unsafe {
                WinHttpReadData(
                    self.request,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &mut read,
                )
            }) == 0
        };
        if failed {
            return Err(win_failure("read"));
        }
        chunk.truncate(read as usize);
        let received = self.received.fetch_add(u64::from(read), Ordering::AcqRel) + u64::from(read);
        if chunk.is_empty() && self.expected.is_some_and(|expected| received < expected) {
            return Err(Failure {
                code: "ECONNRESET",
                message: "read: the connection closed before the whole body arrived".into(),
            });
        }
        Ok(chunk)
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        self.cancel();
        for handle in [self.connection, self.session] {
            if !handle.is_null() {
                // SAFETY: each handle was opened for this exchange and is
                // closed only here.
                unsafe { WinHttpCloseHandle(handle) };
            }
        }
    }
}

/// A request whose response headers have arrived; its body is read from
/// the exchange.
struct Opened {
    exchange: Exchange,
    status: u32,
    status_text: String,
    headers: Vec<(String, String)>,
}

/// Sends a request and waits for the response headers.
fn open(request: &Request, allow_redirects: bool) -> Result<Opened, Failure> {
    let parsed = url::Url::parse(&request.url).map_err(|_| Failure {
        code: "ERR_INVALID_URL",
        message: "Invalid URL".into(),
    })?;
    let host = parsed.host_str().ok_or_else(|| Failure {
        code: "ERR_INVALID_URL",
        message: "Invalid URL".into(),
    })?;
    let secure = parsed.scheme() == "https";
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if secure { 443 } else { 80 });
    let path = format!(
        "{}{}",
        parsed.path(),
        parsed
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    let agent = request.agent.as_deref().map(wide);
    let mut exchange = Exchange {
        session: core::ptr::null_mut(),
        connection: core::ptr::null_mut(),
        request: core::ptr::null_mut(),
        cancelled: AtomicBool::new(false),
        expected: None,
        received: AtomicU64::new(0),
    };
    // SAFETY: the exchange owns each valid handle until it drops.
    exchange.session = unsafe {
        WinHttpOpen(
            agent
                .as_ref()
                .map_or(core::ptr::null(), |agent| agent.as_ptr()),
            if DIRECT_PROXY.load(Ordering::Relaxed) {
                NO_PROXY
            } else {
                AUTO_PROXY
            },
            core::ptr::null(),
            core::ptr::null(),
            0,
        )
    };
    if exchange.session.is_null() {
        return Err(win_failure("open"));
    }
    let server = wide(host);
    // SAFETY: session is valid and server is terminated.
    exchange.connection = unsafe { WinHttpConnect(exchange.session, server.as_ptr(), port, 0) };
    if exchange.connection.is_null() {
        return Err(win_failure("connect"));
    }
    let method = wide(&request.method);
    let path = wide(&path);
    // SAFETY: inputs are terminated and owned for this call.
    exchange.request = unsafe {
        WinHttpOpenRequest(
            exchange.connection,
            method.as_ptr(),
            path.as_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            if secure { SECURE } else { 0 },
        )
    };
    let handle = exchange.request;
    if handle.is_null() {
        return Err(win_failure("request"));
    }
    let headers: String = request
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let headers = wide(&headers);
    let body = request.body.as_ptr();
    if !allow_redirects {
        let disabled = DISABLE_REDIRECTS;
        // SAFETY: the request handle is valid and `disabled` remains alive for
        // the duration of this call.
        if unsafe {
            WinHttpSetOption(
                handle,
                OPTION_DISABLE_FEATURE,
                core::ptr::from_ref(&disabled).cast(),
                core::mem::size_of::<u32>() as u32,
            )
        } == 0
        {
            return Err(win_failure("redirect policy"));
        }
    }
    if secure && request.insecure {
        let flags = IGNORE_CERTIFICATE_ERRORS;
        // SAFETY: the request handle is valid and `flags` remains alive for
        // the duration of this call. Should the option be refused, the
        // certificate is checked as usual.
        unsafe {
            WinHttpSetOption(
                handle,
                OPTION_SECURITY_FLAGS,
                core::ptr::from_ref(&flags).cast(),
                core::mem::size_of::<u32>() as u32,
            )
        };
    }
    if request.upgrade {
        // SAFETY: the request handle is valid; the option takes no buffer.
        let set =
            unsafe { WinHttpSetOption(handle, OPTION_UPGRADE_TO_WEB_SOCKET, core::ptr::null(), 0) };
        if set == 0 {
            return Err(win_failure("upgrade"));
        }
    }
    // SAFETY: all buffers remain alive for this synchronous request call.
    let sent = unsafe {
        WinHttpSendRequest(
            handle,
            headers.as_ptr(),
            u32::MAX,
            body.cast(),
            request.body.len() as u32,
            request.body.len() as u32,
            0,
        )
    };
    // SAFETY: `handle` is the open request created above.
    let received =
        sent != 0 && unsafe { WinHttpReceiveResponse(handle, core::ptr::null_mut()) } != 0;
    if !received {
        return Err(win_failure("send"));
    }
    let mut status = 0_u32;
    let mut size = core::mem::size_of::<u32>() as u32;
    // SAFETY: `status` is writable and its size is supplied to WinHTTP.
    if unsafe {
        WinHttpQueryHeaders(
            handle,
            QUERY_STATUS,
            core::ptr::null(),
            core::ptr::from_mut(&mut status).cast(),
            &mut size,
            core::ptr::null_mut(),
        )
    } == 0
    {
        return Err(win_failure("status"));
    }
    let (status_text, headers) = parse_headers(&query_headers(handle));
    exchange.expected = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.trim().parse().ok());
    Ok(Opened {
        exchange,
        status,
        status_text,
        headers,
    })
}

/// The binding behind the `WebSocket` global. `connect(url, headers,
/// callback)` opens a connection to an `http:` or `https:` URL, where
/// `headers` alternates names and values, and calls `callback(error,
/// socket, protocol, extensions)` once the server has switched protocols.
/// The socket's `send(text, bytes, callback)` sends one message;
/// `receive(callback)` calls `callback(error, kind, data, code)` with the
/// next whole message, `kind` being `text`, `binary` or `close` (then
/// `code` is the close status and `data` its reason); `shutdown(code,
/// reason, callback)` sends a close frame, after which `receive` reports
/// the server's; and `abort()` ends the connection at once.
pub(crate) fn websocket_binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let connect = realm.function("connect", 3, false, |realm, _, args| {
        let url = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy();
        let list = args.get(1).cloned().unwrap_or(Value::Undefined);
        let mut headers = Vec::new();
        if list.as_object().is_some() {
            let count = crate::array_builtin::length_of(realm, &list)?;
            for index in (0..count.saturating_sub(1)).step_by(2) {
                let name = ops::get(realm, &list, &Key::Index(index as u32))?;
                let value = ops::get(realm, &list, &Key::Index(index as u32 + 1))?;
                headers.push((
                    ops::to_string(realm, &name)?.to_rust_lossy(),
                    ops::to_string(realm, &value)?.to_rust_lossy(),
                ));
            }
        }
        let callback = args.get(2).cloned().unwrap_or(Value::Undefined);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
        }
        let request = Request {
            url,
            method: "GET".into(),
            headers,
            body: Vec::new(),
            agent: None,
            insecure: false,
            upgrade: true,
        };
        realm
            .submit_io(
                move || upgrade(&request),
                move |realm, result| {
                    let arguments = match result {
                        Ok((socket, protocol, extensions)) => alloc::vec![
                            Value::Null,
                            Value::Object(socket_object(realm, Arc::new(socket))),
                            Value::str(&protocol),
                            Value::str(&extensions),
                        ],
                        Err(failure) => alloc::vec![Value::Object(failure_error(realm, failure))],
                    };
                    ops::call(realm, &callback, Value::Undefined, &arguments)?;
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(ErrorKind::Error, "could not queue a WebSocket connection"))?;
        Ok(Value::Undefined)
    });
    exports.define(Key::name("connect"), Property::data(Value::Object(connect)));
    exports
}

/// A whole message a WebSocket received.
enum Message {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Close(u16, Vec<u8>),
}

/// An open WebSocket: the socket WinHTTP upgraded the request to, and the
/// session and connection it runs on, closed when the last owner drops it.
struct Socket {
    /// Keeps the session and connection open while the socket is.
    _exchange: Exchange,
    socket: Handle,
    closed: AtomicBool,
}

// SAFETY: WinHTTP allows one send and one receive on a socket at once, from
// any threads; the binding sends one message at a time, receives one at a
// time, and closes the socket handle once.
unsafe impl Send for Socket {}
// SAFETY: as above.
unsafe impl Sync for Socket {}

impl Socket {
    fn send(&self, text: bool, bytes: &[u8]) -> Result<(), Failure> {
        let kind = if text { UTF8_MESSAGE } else { BINARY_MESSAGE };
        // SAFETY: `bytes` is readable for its length during the call; a
        // socket closed meanwhile makes the call fail.
        let code = unsafe {
            WinHttpWebSocketSend(self.socket, kind, bytes.as_ptr().cast(), bytes.len() as u32)
        };
        if code != 0 {
            return Err(win_failure_code("send", code));
        }
        Ok(())
    }

    /// The next whole message, its fragments joined.
    fn receive(&self) -> Result<Message, Failure> {
        let mut message = Vec::new();
        let mut chunk = alloc::vec![0_u8; 64 * 1024];
        loop {
            let (mut read, mut kind) = (0_u32, 0_u32);
            // SAFETY: `chunk` is writable for its length during the call; a
            // socket closed meanwhile makes the call fail.
            let code = unsafe {
                WinHttpWebSocketReceive(
                    self.socket,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &mut read,
                    &mut kind,
                )
            };
            if code != 0 {
                return Err(win_failure_code("receive", code));
            }
            message.extend_from_slice(&chunk[..read as usize]);
            match kind {
                BINARY_MESSAGE => return Ok(Message::Binary(message)),
                UTF8_MESSAGE => return Ok(Message::Text(message)),
                BINARY_FRAGMENT | UTF8_FRAGMENT => {}
                _ => {
                    let mut status = 0_u16;
                    let mut reason = alloc::vec![0_u8; CLOSE_REASON_BYTES];
                    let mut consumed = 0_u32;
                    // SAFETY: the outputs are writable for the lengths given.
                    let code = unsafe {
                        WinHttpWebSocketQueryCloseStatus(
                            self.socket,
                            &mut status,
                            reason.as_mut_ptr().cast(),
                            reason.len() as u32,
                            &mut consumed,
                        )
                    };
                    if code != 0 {
                        // A close frame without a status.
                        return Ok(Message::Close(1005, Vec::new()));
                    }
                    reason.truncate(consumed as usize);
                    return Ok(Message::Close(status, reason));
                }
            }
        }
    }

    fn shutdown(&self, status: u16, reason: &[u8]) -> Result<(), Failure> {
        // SAFETY: `reason` is readable for its length during the call.
        let code = unsafe {
            WinHttpWebSocketShutdown(
                self.socket,
                status,
                reason.as_ptr().cast(),
                reason.len().min(CLOSE_REASON_BYTES) as u32,
            )
        };
        if code != 0 {
            return Err(win_failure_code("close", code));
        }
        Ok(())
    }

    /// Closes the socket, which ends a receive waiting on another thread.
    fn abort(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            // SAFETY: the socket handle is open until this first close.
            unsafe { WinHttpCloseHandle(self.socket) };
        }
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Opens a WebSocket: sends the upgrade request and completes the switch
/// of protocols, with the subprotocol and extensions the server chose.
fn upgrade(request: &Request) -> Result<(Socket, String, String), Failure> {
    let opened = open(request, false)?;
    if opened.status != 101 {
        return Err(Failure {
            code: "ERR_WEBSOCKET_HANDSHAKE",
            message: format!(
                "the server answered {} {} instead of switching protocols",
                opened.status, opened.status_text
            ),
        });
    }
    let header = |name: &str| {
        opened
            .headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let protocol = header("sec-websocket-protocol");
    let extensions = header("sec-websocket-extensions");
    // SAFETY: the request handle is open and its response switched protocols.
    let socket = unsafe { WinHttpWebSocketCompleteUpgrade(opened.exchange.request, 0) };
    if socket.is_null() {
        return Err(win_failure("upgrade"));
    }
    // The socket no longer needs the request handle.
    opened.exchange.cancel();
    Ok((
        Socket {
            _exchange: opened.exchange,
            socket,
            closed: AtomicBool::new(false),
        },
        protocol,
        extensions,
    ))
}

/// The script side of an open socket.
fn socket_object(realm: &mut Realm, socket: Arc<Socket>) -> Object {
    let object = realm.object();
    let sender = socket.clone();
    let send = realm.function("send", 3, false, move |realm, _, args| {
        let text = args.first().is_some_and(ops::to_boolean);
        let bytes = args
            .get(1)
            .and_then(crate::typed_array_builtin::view_bytes)
            .unwrap_or_default();
        let callback = args.get(2).cloned().unwrap_or(Value::Undefined);
        let socket = sender.clone();
        realm
            .submit_io(
                move || socket.send(text, &bytes),
                move |realm, result| {
                    let error = match result {
                        Ok(()) => Value::Null,
                        Err(failure) => Value::Object(failure_error(realm, failure)),
                    };
                    if crate::array_builtin::is_callable(&callback) {
                        ops::call(realm, &callback, Value::Undefined, &[error])?;
                    }
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(ErrorKind::Error, "could not queue a WebSocket send"))?;
        Ok(Value::Undefined)
    });
    object.define(Key::name("send"), Property::data(Value::Object(send)));
    let receiver = socket.clone();
    let receive = realm.function("receive", 1, false, move |realm, _, args| {
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
        }
        let socket = receiver.clone();
        realm
            .submit_io(
                move || socket.receive(),
                move |realm, result| {
                    let arguments = match result {
                        Ok(Message::Text(bytes)) => alloc::vec![
                            Value::Null,
                            Value::str("text"),
                            Value::Object(crate::typed_array_builtin::new_bytes(
                                realm, bytes, None
                            )),
                        ],
                        Ok(Message::Binary(bytes)) => alloc::vec![
                            Value::Null,
                            Value::str("binary"),
                            Value::Object(crate::typed_array_builtin::new_bytes(
                                realm, bytes, None
                            )),
                        ],
                        Ok(Message::Close(code, reason)) => alloc::vec![
                            Value::Null,
                            Value::str("close"),
                            Value::Object(crate::typed_array_builtin::new_bytes(
                                realm, reason, None
                            )),
                            Value::Number(f64::from(code)),
                        ],
                        Err(failure) => alloc::vec![Value::Object(failure_error(realm, failure))],
                    };
                    ops::call(realm, &callback, Value::Undefined, &arguments)?;
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(ErrorKind::Error, "could not queue a WebSocket receive"))?;
        Ok(Value::Undefined)
    });
    object.define(Key::name("receive"), Property::data(Value::Object(receive)));
    let closer = socket.clone();
    let shutdown = realm.function("shutdown", 3, false, move |realm, _, args| {
        let code = ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))?;
        let reason = args
            .get(1)
            .and_then(crate::typed_array_builtin::view_bytes)
            .unwrap_or_default();
        let callback = args.get(2).cloned().unwrap_or(Value::Undefined);
        let socket = closer.clone();
        realm
            .submit_io(
                move || socket.shutdown(code as u16, &reason),
                move |realm, result| {
                    let error = match result {
                        Ok(()) => Value::Null,
                        Err(failure) => Value::Object(failure_error(realm, failure)),
                    };
                    if crate::array_builtin::is_callable(&callback) {
                        ops::call(realm, &callback, Value::Undefined, &[error])?;
                    }
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(ErrorKind::Error, "could not queue a WebSocket close"))?;
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("shutdown"),
        Property::data(Value::Object(shutdown)),
    );
    let abort = realm.function("abort", 0, false, move |_, _, _| {
        socket.abort();
        Ok(Value::Undefined)
    });
    object.define(Key::name("abort"), Property::data(Value::Object(abort)));
    object
}

/// Performs a request and reads its whole body.
fn perform(request: &Request, allow_redirects: bool) -> Result<Response, Failure> {
    let opened = open(request, allow_redirects)?;
    let mut body = Vec::new();
    loop {
        let chunk = opened.exchange.read()?;
        if chunk.is_empty() {
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response {
        status: opened.status,
        status_text: opened.status_text,
        headers: opened.headers,
        body,
    })
}

fn query_headers(handle: Handle) -> String {
    let mut size = 0_u32;
    // SAFETY: a null output asks WinHTTP for the required buffer size.
    unsafe {
        WinHttpQueryHeaders(
            handle,
            QUERY_RAW_HEADERS,
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut size,
            core::ptr::null_mut(),
        );
    }
    if size == 0 {
        return String::new();
    }
    let mut buffer = alloc::vec![0_u16; (size as usize).div_ceil(2) + 1];
    // SAFETY: the output buffer exceeds the byte count returned by WinHTTP.
    let ok = unsafe {
        WinHttpQueryHeaders(
            handle,
            QUERY_RAW_HEADERS,
            core::ptr::null(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            core::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..(size as usize / 2).min(buffer.len())])
}

fn parse_headers(raw: &str) -> (String, Vec<(String, String)>) {
    let mut lines = raw.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let status_text = status.splitn(3, ' ').nth(2).unwrap_or_default().to_owned();
    let mut headers = Vec::<(String, String)>::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name == "set-cookie" {
            headers.push((name, value));
        } else if let Some((_, current)) = headers.iter_mut().find(|(key, _)| *key == name) {
            current.push_str(", ");
            current.push_str(&value);
        } else {
            headers.push((name, value));
        }
    }
    (status_text, headers)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn win_failure(step: &'static str) -> Failure {
    // SAFETY: immediately reads the calling thread's last WinHTTP error.
    let code = unsafe { GetLastError() };
    win_failure_code(step, code)
}

fn win_failure_code(step: &'static str, code: u32) -> Failure {
    let (name, reason) = match code {
        12002 => ("ETIMEDOUT", "the request timed out"),
        12007 => ("ENOTFOUND", "the server name could not be resolved"),
        12029 => ("ECONNREFUSED", "the connection was refused"),
        12030 | 12152 => ("ECONNRESET", "the connection was reset"),
        12037..=12045 | 12175 => ("CERT_INVALID", "the server certificate is invalid"),
        _ => ("EIO", "the request failed"),
    };
    Failure {
        code: name,
        message: format!("{step}: {reason} (WinHTTP error {code})"),
    }
}

#[cfg(test)]
mod tests;
