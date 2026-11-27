//! Service workers of renderer pages. A service worker runs in a realm of
//! its own, as a dedicated worker does, stepped by the page that registered
//! it. Registrations are the process's, by scope, so every page and frame
//! whose URL is in a registration's scope finds it: each connects to the
//! worker over a channel of the process's message ports, which carries the
//! worker's messages, its state, `clients.claim()` and the requests of the
//! pages it controls (see the Web API prelude's `worker.js`).

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{ErrorKind, Key, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::{Cell, RefCell};

/// The global function through which a service worker's prelude receives a
/// client that connects: its port, id, URL and type.
pub const CLIENT_HOOK: &str = "__nativeboat_service_client";

/// A registration: its worker's script, its scope and state, and the
/// address of the worker's realm.
struct Registration {
    id: u32,
    scope: String,
    script: String,
    state: String,
    realm: usize,
}

/// The process's registrations, in the order they were made.
struct Registry {
    registrations: RefCell<Vec<Registration>>,
    next: Cell<u32>,
}

// SAFETY: realms run on the thread that made them; only that thread, which
// runs the application's pages, frames and workers, reads or changes these.
unsafe impl Sync for Registry {}

static REGISTRY: Registry = Registry {
    registrations: RefCell::new(Vec::new()),
    next: Cell::new(0),
};

/// Forgets the registration whose worker runs in the realm at `address`,
/// when that realm goes away.
pub(crate) fn forget(address: usize) {
    if let Ok(mut registrations) = REGISTRY.registrations.try_borrow_mut()
        && !registrations.is_empty()
    {
        registrations.retain(|registration| registration.realm != address);
    }
}

/// Registers the service worker `script` for `scope` from `page`, starting
/// its realm, and returns the registration's id. A scope that has a
/// registration keeps it.
pub(crate) fn register(page: &mut Realm, script: &Value, scope: &Value) -> Result<u32, Thrown> {
    let script = ops::to_string(page, script)?.to_rust_lossy();
    let scope = ops::to_string(page, scope)?.to_rust_lossy();
    if let Some(known) = REGISTRY
        .registrations
        .borrow()
        .iter()
        .find(|registration| registration.scope == scope)
    {
        return Ok(known.id);
    }
    let id = REGISTRY.next.get() + 1;
    REGISTRY.next.set(id);
    // Registered before its script runs, which reports the states it
    // reaches.
    crate::worker::start_service(page, &script.clone(), &scope.clone(), |realm| {
        REGISTRY.registrations.borrow_mut().push(Registration {
            id,
            scope,
            script,
            state: String::from("parsed"),
            realm,
        });
    })?;
    Ok(id)
}

/// The registration whose scope is the longest that starts `url`: its id,
/// scope, script and state, or `null`.
pub(crate) fn matching(realm: &mut Realm, url: &Value) -> Result<Value, Thrown> {
    let url = ops::to_string(realm, url)?.to_rust_lossy();
    let registrations = REGISTRY.registrations.borrow();
    let found = registrations
        .iter()
        .filter(|registration| url.starts_with(&registration.scope))
        .max_by_key(|registration| registration.scope.len());
    let Some(found) = found else {
        return Ok(Value::Null);
    };
    let fields = alloc::vec![
        Value::Number(f64::from(found.id)),
        Value::str(&found.scope),
        Value::str(&found.script),
        Value::str(&found.state),
    ];
    drop(registrations);
    Ok(Value::Object(realm.array(fields)))
}

/// Records the state the service worker running in `worker` reached.
pub(crate) fn set_state(worker: &Realm, state: &str) {
    let address = worker as *const Realm as usize;
    if let Some(registration) = REGISTRY
        .registrations
        .borrow_mut()
        .iter_mut()
        .find(|registration| registration.realm == address)
    {
        registration.state = String::from(state);
    }
}

/// Removes the registration `id`; its worker keeps serving the pages
/// connected to it. Whether there was one.
pub(crate) fn unregister(id: u32) -> bool {
    let mut registrations = REGISTRY.registrations.borrow_mut();
    let before = registrations.len();
    registrations.retain(|registration| registration.id != id);
    registrations.len() != before
}

/// Connects the page `client` to the worker of registration `id` as the
/// client `client_id` at `url` of `kind`: a channel whose one end the
/// worker's prelude receives, in a task of its own; the other end's handle
/// in `client`.
pub(crate) fn connect(
    client: &mut Realm,
    id: u32,
    client_id: &Value,
    url: &Value,
    kind: &Value,
) -> Result<Value, Thrown> {
    let worker = REGISTRY
        .registrations
        .borrow()
        .iter()
        .find(|registration| registration.id == id)
        .map(|registration| registration.realm);
    let Some(worker) = worker else {
        return Err(client.throw(
            ErrorKind::Error,
            "the service worker's registration is gone",
        ));
    };
    if worker == client as *const Realm as usize {
        return Err(client.throw(
            ErrorKind::Error,
            "a service worker cannot be its own client",
        ));
    }
    let details = [client_id, url, kind].map(|value| ops::to_string(client, value));
    let [client_id, url, kind] = details;
    let (client_id, url, kind) = (client_id?, url?, kind?);
    let (near, far) = crate::realm_ports::channel(client);
    let Some(far_id) = crate::realm_ports::end_of(client, &Value::Object(far)) else {
        return Err(client.throw(ErrorKind::Error, "the channel closed"));
    };
    crate::realm_ports::move_out(client, &[far_id]);
    // SAFETY: a worker's realm is boxed at a stable address while it is
    // registered (its drop forgets it), runs on this thread, and is not
    // running: the client's code is.
    let worker = unsafe { &mut *(worker as *mut Realm) };
    let port = crate::realm_ports::adopt_handle(worker, far_id);
    let task = worker.function("", 0, false, move |worker, _, _| {
        let global = Value::Object(worker.global_object());
        let hook = ops::get_named(worker, &global, CLIENT_HOOK)?;
        if matches!(hook, Value::Object(_)) {
            let arguments = [
                Value::Object(port.clone()),
                Value::String(client_id.clone()),
                Value::String(url.clone()),
                Value::String(kind.clone()),
            ];
            ops::call(worker, &hook, Value::Undefined, &arguments)?;
        }
        Ok(Value::Undefined)
    });
    worker.native_timeout(task, 0.0, Rc::new(Cell::new(true)));
    Ok(Value::Object(near))
}

/// The page host's service worker functions.
pub(crate) fn define(realm: &Realm, host: &crate::Object) {
    let register = realm.function("registerServiceWorker", 2, false, |realm, _, args| {
        let id = register(realm, arg(args, 0), arg(args, 1))?;
        Ok(Value::Number(f64::from(id)))
    });
    let matching = realm.function("serviceWorkerFor", 1, false, |realm, _, args| {
        matching(realm, arg(args, 0))
    });
    let connect = realm.function("connectServiceWorker", 4, false, |realm, _, args| {
        let id = ops::to_number(realm, arg(args, 0))? as u32;
        connect(realm, id, arg(args, 1), arg(args, 2), arg(args, 3))
    });
    let unregister = realm.function("unregisterServiceWorker", 1, false, |realm, _, args| {
        let id = ops::to_number(realm, arg(args, 0))? as u32;
        Ok(Value::Boolean(unregister(id)))
    });
    let state = realm.function("serviceWorkerState", 1, false, |realm, _, args| {
        let state = ops::to_string(realm, arg(args, 0))?.to_rust_lossy();
        set_state(realm, &state);
        Ok(Value::Undefined)
    });
    for (name, function) in [
        ("registerServiceWorker", register),
        ("serviceWorkerFor", matching),
        ("connectServiceWorker", connect),
        ("unregisterServiceWorker", unregister),
        ("serviceWorkerState", state),
    ] {
        host.define(Key::name(name), Property::hidden(Value::Object(function)));
    }
}

fn arg(args: &[Value], index: usize) -> &Value {
    args.get(index).unwrap_or(&Value::Undefined)
}
