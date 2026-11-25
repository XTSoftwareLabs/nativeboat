//! In-process Electron main-side IPC registration and event dispatch.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

/// A registered listener; `once` listeners go after their first call.
#[derive(Debug, Clone)]
struct Listener {
    function: Value,
    once: bool,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    listeners: BTreeMap<String, Vec<Listener>>,
    /// Handlers by channel; `true` for `handleOnce` handlers.
    handlers: BTreeMap<String, (Value, bool)>,
}

impl State {
    /// The functions to call for an event on `name`, in order, after
    /// removing the `once` listeners among them, as Node's emitter does.
    fn take_listeners(&mut self, name: &str) -> Vec<Value> {
        let Some(listeners) = self.listeners.get_mut(name) else {
            return Vec::new();
        };
        let functions = listeners
            .iter()
            .map(|listener| listener.function.clone())
            .collect();
        listeners.retain(|listener| !listener.once);
        if listeners.is_empty() {
            self.listeners.remove(name);
        }
        functions
    }

    /// The handler for `name`, removing a `handleOnce` handler.
    pub(crate) fn take_handler(&mut self, name: &str) -> Option<Value> {
        let (handler, once) = self.handlers.get(name)?.clone();
        if once {
            self.handlers.remove(name);
        }
        Some(handler)
    }
}

pub(crate) type StateHandle = Rc<RefCell<State>>;

