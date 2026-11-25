//! Message ports whose two ends live in different realms of this process:
//! the main process's, a window's preload's and its page's. Electron moves a
//! `MessagePortMain` to a window with `webContents.postMessage`, a preload
//! moves the port it receives on to its page with `window.postMessage`, and
//! messages then pass between the realms as structured clones.
//!
//! Every realm of a process shares one [`Hub`] of channel ends. An end lives
//! in one realm at a time, where a native handle stands for it. A realm's own
//! port class wraps the handle: the main process's `MessagePortMain` and the
//! page's `MessagePort`. A realm without one, such as a preload, uses the
//! handle itself as its port, with `postMessage`, `start`, `close`,
//! `onmessage` and `message` and `close` listeners.
//!
//! A message is cloned into the receiving realm when it is posted, or when
//! it arrives if the receiving end was moving, and is delivered there as a
//! task. Ports moved with a message move to the realm that receives it.

use crate::prelude::*;
use crate::{Completion, ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

/// The channel ends of a process's realms. The two ends of a channel are
/// `2n` and `2n + 1`.
#[derive(Default)]
pub(crate) struct Hub {
    next: u64,
    ends: BTreeMap<u64, End>,
    /// The page realm of each window, by window, for preloads'
    /// `window.postMessage`.
    pages: Vec<(usize, usize)>,
    /// Messages preloads posted to their pages: the window, the realm the
    /// value is in, the value and the ends moved with it.
    page_messages: VecDeque<(usize, usize, Value, Vec<u64>)>,
    /// Realms with a delivery task queued.
    flushing: Vec<usize>,
}

impl core::fmt::Debug for Hub {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Hub")
            .field("ends", &self.ends.len())
            .field("pages", &self.pages)
            .finish_non_exhaustive()
    }
}

pub(crate) type HubHandle = Rc<RefCell<Hub>>;

/// A message waiting for its end's realm: the realm its value is in, the
/// value and the ends moved with it.
type Message = (usize, Value, Vec<u64>);

