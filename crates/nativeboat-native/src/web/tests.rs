//! Web API natives tests.
use super::*;

struct Fixture {
    realm: Realm,
    native: Value,
    dom: Rc<RefCell<Dom>>,
}

impl Fixture {
    fn new(html: &str) -> Self {
        let mut realm = Realm::new();
        let dom = realm.install_web(
            html,
            Rc::new(BlankPage {
                url: "file:///editor.html?document=one".into(),
                viewport: (800.0, 600.0),
            }),
        );
        let global = Value::Object(realm.global_object());
        let arguments = ops::get(&mut realm, &global, &Key::name(PRELUDE_ARGUMENTS)).unwrap();
        let native = ops::get(&mut realm, &arguments, &Key::Index(1)).unwrap();
        Self { realm, native, dom }
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Completion {
        let function = ops::get(&mut self.realm, &self.native, &Key::name(name))?;
        ops::call(&mut self.realm, &function, self.native.clone(), args)
    }

    fn query(&mut self, selector: &str) -> Value {
        self.call("querySelector", &[Value::Number(0.0), Value::str(selector)])
            .unwrap()
    }

    fn get(&mut self, object: &Value, index: u32) -> Value {
        ops::get(&mut self.realm, object, &Key::Index(index)).unwrap()
    }
}

fn numeric(value: &Value) -> f64 {
    let Value::Number(value) = value else {
        panic!("expected a number: {value:?}")
    };
    *value
}

fn string_value(value: &Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string: {value:?}")
    };
    value.to_rust_lossy()
}

#[test]
fn editor_mutations_share_the_painted_document_and_queue_observer_records() {
    let mut fixture = Fixture::new("<main id='editor' contenteditable><p>before</p></main>");
    let editor = fixture.query("#editor");
    let paragraph = fixture.query("p");
    let text = fixture
        .call("firstChild", core::slice::from_ref(&paragraph))
        .unwrap();
    let observer = fixture.call("createObserver", &[]).unwrap();
    fixture
        .call(
            "observe",
            &[
                observer.clone(),
                editor.clone(),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Boolean(true),
                Value::Null,
            ],
        )
        .unwrap();
    let before = fixture.dom.borrow().version();
    fixture
        .call("setData", &[text.clone(), Value::str("after 😀")])
        .unwrap();
    fixture
        .call(
            "styleSet",
            &[
                paragraph.clone(),
                Value::str("color"),
                Value::str("red"),
                Value::Boolean(true),
            ],
        )
        .unwrap();
    let added = fixture
        .call("createElement", &[Value::str("span")])
        .unwrap();
    fixture
        .call("insertBefore", &[editor.clone(), added, Value::Null])
        .unwrap();
    assert!(fixture.dom.borrow().version() > before);
    assert_eq!(
        fixture
            .dom
            .borrow()
            .text_content(NodeId(numeric(&editor) as u32))
            .as_deref(),
        Some("after 😀")
    );
    let records = fixture
        .call("takeRecords", core::slice::from_ref(&observer))
        .unwrap();
    let first = fixture.get(&records, 0);
    assert_eq!(string_value(&fixture.get(&first, 0)), "characterData");
    assert_eq!(numeric(&fixture.get(&first, 1)), numeric(&text));
    assert_eq!(string_value(&fixture.get(&first, 7)), "before");
    let second = fixture.get(&records, 1);
    assert_eq!(string_value(&fixture.get(&second, 0)), "attributes");
    let third = fixture.get(&records, 2);
    assert_eq!(string_value(&fixture.get(&third, 0)), "childList");
    assert_eq!(
        string_value(
            &fixture
                .call("stylePriority", &[paragraph, Value::str("color")])
                .unwrap()
        ),
        "important"
    );
    let records = fixture
        .call("takeRecords", core::slice::from_ref(&observer))
        .unwrap();
    assert!(matches!(fixture.get(&records, 0), Value::Undefined));
    fixture.call("disconnect", &[observer]).unwrap();
}

