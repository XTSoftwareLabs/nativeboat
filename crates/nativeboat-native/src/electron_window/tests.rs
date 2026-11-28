//! Electron window objects with a small EventEmitter standing in for the
//! Node.js library's.

use super::*;
use std::sync::{Arc, Mutex};

/// `require` of a library whose `events` module keeps listeners in arrays on
/// `_events`.
fn library(realm: &mut Realm, name: &str) -> Completion {
    assert_eq!(name, "events");
    let constructor = realm.function("EventEmitter", 0, false, |realm, this, _| {
        let events = Value::Object(realm.object());
        ops::set(realm, &this, &Key::name("_events"), events)?;
        Ok(Value::Undefined)
    });
    let prototype = realm.object();
    let on = realm.function("on", 2, false, |realm, this, args| {
        let events = ops::get(realm, &this, &Key::name("_events"))?;
        let name = ops::to_key(realm, &args[0])?;
        let listeners = match ops::get(realm, &events, &name)? {
            Value::Undefined => Value::Object(realm.array(Vec::new())),
            listeners => listeners,
        };
        let push = ops::get(realm, &listeners, &Key::name("push"))?;
        ops::call(realm, &push, listeners.clone(), &args[1..2])?;
        ops::set(realm, &events, &name, listeners)?;
        Ok(this)
    });
    let emit = realm.function("emit", 1, false, |realm, this, args| {
        let events = ops::get(realm, &this, &Key::name("_events"))?;
        let name = ops::to_key(realm, &args[0])?;
        let listeners = ops::get(realm, &events, &name)?;
        if matches!(listeners, Value::Undefined) {
            return Ok(Value::Boolean(false));
        }
        for listener in crate::iter::collect(realm, &listeners)? {
            ops::call(realm, &listener, this.clone(), &args[1..])?;
        }
        Ok(Value::Boolean(true))
    });
    for (key, function) in [("on", on), ("emit", emit)] {
        prototype.define(Key::name(key), Property::data(Value::Object(function)));
    }
    constructor.define(
        Key::name("prototype"),
        Property::data(Value::Object(prototype)),
    );
    Ok(Value::Object(constructor))
}

type Log = Arc<Mutex<Vec<String>>>;

fn recorder(realm: &Realm, log: &Log, label: &'static str) -> Value {
    let log = log.clone();
    Value::Object(realm.function("", 0, false, move |realm, _, args| {
        let mut entry = label.to_owned();
        for value in args.iter().skip(1) {
            entry.push(' ');
            entry.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
        }
        log.lock().unwrap().push(entry);
        Ok(Value::Undefined)
    }))
}

fn listen(realm: &mut Realm, target: &Object, name: &str, listener: Value) {
    let on = ops::get(realm, &Value::Object(target.clone()), &Key::name("on")).unwrap();
    ops::call(
        realm,
        &on,
        Value::Object(target.clone()),
        &[Value::str(name), listener],
    )
    .unwrap();
}

fn entries(log: &Log) -> Vec<String> {
    core::mem::take(&mut *log.lock().unwrap())
}

#[test]
fn emitters_need_the_node_library() {
    let mut realm = Realm::new();
    assert!(event_emitter(&mut realm).unwrap().is_none());
    let object = realm.object();
    inert_listeners(&realm, &object);
    let target = Value::Object(object.clone());
    let on = ops::get(&mut realm, &target, &Key::name("on")).unwrap();
    let returned = ops::call(&mut realm, &on, target.clone(), &[]).unwrap();
    assert!(crate::same_value(&returned, &target));
    assert!(!emit(&mut realm, &object, "x", &[]).unwrap());
    let plain = realm.object();
    assert!(!emit(&mut realm, &plain, "x", &[]).unwrap());
    realm.node_library = Some(library);
    assert!(event_emitter(&mut realm).unwrap().is_some());
}

