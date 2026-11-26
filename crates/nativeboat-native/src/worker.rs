//! Dedicated workers of renderer pages. A worker runs in a realm of its own
//! on the page's thread: the page steps its workers each time the host
//! steps it, so a worker's tasks interleave with the page's rather than run
//! beside them. A worker's global scope is the Web API prelude's, which the
//! renderer graph runs first, in its worker form. The worker's script is a
//! compiled module of the renderer graph that its URL names, or, for a
//! script the program made while it ran, as a blob, what the prelude
//! evaluates of it.
//!
//! The page's `Worker` and the worker's global scope talk over two channels
//! of the process's message ports: one for messages, one for the errors the
//! worker reports to its page.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{ErrorKind, HostWait, Key, Property, Realm, Thrown, Value, ops};
use alloc::boxed::Box;

/// The global through which the prelude receives a worker's arguments:
/// its host, its message and error ports, its URL, its name and the text
/// of a script made at run time.
pub const PRELUDE_ARGUMENTS: &str = "__nativeboat_worker";

/// A worker a page started.
pub(crate) struct Worker {
    id: u32,
    realm: Box<Realm>,
}

impl Worker {
    /// A worker, or a frame, with its realm.
    pub(crate) fn new(id: u32, realm: Box<Realm>) -> Worker {
        Worker { id, realm }
    }
}