#[test]
fn selection_ranges_survive_text_splits_and_normalization() {
    let mut fixture = Fixture::new("<main id='editor' contenteditable>ab😀cd</main>");
    let editor = fixture.query("#editor");
    let text = fixture
        .call("firstChild", core::slice::from_ref(&editor))
        .unwrap();
    let tail = fixture
        .call("splitText", &[text.clone(), Value::Number(2.0)])
        .unwrap();
    let range = fixture
        .call(
            "createRange",
            &[tail.clone(), Value::Number(0.0), tail, Value::Number(4.0)],
        )
        .unwrap();
    fixture
        .call("setSelection", core::slice::from_ref(&range))
        .unwrap();
    fixture
        .call("normalize", core::slice::from_ref(&editor))
        .unwrap();
    let boundaries = fixture
        .call("range", core::slice::from_ref(&range))
        .unwrap();
    assert_eq!(numeric(&fixture.get(&boundaries, 0)), numeric(&text));
    assert_eq!(numeric(&fixture.get(&boundaries, 1)), 2.0);
    assert_eq!(numeric(&fixture.get(&boundaries, 3)), 6.0);
    assert_eq!(
        numeric(
            &fixture
                .call("rangeField", &[range.clone(), Value::Number(5.0)])
                .unwrap()
        ),
        numeric(&text)
    );
    assert!(matches!(
        fixture
            .call("rangeField", &[range.clone(), Value::Number(4.0)])
            .unwrap(),
        Value::Boolean(false)
    ));
    fixture.call("setSelection", &[Value::Null]).unwrap();
    fixture
        .call("dropRange", core::slice::from_ref(&range))
        .unwrap();
    assert!(
        fixture
            .call("rangeField", &[range, Value::Number(0.0)])
            .is_err()
    );
}

#[test]
fn editing_history_journals_revert_on_the_original_nodes() {
    let mut fixture = Fixture::new("<main id='editor' contenteditable>original</main>");
    let editor = fixture.query("#editor");
    let text = fixture
        .call("firstChild", core::slice::from_ref(&editor))
        .unwrap();
    let changes = |fixture: &mut Fixture| {
        numeric(
            &fixture
                .call("historyChanges", core::slice::from_ref(&editor))
                .unwrap(),
        )
    };
    let recording = fixture
        .call("historyBegin", core::slice::from_ref(&editor))
        .unwrap();
    let start = changes(&mut fixture);
    fixture
        .call(
            "replaceData",
            &[
                text.clone(),
                Value::Number(0.0),
                Value::Number(8.0),
                Value::str("changed"),
            ],
        )
        .unwrap();
    assert_eq!(changes(&mut fixture), start + 1.0);
    let journal = fixture.call("historyEnd", &[recording]).unwrap();
    assert!(matches!(journal, Value::Object(_)));
    let joined = fixture
        .call("historyJoin", &[journal.clone(), journal.clone()])
        .unwrap();
    assert!(matches!(joined, Value::Object(_)));
    fixture.call("historyRevert", &[journal]).unwrap();
    let after = fixture
        .call("firstChild", core::slice::from_ref(&editor))
        .unwrap();
    assert_eq!(numeric(&after), numeric(&text));
    assert_eq!(
        string_value(
            &fixture
                .call("text", core::slice::from_ref(&editor))
                .unwrap()
        ),
        "original"
    );
    // A recording without changes has no journal.
    let recording = fixture.call("historyBegin", &[editor]).unwrap();
    assert!(matches!(
        fixture.call("historyEnd", &[recording]).unwrap(),
        Value::Null
    ));
    assert!(fixture.call("historyRevert", &[Value::Null]).is_err());
    assert!(
        fixture
            .call("historyJoin", &[Value::Null, Value::Null])
            .is_err()
    );
}

