use super::*;

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn event_replies_are_queued_for_the_originating_window_and_cloned_on_delivery() {
    let mut main = Realm::new();
    let mut renderer = Realm::new();
    renderer.electron_ipc_main = (&mut main as *mut Realm) as usize;
    let ipc_main = module(&main);
    let on = property(&mut main, &ipc_main, "on");
    let listener = main.function("request", 2, false, |realm, _, args| {
        let event = args[0].as_object().unwrap();
        let reply = property(realm, event, "reply");
        ops::call(
            realm,
            &reply,
            Value::Undefined,
            &[Value::str("response"), args[1].clone()],
        )
    });
    ops::call(
        &mut main,
        &on,
        Value::Object(ipc_main),
        &[Value::str("request"), Value::Object(listener)],
    )
    .unwrap();
    let original = renderer.object();
    original.define(Key::name("language"), Property::data(Value::str("en")));
    send_sync(
        &mut main,
        &mut renderer,
        42,
        "request",
        &[Value::Object(original.clone())],
    )
    .unwrap();
    original.define(Key::name("language"), Property::data(Value::str("changed")));
    let exports = crate::electron_builtin::module(&mut renderer);
    let ipc_renderer = property(&mut renderer, &exports, "ipcRenderer");
    let on = property(&mut renderer, ipc_renderer.as_object().unwrap(), "on");
    let received = Rc::new(RefCell::new(None));
    let observed = received.clone();
    let response = renderer.function("response", 2, false, move |_, _, args| {
        *observed.borrow_mut() = Some(args[1].clone());
        Ok(Value::Undefined)
    });
    ops::call(
        &mut renderer,
        &on,
        ipc_renderer,
        &[Value::str("response"), Value::Object(response)],
    )
    .unwrap();
    main.deliver_electron_messages(43, &mut renderer);
    assert!(received.borrow().is_none());
    main.deliver_electron_messages(42, &mut renderer);
    let reply = received.borrow_mut().take().unwrap();
    assert_ne!(reply.as_object().unwrap(), &original);
    assert!(
        matches!(ops::get(&mut renderer, &reply, &Key::name("language")).unwrap(), Value::String(value) if value.to_rust_lossy() == "en")
    );
    main.deliver_electron_messages(42, &mut renderer);
    assert!(received.borrow().is_none());
    let event = ipc_event(&main, 42);
    let reply = property(&mut main, &event, "reply");
    assert!(ops::call(&mut main, &reply, Value::Object(event), &[]).is_err());
    assert!(main.electron_outgoing.is_empty());
}

#[test]
fn ipc_main_emits_registered_events_with_arguments_and_receiver() {
    let mut realm = Realm::new();
    let ipc_main = module(&realm);
    let on = property(&mut realm, &ipc_main, "on");
    let emit = property(&mut realm, &ipc_main, "emit");
    let observed = Rc::new(RefCell::new(None::<f64>));
    let listener_observed = observed.clone();
    let listener = realm.function("listener", 1, false, move |realm, this, args| {
        let number = match args.first() {
            Some(Value::Number(value)) => *value,
            _ => -1.0,
        };
        *listener_observed.borrow_mut() = Some(number);
        ops::set(
            realm,
            &this,
            &Key::name("calledWithEmitter"),
            Value::Boolean(true),
        )?;
        Ok(Value::Undefined)
    });
    ops::call(
        &mut realm,
        &on,
        Value::Object(ipc_main.clone()),
        &[Value::str("marktext:test"), Value::Object(listener)],
    )
    .unwrap();
    assert!(matches!(
        ops::call(
            &mut realm,
            &emit,
            Value::Object(ipc_main.clone()),
            &[Value::str("marktext:test"), Value::Number(42.0)]
        ),
        Ok(Value::Boolean(true))
    ));
    assert_eq!(*observed.borrow(), Some(42.0));
    assert!(matches!(
        property(&mut realm, &ipc_main, "calledWithEmitter"),
        Value::Boolean(true)
    ));
    assert!(matches!(
        ops::call(
            &mut realm,
            &emit,
            Value::Object(ipc_main),
            &[Value::str("missing")]
        ),
        Ok(Value::Boolean(false))
    ));
}

