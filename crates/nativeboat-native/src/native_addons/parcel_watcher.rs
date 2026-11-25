//! `@parcel/watcher`'s binding (`watcher.node`), as its Windows backend
//! watches: `subscribe(dir, fn, { ignorePaths, ignoreGlobs })` watches a
//! directory tree with `ReadDirectoryChangesW` and calls `fn(null, events)`
//! with `{ path, type }` events. As parcel does, additions and new names of
//! existing entries are `create`, modifications of files are `update`,
//! removals and old names are `delete`; events of a path coalesce (a path
//! created and deleted between calls is not reported, and one deleted and
//! created again is an `update`), are sorted by path,
//! and are reported once changes pause for 50 ms, or at least every 500 ms.
//! Ignored paths and paths below them, and paths whose relative path an
//! ignore glob's regular expression matches whole, are left out.
//! `unsubscribe(dir, fn)` stops the subscription. Snapshots are not
//! written: `writeSnapshot` and `getEventsSince` reject.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicBool, Ordering};

const FILE_ACTION_ADDED: u32 = 1;
const FILE_ACTION_REMOVED: u32 = 2;
const FILE_ACTION_MODIFIED: u32 = 3;
const FILE_ACTION_RENAMED_OLD_NAME: u32 = 4;
const FILE_ACTION_RENAMED_NEW_NAME: u32 = 5;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const MIN_WAIT_MS: f64 = 50.0;
const MAX_WAIT_MS: f64 = 500.0;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
}

/// A path's coalesced change, as parcel's event list keeps it.
#[derive(Default, Clone, Copy)]
struct Change {
    created: bool,
    deleted: bool,
}

struct Subscription {
    dir: String,
    callback: Value,
    ignore_paths: Vec<String>,
    /// `RegExp` objects of the ignore globs.
    ignore_globs: Vec<Value>,
    handle: Option<usize>,
    cancelled: Arc<AtomicBool>,
    pending: bool,
    closed: bool,
    /// Whether the subscription keeps the program alive: from subscribing
    /// until unsubscribing, as the package's thread-safe callback does.
    referenced: Rc<Cell<bool>>,
    events: BTreeMap<String, Change>,
    /// Whether a report is scheduled, and when the last one was made.
    scheduled: bool,
    last_report: f64,
}

type Shared = Rc<RefCell<Subscription>>;

fn attributes(path: &str) -> u32 {
    let path: Vec<u16> = path.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: the path is nul-terminated.
    unsafe { GetFileAttributesW(path.as_ptr()) }
}

fn now(_: &mut Realm) -> f64 {
    crate::platform::now()
}