fn define(
    realm: &Realm,
    target: &Object,
    name: &str,
    length: u32,
    body: impl Fn(&mut Realm, Value, &[Value]) -> Result<Value, Thrown> + 'static,
) {
    let function = realm.function(name, length, false, body);
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

fn callable(realm: &Realm, value: Option<&Value>, what: &str) -> Result<Value, Thrown> {
    match value {
        Some(value) if crate::array_builtin::is_callable(value) => Ok(value.clone()),
        _ => Err(realm.throw(ErrorKind::Type, &format!("{what} must be a function"))),
    }
}

/// Creates the native `ipcMain` for one realm: an event emitter of the
/// channels pages send on, and the registry of `invoke` handlers.
pub fn module(realm: &Realm) -> Object {
    let ipc_main = realm.object();
    let state = Rc::new(RefCell::new(State::default()));
    // Keep the registry reachable to the generated preload/renderer adapters,
    // which dispatch into this realm while preserving listener ownership.
    // `Realm` is mutable in normal use, so install through its interior cache.
    realm.cache_electron_ipc(state.clone());

    // on, addListener, once, prependListener and prependOnceListener.
    for (name, once, prepend) in [
        ("on", false, false),
        ("addListener", false, false),
        ("once", true, false),
        ("prependListener", false, true),
        ("prependOnceListener", true, true),
    ] {
        let state = state.clone();
        define(realm, &ipc_main, name, 2, move |realm, this, args| {
            let name = channel(realm, args)?;
            let function = callable(realm, args.get(1), "listener")?;
            let mut state = state.borrow_mut();
            let listeners = state.listeners.entry(name).or_default();
            let listener = Listener { function, once };
            if prepend {
                listeners.insert(0, listener);
            } else {
                listeners.push(listener);
            }
            Ok(this)
        });
    }
    for name in ["off", "removeListener"] {
        let state = state.clone();
        define(realm, &ipc_main, name, 2, move |realm, this, args| {
            let name = channel(realm, args)?;
            let function = callable(realm, args.get(1), "listener")?;
            let mut state = state.borrow_mut();
            if let Some(listeners) = state.listeners.get_mut(&name) {
                // Node removes the most recently added match.
                if let Some(index) = listeners
                    .iter()
                    .rposition(|listener| crate::same_value(&listener.function, &function))
                {
                    listeners.remove(index);
                }
                if listeners.is_empty() {
                    state.listeners.remove(&name);
                }
            }
            Ok(this)
        });
    }
    let all = state.clone();
    define(
        realm,
        &ipc_main,
        "removeAllListeners",
        1,
        move |realm, this, args| {
            match args.first() {
                None | Some(Value::Undefined) => all.borrow_mut().listeners.clear(),
                Some(_) => {
                    let name = channel(realm, args)?;
                    all.borrow_mut().listeners.remove(&name);
                }
            }
            Ok(this)
        },
    );
    let count = state.clone();
    define(
        realm,
        &ipc_main,
        "listenerCount",
        1,
        move |realm, _, args| {
            let name = channel(realm, args)?;
            let total = count.borrow().listeners.get(&name).map_or(0, Vec::len);
            Ok(Value::Number(total as f64))
        },
    );
    for name in ["listeners", "rawListeners"] {
        let state = state.clone();
        define(realm, &ipc_main, name, 1, move |realm, _, args| {
            let name = channel(realm, args)?;
            let functions = state
                .borrow()
                .listeners
                .get(&name)
                .map(|listeners| {
                    listeners
                        .iter()
                        .map(|listener| listener.function.clone())
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Object(realm.array(functions)))
        });
    }
    let names = state.clone();
    define(realm, &ipc_main, "eventNames", 0, move |realm, _, _| {
        let names = names
            .borrow()
            .listeners
            .keys()
            .map(|name| Value::str(name))
            .collect();
        Ok(Value::Object(realm.array(names)))
    });
    define(realm, &ipc_main, "setMaxListeners", 1, |_, this, _| {
        Ok(this)
    });
    define(realm, &ipc_main, "getMaxListeners", 0, |_, _, _| {
        Ok(Value::Number(10.0))
    });
    let emit = state.clone();
    define(realm, &ipc_main, "emit", 1, move |realm, this, args| {
        let name = channel(realm, args)?;
        let listeners = emit.borrow_mut().take_listeners(&name);
        let event_args = args.get(1..).unwrap_or_default();
        for listener in &listeners {
            ops::call(realm, listener, this.clone(), event_args)?;
        }
        Ok(Value::Boolean(!listeners.is_empty()))
    });

    for (name, once) in [("handle", false), ("handleOnce", true)] {
        let state = state.clone();
        define(realm, &ipc_main, name, 2, move |realm, _, args| {
            let name = channel(realm, args)?;
            let handler = callable(realm, args.get(1), "handler")?;
            crate::platform::trace_ipc(|| {
                alloc::format!("handle {name} registry {:p}", Rc::as_ptr(&state))
            });
            let mut state = state.borrow_mut();
            if state.handlers.contains_key(&name) {
                return Err(realm.throw(
                    ErrorKind::Error,
                    &format!("Attempted to register a second handler for '{name}'"),
                ));
            }
            state.handlers.insert(name, (handler, once));
            Ok(Value::Undefined)
        });
    }
    let remove = state.clone();
    define(
        realm,
        &ipc_main,
        "removeHandler",
        1,
        move |realm, _, args| {
            let name = channel(realm, args)?;
            remove.borrow_mut().handlers.remove(&name);
            Ok(Value::Undefined)
        },
    );

    ipc_main
}

/// Dispatches a preload's synchronous IPC to the main realm and clones its
/// `event.returnValue` back across the execution-context boundary.
pub(crate) fn send_sync(
    main: &mut Realm,
    preload: &mut Realm,
    sender_id: u32,
    channel: &str,
    args: &[Value],
) -> Result<Value, String> {
    let listeners = main
        .electron_ipc_registry()
        .ok_or_else(|| "the main IPC registry is not initialized".to_owned())?
        .borrow_mut()
        .take_listeners(channel);
    crate::platform::trace_ipc(|| {
        alloc::format!(
            "receive {sender_id} {channel} ({} listeners)",
            listeners.len()
        )
    });
    let mut event_args = Vec::with_capacity(args.len() + 1);
    event_args.push(Value::Object(ipc_event(main, sender_id)));
    for argument in args {
        event_args.push(
            crate::structured_clone_builtin::clone_between(preload, main, argument)
                .map_err(|_| "IPC arguments could not be cloned".to_owned())?,
        );
    }
    for listener in &listeners {
        ops::call(main, listener, Value::Undefined, &event_args)
            .map_err(|_| format!("IPC listener for '{channel}' failed"))?;
    }
    let result = ops::get(
        main,
        &Value::Object(event_args[0].as_object().unwrap().clone()),
        &Key::name("returnValue"),
    )
    .map_err(|_| "cannot read synchronous IPC return value".to_owned())?;
    crate::structured_clone_builtin::clone_between(main, preload, &result)
        .map_err(|_| "IPC return value could not be cloned".to_owned())
}

/// Dispatches a preload's `ipcRenderer.postMessage` to the main realm's
/// listeners, with the message ports it moved as the event's `ports`.
pub(crate) fn post_message(
    main: &mut Realm,
    preload: &mut Realm,
    sender_id: u32,
    channel: &str,
    message: &Value,
    moved: &[u64],
) -> Result<(), String> {
    let listeners = main
        .electron_ipc_registry()
        .ok_or_else(|| "the main IPC registry is not initialized".to_owned())?
        .borrow_mut()
        .take_listeners(channel);
    crate::platform::trace_ipc(|| {
        alloc::format!(
            "postMessage {sender_id} {channel} ({} listeners, {} ports)",
            listeners.len(),
            moved.len()
        )
    });
    let event = ipc_event(main, sender_id);
    let ports = crate::realm_ports::adopt(main, moved);
    event.define(
        Key::name("ports"),
        Property::data(Value::Object(main.array(ports))),
    );
    let message = crate::structured_clone_builtin::clone_between(preload, main, message)
        .map_err(|_| "IPC arguments could not be cloned".to_owned())?;
    let arguments = [Value::Object(event), message];
    for listener in &listeners {
        ops::call(main, listener, Value::Undefined, &arguments)
            .map_err(|_| format!("IPC listener for '{channel}' failed"))?;
    }
    Ok(())
}

/// Dispatches an `ipcRenderer.invoke` request through a registered main
/// handler and settles the preload's promise with its result. A promise
/// result settles it when the main realm's promise settles.
pub(crate) fn invoke(
    main: &mut Realm,
    preload: &mut Realm,
    sender_id: u32,
    channel: &str,
    args: &[Value],
    settle: (Value, Value),
) -> Result<(), String> {
    let registry = main
        .electron_ipc_registry()
        .ok_or_else(|| "the main IPC registry is not initialized".to_owned())?;
    crate::platform::trace_ipc(|| {
        alloc::format!(
            "invoke {sender_id} {channel} registry {:p} ({} handlers)",
            Rc::as_ptr(&registry),
            registry.borrow().handlers.len()
        )
    });
    let handler = registry
        .borrow_mut()
        .take_handler(channel)
        .ok_or_else(|| format!("No handler registered for '{channel}'"))?;
    let mut handler_args = Vec::with_capacity(args.len() + 1);
    handler_args.push(Value::Object(ipc_event(main, sender_id)));
    for argument in args {
        handler_args.push(
            crate::structured_clone_builtin::clone_between(preload, main, argument)
                .map_err(|_| "IPC arguments could not be cloned".to_owned())?,
        );
    }
    let result = ops::call(main, &handler, Value::Undefined, &handler_args)
        .map_err(|_| format!("IPC handler for '{channel}' failed"))?;
    let is_promise = result
        .as_object()
        .is_some_and(|object| matches!(object.data().kind, crate::Kind::Promise(_)));
    if !is_promise {
        let value = crate::structured_clone_builtin::clone_between(main, preload, &result)
            .map_err(|_| "IPC result could not be cloned".to_owned())?;
        return ops::call(preload, &settle.0, Value::Undefined, &[value])
            .map(drop)
            .map_err(|_| "IPC result could not be delivered".to_owned());
    }
    // The preload realm lives as long as the host, at a stable address.
    let target = preload as *mut Realm as usize;
    let channel = String::from(channel);
    let (resolve, reject) = settle;
    let reject_clone = reject.clone();
    let fulfilled = main.function("", 1, false, move |main, _, args| {
        let reject = &reject_clone;
        // SAFETY: the generated host keeps preload realms boxed and alive
        // for the message loop, and runs all realms on one thread.
        let preload = unsafe { &mut *(target as *mut Realm) };
        let value = args.first().cloned().unwrap_or_default();
        match crate::structured_clone_builtin::clone_between(main, preload, &value) {
            Ok(value) => ops::call(preload, &resolve, Value::Undefined, &[value]).map(drop),
            Err(_) => {
                let error = preload.throw(ErrorKind::Error, "IPC result could not be cloned");
                ops::call(preload, reject, Value::Undefined, &[error.0]).map(drop)
            }
        }
        .map(|()| Value::Undefined)
    });
    let rejected = {
        let reject = reject.clone();
        main.function("", 1, false, move |main, _, args| {
            // SAFETY: as above.
            let preload = unsafe { &mut *(target as *mut Realm) };
            let reason = args.first().cloned().unwrap_or_default();
            let detail = ops::to_string(main, &reason)
                .map(|text| text.to_rust_lossy())
                .unwrap_or_default();
            let error = preload.throw(
                ErrorKind::Error,
                &format!("Error invoking remote method '{channel}': {detail}"),
            );
            ops::call(preload, &reject, Value::Undefined, &[error.0]).map(|_| Value::Undefined)
        })
    };
    let then = ops::get(main, &result, &Key::name("then"))
        .map_err(|_| "IPC result is not a usable promise".to_owned())?;
    ops::call(
        main,
        &then,
        result,
        &[Value::Object(fulfilled), Value::Object(rejected)],
    )
    .map(drop)
    .map_err(|_| "IPC result is not a usable promise".to_owned())
}

fn ipc_event(realm: &Realm, sender_id: u32) -> Object {
    let event = realm.object();
    // The sender is the window's `webContents`, so replies reach its page.
    let sender = realm
        .electron_web_contents(sender_id as usize)
        .unwrap_or_else(|| {
            let sender = realm.object();
            sender.define(
                Key::name("id"),
                Property::data(Value::Number(f64::from(sender_id))),
            );
            sender
        });
    event.define(Key::name("sender"), Property::data(Value::Object(sender)));
    let reply = realm.function("reply", 1, false, move |realm, _, args| {
        let name = channel(realm, args)?;
        realm.electron_outgoing.push((
            sender_id as usize,
            name,
            args.get(1..).unwrap_or_default().to_vec(),
            Vec::new(),
        ));
        Ok(Value::Undefined)
    });
    event.define(Key::name("reply"), Property::data(Value::Object(reply)));
    event.define(Key::name("returnValue"), Property::data(Value::Undefined));
    event.define(
        Key::name("ports"),
        Property::data(Value::Object(realm.array(Vec::new()))),
    );
    event
}

fn channel(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    let Some(value) = args.first() else {
        return Err(realm.throw(ErrorKind::Type, "channel name is required"));
    };
    Ok(ops::to_string(realm, value)?.to_rust_lossy())
}

#[cfg(test)]
#[path = "electron_ipc/tests.rs"]
mod tests;