#[test]
fn invalid_nodes_and_dom_mutations_report_errors_without_corrupting_the_document() {
    let mut fixture = Fixture::new("<main id='editor'><p>unchanged</p></main>");
    let editor = fixture.query("#editor");
    let paragraph = fixture.query("p");
    for invalid in [
        Value::Null,
        Value::Number(-1.0),
        Value::Number(0.5),
        Value::Number(f64::NAN),
        Value::Number(f64::INFINITY),
        Value::Number(10000.0),
    ] {
        assert!(fixture.call("text", &[invalid]).is_err());
    }
    let before = fixture.dom.borrow().version();
    let thrown = fixture
        .call("insertBefore", &[paragraph, editor.clone(), Value::Null])
        .unwrap_err();
    let message = ops::to_string(&mut fixture.realm, &thrown.0)
        .unwrap()
        .to_rust_lossy();
    assert!(message.contains("HierarchyRequestError"), "{message}");
    assert_eq!(fixture.dom.borrow().version(), before);
    assert!(
        fixture
            .call("querySelector", &[editor.clone(), Value::str("[")])
            .is_err()
    );
    assert_eq!(
        string_value(&fixture.call("text", &[editor]).unwrap()),
        "unchanged"
    );
}

#[test]
fn renderer_callbacks_keep_their_receiver_and_original_exception() {
    let mut fixture = Fixture::new("<main id='editor'></main>");
    let host = fixture.realm.web_host.clone().unwrap();
    let web = fixture.realm.object();
    let expected_receiver = web.clone();
    let marker = fixture.realm.object();
    let expected_error = marker.clone();
    let ready = fixture
        .realm
        .function("ready", 0, false, move |_, receiver, args| {
            assert_eq!(receiver.as_object().unwrap(), &expected_receiver);
            assert!(args.is_empty());
            Err(Thrown(Value::Object(expected_error.clone())))
        });
    web.define(Key::name("ready"), Property::data(Value::Object(ready)));
    host.define(Key::name("web"), Property::data(Value::Object(web)));
    let thrown = fixture.realm.web_call("ready", &[]).unwrap_err();
    assert_eq!(thrown.0.as_object().unwrap(), &marker);
    assert!(matches!(
        fixture.realm.web_call("optional", &[]).unwrap(),
        Value::Undefined
    ));
}

#[test]
fn renderer_errors_reach_the_installed_window_error_dispatcher() {
    let fixture = Fixture::new("<main></main>");
    let mut realm = fixture.realm;
    let host = realm.web_host.clone().unwrap();
    let received = Rc::new(RefCell::new(None));
    let observed = received.clone();
    let handler = realm.function("onerror", 1, false, move |_, _, args| {
        *observed.borrow_mut() = args.first().cloned();
        Ok(Value::Boolean(true))
    });
    host.define(Key::name("onerror"), Property::data(Value::Object(handler)));
    let thrown = realm.throw(ErrorKind::Error, "renderer failure");
    uncaught(&mut realm, thrown.clone());
    assert_eq!(
        received.borrow().as_ref().unwrap().as_object(),
        thrown.0.as_object()
    );
}

#[test]
fn context_menus_of_pages_reach_their_windows_web_contents() {
    let fixture = Fixture::new("<main></main>");
    let mut renderer = fixture.realm;
    let host = Value::Object(renderer.web_host.clone().unwrap());
    let context_menu = ops::get(&mut renderer, &host, &Key::name("contextMenu")).unwrap();
    let mut main = Realm::new();
    let (instance, contents) = (main.object(), main.object());
    main.register_electron_window(9, instance, contents.clone());
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = seen.clone();
    let emit = main.function("emit", 3, false, move |realm, _, args| {
        let name = ops::to_string(realm, &args[0])?.to_rust_lossy();
        let x = ops::get(realm, &args[2], &Key::name("x"))?;
        let prevent = ops::get(realm, &args[1], &Key::name("preventDefault"))?;
        record.borrow_mut().push((
            name,
            numeric(&x),
            crate::array_builtin::is_callable(&prevent),
        ));
        Ok(Value::Boolean(true))
    });
    contents.define(Key::name("emit"), Property::data(Value::Object(emit)));
    let params = renderer.object();
    params.define(Key::name("x"), Property::data(Value::Number(12.0)));
    let call = |renderer: &mut Realm| {
        ops::call(
            renderer,
            &context_menu,
            Value::Undefined,
            &[Value::Object(params.clone())],
        )
        .unwrap();
    };
    // Pages outside Electron windows have no main realm to tell.
    call(&mut renderer);
    renderer.electron_ipc_main = (&mut main as *mut Realm) as usize;
    call(&mut renderer);
    renderer.electron_window = 9;
    call(&mut renderer);
    assert!(seen.borrow().is_empty());
    main.run_jobs().unwrap();
    assert_eq!(*seen.borrow(), [("context-menu".to_owned(), 12.0, true)]);
    // Windows that closed meanwhile hear nothing.
    renderer.electron_window = 10;
    call(&mut renderer);
    main.run_jobs().unwrap();
    assert_eq!(seen.borrow().len(), 1);
}