#[test]
fn ipc_main_validates_listener_and_handler_registration() {
    let mut realm = Realm::new();
    let ipc_main = module(&realm);
    let on = property(&mut realm, &ipc_main, "on");
    let handle = property(&mut realm, &ipc_main, "handle");
    let this = Value::Object(ipc_main);
    assert!(ops::call(&mut realm, &on, this.clone(), &[Value::str("event")]).is_err());
    assert!(
        ops::call(
            &mut realm,
            &on,
            this.clone(),
            &[Value::str("event"), Value::Number(1.0)]
        )
        .is_err()
    );
    assert!(ops::call(&mut realm, &handle, this.clone(), &[Value::str("request")]).is_err());
    let handler = realm.function("handler", 0, false, |_, _, _| Ok(Value::Undefined));
    assert!(
        ops::call(
            &mut realm,
            &handle,
            this.clone(),
            &[Value::str("request"), Value::Number(1.0)]
        )
        .is_err()
    );
    assert!(matches!(
        ops::call(
            &mut realm,
            &handle,
            this.clone(),
            &[Value::str("request"), Value::Object(handler.clone())]
        ),
        Ok(Value::Undefined)
    ));
    assert!(
        ops::call(
            &mut realm,
            &handle,
            this,
            &[Value::str("request"), Value::Object(handler)]
        )
        .is_err()
    );
}

#[test]
fn ipc_main_is_an_event_emitter_with_once_off_and_one_shot_handlers() {
    let mut main = Realm::new();
    let ipc_main = module(&main);
    let target = Value::Object(ipc_main.clone());
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let listener = |realm: &Realm, tag: &'static str| {
        let log = log.clone();
        Value::Object(realm.function(tag, 0, false, move |_, _, _| {
            log.borrow_mut().push(tag.into());
            Ok(Value::Undefined)
        }))
    };
    let (always, once, first) = (
        listener(&main, "always"),
        listener(&main, "once"),
        listener(&main, "first"),
    );
    let call = |realm: &mut Realm, name: &'static str, args: &[Value]| {
        let method = property(realm, &ipc_main, name);
        ops::call(realm, &method, target.clone(), args).unwrap()
    };
    let channel = Value::str("ready");
    call(&mut main, "on", &[channel.clone(), always.clone()]);
    call(&mut main, "once", &[channel.clone(), once]);
    call(
        &mut main,
        "prependListener",
        &[channel.clone(), first.clone()],
    );
    let count = call(&mut main, "listenerCount", std::slice::from_ref(&channel));
    assert!(crate::same_value(&count, &Value::Number(3.0)));
    call(&mut main, "emit", std::slice::from_ref(&channel));
    call(&mut main, "emit", std::slice::from_ref(&channel));
    assert_eq!(
        *log.borrow(),
        ["first", "always", "once", "first", "always"]
    );
    call(&mut main, "off", &[channel.clone(), first]);
    call(&mut main, "emit", std::slice::from_ref(&channel));
    assert_eq!(log.borrow().last().map(String::as_str), Some("always"));
    assert_eq!(log.borrow().len(), 6);
    call(
        &mut main,
        "removeAllListeners",
        std::slice::from_ref(&channel),
    );
    let none = call(&mut main, "emit", &[channel]);
    assert!(matches!(none, Value::Boolean(false)));

    // One-shot handlers go after their first use; removeHandler frees a name.
    let handler = listener(&main, "handler");
    call(
        &mut main,
        "handleOnce",
        &[Value::str("ask"), handler.clone()],
    );
    let state = main.electron_ipc_registry().unwrap();
    assert!(state.borrow_mut().take_handler("ask").is_some());
    assert!(state.borrow_mut().take_handler("ask").is_none());
    call(&mut main, "handle", &[Value::str("ask"), handler.clone()]);
    call(&mut main, "removeHandler", &[Value::str("ask")]);
    call(&mut main, "handle", &[Value::str("ask"), handler]);
    assert!(state.borrow_mut().take_handler("ask").is_some());
    assert!(state.borrow_mut().take_handler("ask").is_some());
}