#[derive(Default)]
struct End {
    /// The realm the end lives in; 0 while a message moves it.
    realm: usize,
    /// The handle standing for the end in its realm.
    handle: Option<Object>,
    /// What gets the end's messages, when its realm's port class wraps the
    /// handle: an object with `_deliver(data, ports)` and `_peerClosed()`.
    receiver: Option<Object>,
    /// Messages not delivered yet.
    inbox: VecDeque<Message>,
    /// Delivers the inbox: a bound receiver, or a handle used as a port that
    /// was started.
    started: bool,
    /// The other end closed; reported once the inbox is delivered.
    peer_closed: bool,
    /// This end closed, or its close was reported: it neither sends nor
    /// receives.
    closed: bool,
    /// A handle used as a port: its `onmessage` and its listeners.
    onmessage: Option<Object>,
    listeners: Vec<(&'static str, Object)>,
}

fn address(realm: &Realm) -> usize {
    realm as *const Realm as usize
}

/// Runs `work` with the realm at `target`, which may be `current`.
///
/// The host keeps every realm of a process boxed at a stable address until
/// it exits, and runs them all on one thread.
fn with_realm<T>(current: &mut Realm, target: usize, work: impl FnOnce(&mut Realm) -> T) -> T {
    if target == address(current) {
        work(current)
    } else {
        // SAFETY: as above; `target` is a different realm than `current`,
        // and no other reference to it is live while this realm runs.
        work(unsafe { &mut *(target as *mut Realm) })
    }
}

/// Clones a value from the realm at `source` into `target`.
fn clone_into(target: &mut Realm, source: usize, value: &Value) -> Result<Value, Thrown> {
    if source == address(target) {
        return crate::structured_clone_builtin::clone_within(target, value);
    }
    // SAFETY: as for `with_realm`.
    let source = unsafe { &mut *(source as *mut Realm) };
    crate::structured_clone_builtin::clone_between(source, target, value)
}

/// Opens a channel whose two ends live in `realm`, and returns their
/// handles.
pub(crate) fn channel(realm: &mut Realm) -> (Object, Object) {
    let here = address(realm);
    let first = {
        let mut hub = realm.message_ports.borrow_mut();
        let first = hub.next * 2;
        hub.next += 1;
        for id in [first, first + 1] {
            hub.ends.insert(
                id,
                End {
                    realm: here,
                    ..End::default()
                },
            );
        }
        first
    };
    (handle(realm, first), handle(realm, first + 1))
}

/// The end a handle of `realm` stands for, if it is a live one.
pub(crate) fn end_of(realm: &Realm, value: &Value) -> Option<u64> {
    let Value::Object(object) = value else {
        return None;
    };
    let here = address(realm);
    let hub = realm.message_ports.borrow();
    hub.ends
        .iter()
        .find(|(_, end)| end.realm == here && !end.closed && end.handle.as_ref() == Some(object))
        .map(|(id, _)| *id)
}

/// Takes ends out of `realm` to move them with a message: their handles
/// there no longer send or receive.
pub(crate) fn move_out(realm: &mut Realm, ends: &[u64]) {
    let mut hub = realm.message_ports.borrow_mut();
    for id in ends {
        if let Some(end) = hub.ends.get_mut(id) {
            end.realm = 0;
            end.handle = None;
            end.receiver = None;
            end.started = false;
            end.onmessage = None;
            end.listeners.clear();
        }
    }
}

/// The ends of a transfer list of handles of `realm`: an array, or an
/// options object with a `transfer` array, as `postMessage` takes.
pub(crate) fn transfer_list(
    realm: &mut Realm,
    transfer: &Value,
    sender: Option<u64>,
) -> Result<Vec<u64>, Thrown> {
    let list = match transfer {
        Value::Undefined | Value::Null => return Ok(Vec::new()),
        Value::Object(_) if !crate::array_builtin::is_array(realm, transfer)? => {
            let list = ops::get_named(realm, transfer, "transfer")?;
            if matches!(list, Value::Undefined | Value::Null) {
                return Ok(Vec::new());
            }
            list
        }
        _ => transfer.clone(),
    };
    let length = ops::get_named(realm, &list, "length")?;
    let length = ops::to_number(realm, &length)? as u32;
    let mut ends = Vec::new();
    for index in 0..length {
        let item = ops::get(realm, &list, &Key::Index(index))?;
        let Some(id) = end_of(realm, &item) else {
            return Err(data_clone_error(
                realm,
                "Value at index {index} does not have a transferable type.",
                index,
            ));
        };
        if Some(id) == sender || ends.contains(&id) {
            return Err(data_clone_error(
                realm,
                "Message port at index {index} is a duplicate of an earlier port, or the port posting the message.",
                index,
            ));
        }
        ends.push(id);
    }
    Ok(ends)
}

fn data_clone_error(realm: &mut Realm, message: &str, index: u32) -> Thrown {
    let message = message.replace("{index}", &index.to_string());
    let error = realm.error(ErrorKind::Error, &message);
    error.define(
        Key::name("name"),
        Property::hidden(Value::str("DataCloneError")),
    );
    crate::ops::thrown(Value::Object(error))
}

/// Posts a message from the end `id` of `realm` to the other end, with the
/// ends `moved`.
pub(crate) fn post(
    realm: &mut Realm,
    id: u64,
    value: &Value,
    moved: Vec<u64>,
) -> Result<(), Thrown> {
    let here = address(realm);
    let peer = id ^ 1;
    let target = {
        let hub = realm.message_ports.borrow();
        match (hub.ends.get(&id), hub.ends.get(&peer)) {
            (Some(end), Some(other)) if end.realm == here && !end.closed && !other.closed => {
                Some(other.realm)
            }
            _ => None,
        }
    };
    let Some(target) = target else {
        // A message to a closed port goes nowhere; the ports it carried
        // close.
        for end in moved {
            close(realm, end);
        }
        return Ok(());
    };
    // The value as it is now, in the realm that gets it, or in this one
    // while the other end moves.
    let (value_realm, value) = if target == 0 || target == here {
        (
            here,
            crate::structured_clone_builtin::clone_within(realm, value)?,
        )
    } else {
        // SAFETY: as for `with_realm`.
        let target_realm = unsafe { &mut *(target as *mut Realm) };
        let cloned = crate::structured_clone_builtin::clone_between(realm, target_realm, value);
        (target, cloned?)
    };
    move_out(realm, &moved);
    realm
        .message_ports
        .borrow_mut()
        .ends
        .get_mut(&peer)
        .expect("checked above")
        .inbox
        .push_back((value_realm, value, moved));
    if target != 0 {
        schedule(realm, target);
    }
    Ok(())
}

/// Closes the end `id`; its peer learns of it once its messages are
/// delivered.
pub(crate) fn close(realm: &mut Realm, id: u64) {
    let target = {
        let mut hub = realm.message_ports.borrow_mut();
        let Some(end) = hub.ends.get_mut(&id) else {
            return;
        };
        if end.closed {
            return;
        }
        end.closed = true;
        end.inbox.clear();
        end.receiver = None;
        end.onmessage = None;
        end.listeners.clear();
        let gone = hub.ends.get(&(id ^ 1)).is_none_or(|peer| peer.closed);
        if gone {
            hub.ends.remove(&id);
            hub.ends.remove(&(id ^ 1));
            return;
        }
        let peer = hub.ends.get_mut(&(id ^ 1)).expect("checked above");
        peer.peer_closed = true;
        peer.realm
    };
    if target != 0 {
        schedule(realm, target);
    }
}

/// Makes ends that moved with a message live in `realm`, and returns its
/// ports for them.
pub(crate) fn adopt(realm: &mut Realm, ends: &[u64]) -> Vec<Value> {
    let here = address(realm);
    let mut ports = Vec::with_capacity(ends.len());
    for id in ends {
        let pending = {
            let mut hub = realm.message_ports.borrow_mut();
            let Some(end) = hub.ends.get_mut(id) else {
                continue;
            };
            end.realm = here;
            !end.inbox.is_empty() || end.peer_closed
        };
        let handle = handle(realm, *id);
        ports.push(wrap(realm, handle));
        if pending {
            schedule(realm, here);
        }
    }
    ports
}

/// Makes the end `id`, which moved out of another realm, live in `realm`
/// as a handle, for a worker's prelude to wrap.
#[cfg(feature = "dom")]
pub(crate) fn adopt_handle(realm: &mut Realm, id: u64) -> Object {
    let here = address(realm);
    let pending = {
        let mut hub = realm.message_ports.borrow_mut();
        hub.ends.get_mut(&id).is_some_and(|end| {
            end.realm = here;
            !end.inbox.is_empty() || end.peer_closed
        })
    };
    let handle = handle(realm, id);
    if pending {
        schedule(realm, here);
    }
    handle
}

/// Closes the ends living in `realm` before it goes away, so that no
/// message is delivered to it after.
#[cfg(feature = "dom")]
pub(crate) fn release(realm: &mut Realm) {
    let here = address(realm);
    let ends: Vec<u64> = realm
        .message_ports
        .borrow()
        .ends
        .iter()
        .filter(|(_, end)| end.realm == here)
        .map(|(id, _)| *id)
        .collect();
    for id in ends {
        close(realm, id);
    }
    let mut hub = realm.message_ports.borrow_mut();
    for end in hub.ends.values_mut() {
        if end.realm == here {
            end.realm = 0;
            end.handle = None;
        }
    }
    hub.flushing.retain(|known| *known != here);
}

/// A realm's port for a handle: its port class's, or the handle itself.
fn wrap(realm: &mut Realm, handle: Object) -> Value {
    let argument = [Value::Object(handle.clone())];
    #[cfg(feature = "dom")]
    if realm.web_host.is_some()
        && let Ok(port @ Value::Object(_)) = realm.web_call("port", &argument)
    {
        return port;
    }
    // The main process's ports are `MessagePortMain`s.
    if realm.electron_ipc_main == 0
        && realm.node_library.is_some()
        && let Ok(module @ Value::Object(_)) =
            crate::modules::load(realm, "node:internal/electron/utility_process")
        && let Ok(adopt @ Value::Object(_)) = ops::get_named(realm, &module, "_adoptNativePort")
        && let Ok(port @ Value::Object(_)) = ops::call(realm, &adopt, Value::Undefined, &argument)
    {
        return port;
    }
    Value::Object(handle)
}

/// Registers `realm` as the page of its window, for its preload's
/// `window.postMessage`.
pub(crate) fn register_page(realm: &mut Realm) {
    let here = address(realm);
    let window = realm.electron_window;
    let waiting = {
        let mut hub = realm.message_ports.borrow_mut();
        hub.pages.retain(|(known, _)| *known != window);
        hub.pages.push((window, here));
        hub.page_messages.iter().any(|(known, ..)| *known == window)
    };
    if waiting {
        schedule(realm, here);
    }
}

/// A preload's `window.postMessage(message, targetOrigin, transfer)`: the
/// page's window gets a `message` event with the ports moved to it.
pub(crate) fn post_to_page(
    realm: &mut Realm,
    value: &Value,
    transfer: &Value,
) -> Result<(), Thrown> {
    let moved = transfer_list(realm, transfer, None)?;
    let value = crate::structured_clone_builtin::clone_within(realm, value)?;
    move_out(realm, &moved);
    let here = address(realm);
    let window = realm.electron_window;
    let page = {
        let mut hub = realm.message_ports.borrow_mut();
        hub.page_messages.push_back((window, here, value, moved));
        hub.pages
            .iter()
            .find(|(known, _)| *known == window)
            .map(|(_, page)| *page)
    };
    if let Some(page) = page {
        schedule(realm, page);
    }
    Ok(())
}

/// Queues a task in the realm at `target` that delivers its ends' messages.
fn schedule(current: &mut Realm, target: usize) {
    {
        let mut hub = current.message_ports.borrow_mut();
        if hub.flushing.contains(&target) {
            return;
        }
        hub.flushing.push(target);
    }
    with_realm(current, target, |realm| {
        let task = realm.function("", 0, false, |realm, _, _| {
            deliver(realm);
            Ok(Value::Undefined)
        });
        realm.native_timeout(task, 0.0, Rc::new(Cell::new(true)));
    });
}

/// What an end has for its realm next.
enum Next {
    Message(Message, Option<Object>, Option<Object>),
    Closed(Option<Object>, Option<Object>),
}

/// Delivers what waits for the ends of `realm`, and for its page.
fn deliver(realm: &mut Realm) {
    let here = address(realm);
    realm
        .message_ports
        .borrow_mut()
        .flushing
        .retain(|known| *known != here);
    loop {
        let next = {
            let mut hub = realm.message_ports.borrow_mut();
            let mut found = None;
            for end in hub.ends.values_mut() {
                if end.realm != here || end.closed || !end.started {
                    continue;
                }
                if let Some(message) = end.inbox.pop_front() {
                    found = Some(Next::Message(
                        message,
                        end.receiver.clone(),
                        end.handle.clone(),
                    ));
                    break;
                }
                if end.peer_closed {
                    end.closed = true;
                    found = Some(Next::Closed(end.receiver.take(), end.handle.clone()));
                    break;
                }
            }
            found
        };
        let Some(next) = next else {
            break;
        };
        let result = match next {
            Next::Message((source, value, moved), receiver, handle) => {
                // Values posted while this end moved are still the sender's.
                let value = if source == here {
                    Ok(value)
                } else {
                    clone_into(realm, source, &value)
                };
                match value {
                    Ok(value) => {
                        let ports = adopt(realm, &moved);
                        let ports = Value::Object(realm.array(ports));
                        dispatch(realm, receiver, handle, value, ports)
                    }
                    Err(_) => Ok(()),
                }
            }
            Next::Closed(receiver, handle) => closed(realm, receiver, handle),
        };
        if let Err(thrown) = result {
            report(realm, thrown);
        }
    }
    deliver_page_messages(realm);
}

/// Delivers a message to an end's receiver, or to the listeners of a handle
/// used as a port.
fn dispatch(
    realm: &mut Realm,
    receiver: Option<Object>,
    handle: Option<Object>,
    value: Value,
    ports: Value,
) -> Result<(), Thrown> {
    if let Some(receiver) = receiver {
        let receiver = Value::Object(receiver);
        let method = ops::get_named(realm, &receiver, "_deliver")?;
        return ops::call(realm, &method, receiver, &[value, ports]).map(drop);
    }
    let Some(handle) = handle else {
        return Ok(());
    };
    let event = realm.object();
    for (name, field) in [
        ("type", Value::str("message")),
        ("data", value),
        ("ports", ports),
        ("target", Value::Object(handle.clone())),
        ("currentTarget", Value::Object(handle.clone())),
    ] {
        event.define(Key::name(name), Property::data(field));
    }
    notify(realm, &handle, "message", Value::Object(event))
}

/// Reports a closed peer to an end's receiver, or to the `close` listeners
/// of a handle used as a port.
fn closed(
    realm: &mut Realm,
    receiver: Option<Object>,
    handle: Option<Object>,
) -> Result<(), Thrown> {
    if let Some(receiver) = receiver {
        let receiver = Value::Object(receiver);
        let method = ops::get_named(realm, &receiver, "_peerClosed")?;
        return ops::call(realm, &method, receiver, &[]).map(drop);
    }
    let Some(handle) = handle else {
        return Ok(());
    };
    let event = realm.object();
    event.define(Key::name("type"), Property::data(Value::str("close")));
    event.define(
        Key::name("target"),
        Property::data(Value::Object(handle.clone())),
    );
    notify(realm, &handle, "close", Value::Object(event))
}

/// Calls a handle's `onmessage` and its listeners of `kind`.
fn notify(realm: &mut Realm, handle: &Object, kind: &str, event: Value) -> Result<(), Thrown> {
    let (onmessage, listeners) = {
        let hub = realm.message_ports.borrow();
        let Some(end) = hub
            .ends
            .values()
            .find(|end| end.handle.as_ref() == Some(handle))
        else {
            return Ok(());
        };
        let onmessage = end.onmessage.clone().filter(|_| kind == "message");
        let listeners: Vec<Object> = end
            .listeners
            .iter()
            .filter(|(known, _)| *known == kind)
            .map(|(_, listener)| listener.clone())
            .collect();
        (onmessage, listeners)
    };
    let this = Value::Object(handle.clone());
    for listener in onmessage.into_iter().chain(listeners) {
        if let Err(thrown) = ops::call(
            realm,
            &Value::Object(listener),
            this.clone(),
            core::slice::from_ref(&event),
        ) {
            report(realm, thrown);
        }
    }
    Ok(())
}

/// Delivers the messages a preload posted to this realm's window, as
/// `message` events of the page's window.
fn deliver_page_messages(realm: &mut Realm) {
    #[cfg(feature = "dom")]
    {
        let here = address(realm);
        let window = realm.electron_window;
        loop {
            let message = {
                let mut hub = realm.message_ports.borrow_mut();
                if !hub.pages.contains(&(window, here)) {
                    return;
                }
                let Some(index) = hub
                    .page_messages
                    .iter()
                    .position(|(known, ..)| *known == window)
                else {
                    return;
                };
                hub.page_messages.remove(index)
            };
            let Some((_, source, value, moved)) = message else {
                return;
            };
            let Ok(value) = clone_into(realm, source, &value) else {
                continue;
            };
            let ports = adopt(realm, &moved);
            let ports = Value::Object(realm.array(ports));
            if let Err(thrown) = realm.web_call("message", &[value, ports]) {
                report(realm, thrown);
            }
        }
    }
    #[cfg(not(feature = "dom"))]
    let _ = realm;
}

fn report(realm: &mut Realm, thrown: Thrown) {
    #[cfg(feature = "dom")]
    if realm.web_host.is_some() {
        crate::web::uncaught(realm, thrown);
        return;
    }
    realm.host_errors.push(thrown);
}

/// The handle standing for the end `id` in `realm`.
fn handle(realm: &mut Realm, id: u64) -> Object {
    let handle = realm.object();
    realm
        .message_ports
        .borrow_mut()
        .ends
        .get_mut(&id)
        .expect("a handle is made for a known end")
        .handle = Some(handle.clone());
    let live = move |realm: &Realm, this: &Value| -> bool {
        let here = address(realm);
        let hub = realm.message_ports.borrow();
        hub.ends.get(&id).is_some_and(|end| {
            end.realm == here
                && !end.closed
                && matches!(this, Value::Object(object) if end.handle.as_ref() == Some(object))
        })
    };
    define_method(
        realm,
        &handle,
        "postMessage",
        1,
        move |realm, this, args| {
            if !live(realm, &this) {
                return Ok(Value::Undefined);
            }
            let value = args.first().cloned().unwrap_or_default();
            let transfer = args.get(1).cloned().unwrap_or_default();
            let moved = transfer_list(realm, &transfer, Some(id))?;
            post(realm, id, &value, moved)?;
            Ok(Value::Undefined)
        },
    );
    define_method(realm, &handle, "close", 0, move |realm, this, _| {
        if live(realm, &this) {
            close(realm, id);
        }
        Ok(Value::Undefined)
    });
    define_method(realm, &handle, "start", 0, move |realm, this, _| {
        if live(realm, &this) {
            start(realm, id, None);
        }
        Ok(Value::Undefined)
    });
    // A realm's port class wraps the handle and gets its messages.
    define_method(realm, &handle, "bind", 1, move |realm, this, args| {
        if !live(realm, &this) {
            return Ok(Value::Undefined);
        }
        let Some(Value::Object(receiver)) = args.first() else {
            return Err(realm.throw(ErrorKind::Type, "a port receiver is required"));
        };
        start(realm, id, Some(receiver.clone()));
        Ok(Value::Undefined)
    });
    for (name, add) in [("addEventListener", true), ("removeEventListener", false)] {
        define_method(realm, &handle, name, 2, move |realm, this, args| {
            if !live(realm, &this) {
                return Ok(Value::Undefined);
            }
            let kind =
                ops::to_string(realm, &args.first().cloned().unwrap_or_default())?.to_rust_lossy();
            let kind = match kind.as_str() {
                "message" => "message",
                "close" => "close",
                _ => return Ok(Value::Undefined),
            };
            let Some(Value::Object(listener)) = args.get(1) else {
                return Ok(Value::Undefined);
            };
            let mut hub = realm.message_ports.borrow_mut();
            let Some(end) = hub.ends.get_mut(&id) else {
                return Ok(Value::Undefined);
            };
            end.listeners
                .retain(|(known, existing)| !(*known == kind && existing == listener));
            if add {
                end.listeners.push((kind, listener.clone()));
            }
            Ok(Value::Undefined)
        });
    }
    // Setting `onmessage` starts the port, as it does a DOM port.
    let get = realm.function("get onmessage", 0, false, move |realm, _, _| {
        let hub = realm.message_ports.borrow();
        Ok(hub
            .ends
            .get(&id)
            .and_then(|end| end.onmessage.clone())
            .map_or(Value::Null, Value::Object))
    });
    let set = realm.function("set onmessage", 1, false, move |realm, this, args| {
        if !live(realm, &this) {
            return Ok(Value::Undefined);
        }
        let listener = match args.first() {
            Some(Value::Object(listener))
                if crate::array_builtin::is_callable(&Value::Object(listener.clone())) =>
            {
                Some(listener.clone())
            }
            _ => None,
        };
        let starts = listener.is_some();
        if let Some(end) = realm.message_ports.borrow_mut().ends.get_mut(&id) {
            end.onmessage = listener;
        }
        if starts {
            start(realm, id, None);
        }
        Ok(Value::Undefined)
    });
    ops::define_accessor(&handle, Key::name("onmessage"), Some(get), Some(set));
    handle
}

fn define_method(
    realm: &Realm,
    target: &Object,
    name: &str,
    length: u32,
    code: impl Fn(&mut Realm, Value, &[Value]) -> Completion + 'static,
) {
    let function = realm.function(name, length, false, code);
    target.define(Key::name(name), Property::hidden(Value::Object(function)));
}

/// Starts delivering an end's messages, to `receiver` when one is given.
fn start(realm: &mut Realm, id: u64, receiver: Option<Object>) {
    let pending = {
        let mut hub = realm.message_ports.borrow_mut();
        let Some(end) = hub.ends.get_mut(&id) else {
            return;
        };
        end.started = true;
        if receiver.is_some() {
            end.receiver = receiver;
        }
        !end.inbox.is_empty() || end.peer_closed
    };
    if pending {
        let here = address(realm);
        schedule(realm, here);
    }
}

/// The ends for a main realm's transfer list of `MessagePortMain`s, moving
/// with a message to a window. Each port's peer talks to a new channel's
/// first end from now on, and the second end moves.
pub(crate) fn export_ports(realm: &mut Realm, transfer: &Value) -> Result<Vec<u64>, Thrown> {
    if matches!(transfer, Value::Undefined | Value::Null) {
        return Ok(Vec::new());
    }
    if !crate::array_builtin::is_array(realm, transfer)? {
        return Err(realm.throw(
            ErrorKind::Type,
            "The transfer list must be an array of MessagePortMain objects",
        ));
    }
    let length = ops::get_named(realm, transfer, "length")?;
    let length = ops::to_number(realm, &length)? as u32;
    if length == 0 {
        return Ok(Vec::new());
    }
    let module = crate::modules::load(realm, "node:internal/electron/utility_process")?;
    let export = ops::get_named(realm, &module, "_exportPort")?;
    let mut ends = Vec::new();
    for index in 0..length {
        let port = ops::get(realm, transfer, &Key::Index(index))?;
        let far = ops::call(realm, &export, Value::Undefined, &[port])?;
        let Some(id) = end_of(realm, &far) else {
            return Err(realm.throw(ErrorKind::Type, "A MessagePortMain could not be moved"));
        };
        ends.push(id);
    }
    move_out(realm, &ends);
    Ok(ends)
}

/// `internalBinding('message_ports')`: `channel()` opens a channel whose
/// ends can move to other realms, as a pair of handles.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let channel = realm.function("channel", 0, false, |realm, _, _| {
        let (first, second) = channel(realm);
        Ok(Value::Object(
            realm.array(vec![Value::Object(first), Value::Object(second)]),
        ))
    });
    exports.define(Key::name("channel"), Property::data(Value::Object(channel)));
    exports
}

#[cfg(test)]
#[path = "realm_ports/tests.rs"]
mod tests;