#[test]
fn web_contents_queue_messages_for_their_page() {
    let mut realm = Realm::new();
    realm.node_library = Some(library);
    let emitter = event_emitter(&mut realm).unwrap();
    let window = Window::from_raw(7);
    let contents = web_contents(&mut realm, emitter.as_ref(), window).unwrap();
    let target = Value::Object(contents.clone());
    let send = ops::get(&mut realm, &target, &Key::name("send")).unwrap();
    let payload = Value::Object(realm.array(vec![Value::Number(1.0)]));
    ops::call(
        &mut realm,
        &send,
        target.clone(),
        &[Value::str("ping"), payload],
    )
    .unwrap();
    ops::call(&mut realm, &send, target.clone(), &[Value::str("once")]).unwrap();
    assert_eq!(realm.electron_outgoing.len(), 2);
    // Messages for other windows wait.
    realm
        .electron_outgoing
        .push((8, "other".into(), Vec::new(), Vec::new()));
    let mut preload = Realm::new();
    let log = Log::default();
    let listeners: RendererListeners = Rc::default();
    for (channel, label, once) in [("ping", "ping", false), ("once", "once", true)] {
        let Value::Object(listener) = recorder(&preload, &log, label) else {
            unreachable!()
        };
        listeners
            .borrow_mut()
            .push((channel.into(), listener, once));
    }
    preload.electron_renderer_listeners = Some(listeners.clone());
    assert!(realm.has_electron_messages(7) && realm.has_electron_messages(8));
    assert!(!realm.has_electron_messages(9));
    realm.deliver_electron_messages(7, &mut preload);
    assert!(!realm.has_electron_messages(7));
    assert_eq!(entries(&log), ["ping 1", "once"]);
    assert_eq!(listeners.borrow().len(), 1);
    assert_eq!(realm.electron_outgoing.len(), 1);
    // A realm without ipcRenderer listeners has none of them.
    realm.deliver_electron_messages(8, &mut Realm::new());
    assert!(realm.electron_outgoing.is_empty());
    // The page's realm gets the messages too, as a page with Node
    // integration listening through `require('electron')`; then they go.
    let mut page = Realm::new();
    let page_log = Log::default();
    let page_listeners: RendererListeners = Rc::default();
    let Value::Object(listener) = recorder(&page, &page_log, "page") else {
        unreachable!()
    };
    page_listeners
        .borrow_mut()
        .push(("ping".into(), listener, false));
    page.electron_renderer_listeners = Some(page_listeners);
    realm
        .electron_outgoing
        .push((9, "ping".into(), Vec::new(), Vec::new()));
    realm.deliver_electron_messages_to_page(7, &mut page);
    assert_eq!(entries(&page_log), ["page 1"]);
    realm.deliver_electron_messages_to_page(9, &mut page);
    assert_eq!(entries(&page_log), ["page"]);
    realm.deliver_electron_messages_to_page(7, &mut page);
    realm.deliver_electron_messages_to_page(8, &mut page);
    assert!(entries(&page_log).is_empty());
    assert!(realm.electron_preloaded.is_empty() && realm.electron_outgoing.is_empty());
    for (name, expected) in [("getZoomFactor", 1.0), ("getZoomLevel", 0.0)] {
        let method = ops::get(&mut realm, &target, &Key::name(name)).unwrap();
        let value = ops::call(&mut realm, &method, target.clone(), &[]).unwrap();
        assert!(
            matches!(value, Value::Number(number) if number == expected),
            "{name}"
        );
    }
    let session = ops::get(&mut realm, &target, &Key::name("session")).unwrap();
    // The spell checker's languages are the process's.
    let _serial = crate::spelling::SERIAL
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let languages = ops::get(&mut realm, &session, &Key::name("getSpellCheckerLanguages")).unwrap();
    assert!(ops::call(&mut realm, &languages, session, &[]).is_ok());
}