#[test]
fn user_agents_are_electrons_with_the_applications_name() {
    assert_eq!(super::chrome_major(Some("42.0.1")), 148);
    assert_eq!(super::chrome_major(Some("30.0.0")), 124);
    assert_eq!(super::chrome_major(None), 124);
    let mut realm = Realm::new();
    assert_eq!(
        super::user_agent(&realm),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
    );
    realm.electron_version = Some("42.0.1");
    realm.electron_app_name = Some("marktext");
    realm.electron_app_version = Some("0.20.0");
    assert_eq!(
        super::user_agent(&realm),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) marktext/0.20.0 Chrome/148.0.0.0 Electron/42.0.1 Safari/537.36"
    );
}

#[test]
fn pages_download_images_with_their_user_agent() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let read = socket.read(&mut request).unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nPNG!")
            .unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let mut fixture = Fixture::new("<main></main>");
    let realm = &mut fixture.realm;
    realm.electron_version = Some("38.7.2");
    realm.electron_app_name = Some("picgo");
    realm.electron_app_version = Some("3.0.3");
    let global = Value::Object(realm.global_object());
    let arguments = ops::get(realm, &global, &Key::name(PRELUDE_ARGUMENTS)).unwrap();
    let page = ops::get(realm, &arguments, &Key::Index(2)).unwrap();
    let fetch_image = ops::get(realm, &page, &Key::name("fetchImage")).unwrap();
    let seen = Rc::new(RefCell::new(None));
    let record = seen.clone();
    let loaded = realm.function("loaded", 1, false, move |_, _, args| {
        *record.borrow_mut() = crate::typed_array_builtin::view_bytes(&args[0]);
        Ok(Value::Undefined)
    });
    let url = Value::str(&format!("http://{address}/pic.png"));
    ops::call(realm, &fetch_image, page, &[url, Value::Object(loaded)]).unwrap();
    realm.run_event_loop().unwrap();
    let request = server.join().unwrap();
    // As Chromium asks for a page's images, with the user agent the page
    // reads.
    assert!(
        request.contains(&format!("User-Agent: {}\r\n", super::user_agent(realm))),
        "{request}"
    );
    assert_eq!(seen.borrow().as_deref(), Some(&b"PNG!"[..]));
}

