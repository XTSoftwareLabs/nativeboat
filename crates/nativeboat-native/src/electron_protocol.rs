//! Electron's `protocol` (also `session.defaultSession.protocol`): custom
//! schemes the main process serves to its pages, such as VS Code's
//! `vscode-file:`. Handlers registered with `registerFileProtocol`,
//! `registerBufferProtocol`, `registerStringProtocol` or `handle` run in the
//! main realm when a page loads one of the scheme's URLs: a style sheet,
//! image, font or media file, or a renderer `fetch`. `handle('file', …)`
//! intercepts `file:` subresources too, as in Electron.
//!
//! Pages load synchronously, so a handler's answer is taken once the main
//! realm's queued jobs have run: callbacks given at once or from a promise
//! job are seen; answers that wait for I/O or a timer are not, and the
//! resource then does not load.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicUsize, Ordering};

/// How a scheme's handler answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerKind {
    /// `registerFileProtocol`: a file path.
    File,
    /// `registerBufferProtocol`: bytes.
    Buffer,
    /// `registerStringProtocol`: text.
    String,
    /// `handle`: a `Response`.
    Handle,
}

/// What a served URL gives a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    /// The file at this path.
    File(String),
    /// These bytes.
    Bytes(Vec<u8>),
}

/// The schemes the main process serves, with the main realm that runs their
/// handlers.
struct Registry {
    main: *mut Realm,
    /// Lowercase scheme, how it is handled, and the handler.
    schemes: RefCell<Vec<(String, HandlerKind, Value)>>,
    /// Whether a handler is running, so a load it causes is not served
    /// again from inside it.
    serving: Cell<bool>,
}

/// The process's registry, created when the main process first registers a
/// scheme and kept for the program's life. Only the UI thread uses it.
static REGISTRY: AtomicUsize = AtomicUsize::new(0);

fn registry() -> Option<&'static Registry> {
    let pointer = REGISTRY.load(Ordering::Acquire);
    // SAFETY: a non-zero pointer is the registry leaked by `registry_for`,
    // which is never freed.
    (pointer != 0).then(|| unsafe { &*(pointer as *const Registry) })
}

fn registry_for(realm: &mut Realm) -> &'static Registry {
    if let Some(registry) = registry() {
        return registry;
    }
    let registry = alloc::boxed::Box::leak(alloc::boxed::Box::new(Registry {
        main: realm as *mut Realm,
        schemes: RefCell::new(Vec::new()),
        serving: Cell::new(false),
    }));
    REGISTRY.store(registry as *const Registry as usize, Ordering::Release);
    registry
}

/// The lowercase scheme of an absolute URL, without its colon.
fn scheme_of(url: &str) -> Option<String> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let valid = scheme.len() > 1
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+-.".contains(&byte));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Whether the main process serves this URL's scheme.
pub fn serves(url: &str) -> bool {
    let (Some(registry), Some(scheme)) = (registry(), scheme_of(url.trim())) else {
        return false;
    };
    registry
        .schemes
        .borrow()
        .iter()
        .any(|(known, _, _)| *known == scheme)
}

/// A handler's answer: its status, body and content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The HTTP status: 200 unless a `Response` or callback says otherwise.
    pub status: u16,
    /// The file or bytes.
    pub body: Served,
    /// The content type the handler gave, if any.
    pub mime: Option<String>,
}

/// What the main process serves for a URL that a page loads: `None` when
/// its scheme is not served, the handler refuses it, answers with a status
/// other than success, or does not answer in time.
pub fn serve(url: &str) -> Option<Served> {
    reply(url)
        .filter(|reply| (200..300).contains(&reply.status))
        .map(|reply| reply.body)
}

/// The main process's answer for a URL, whatever its status: `None` when
/// its scheme is not served, the handler refuses it or does not answer in
/// time.
pub fn reply(url: &str) -> Option<Reply> {
    let registry = registry()?;
    let url = url.trim();
    let scheme = scheme_of(url)?;
    let (kind, handler) = registry
        .schemes
        .borrow()
        .iter()
        .find(|(known, _, _)| *known == scheme)
        .map(|(_, kind, handler)| (*kind, handler.clone()))?;
    if registry.serving.replace(true) {
        return None;
    }
    // SAFETY: the main realm lives for the program, and pages load on the
    // UI thread after the main realm's code has yielded (as for `sendSync`).
    let main = unsafe { &mut *registry.main };
    let replied = run_handler(main, kind, &handler, url);
    registry.serving.set(false);
    replied.ok().flatten()
}