#[test]
fn page_loads_and_window_messages_become_events() {
    let mut realm = Realm::new();
    realm.node_library = Some(library);
    let emitter = event_emitter(&mut realm).unwrap().unwrap();
    let (instance, contents, app) = (realm.object(), realm.object(), realm.object());
    for object in [&instance, &contents, &app] {
        make_emitter(&mut realm, &emitter, object, None).unwrap();
    }
    realm.register_electron_window(5, instance.clone(), contents.clone());
    realm.electron_app = Some(app.clone());
    let log = Log::default();
    for (target, name) in [
        (&contents, "dom-ready"),
        (&contents, "did-finish-load"),
        (&instance, "ready-to-show"),
        (&instance, "closed"),
        (&instance, "focus"),
        (&instance, "maximize"),
        (&instance, "unmaximize"),
        (&instance, "minimize"),
        (&instance, "restore"),
        (&instance, "enter-full-screen"),
        (&instance, "leave-full-screen"),
        (&contents, "zoom-changed"),
        (&app, "window-all-closed"),
    ] {
        let listener = recorder(&realm, &log, name);
        listen(&mut realm, target, name, listener);
    }
    realm.electron_page_loaded(5);
    realm.electron_page_loaded(6);
    assert_eq!(
        entries(&log),
        ["dom-ready", "did-finish-load", "ready-to-show"]
    );
    // `close` listeners may refuse.
    let refuse = realm.function("", 1, false, |realm, _, args| {
        let prevent = ops::get(realm, &args[0], &Key::name("preventDefault"))?;
        ops::call(realm, &prevent, args[0].clone(), &[])
    });
    listen(&mut realm, &instance, "close", Value::Object(refuse));
    let context = (&mut realm as *mut Realm).cast::<c_void>();
    // SAFETY: the realm outlives these calls on this thread.
    unsafe {
        assert!(!window_event(context, 5, WindowEvent::Close));
        assert!(window_event(context, 5, WindowEvent::Focus));
        for event in [
            WindowEvent::Maximize,
            WindowEvent::Unmaximize,
            WindowEvent::Minimize,
            WindowEvent::Restore,
            WindowEvent::EnterFullScreen,
            WindowEvent::LeaveFullScreen,
            WindowEvent::ZoomIn,
            WindowEvent::ZoomOut,
        ] {
            assert!(window_event(context, 5, event));
        }
        assert!(window_event(context, 9, WindowEvent::Close));
        assert!(window_event(core::ptr::null_mut(), 5, WindowEvent::Close));
        assert!(window_event(context, 5, WindowEvent::Closed));
    }
    assert_eq!(
        entries(&log),
        [
            "focus",
            "maximize",
            "unmaximize",
            "minimize",
            "restore",
            "enter-full-screen",
            "leave-full-screen",
            "zoom-changed in",
            "zoom-changed out",
            "closed",
            "window-all-closed"
        ]
    );
    assert!(realm.electron_web_contents(5).is_none());
    // Listener errors wait for the host step.
    let throwing = realm.function("", 0, false, |realm, _, _| {
        Err(realm.throw(crate::ErrorKind::Range, "listener"))
    });
    let other = realm.object();
    make_emitter(&mut realm, &emitter, &other, None).unwrap();
    listen(
        &mut realm,
        &other,
        "resize",
        Value::Object(throwing.clone()),
    );
    listen(
        &mut realm,
        &other,
        "did-finish-load",
        Value::Object(throwing),
    );
    realm.register_electron_window(6, other.clone(), other);
    // SAFETY: as above.
    unsafe { window_event(context, 6, WindowEvent::Resize) };
    realm.electron_page_loaded(6);
    assert_eq!(realm.host_errors.len(), 2);
}

