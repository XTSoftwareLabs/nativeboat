use super::*;
use alloc::boxed::Box;
use alloc::sync::Arc;
use std::sync::Mutex;

type Log = Arc<Mutex<Vec<String>>>;

/// Two realms of one process, sharing the main realm's ports.
fn realms() -> (Box<Realm>, Box<Realm>) {
    let mut main = Box::new(Realm::new());
    let mut other = Box::new(Realm::new());
    // SAFETY: both realms are boxed and outlive the test's calls.
    unsafe { other.connect_electron_ipc(&mut *main as *mut Realm) };
    (main, other)
}

fn get(realm: &mut Realm, value: &Value, name: &str) -> Value {
    match ops::get(realm, value, &Key::name(name)) {
        Ok(value) => value,
        Err(thrown) => {
            let message = ops::to_string(realm, &thrown.0).unwrap().to_rust_lossy();
            panic!("{name}: {message}")
        }
    }
}

fn call(realm: &mut Realm, target: &Value, name: &str, args: &[Value]) -> Completion {
    let method = get(realm, target, name);
    ops::call(realm, &method, target.clone(), args)
}

/// A listener that logs `label`, the event's `data` and its port count.
fn listener(realm: &Realm, log: &Log, label: &'static str) -> Value {
    let log = log.clone();
    Value::Object(realm.function("", 1, false, move |realm, _, args| {
        let event = args.first().cloned().unwrap_or_default();
        let data = get(realm, &event, "data");
        let data = match &data {
            Value::Object(_) => {
                let text = get(realm, &data, "text");
                ops::to_string(realm, &text)?.to_rust_lossy()
            }
            other => ops::to_string(realm, other)?.to_rust_lossy(),
        };
        let ports = get(realm, &event, "ports");
        let count = match ports {
            Value::Object(_) => length(realm, &ports)?,
            _ => 0.0,
        };
        log.lock().unwrap().push(format!("{label} {data} {count}"));
        Ok(Value::Undefined)
    }))
}

fn length(realm: &mut Realm, value: &Value) -> Result<f64, Thrown> {
    let length = get(realm, value, "length");
    ops::to_number(realm, &length)
}

fn entries(log: &Log) -> Vec<String> {
    core::mem::take(&mut *log.lock().unwrap())
}

/// Moves the end of `handle` from `from` to `to`, as a message would.
fn moved(from: &mut Realm, to: &mut Realm, handle: &Object) -> Value {
    let id = end_of(from, &Value::Object(handle.clone())).unwrap();
    move_out(from, &[id]);
    adopt(to, &[id]).remove(0)
}

#[test]
fn ports_post_clones_between_realms_once_started() {
    let (mut main, mut page) = realms();
    let (near, far) = channel(&mut main);
    let near = Value::Object(near);
    let port = moved(&mut main, &mut page, &far);
    // The handle that moved no longer sends.
    call(
        &mut main,
        &Value::Object(far),
        "postMessage",
        &[Value::str("lost")],
    )
    .unwrap();
    let original = main.object();
    original.define(Key::name("text"), Property::data(Value::str("first")));
    call(
        &mut main,
        &near,
        "postMessage",
        &[Value::Object(original.clone())],
    )
    .unwrap();
    original.define(Key::name("text"), Property::data(Value::str("changed")));
    // Nothing is delivered before the port starts.
    page.run_event_loop().unwrap();
    let log = Log::default();
    let on_message = listener(&page, &log, "page");
    ops::set(
        &mut page,
        &port,
        &Key::name("onmessage"),
        on_message.clone(),
    )
    .unwrap();
    assert!(entries(&log).is_empty());
    page.run_event_loop().unwrap();
    assert_eq!(entries(&log), ["page first 0"]);
    assert!(matches!(
        get(&mut page, &port, "onmessage"),
        Value::Object(_)
    ));
    // Replies reach the other realm's listeners after it starts.
    call(&mut page, &port, "postMessage", &[Value::str("reply")]).unwrap();
    let main_log = Log::default();
    let reply = listener(&main, &main_log, "main");
    call(
        &mut main,
        &near,
        "addEventListener",
        &[Value::str("message"), reply.clone()],
    )
    .unwrap();
    main.run_event_loop().unwrap();
    assert!(entries(&main_log).is_empty());
    call(&mut main, &near, "start", &[]).unwrap();
    main.run_event_loop().unwrap();
    assert_eq!(entries(&main_log), ["main reply 0"]);
    call(
        &mut main,
        &near,
        "removeEventListener",
        &[Value::str("message"), reply],
    )
    .unwrap();
    call(&mut page, &port, "postMessage", &[Value::str("unheard")]).unwrap();
    main.run_event_loop().unwrap();
    assert!(entries(&main_log).is_empty());
}