fn status_of(realm: &mut Realm, value: &Value, key: &'static str) -> Result<Option<u16>, Thrown> {
    Ok(match ops::get_named(realm, value, key)? {
        Value::Number(status) if (100.0..1000.0).contains(&status) => Some(status as u16),
        _ => None,
    })
}

fn text_of(realm: &mut Realm, value: &Value, key: &'static str) -> Result<Option<String>, Thrown> {
    Ok(match ops::get_named(realm, value, key)? {
        Value::String(text) => Some(text.to_rust_lossy()),
        _ => None,
    })
}

/// Runs a handler for `url` in the main realm and reads its answer.
fn run_handler(
    realm: &mut Realm,
    kind: HandlerKind,
    handler: &Value,
    url: &str,
) -> Result<Option<Reply>, Thrown> {
    if kind == HandlerKind::Handle {
        return handle_request(realm, handler, url);
    }
    let request = realm.object();
    request.define(Key::name("url"), Property::data(Value::str(url)));
    request.define(Key::name("method"), Property::data(Value::str("GET")));
    request.define(Key::name("referrer"), Property::data(Value::str("")));
    request.define(
        Key::name("headers"),
        Property::data(Value::Object(realm.object())),
    );
    let answer: Rc<RefCell<Option<Value>>> = Rc::default();
    let slot = answer.clone();
    let callback = realm.function("callback", 1, false, move |_, _, args| {
        slot.borrow_mut()
            .get_or_insert(args.first().cloned().unwrap_or(Value::Undefined));
        Ok(Value::Undefined)
    });
    ops::call(
        realm,
        handler,
        Value::Undefined,
        &[Value::Object(request), Value::Object(callback)],
    )?;
    if answer.borrow().is_none() {
        realm.run_jobs()?;
    }
    let Some(answer) = answer.borrow_mut().take() else {
        return Ok(None);
    };
    // An object answer carries its result in `path` or `data`, with
    // `statusCode` and `mimeType`, or fails with `error`.
    let (value, status, mime) = match &answer {
        Value::Object(object) if !is_bytes(object) => {
            if !matches!(ops::get_named(realm, &answer, "error")?, Value::Undefined) {
                return Ok(None);
            }
            let key = if kind == HandlerKind::File {
                "path"
            } else {
                "data"
            };
            (
                ops::get_named(realm, &answer, key)?,
                status_of(realm, &answer, "statusCode")?,
                text_of(realm, &answer, "mimeType")?,
            )
        }
        _ => (answer.clone(), None, None),
    };
    let body = match (kind, value) {
        (HandlerKind::File, Value::String(path)) => Served::File(path.to_rust_lossy()),
        (HandlerKind::String, Value::String(text)) => {
            Served::Bytes(text.to_rust_lossy().into_bytes())
        }
        (HandlerKind::Buffer, value) => match bytes_of(&value) {
            Some(bytes) => Served::Bytes(bytes),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(Reply {
        status: status.unwrap_or(200),
        body,
        mime,
    }))
}

fn is_bytes(object: &Object) -> bool {
    matches!(
        object.data().kind,
        Kind::TypedArray(_) | Kind::ArrayBuffer(_) | Kind::DataView(_)
    )
}

/// The bytes of a `Buffer`, typed array, `DataView` or `ArrayBuffer`.
fn bytes_of(value: &Value) -> Option<Vec<u8>> {
    if let Value::Object(object) = value
        && let Kind::ArrayBuffer(bytes) = &object.data().kind
    {
        return Some(bytes.borrow().clone());
    }
    crate::typed_array_builtin::view_bytes(value)
}

/// The value a promise settled with, once the main realm's jobs have run.
fn settled(realm: &mut Realm, value: Value) -> Result<Option<Value>, Thrown> {
    let Value::Object(object) = &value else {
        return Ok(Some(value));
    };
    if !matches!(object.data().kind, Kind::Promise(_)) {
        return Ok(Some(value));
    }
    realm.run_jobs()?;
    let state = match &object.data().kind {
        Kind::Promise(state) => state.borrow().state.clone(),
        _ => unreachable!("checked above"),
    };
    Ok(match state {
        crate::promise_builtin::State::Fulfilled(value) => Some(value),
        _ => None,
    })
}

/// `protocol.handle`: the handler gets a `Request` and answers with a
/// `Response`, or a promise of one; its body is the resource. A response
/// of type `error` (`Response.error()`) refuses the request.
fn handle_request(realm: &mut Realm, handler: &Value, url: &str) -> Result<Option<Reply>, Thrown> {
    let global = Value::Object(realm.global_object());
    let request = match ops::get_named(realm, &global, "Request")? {
        constructor @ Value::Object(_) => ops::construct(realm, &constructor, &[Value::str(url)])?,
        _ => {
            let request = realm.object();
            request.define(Key::name("url"), Property::data(Value::str(url)));
            request.define(Key::name("method"), Property::data(Value::str("GET")));
            Value::Object(request)
        }
    };
    let answer = ops::call(realm, handler, Value::Undefined, &[request])?;
    let Some(response) = settled(realm, answer)? else {
        return Ok(None);
    };
    if !matches!(response, Value::Object(_))
        || text_of(realm, &response, "type")?.as_deref() == Some("error")
    {
        return Ok(None);
    }
    let status = status_of(realm, &response, "status")?.unwrap_or(200);
    let headers = ops::get_named(realm, &response, "headers")?;
    let get = match &headers {
        Value::Object(_) => ops::get_named(realm, &headers, "get")?,
        _ => Value::Undefined,
    };
    let mime = match get {
        Value::Object(_) => match ops::call(realm, &get, headers, &[Value::str("content-type")])? {
            Value::String(text) => Some(text.to_rust_lossy()),
            _ => None,
        },
        _ => None,
    };
    let read = ops::get_named(realm, &response, "arrayBuffer")?;
    if !matches!(read, Value::Object(_)) {
        return Ok(None);
    }
    let body = ops::call(realm, &read, response, &[])?;
    Ok(settled(realm, body)?
        .and_then(|body| bytes_of(&body))
        .map(|bytes| Reply {
            status,
            body: Served::Bytes(bytes),
            mime,
        }))
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn scheme_arg(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    match args.first() {
        Some(Value::String(scheme)) => Ok(scheme.to_rust_lossy().to_ascii_lowercase()),
        _ => Err(realm.throw(ErrorKind::Type, "a protocol scheme must be a string")),
    }
}

fn method(
    realm: &mut Realm,
    target: &Object,
    name: &'static str,
    length: u32,
    body: impl Fn(&mut Realm, &[Value]) -> Result<Value, Thrown> + 'static,
) {
    let function = realm.function(name, length, false, move |realm, _, args| body(realm, args));
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

/// Registers `scheme` with `kind`. A scheme registers once per kind family:
/// `register*Protocol` schemes and `handle` schemes each refuse a second
/// registration of the same scheme.
fn register(realm: &mut Realm, args: &[Value], kind: HandlerKind) -> Result<bool, Thrown> {
    let scheme = scheme_arg(realm, args)?;
    let handler = arg(args, 1);
    if !matches!(&handler, Value::Object(object) if object.code().is_some()) {
        return Err(realm.throw(ErrorKind::Type, "a protocol handler must be a function"));
    }
    let registry = registry_for(realm);
    let mut schemes = registry.schemes.borrow_mut();
    let handles = kind == HandlerKind::Handle;
    if schemes.iter().any(|(known, known_kind, _)| {
        *known == scheme && (*known_kind == HandlerKind::Handle) == handles
    }) {
        return Ok(false);
    }
    // `handle` comes first for a scheme also registered the older way, as
    // Electron consults handled schemes before registered ones.
    if handles {
        schemes.insert(0, (scheme, kind, handler));
    } else {
        schemes.push((scheme, kind, handler));
    }
    Ok(true)
}

/// Removes the scheme's registration of `handle` or of `register*Protocol`.
fn unregister(realm: &mut Realm, args: &[Value], handles: bool) -> Result<bool, Thrown> {
    let scheme = scheme_arg(realm, args)?;
    let Some(registry) = registry() else {
        return Ok(false);
    };
    let mut schemes = registry.schemes.borrow_mut();
    let before = schemes.len();
    schemes.retain(|(known, kind, _)| {
        !(*known == scheme && (*kind == HandlerKind::Handle) == handles)
    });
    Ok(schemes.len() != before)
}

fn registered(realm: &mut Realm, args: &[Value], handles: bool) -> Result<bool, Thrown> {
    let scheme = scheme_arg(realm, args)?;
    Ok(registry().is_some_and(|registry| {
        registry
            .schemes
            .borrow()
            .iter()
            .any(|(known, kind, _)| *known == scheme && (*kind == HandlerKind::Handle) == handles)
    }))
}

/// `protocol`: privileged schemes are recorded, and custom schemes are
/// served by the handlers the main process registers.
pub(crate) fn protocol(realm: &mut Realm) -> Object {
    let protocol = realm.object();
    let schemes: Rc<RefCell<Vec<Value>>> = Rc::default();
    method(
        realm,
        &protocol,
        "registerSchemesAsPrivileged",
        1,
        move |realm, args| {
            let custom = arg(args, 0);
            if !crate::array_builtin::is_array(realm, &custom)? {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "registerSchemesAsPrivileged expects an array of schemes",
                ));
            }
            schemes.borrow_mut().push(custom);
            Ok(Value::Undefined)
        },
    );
    for (name, kind) in [
        ("registerFileProtocol", HandlerKind::File),
        ("registerBufferProtocol", HandlerKind::Buffer),
        ("registerStringProtocol", HandlerKind::String),
    ] {
        method(realm, &protocol, name, 2, move |realm, args| {
            register(realm, args, kind).map(Value::Boolean)
        });
    }
    method(realm, &protocol, "handle", 2, |realm, args| {
        let scheme = scheme_arg(realm, args)?;
        if !register(realm, args, HandlerKind::Handle)? {
            return Err(realm.throw(
                ErrorKind::Error,
                &alloc::format!("Failed to register protocol: {scheme}"),
            ));
        }
        Ok(Value::Undefined)
    });
    method(realm, &protocol, "unhandle", 1, |realm, args| {
        unregister(realm, args, true).map(|_| Value::Undefined)
    });
    method(realm, &protocol, "isProtocolHandled", 1, |realm, args| {
        registered(realm, args, true).map(Value::Boolean)
    });
    method(realm, &protocol, "unregisterProtocol", 1, |realm, args| {
        unregister(realm, args, false).map(Value::Boolean)
    });
    method(
        realm,
        &protocol,
        "isProtocolRegistered",
        1,
        |realm, args| registered(realm, args, false).map(Value::Boolean),
    );
    protocol
}

/// `internalBinding('protocol')` for a renderer's `fetch`: `serves(url)`,
/// and `serve(url)` giving the main process's answer as `{ status, body,
/// mimeType }` (`body` a `Uint8Array`), or `undefined` when its scheme is
/// not served or the handler refuses it.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let binding = realm.object();
    method(realm, &binding, "serves", 1, |realm, args| {
        let url = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        Ok(Value::Boolean(serves(&url)))
    });
    method(realm, &binding, "serve", 1, |realm, args| {
        let url = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let Some(reply) = reply(&url) else {
            return Ok(Value::Undefined);
        };
        let bytes = match reply.body {
            Served::File(path) => match crate::platform::read_file(&path) {
                Some(bytes) => bytes,
                None => return Ok(Value::Undefined),
            },
            Served::Bytes(bytes) => bytes,
        };
        let answer = realm.object();
        answer.define(
            Key::name("status"),
            Property::data(Value::Number(f64::from(reply.status))),
        );
        let body = crate::typed_array_builtin::new_bytes(realm, bytes, None);
        answer.define(Key::name("body"), Property::data(Value::Object(body)));
        if let Some(mime) = reply.mime {
            answer.define(Key::name("mimeType"), Property::data(Value::str(&mime)));
        }
        Ok(Value::Object(answer))
    });
    binding
}

#[cfg(test)]
mod tests;