#[test]
fn keys_reach_before_input_listeners_which_may_keep_them_from_the_page() {
    let mut realm = Realm::new();
    realm.node_library = Some(library);
    let emitter = event_emitter(&mut realm).unwrap().unwrap();
    let (instance, contents) = (realm.object(), realm.object());
    for object in [&instance, &contents] {
        make_emitter(&mut realm, &emitter, object, None).unwrap();
    }
    realm.register_electron_window(5, instance, contents.clone());
    let log = Log::default();
    let seen = log.clone();
    // Records the input and prevents Control+S presses.
    let listener = realm.function("", 2, false, move |realm, _, args| {
        let mut entry = String::new();
        for name in [
            "type",
            "key",
            "code",
            "isAutoRepeat",
            "isComposing",
            "shift",
            "control",
            "alt",
            "meta",
            "location",
            "modifiers",
        ] {
            let value = ops::get(realm, &args[1], &Key::name(name))?;
            entry.push_str(&ops::to_string(realm, &value)?.to_rust_lossy());
            entry.push(' ');
        }
        seen.lock().unwrap().push(entry.trim_end().to_owned());
        let control = ops::get(realm, &args[1], &Key::name("control"))?;
        let kind = ops::get(realm, &args[1], &Key::name("type"))?;
        if ops::to_boolean(&control) && ops::to_string(realm, &kind)?.to_rust_lossy() == "keyDown" {
            let prevent = ops::get(realm, &args[0], &Key::name("preventDefault"))?;
            ops::call(realm, &prevent, args[0].clone(), &[])?;
        }
        Ok(Value::Undefined)
    });
    listen(
        &mut realm,
        &contents,
        "before-input-event",
        Value::Object(listener),
    );
    let save = KeyInput {
        down: true,
        key: "s",
        code: "KeyS",
        control: true,
        num_lock: true,
        ..KeyInput::default()
    };
    assert!(realm.electron_before_input(5, &save));
    assert!(!realm.electron_before_input(
        5,
        &KeyInput {
            down: false,
            ..save.clone()
        }
    ));
    let keypad = KeyInput {
        down: true,
        key: "1",
        code: "Numpad1",
        repeat: true,
        shift: true,
        location: 3,
        ..KeyInput::default()
    };
    assert!(!realm.electron_before_input(5, &keypad));
    // Keys of other windows have no listeners.
    assert!(!realm.electron_before_input(6, &save));
    assert_eq!(
        entries(&log),
        [
            "keyDown s KeyS false false false true false false 0 control,numlock",
            "keyUp s KeyS false false false true false false 0 control,numlock",
            "keyDown 1 Numpad1 true false true false false false 3 shift,isautorepeat,iskeypad",
        ]
    );
    // Listener errors wait for the host step.
    let throwing = realm.function("", 0, false, |realm, _, _| {
        Err(realm.throw(crate::ErrorKind::Range, "listener"))
    });
    listen(
        &mut realm,
        &contents,
        "before-input-event",
        Value::Object(throwing),
    );
    assert!(realm.electron_before_input(5, &save));
    assert_eq!(realm.host_errors.len(), 1);
}

#[test]
fn the_main_frame_sends_through_its_contents() {
    let mut realm = Realm::new();
    realm.node_library = Some(library);
    let emitter = event_emitter(&mut realm).unwrap();
    let contents = web_contents(&mut realm, emitter.as_ref(), Window::from_raw(9)).unwrap();
    let target = Value::Object(contents);
    let frame = ops::get(&mut realm, &target, &Key::name("mainFrame")).unwrap();
    let top = ops::get(&mut realm, &frame, &Key::name("top")).unwrap();
    assert!(crate::same_value(&top, &frame));
    let url = ops::get(&mut realm, &frame, &Key::name("url")).unwrap();
    assert!(matches!(url, Value::String(text) if text.to_rust_lossy().is_empty()));
    let send = ops::get(&mut realm, &frame, &Key::name("send")).unwrap();
    ops::call(&mut realm, &send, frame.clone(), &[Value::str("ping")]).unwrap();
    assert_eq!(realm.electron_outgoing.len(), 1);
    assert_eq!(realm.electron_outgoing[0].1, "ping");
}