impl core::fmt::Debug for Worker {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Worker")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Evaluates the worker scope of a renderer graph in a realm: the graph's
/// Web API prelude, which starts the worker's script.
pub type WorkerScope = fn(&mut Realm) -> Result<(), Thrown>;

impl Realm {
    /// Lets this page start workers, whose realms evaluate `scope` first.
    pub fn set_worker_scope(&mut self, scope: WorkerScope) {
        self.worker_scope = Some(scope);
    }
}

/// Starts a worker for the page `page`: a realm whose prelude adopts the
/// ends `port` and `errors` of two channels, and runs the script `url`
/// names, or `script` when the program made it. Returns the worker's id.
pub(crate) fn start(
    page: &mut Realm,
    port: &Value,
    errors: &Value,
    url: &Value,
    name: &Value,
    script: &Value,
) -> Result<u32, Thrown> {
    let Some(scope) = page.worker_scope else {
        return Err(page.throw(
            ErrorKind::Type,
            "workers need the renderer libraries this build was made without",
        ));
    };
    let ends = [port, errors].map(|end| crate::realm_ports::end_of(page, end));
    let [Some(port), Some(errors)] = ends else {
        return Err(page.throw(
            ErrorKind::Type,
            "a worker's ports must be live channel ends",
        ));
    };
    let url = ops::to_string(page, url)?;
    let name = ops::to_string(page, name)?;
    let script = match script {
        Value::Undefined | Value::Null => None,
        script => Some(ops::to_string(page, script)?),
    };
    crate::realm_ports::move_out(page, &[port, errors]);
    let mut realm = Box::new(Realm::new());
    realm.message_ports = page.message_ports.clone();
    realm.node_library = page.node_library;
    realm.worker_scope = page.worker_scope;
    realm.register_scripts(page.registered_scripts());
    realm.source_functions = page.source_functions.clone();
    let host = crate::web::worker_host(&mut realm);
    realm.web_host = Some(host.clone());
    let port = crate::realm_ports::adopt_handle(&mut realm, port);
    let errors = crate::realm_ports::adopt_handle(&mut realm, errors);
    let script = script.map_or(Value::Null, Value::String);
    let arguments = realm.array(vec![
        Value::Object(host),
        Value::Object(port),
        Value::Object(errors),
        Value::String(url),
        Value::String(name),
        script,
    ]);
    realm.global_object().define(
        Key::name(PRELUDE_ARGUMENTS),
        Property::hidden(Value::Object(arguments)),
    );
    page.next_worker += 1;
    let id = page.next_worker;
    // Exceptions reach the worker's `error` handlers and its page, as the
    // prelude's `host.onerror` reports them; the worker keeps running.
    let _ = realm.start_host(scope);
    page.workers.push(Worker::new(id, realm));
    Ok(id)
}

/// Starts the service worker `script` for `scope` in a realm of its own,
/// which `page` steps as it steps its workers, and returns the realm's
/// address. Its prelude takes no ports: pages connect to it (see
/// [`crate::service_worker`]).
pub(crate) fn start_service(page: &mut Realm, script: &str, scope: &str) -> Result<usize, Thrown> {
    let Some(scope_prelude) = page.worker_scope else {
        return Err(page.throw(
            ErrorKind::Type,
            "service workers need the renderer libraries this build was made without",
        ));
    };
    let mut realm = Box::new(Realm::new());
    realm.message_ports = page.message_ports.clone();
    realm.node_library = page.node_library;
    realm.worker_scope = page.worker_scope;
    realm.register_scripts(page.registered_scripts());
    realm.source_functions = page.source_functions.clone();
    let host = crate::web::worker_host(&mut realm);
    realm.web_host = Some(host.clone());
    let arguments = realm.array(vec![
        Value::Object(host),
        Value::Null,
        Value::Null,
        Value::str(script),
        Value::str(""),
        Value::Null,
        Value::str(scope),
    ]);
    realm.global_object().define(
        Key::name(PRELUDE_ARGUMENTS),
        Property::hidden(Value::Object(arguments)),
    );
    page.next_worker += 1;
    let id = page.next_worker;
    let address = &*realm as *const Realm as usize;
    let _ = realm.start_host(scope_prelude);
    page.workers.push(Worker::new(id, realm));
    Ok(address)
}

/// Ends the worker `id` of `page`, as `worker.terminate()` does: its realm
/// and its ports go away, and its queued work never runs.
pub(crate) fn terminate(page: &mut Realm, id: u32) {
    if let Some(index) = page.workers.iter().position(|worker| worker.id == id) {
        let mut worker = page.workers.remove(index);
        // A frame's page lets its realm go first.
        if let Some(frame_end) = page.frame_end {
            frame_end(&mut *worker.realm);
        }
        end(&mut worker.realm);
    }
}

/// Closes the ports of a worker's realm, and of the workers it started,
/// before the realm goes away.
fn end(realm: &mut Realm) {
    for mut worker in core::mem::take(&mut realm.workers) {
        end(&mut worker.realm);
    }
    crate::realm_ports::release(realm);
}

/// Runs the work due in `page`'s workers after the page's own step, and
/// returns the earlier of `wait` and theirs. Workers that closed themselves
/// go away.
pub(crate) fn step(page: &mut Realm, wait: HostWait) -> HostWait {
    if page.workers.is_empty() {
        return wait;
    }
    let mut workers = core::mem::take(&mut page.workers);
    let mut signals = Vec::new();
    let mut wait = wait;
    for worker in &mut workers {
        // A worker's exceptions never end the program: its prelude reports
        // them, so stepping it yields a wait.
        if let Ok(next) = worker.realm.step_host() {
            wait = wait.merge(next, &mut signals);
        }
    }
    workers.retain_mut(|worker| {
        if worker.realm.worker_closing {
            end(&mut worker.realm);
            false
        } else {
            true
        }
    });
    // Workers a page started while its workers ran come after them.
    workers.append(&mut page.workers);
    page.workers = workers;
    HostWait {
        delay: wait.delay,
        signal: signals.first().copied().or(wait.signal),
    }
}

/// Milliseconds until a worker of `realm` has work to run.
pub(crate) fn next_wake(realm: &Realm) -> Option<u32> {
    realm
        .workers
        .iter()
        .filter_map(|worker| worker.realm.next_wake())
        .min()
}

/// Whether a worker of `realm` has jobs queued.
pub(crate) fn has_queued_jobs(realm: &Realm) -> bool {
    realm
        .workers
        .iter()
        .any(|worker| worker.realm.has_queued_jobs())
}