#[test]
fn ports_move_with_messages_and_report_closing() {
    let (mut main, mut page) = realms();
    let (near, far) = channel(&mut main);
    let near = Value::Object(near);
    let port = moved(&mut main, &mut page, &far);
    let log = Log::default();
    let on_message = listener(&page, &log, "page");
    ops::set(&mut page, &port, &Key::name("onmessage"), on_message).unwrap();
    // A second channel's end moves with a message to the page.
    let (kept, carried) = channel(&mut main);
    let kept = Value::Object(kept);
    let transfer = Value::Object(main.array(vec![Value::Object(carried.clone())]));
    call(
        &mut main,
        &near,
        "postMessage",
        &[Value::str("carry"), transfer],
    )
    .unwrap();
    assert!(end_of(&main, &Value::Object(carried)).is_none());
    page.run_event_loop().unwrap();
    assert_eq!(entries(&log), ["page carry 1"]);
    // Posting a port with itself, or a value that is no port, throws.
    let itself = Value::Object(page.array(vec![port.clone()]));
    let error = call(&mut page, &port, "postMessage", &[Value::Null, itself]).unwrap_err();
    let name = get(&mut page, &error.0, "name");
    assert_eq!(
        ops::to_string(&mut page, &name).unwrap().to_rust_lossy(),
        "DataCloneError"
    );
    let options = page.object();
    let list = Value::Object(page.array(vec![Value::Number(1.0)]));
    options.define(Key::name("transfer"), Property::data(list));
    assert!(
        call(
            &mut page,
            &port,
            "postMessage",
            &[Value::Null, Value::Object(options)]
        )
        .is_err()
    );
    // The page closes its port: the main realm hears once its messages
    // are delivered, and its port no longer sends.
    let main_log = Log::default();
    let closed = listener(&main, &main_log, "closed");
    call(
        &mut main,
        &near,
        "addEventListener",
        &[Value::str("close"), closed],
    )
    .unwrap();
    call(&mut main, &near, "start", &[]).unwrap();
    call(&mut page, &port, "postMessage", &[Value::str("last")]).unwrap();
    call(&mut page, &port, "close", &[]).unwrap();
    let messages = listener(&main, &main_log, "main");
    call(
        &mut main,
        &near,
        "addEventListener",
        &[Value::str("message"), messages],
    )
    .unwrap();
    main.run_event_loop().unwrap();
    assert_eq!(entries(&main_log), ["main last 0", "closed undefined 0"]);
    call(&mut main, &near, "postMessage", &[Value::str("gone")]).unwrap();
    page.run_event_loop().unwrap();
    assert!(entries(&log).is_empty());
    // A message to a port that closed closes the ports it carried; a closed
    // port posts nothing.
    let (sender, receiver) = channel(&mut main);
    let sender = Value::Object(sender);
    call(&mut main, &Value::Object(receiver), "close", &[]).unwrap();
    let (spare, _) = channel(&mut main);
    let transfer = Value::Object(main.array(vec![Value::Object(spare.clone())]));
    call(&mut main, &sender, "postMessage", &[Value::Null, transfer]).unwrap();
    assert!(end_of(&main, &Value::Object(spare)).is_none());
    let (unused, _) = channel(&mut main);
    let transfer = Value::Object(main.array(vec![Value::Object(unused.clone())]));
    call(&mut main, &kept, "close", &[]).unwrap();
    call(&mut main, &kept, "postMessage", &[Value::Null, transfer]).unwrap();
    assert!(end_of(&main, &Value::Object(unused)).is_some());
}