#[test]
fn page_notifications_are_shown_by_the_main_realm_and_their_events_return() {
    let fixture = Fixture::new("<main></main>");
    let mut renderer = fixture.realm;
    let host = Value::Object(renderer.web_host.clone().unwrap());
    let notify = ops::get(&mut renderer, &host, &Key::name("notify")).unwrap();
    let close = ops::get(&mut renderer, &host, &Key::name("closeNotification")).unwrap();
    let args = [
        Value::Number(1.0),
        Value::str("Downloaded"),
        Value::str("video.mp4"),
        Value::Boolean(true),
    ];
    // Pages outside Electron windows cannot show notifications.
    let shown = ops::call(&mut renderer, &notify, Value::Undefined, &args).unwrap();
    assert!(matches!(shown, Value::Boolean(false)));
    let mut main = Realm::new();
    renderer.electron_ipc_main = (&mut main as *mut Realm) as usize;
    renderer.electron_window = 9;
    let shown = ops::call(&mut renderer, &notify, Value::Undefined, &args).unwrap();
    assert!(matches!(shown, Value::Boolean(true)));
    assert!(main.page_notification_events.is_empty());
    main.run_jobs().unwrap();
    // Shown, or failed where the test has no notification area.
    let events = main.page_notification_events.clone();
    assert!(
        matches!(events.as_slice(), [(9, 1, "show" | "error")]),
        "{events:?}"
    );
    assert!(main.has_electron_messages(9));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = seen.clone();
    let notification = renderer.function("notification", 2, false, move |realm, _, args| {
        let kind = ops::to_string(realm, &args[1])?.to_rust_lossy();
        record.borrow_mut().push((numeric(&args[0]), kind));
        Ok(Value::Undefined)
    });
    let web = renderer.object();
    web.define(
        Key::name("notification"),
        Property::data(Value::Object(notification)),
    );
    host.as_object()
        .unwrap()
        .define(Key::name("web"), Property::data(Value::Object(web)));
    main.deliver_electron_messages_to_page(9, &mut renderer);
    assert_eq!(seen.borrow().len(), 1);
    assert!(!main.has_electron_messages(9));
    // A click on a shown notification reaches its page, which then closes.
    main.notify_targets.insert(
        77,
        crate::electron_notify::Target::Page {
            window: 9,
            notification: 2,
            icon: None,
        },
    );
    // SAFETY: the context is this live main realm, on this thread.
    unsafe {
        crate::electron_notify::notify_event(
            (&mut main as *mut Realm).cast(),
            crate::app_host::NotifyEvent::BalloonClick,
            77,
        );
    }
    main.deliver_electron_messages_to_page(9, &mut renderer);
    assert_eq!(
        seen.borrow()[1..],
        [(2.0, "click".to_owned()), (2.0, "close".to_owned())]
    );
    // Closing it from the page closes the shown icon, once.
    main.notify_targets.insert(
        78,
        crate::electron_notify::Target::Page {
            window: 9,
            notification: 3,
            icon: None,
        },
    );
    for _ in 0..2 {
        ops::call(
            &mut renderer,
            &close,
            Value::Undefined,
            &[Value::Number(3.0)],
        )
        .unwrap();
        main.run_jobs().unwrap();
    }
    main.deliver_electron_messages_to_page(9, &mut renderer);
    assert_eq!(seen.borrow()[3..], [(3.0, "close".to_owned())]);
}

#[test]
fn input_patterns_match_whole_values_with_the_v_flag() {
    assert_eq!(input_pattern_matches("[0-9]+", "123"), Some(true));
    assert_eq!(input_pattern_matches("[0-9]+", "12a"), Some(false));
    // The pattern is grouped before it is anchored.
    assert_eq!(input_pattern_matches("a|b", "ab"), Some(false));
    assert_eq!(input_pattern_matches("a|b", "b"), Some(true));
    // Property escapes need the Unicode flags.
    assert_eq!(
        input_pattern_matches("\\p{Lu}+", "\u{c9}T\u{c9}"),
        Some(true)
    );
    assert_eq!(
        input_pattern_matches("\\p{Lu}+", "\u{c9}t\u{c9}"),
        Some(false)
    );
    assert_eq!(input_pattern_matches("(", "x"), None);
}

#[test]
fn constraint_natives_report_the_documents_validity_with_the_runtimes_patterns() {
    let mut fixture =
        Fixture::new("<input id=code pattern='[0-9]+' value=12a><input id=off disabled>");
    let code = fixture.query("#code");
    let off = fixture.query("#off");
    // The page's document matches patterns with the runtime's expressions.
    let flags = fixture
        .call("validity", core::slice::from_ref(&code))
        .unwrap();
    assert_eq!(
        numeric(&flags),
        f64::from(nativeboat_dom::validity::PATTERN_MISMATCH)
    );
    let message = fixture
        .call("validationMessage", core::slice::from_ref(&code))
        .unwrap();
    assert_eq!(string_value(&message), "Please match the requested format.");
    fixture
        .call("setCustomValidity", &[code.clone(), Value::str("Digits")])
        .unwrap();
    let message = fixture
        .call("validationMessage", core::slice::from_ref(&code))
        .unwrap();
    assert_eq!(string_value(&message), "Digits");
    for (node, expected) in [(code, true), (off, false)] {
        let will = fixture.call("willValidate", &[node]).unwrap();
        assert!(matches!(will, Value::Boolean(value) if value == expected));
    }
}
