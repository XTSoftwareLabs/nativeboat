//! Network addons:
//!
//! - `@vscode/windows-ca-certs`: `new Crypt32([store])` enumerates a system
//!   certificate store (`ROOT` by default); `next()` gives each certificate's
//!   DER bytes as a Buffer, then `undefined`, `done()` closes the store and
//!   `none()` says whether it is closed.
//! - `@vscode/os-proxy-resolver`: the proxies for a URL from the Windows
//!   settings, as the runtime's `session.resolveProxy` resolves them, and the
//!   settings themselves.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::ffi::c_void;

/// `CERT_CONTEXT`.
#[repr(C)]
struct CertificateContext {
    encoding: u32,
    encoded: *const u8,
    encoded_length: u32,
    info: *mut c_void,
    store: *mut c_void,
}

late_bound!("crypt32.dll" {
    fn CertOpenSystemStoreW(provider: usize, name: *const u16) -> *mut c_void = core::ptr::null_mut();
    fn CertEnumCertificatesInStore(store: *mut c_void, previous: *const CertificateContext) -> *const CertificateContext = core::ptr::null();
    fn CertCloseStore(store: *mut c_void, flags: u32) -> i32 = 0;
});

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn method(
    realm: &Realm,
    target: &Object,
    name: &'static str,
    length: u32,
    body: impl Fn(&mut Realm, &[Value]) -> crate::Completion + 'static,
) {
    let function = realm.function(name, length, false, move |realm, _, args| body(realm, args));
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

/// A Node Buffer of `bytes`.
fn buffer(realm: &mut Realm, bytes: Vec<u8>) -> crate::Completion {
    if realm.buffer_prototype.is_none()
        && let Some(require) = realm.node_library
    {
        let _ = require(realm, "buffer")?;
    }
    let prototype = realm.buffer_prototype.clone();
    Ok(Value::Object(crate::typed_array_builtin::new_bytes(
        realm, bytes, prototype,
    )))
}

/// An open store and the certificate last given, closed when done.
struct Store {
    store: Cell<*mut c_void>,
    current: Cell<*const CertificateContext>,
}

impl Store {
    fn close(&self) {
        let store = self.store.replace(core::ptr::null_mut());
        if !store.is_null() {
            // SAFETY: the store was opened by this object; closing it frees
            // the context enumeration holds.
            unsafe { CertCloseStore(store, 0) };
        }
        self.current.set(core::ptr::null());
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn ca_certificates(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    let constructor = realm.function("Crypt32", 0, true, move |realm, _, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructor Crypt32 cannot be invoked without 'new'",
            ));
        }
        let name = match arg(args, 0) {
            Value::String(name) => name.to_rust_lossy(),
            _ => "ROOT".into(),
        };
        let name: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
        // SAFETY: the name is nul-terminated; a null store is a closed one.
        let store = unsafe { CertOpenSystemStoreW(0, name.as_ptr()) };
        let state = Rc::new(Store {
            store: Cell::new(store),
            current: Cell::new(core::ptr::null()),
        });
        let object = realm.object();
        object.set_prototype(Some(instance_prototype.clone()));
        let next = state.clone();
        method(realm, &object, "next", 0, move |realm, _| {
            let store = next.store.get();
            if store.is_null() {
                return Ok(Value::Undefined);
            }
            // SAFETY: the store is open; the previous context came from it
            // and is released by this call.
            let context = unsafe { CertEnumCertificatesInStore(store, next.current.get()) };
            next.current.set(context);
            if context.is_null() {
                next.close();
                return Ok(Value::Undefined);
            }
            // SAFETY: the context is live until the next enumeration and
            // holds `encoded_length` encoded bytes.
            let bytes = unsafe {
                core::slice::from_raw_parts((*context).encoded, (*context).encoded_length as usize)
            }
            .to_vec();
            buffer(realm, bytes)
        });
        let done = state.clone();
        method(realm, &object, "done", 0, move |_, _| {
            done.close();
            Ok(Value::Undefined)
        });
        let none = state;
        method(realm, &object, "none", 0, move |_, _| {
            Ok(Value::Boolean(none.store.get().is_null()))
        });
        Ok(Value::Object(object))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    exports.define(
        Key::name("Crypt32"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

/// Chromium's proxy list as the resolver's proxies: `DIRECT`, `PROXY` and
/// `HTTPS` servers as `http`, `SOCKS` servers as `socks`.
fn proxies(list: &str) -> Vec<(&'static str, Option<String>)> {
    list.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let (kind, host) = entry.split_once(' ').unwrap_or((entry, ""));
            let host = host.trim();
            match kind.to_ascii_uppercase().as_str() {
                "DIRECT" => Some(("direct", None)),
                "PROXY" | "HTTP" | "HTTPS" => Some(("http", Some(host.into()))),
                "SOCKS" | "SOCKS4" | "SOCKS5" => Some(("socks", Some(host.into()))),
                _ => None,
            }
        })
        .collect()
}

fn proxy_object(realm: &Realm, kind: &str, host: Option<&str>) -> Value {
    let proxy = realm.object();
    proxy.define(Key::name("kind"), Property::data(Value::str(kind)));
    if let Some(host) = host {
        proxy.define(Key::name("host"), Property::data(Value::str(host)));
    }
    Value::Object(proxy)
}

/// A promise of the proxies for a URL, resolved on the I/O pool, since
/// automatic detection can wait on the network.
fn resolve_proxy(realm: &mut Realm, url: &Value) -> crate::Completion {
    let url = ops::to_string(realm, url)?.to_rust_lossy();
    let (promise, resolve, _) = crate::promise_builtin::capability(realm);
    let submitted = realm.submit_io(
        move || crate::system_proxy::resolve(&url, false),
        move |realm, list| {
            let values = proxies(&list)
                .into_iter()
                .map(|(kind, host)| proxy_object(realm, kind, host.as_deref()))
                .collect();
            let array = Value::Object(realm.array(values));
            ops::call(realm, &resolve, Value::Undefined, &[array]).map(drop)
        },
    );
    if submitted.is_err() {
        return Err(realm.throw(ErrorKind::Error, "Could not queue proxy resolution"));
    }
    Ok(promise)
}

/// The settings' fixed proxy rules: one server for every scheme, or
/// `scheme=server` entries.
fn static_rules(realm: &Realm, proxy: &str) -> Value {
    let rules = realm.object();
    for entry in proxy.split([';', ' ']).filter(|entry| !entry.is_empty()) {
        let (scheme, server) = match entry.split_once('=') {
            Some((scheme, server)) => (Some(scheme.to_ascii_lowercase()), server),
            None => (None, entry),
        };
        let server = server
            .split_once("://")
            .map_or(server, |(_, address)| address);
        let targets: &[(&str, &str)] = match scheme.as_deref() {
            None => &[("http", "http"), ("https", "http")],
            Some("http") => &[("http", "http")],
            Some("https") => &[("https", "http")],
            Some("socks") => &[("socks", "socks")],
            Some(_) => &[],
        };
        for (name, kind) in targets {
            rules.define(
                Key::name(name),
                Property::data(proxy_object(realm, kind, Some(server))),
            );
        }
    }
    Value::Object(rules)
}

fn status(realm: &Realm, state: &str, url: Option<&str>) -> Value {
    let status = realm.object();
    status.define(Key::name("state"), Property::data(Value::str(state)));
    if let Some(url) = url {
        status.define(Key::name("url"), Property::data(Value::str(url)));
    }
    Value::Object(status)
}

/// The proxy configuration: the environment's proxy variables, the user's
/// settings, and the state of each script source. Detected scripts are not
/// downloaded here, so their state is `unknown` when detection is on.
fn read_config(realm: &mut Realm) -> crate::Completion {
    let config = realm.object();
    let environment = realm.object();
    for (name, variables) in [
        ("httpProxy", ["HTTP_PROXY", "http_proxy"]),
        ("httpsProxy", ["HTTPS_PROXY", "https_proxy"]),
        ("allProxy", ["ALL_PROXY", "all_proxy"]),
        ("noProxy", ["NO_PROXY", "no_proxy"]),
    ] {
        for variable in variables {
            if let Some(value) = crate::platform::environment_value(&crate::JsStr::from(variable)) {
                let entry = realm.object();
                entry.define(Key::name("variable"), Property::data(Value::str(variable)));
                entry.define(Key::name("value"), Property::data(Value::String(value)));
                environment.define(Key::name(name), Property::data(Value::Object(entry)));
                break;
            }
        }
    }
    config.define(
        Key::name("environment"),
        Property::data(Value::Object(environment)),
    );
    let settings = crate::system_proxy::settings();
    let auto_detect = settings
        .as_ref()
        .is_some_and(|settings| settings.auto_detect);
    config.define(
        Key::name("autoDetect"),
        Property::data(Value::Boolean(auto_detect)),
    );
    let script = settings
        .as_ref()
        .and_then(|settings| settings.script.clone());
    if let Some(script) = &script {
        config.define(Key::name("pacUrl"), Property::data(Value::str(script)));
    }
    let detection = if auto_detect { "unknown" } else { "disabled" };
    config.define(
        Key::name("wpadDhcp"),
        Property::data(status(realm, detection, None)),
    );
    config.define(
        Key::name("wpadDns"),
        Property::data(status(realm, detection, None)),
    );
    let configured = match &script {
        Some(url) => status(realm, "unknown", Some(url)),
        None => status(realm, "unconfigured", None),
    };
    config.define(Key::name("configuredPac"), Property::data(configured));
    let proxy = settings
        .as_ref()
        .and_then(|settings| settings.proxy.clone());
    if let Some(proxy) = &proxy {
        config.define(
            Key::name("staticRules"),
            Property::data(static_rules(realm, proxy)),
        );
    }
    let platform = realm.object();
    platform.define(Key::name("kind"), Property::data(Value::str("windows")));
    if let Some(proxy) = &proxy {
        platform.define(Key::name("proxy"), Property::data(Value::str(proxy)));
    }
    if let Some(bypass) = settings
        .as_ref()
        .and_then(|settings| settings.bypass.as_deref())
    {
        platform.define(Key::name("proxyBypass"), Property::data(Value::str(bypass)));
    }
    config.define(
        Key::name("platform"),
        Property::data(Value::Object(platform)),
    );
    let (promise, resolve, _) = crate::promise_builtin::capability(realm);
    ops::call(realm, &resolve, Value::Undefined, &[Value::Object(config)])?;
    Ok(promise)
}

pub(crate) fn proxy_resolver(realm: &mut Realm) -> Object {
    let exports = realm.object();
    method(realm, &exports, "resolveProxy", 1, |realm, args| {
        resolve_proxy(realm, &arg(args, 0))
    });
    method(realm, &exports, "readProxyConfig", 0, |realm, _| {
        read_config(realm)
    });
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    // A resolver reads the settings each time; it reports no changes.
    let constructor = realm.function("ProxyResolver", 0, true, move |realm, _, _| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructor ProxyResolver cannot be invoked without 'new'",
            ));
        }
        let resolver = realm.object();
        resolver.set_prototype(Some(instance_prototype.clone()));
        method(realm, &resolver, "resolve", 1, |realm, args| {
            resolve_proxy(realm, &arg(args, 0))
        });
        method(realm, &resolver, "readProxyConfig", 0, |realm, _| {
            read_config(realm)
        });
        resolver.define(
            Key::name("configGeneration"),
            Property::data(Value::Number(0.0)),
        );
        method(realm, &resolver, "reportProxyFailed", 1, |_, _| {
            Ok(Value::Undefined)
        });
        let subscriptions = Rc::new(Cell::new(0_u32));
        method(realm, &resolver, "onChange", 1, move |_, _| {
            subscriptions.set(subscriptions.get() + 1);
            Ok(Value::Number(f64::from(subscriptions.get())))
        });
        method(realm, &resolver, "offChange", 1, |_, _| {
            Ok(Value::Undefined)
        });
        method(realm, &resolver, "close", 0, |_, _| Ok(Value::Undefined));
        Ok(Value::Object(resolver))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    exports.define(
        Key::name("ProxyResolver"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_lists_become_resolver_proxies() {
        assert_eq!(
            proxies("PROXY a:80; SOCKS b:1080;DIRECT;HTTPS c:443"),
            [
                ("http", Some(String::from("a:80"))),
                ("socks", Some(String::from("b:1080"))),
                ("direct", None),
                ("http", Some(String::from("c:443"))),
            ]
        );
    }

    #[test]
    fn root_certificates_are_enumerated_as_der() {
        let mut realm = Realm::new();
        let module = ca_certificates(&mut realm);
        let constructor = ops::get_named(&mut realm, &Value::Object(module), "Crypt32").unwrap();
        let store = ops::construct(&mut realm, &constructor, &[]).unwrap();
        let next = ops::get_named(&mut realm, &store, "next").unwrap();
        let first = ops::call(&mut realm, &next, store.clone(), &[]).unwrap();
        // DER certificates are SEQUENCEs.
        let Value::Object(first) = first else {
            panic!("a certificate");
        };
        let byte = ops::get(&mut realm, &Value::Object(first), &Key::Index(0)).unwrap();
        assert!(matches!(byte, Value::Number(tag) if tag == 48.0));
        let none = ops::get_named(&mut realm, &store, "none").unwrap();
        assert!(matches!(
            ops::call(&mut realm, &none, store.clone(), &[]).unwrap(),
            Value::Boolean(false)
        ));
        let done = ops::get_named(&mut realm, &store, "done").unwrap();
        ops::call(&mut realm, &done, store.clone(), &[]).unwrap();
        assert!(matches!(
            ops::call(&mut realm, &none, store.clone(), &[]).unwrap(),
            Value::Boolean(true)
        ));
        assert!(matches!(
            ops::call(&mut realm, &next, store, &[]).unwrap(),
            Value::Undefined
        ));
    }
}