#[test]
fn receivers_get_the_messages_of_bound_handles() {
    let (mut main, mut page) = realms();
    let (near, far) = channel(&mut main);
    let near = Value::Object(near);
    let far = moved(&mut main, &mut page, &far);
    let log = Log::default();
    let receiver = page.object();
    let deliver = {
        let log = log.clone();
        page.function("_deliver", 2, false, move |realm, _, args| {
            let data = ops::to_string(realm, &args[0])?.to_rust_lossy();
            let count = length(realm, &args[1])?;
            log.lock().unwrap().push(format!("deliver {data} {count}"));
            Ok(Value::Undefined)
        })
    };
    let peer_closed = {
        let log = log.clone();
        page.function("_peerClosed", 0, false, move |_, _, _| {
            log.lock().unwrap().push("peer closed".into());
            Ok(Value::Undefined)
        })
    };
    receiver.define(
        Key::name("_deliver"),
        Property::data(Value::Object(deliver)),
    );
    receiver.define(
        Key::name("_peerClosed"),
        Property::data(Value::Object(peer_closed)),
    );
    call(&mut main, &near, "postMessage", &[Value::str("early")]).unwrap();
    call(&mut page, &far, "bind", &[Value::Object(receiver)]).unwrap();
    assert!(call(&mut page, &far, "bind", &[]).is_err());
    call(&mut main, &near, "close", &[]).unwrap();
    page.run_event_loop().unwrap();
    assert_eq!(entries(&log), ["deliver early 0", "peer closed"]);
}

#[test]
fn preloads_post_to_their_pages_window_with_ports() {
    let (mut main, mut preload) = realms();
    preload.set_electron_window(5);
    preload.install_preload_window();
    let window = ops::get_global(&mut preload, "window").unwrap();
    assert_eq!(window.as_object(), Some(&preload.global_object()));
    let (near, far) = channel(&mut main);
    let port = moved(&mut main, &mut preload, &far);
    let transfer = Value::Object(preload.array(vec![port.clone()]));
    // The page is not there yet: the message waits for it.
    call(
        &mut preload,
        &window,
        "postMessage",
        &[Value::str("nonce"), Value::str("*"), transfer],
    )
    .unwrap();
    assert!(end_of(&preload, &port).is_none());
    let mut page = Box::new(Realm::new());
    page.set_electron_window(5);
    // SAFETY: the realms are boxed and outlive the test's calls.
    unsafe {
        page.connect_electron_ipc(&mut *main as *mut Realm);
        page.run_page_scripts(&mut *preload as *mut Realm);
    }
    #[cfg(feature = "dom")]
    {
        // The page's prelude makes its ports and fires its window's event.
        let log = Log::default();
        let host = page.object();
        let web = page.object();
        let port_log = log.clone();
        let make_port = page.function("port", 1, false, move |_, _, args| {
            port_log.lock().unwrap().push("port".into());
            Ok(args[0].clone())
        });
        let message_log = log.clone();
        let message = page.function("message", 2, false, move |realm, _, args| {
            let data = ops::to_string(realm, &args[0])?.to_rust_lossy();
            let count = length(realm, &args[1])?;
            message_log
                .lock()
                .unwrap()
                .push(format!("message {data} {count}"));
            Ok(Value::Undefined)
        });
        web.define(Key::name("port"), Property::data(Value::Object(make_port)));
        web.define(Key::name("message"), Property::data(Value::Object(message)));
        host.define(Key::name("web"), Property::data(Value::Object(web)));
        page.web_host = Some(host);
        page.run_event_loop().unwrap();
        assert_eq!(entries(&log), ["port", "message nonce 1"]);
        // The page's port talks to the main realm's.
        let ports = page
            .message_ports
            .borrow()
            .ends
            .iter()
            .filter(|(_, end)| end.realm == address(&page))
            .count();
        assert_eq!(ports, 1);
        let main_log = Log::default();
        let near = Value::Object(near);
        let on_message = listener(&main, &main_log, "main");
        ops::set(&mut main, &near, &Key::name("onmessage"), on_message).unwrap();
        let handle = {
            let hub = page.message_ports.borrow();
            let end = hub
                .ends
                .values()
                .find(|end| end.realm == address(&page))
                .unwrap();
            Value::Object(end.handle.clone().unwrap())
        };
        call(&mut page, &handle, "postMessage", &[Value::str("hi")]).unwrap();
        main.run_event_loop().unwrap();
        assert_eq!(entries(&main_log), ["main hi 0"]);
    }
    #[cfg(not(feature = "dom"))]
    drop(near);
}

