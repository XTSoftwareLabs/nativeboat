//! Native backing for Node's `internalBinding('fs_event_wrap')`.

use crate::prelude::*;
use crate::{Completion, Key, Object, Property, Realm, Slot, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static NEXT_ASYNC_ID: AtomicU64 = AtomicU64::new(1);

struct Watcher {
    handle: Option<usize>,
    cancelled: Arc<AtomicBool>,
    recursive: bool,
    pending: bool,
    closed: bool,
    async_id: u64,
    referenced: Rc<Cell<bool>>,
    path: Vec<u16>,
    filename: Option<Vec<u16>>,
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn path_units(value: &Value) -> Option<Vec<u16>> {
    let Value::String(path) = value else {
        return None;
    };
    if path.units().contains(&0) {
        return None;
    }
    let mut units = path.units().to_vec();
    units.push(0);
    Some(units)
}

fn finish_watcher(state: &Rc<RefCell<Watcher>>) {
    let (handle, referenced) = {
        let mut watcher = state.borrow_mut();
        (watcher.handle.take(), watcher.referenced.clone())
    };
    referenced.set(false);
    if let Some(handle) = handle {
        crate::fs_win::watch_close(handle as crate::fs_win::Handle);
    }
}

fn call_change(
    realm: &mut Realm,
    object: &Object,
    status: i32,
    event_type: Value,
    filename: Value,
) -> Completion {
    let callback = ops::get(
        realm,
        &Value::Object(object.clone()),
        &Key::name("onchange"),
    )?;
    ops::call(
        realm,
        &callback,
        Value::Undefined,
        &[Value::Number(status as f64), event_type, filename],
    )
}

fn queue_read(
    realm: &mut Realm,
    object: Object,
    state: Rc<RefCell<Watcher>>,
) -> Result<(), Thrown> {
    let (handle, recursive, cancelled) = {
        let mut watcher = state.borrow_mut();
        if watcher.closed || watcher.pending {
            return Ok(());
        }
        let Some(handle) = watcher.handle else {
            return Ok(());
        };
        watcher.pending = true;
        (handle, watcher.recursive, watcher.cancelled.clone())
    };
    let worker_state = state.clone();
    let referenced = state.borrow().referenced.clone();
    // Begun here, so changes made once the watch exists are recorded even
    // before the worker waits for them.
    // SAFETY: the watcher owns the open handle until the worker's wait
    // returns; close waits for worker completion before releasing it.
    let pending = unsafe {
        crate::fs_win::watch_begin(handle as crate::fs_win::Handle, recursive, &cancelled)
    };
    let submitted = realm.submit_io_with_ref(
        move || {
            // SAFETY: as above; the read was begun on this handle.
            unsafe {
                crate::fs_win::watch_complete(handle as crate::fs_win::Handle, pending, &cancelled)
            }
        },
        referenced,
        move |realm, result| {
            worker_state.borrow_mut().pending = false;
            match result {
                Ok(Some(changes)) => {
                    let filename = worker_state.borrow().filename.clone();
                    for (action, name) in changes {
                        if worker_state.borrow().closed {
                            break;
                        }
                        if filename.as_deref().is_some_and(|filename| {
                            !crate::fs_win::watch_name_matches(filename, &name)
                        }) {
                            continue;
                        }
                        let event_type = if action == 3 {
                            Value::str("change")
                        } else {
                            Value::str("rename")
                        };
                        if let Err(error) = call_change(
                            realm,
                            &object,
                            0,
                            event_type,
                            Value::String(crate::JsStr::from_units(&name)),
                        ) {
                            worker_state.borrow_mut().closed = true;
                            worker_state
                                .borrow()
                                .cancelled
                                .store(true, Ordering::Release);
                            finish_watcher(&worker_state);
                            return Err(error);
                        }
                    }
                }
                Ok(None) => {
                    worker_state.borrow_mut().closed = true;
                }
                Err(status) => {
                    let path = worker_state.borrow().path.clone();
                    if let Err(error) = call_change(
                        realm,
                        &object,
                        status,
                        Value::Undefined,
                        Value::String(crate::JsStr::from_units(
                            &path[..path.len().saturating_sub(1)],
                        )),
                    ) {
                        worker_state.borrow_mut().closed = true;
                        worker_state
                            .borrow()
                            .cancelled
                            .store(true, Ordering::Release);
                        finish_watcher(&worker_state);
                        return Err(error);
                    }
                    worker_state.borrow_mut().closed = true;
                }
            }
            if worker_state.borrow().closed {
                finish_watcher(&worker_state);
                return Ok(());
            }
            queue_read(realm, object, worker_state)
        },
    );
    if submitted.is_err() {
        {
            let mut watcher = state.borrow_mut();
            watcher.pending = false;
            watcher.closed = true;
            watcher.cancelled.store(true, Ordering::Release);
        }
        finish_watcher(&state);
        return Err(realm.throw(
            crate::ErrorKind::Error,
            "Could not submit native filesystem watcher request",
        ));
    }
    Ok(())
}

fn start(
    realm: &mut Realm,
    this: Value,
    args: &[Value],
    state: &Rc<RefCell<Watcher>>,
) -> Completion {
    let Some(object) = this.as_object().cloned() else {
        return Ok(Value::Number(crate::fs_win::UV_EINVAL as f64));
    };
    let Some(path) = path_units(&arg(args, 0)) else {
        return Ok(Value::Number(crate::fs_win::UV_EINVAL as f64));
    };
    if state.borrow().handle.is_some() || state.borrow().closed {
        return Ok(Value::Number(0.0));
    }
    let opened = match crate::fs_win::watch_open(&path) {
        Ok(opened) => opened,
        Err(error) => return Ok(Value::Number(error as f64)),
    };
    {
        let mut watcher = state.borrow_mut();
        watcher.handle = Some(opened.handle as usize);
        watcher.recursive = matches!(args.get(2), Some(Value::Boolean(true)));
        watcher.path = path;
        watcher.filename = opened.filename;
        watcher
            .referenced
            .set(!matches!(args.get(1), Some(Value::Boolean(false))));
        watcher.cancelled.store(false, Ordering::Release);
    }
    queue_read(realm, object.clone(), state.clone())?;
    Ok(Value::Number(0.0))
}

/// `internalBinding('fs_event_wrap')`: native Windows directory notifications.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let constructor = realm.function("FSEvent", 0, true, |realm, this, _| {
        let object = this.as_object().cloned().unwrap_or_else(|| realm.object());
        let state = Rc::new(RefCell::new(Watcher {
            handle: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            recursive: false,
            pending: false,
            closed: false,
            async_id: NEXT_ASYNC_ID.fetch_add(1, Ordering::Relaxed),
            referenced: Rc::new(Cell::new(false)),
            path: Vec::new(),
            filename: None,
        }));
        let initialized_state = state.clone();
        let initialized = realm.function("get", 0, false, move |_, _, _| {
            let watcher = initialized_state.borrow();
            Ok(Value::Boolean(watcher.handle.is_some() && !watcher.closed))
        });
        object.define(
            Key::name("initialized"),
            Property {
                slot: Slot::Accessor {
                    get: Some(initialized),
                    set: None,
                },
                enumerable: true,
                configurable: false,
            },
        );

        let start_state = state.clone();
        let start_fn = realm.function("start", 4, false, move |realm, this, args| {
            start(realm, this, args, &start_state)
        });
        object.define(
            Key::name("start"),
            Property::hidden(Value::Object(start_fn)),
        );

        let close_state = state.clone();
        let close_fn = realm.function("close", 0, false, move |_, _, _| {
            close_state.borrow_mut().closed = true;
            let (handle, pending, cancelled, referenced) = {
                let watcher = close_state.borrow();
                (
                    watcher.handle,
                    watcher.pending,
                    watcher.cancelled.clone(),
                    watcher.referenced.clone(),
                )
            };
            // Cancellation completes asynchronously; keep the loop alive until
            // the worker returns the native handle for safe release.
            referenced.set(pending);
            cancelled.store(true, Ordering::Release);
            if let Some(handle) = handle {
                // SAFETY: watcher state retains the open handle until the
                // pending worker completes and releases it.
                unsafe { crate::fs_win::watch_cancel(handle as crate::fs_win::Handle) };
                if !pending {
                    finish_watcher(&close_state);
                }
            }
            Ok(Value::Undefined)
        });
        object.define(
            Key::name("close"),
            Property::hidden(Value::Object(close_fn)),
        );

        for (name, value) in [("ref", true), ("unref", false)] {
            let reference_state = state.clone();
            let reference = realm.function(name, 0, false, move |_, this, _| {
                let watcher = reference_state.borrow();
                if watcher.handle.is_some() && !watcher.closed {
                    watcher.referenced.set(value);
                }
                Ok(this)
            });
            object.define(Key::name(name), Property::hidden(Value::Object(reference)));
        }

        let async_id = state.borrow().async_id;
        let get_async_id = realm.function("getAsyncId", 0, false, move |_, _, _| {
            Ok(Value::Number(async_id as f64))
        });
        object.define(
            Key::name("getAsyncId"),
            Property::hidden(Value::Object(get_async_id)),
        );
        Ok(Value::Object(object))
    });
    exports.define(
        Key::name("FSEvent"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

#[cfg(test)]
mod tests;