fn ignored(realm: &mut Realm, subscription: &Subscription, path: &str) -> Result<bool, Thrown> {
    for ignore in &subscription.ignore_paths {
        if path == ignore
            || path
                .strip_prefix(ignore.as_str())
                .is_some_and(|rest| rest.starts_with('\\'))
        {
            return Ok(true);
        }
    }
    let base = alloc::format!("{}\\", subscription.dir);
    let Some(relative) = path.strip_prefix(&base) else {
        return Ok(false);
    };
    for glob in &subscription.ignore_globs {
        let test = ops::get_named(realm, glob, "test")?;
        let matched = ops::call(realm, &test, glob.clone(), &[Value::str(relative)])?;
        if matches!(matched, Value::Boolean(true)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn record(realm: &mut Realm, state: &Shared, action: u32, name: &[u16]) -> Result<(), Thrown> {
    let path = alloc::format!("{}\\{}", state.borrow().dir, String::from_utf16_lossy(name));
    if ignored(realm, &state.borrow(), &path)? {
        return Ok(());
    }
    let mut subscription = state.borrow_mut();
    match action {
        FILE_ACTION_ADDED | FILE_ACTION_RENAMED_NEW_NAME => {
            if attributes(&path) != INVALID_FILE_ATTRIBUTES {
                let change = subscription.events.entry(path).or_default();
                if change.deleted {
                    change.deleted = false;
                } else {
                    change.created = true;
                }
            }
        }
        FILE_ACTION_MODIFIED => {
            let found = attributes(&path);
            if found != INVALID_FILE_ATTRIBUTES && found & FILE_ATTRIBUTE_DIRECTORY == 0 {
                subscription.events.entry(path).or_default();
            }
        }
        FILE_ACTION_REMOVED | FILE_ACTION_RENAMED_OLD_NAME => {
            subscription.events.entry(path).or_default().deleted = true;
        }
        _ => {}
    }
    Ok(())
}

/// Calls the subscriber with the coalesced events, if any remain.
fn report(realm: &mut Realm, state: &Shared) -> Result<(), Thrown> {
    let (callback, events) = {
        let mut subscription = state.borrow_mut();
        subscription.scheduled = false;
        if subscription.closed {
            return Ok(());
        }
        let events = core::mem::take(&mut subscription.events);
        (subscription.callback.clone(), events)
    };
    state.borrow_mut().last_report = now(realm);
    let events: Vec<Value> = events
        .into_iter()
        .filter(|(_, change)| !(change.created && change.deleted))
        .map(|(path, change)| {
            let event = realm.object();
            event.define(Key::name("path"), Property::data(Value::str(&path)));
            let kind = if change.created {
                "create"
            } else if change.deleted {
                "delete"
            } else {
                "update"
            };
            event.define(Key::name("type"), Property::data(Value::str(kind)));
            Value::Object(event)
        })
        .collect();
    if events.is_empty() {
        return Ok(());
    }
    let list = Value::Object(realm.array(events));
    ops::call(realm, &callback, Value::Undefined, &[Value::Null, list]).map(drop)
}

/// Reports after changes pause for 50 ms, or at once when the last report
/// was over 500 ms ago, as parcel's debounce does.
fn schedule(realm: &mut Realm, state: &Shared) -> Result<(), Thrown> {
    if state.borrow().events.is_empty() || state.borrow().scheduled {
        return Ok(());
    }
    if now(realm) - state.borrow().last_report > MAX_WAIT_MS {
        return report(realm, state);
    }
    state.borrow_mut().scheduled = true;
    let later = state.clone();
    let timer = realm.function("", 0, false, move |realm, _, _| {
        report(realm, &later).map(|()| Value::Undefined)
    });
    let global = Value::Object(realm.global_object());
    let set_timeout = ops::get_named(realm, &global, "setTimeout")?;
    let handle = ops::call(
        realm,
        &set_timeout,
        Value::Undefined,
        &[Value::Object(timer), Value::Number(MIN_WAIT_MS)],
    )?;
    if let Value::Object(_) = &handle
        && let Ok(unref) = ops::get_named(realm, &handle, "unref")
        && crate::array_builtin::is_callable(&unref)
    {
        ops::call(realm, &unref, handle.clone(), &[])?;
    }
    Ok(())
}

fn error_value(realm: &mut Realm, message: &str) -> Value {
    Value::Object(realm.error(ErrorKind::Error, message))
}

fn finish(state: &Shared) {
    state.borrow().referenced.set(false);
    let handle = state.borrow_mut().handle.take();
    if let Some(handle) = handle {
        crate::fs_win::watch_close(handle as crate::fs_win::Handle);
    }
}

fn read(realm: &mut Realm, state: Shared) -> Result<(), Thrown> {
    let (handle, cancelled, referenced) = {
        let mut subscription = state.borrow_mut();
        if subscription.closed || subscription.pending {
            return Ok(());
        }
        let Some(handle) = subscription.handle else {
            return Ok(());
        };
        subscription.pending = true;
        (
            handle,
            subscription.cancelled.clone(),
            subscription.referenced.clone(),
        )
    };
    let worker = state.clone();
    let submitted = realm.submit_io_with_ref(
        move || {
            // SAFETY: this worker owns the open handle until its read
            // returns; unsubscribing waits for it before closing the handle.
            unsafe { crate::fs_win::watch_read(handle as crate::fs_win::Handle, true, &cancelled) }
        },
        referenced,
        move |realm, result| {
            worker.borrow_mut().pending = false;
            match result {
                Ok(Some(changes)) => {
                    for (action, name) in changes {
                        if worker.borrow().closed {
                            break;
                        }
                        record(realm, &worker, action, &name)?;
                    }
                    schedule(realm, &worker)?;
                }
                Ok(None) => worker.borrow_mut().closed = true,
                Err(_) => {
                    // The watched directory itself is gone: its removal is
                    // the last event, as parcel reports it.
                    let dir = worker.borrow().dir.clone();
                    if attributes(&dir) == INVALID_FILE_ATTRIBUTES {
                        worker.borrow_mut().events.entry(dir).or_default().deleted = true;
                        report(realm, &worker)?;
                    } else {
                        let callback = worker.borrow().callback.clone();
                        let error = error_value(realm, "Unknown error");
                        let empty = Value::Object(realm.array(Vec::new()));
                        ops::call(realm, &callback, Value::Undefined, &[error, empty])?;
                    }
                    worker.borrow_mut().closed = true;
                }
            }
            if worker.borrow().closed {
                finish(&worker);
                return Ok(());
            }
            read(realm, worker)
        },
    );
    if submitted.is_err() {
        state.borrow_mut().pending = false;
        state.borrow_mut().closed = true;
        finish(&state);
        return Err(realm.throw(ErrorKind::Error, "Could not submit the watcher's read"));
    }
    Ok(())
}

fn string_list(
    realm: &mut Realm,
    options: &Value,
    name: &'static str,
) -> Result<Vec<String>, Thrown> {
    let Value::Object(_) = options else {
        return Ok(Vec::new());
    };
    let list = ops::get_named(realm, options, name)?;
    let Value::Object(array) = &list else {
        return Ok(Vec::new());
    };
    if !array.is_array() {
        return Ok(Vec::new());
    }
    let length = match ops::get_named(realm, &list, "length")? {
        Value::Number(length) => length as u32,
        _ => 0,
    };
    let mut items = Vec::new();
    for index in 0..length {
        let item = ops::get(realm, &list, &Key::Index(index))?;
        items.push(ops::to_string(realm, &item)?.to_rust_lossy());
    }
    Ok(items)
}

fn rejected(realm: &mut Realm, message: &str) -> crate::Completion {
    let (promise, _, reject) = crate::promise_builtin::capability(realm);
    let error = error_value(realm, message);
    ops::call(realm, &reject, Value::Undefined, &[error])?;
    Ok(promise)
}

fn resolved(realm: &mut Realm) -> crate::Completion {
    let (promise, resolve, _) = crate::promise_builtin::capability(realm);
    ops::call(realm, &resolve, Value::Undefined, &[])?;
    Ok(promise)
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let subscriptions: Rc<RefCell<Vec<Shared>>> = Rc::default();
    let active = subscriptions.clone();
    let subscribe = realm.function("subscribe", 3, false, move |realm, _, args| {
        let Value::String(dir) = arg(args, 0) else {
            return Err(realm.throw(ErrorKind::Type, "Expected a string"));
        };
        let callback = arg(args, 1);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(ErrorKind::Type, "Expected a function"));
        }
        let options = arg(args, 2);
        let dir = dir.to_rust_lossy();
        let ignore_paths = string_list(realm, &options, "ignorePaths")?;
        let mut ignore_globs = Vec::new();
        for source in string_list(realm, &options, "ignoreGlobs")? {
            let global = Value::Object(realm.global_object());
            let constructor = ops::get_named(realm, &global, "RegExp")?;
            // `std::regex_match` matches the whole relative path.
            let whole = alloc::format!("^(?:{source})$");
            ignore_globs.push(ops::construct(realm, &constructor, &[Value::str(&whole)])?);
        }
        let path: Vec<u16> = dir.encode_utf16().chain(core::iter::once(0)).collect();
        let opened = match crate::fs_win::watch_open(&path) {
            Ok(opened) => opened,
            Err(_) => return rejected(realm, &alloc::format!("Failed to open directory {dir}")),
        };
        let state = Rc::new(RefCell::new(Subscription {
            dir,
            callback,
            ignore_paths,
            ignore_globs,
            handle: Some(opened.handle as usize),
            cancelled: Arc::new(AtomicBool::new(false)),
            pending: false,
            closed: false,
            referenced: Rc::new(Cell::new(true)),
            events: BTreeMap::new(),
            scheduled: false,
            last_report: 0.0,
        }));
        state.borrow_mut().last_report = now(realm);
        active.borrow_mut().push(state.clone());
        read(realm, state)?;
        resolved(realm)
    });
    exports.define(
        Key::name("subscribe"),
        Property::data(Value::Object(subscribe)),
    );
    let unsubscribe = realm.function("unsubscribe", 3, false, move |realm, _, args| {
        let dir = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let callback = arg(args, 1);
        let mut found = Vec::new();
        subscriptions.borrow_mut().retain(|state| {
            let matches = {
                let subscription = state.borrow();
                subscription.dir == dir && crate::same_value(&subscription.callback, &callback)
            };
            if matches {
                found.push(state.clone());
            }
            !matches
        });
        for state in found {
            let (handle, pending, cancelled) = {
                let mut subscription = state.borrow_mut();
                subscription.closed = true;
                subscription.referenced.set(false);
                (
                    subscription.handle,
                    subscription.pending,
                    subscription.cancelled.clone(),
                )
            };
            cancelled.store(true, Ordering::Release);
            if let Some(handle) = handle {
                // SAFETY: the subscription keeps the handle until the pending
                // read returns and releases it.
                unsafe { crate::fs_win::watch_cancel(handle as crate::fs_win::Handle) };
                if !pending {
                    finish(&state);
                }
            }
        }
        resolved(realm)
    });
    exports.define(
        Key::name("unsubscribe"),
        Property::data(Value::Object(unsubscribe)),
    );
    for name in ["writeSnapshot", "getEventsSince"] {
        let function = realm.function(name, 3, false, |realm, _, _| {
            rejected(realm, "Snapshots are not available in this watcher")
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}