#[test]
fn ipc_messages_carry_ports_to_the_first_listening_realm() {
    let (mut main, mut preload) = realms();
    preload.set_electron_window(7);
    let exports = crate::electron_builtin::module(&mut preload);
    let ipc_renderer = get(&mut preload, &Value::Object(exports), "ipcRenderer");
    let log = Log::default();
    let on_port = {
        let log = log.clone();
        preload.function("", 2, false, move |realm, _, args| {
            let ports = get(realm, &args[0], "ports");
            let count = length(realm, &ports)?;
            let nonce = ops::to_string(realm, &args[1])?.to_rust_lossy();
            log.lock().unwrap().push(format!("preload {nonce} {count}"));
            Ok(Value::Undefined)
        })
    };
    call(
        &mut preload,
        &ipc_renderer,
        "on",
        &[Value::str("port"), Value::Object(on_port)],
    )
    .unwrap();
    let (_, far) = channel(&mut main);
    let id = end_of(&main, &Value::Object(far)).unwrap();
    move_out(&mut main, &[id]);
    main.electron_outgoing
        .push((7, "port".into(), vec![Value::str("n1")], vec![id]));
    main.electron_outgoing
        .push((7, "port".into(), vec![Value::str("n2")], Vec::new()));
    main.deliver_electron_messages(7, &mut preload);
    assert_eq!(entries(&log), ["preload n1 1", "preload n2 0"]);
    // The page's realm gets the messages too, without the ports.
    let mut page = Box::new(Realm::new());
    page.set_electron_window(7);
    // SAFETY: both realms are boxed and outlive the test's calls.
    unsafe { page.connect_electron_ipc(&mut *main as *mut Realm) };
    let page_exports = crate::electron_builtin::module(&mut page);
    let page_ipc = get(&mut page, &Value::Object(page_exports), "ipcRenderer");
    let page_log = Log::default();
    let on_page = {
        let log = page_log.clone();
        page.function("", 2, false, move |realm, _, args| {
            let ports = get(realm, &args[0], "ports");
            let count = length(realm, &ports)?;
            log.lock().unwrap().push(format!("page {count}"));
            Ok(Value::Undefined)
        })
    };
    call(
        &mut page,
        &page_ipc,
        "on",
        &[Value::str("port"), Value::Object(on_page)],
    )
    .unwrap();
    main.deliver_electron_messages_to_page(7, &mut page);
    assert_eq!(entries(&page_log), ["page 0", "page 0"]);
}

#[test]
fn renderers_post_ports_to_main_listeners() {
    let (mut main, mut preload) = realms();
    preload.set_electron_window(3);
    let ipc_main = crate::electron_ipc::module(&main);
    let log = Log::default();
    let listener = {
        let log = log.clone();
        main.function("", 2, false, move |realm, _, args| {
            let ports = get(realm, &args[0], "ports");
            let count = length(realm, &ports)?;
            let data = ops::to_string(realm, &args[1])?.to_rust_lossy();
            log.lock().unwrap().push(format!("main {data} {count}"));
            Ok(Value::Undefined)
        })
    };
    call(
        &mut main,
        &Value::Object(ipc_main),
        "on",
        &[Value::str("ports"), Value::Object(listener)],
    )
    .unwrap();
    let exports = crate::electron_builtin::module(&mut preload);
    let ipc_renderer = get(&mut preload, &Value::Object(exports), "ipcRenderer");
    let (first, _) = channel(&mut preload);
    let transfer = Value::Object(preload.array(vec![Value::Object(first.clone())]));
    call(
        &mut preload,
        &ipc_renderer,
        "postMessage",
        &[Value::str("ports"), Value::str("hello"), transfer],
    )
    .unwrap();
    assert_eq!(entries(&log), ["main hello 1"]);
    assert!(end_of(&preload, &Value::Object(first)).is_none());
    // Without the main realm's host, posting throws.
    let mut lone = Box::new(Realm::new());
    lone.electron_ipc_main = &*lone as *const Realm as usize;
    let exports = crate::electron_builtin::module(&mut lone);
    let ipc_renderer = get(&mut lone, &Value::Object(exports), "ipcRenderer");
    assert!(
        call(
            &mut lone,
            &ipc_renderer,
            "postMessage",
            &[Value::str("ports")]
        )
        .is_err()
    );
}

#[test]
fn bindings_open_channels() {
    let mut realm = Realm::new();
    let binding = Value::Object(binding(&mut realm));
    let pair = call(&mut realm, &binding, "channel", &[]).unwrap();
    let first = ops::get(&mut realm, &pair, &Key::Index(0)).unwrap();
    let second = ops::get(&mut realm, &pair, &Key::Index(1)).unwrap();
    assert_eq!(
        end_of(&realm, &first).map(|id| id ^ 1),
        end_of(&realm, &second)
    );
    assert_eq!(
        export_ports(&mut realm, &Value::Undefined).unwrap(),
        Vec::<u64>::new()
    );
    let empty = Value::Object(realm.array(Vec::new()));
    assert_eq!(export_ports(&mut realm, &empty).unwrap(), Vec::<u64>::new());
    assert!(export_ports(&mut realm, &Value::str("x")).is_err());
}
