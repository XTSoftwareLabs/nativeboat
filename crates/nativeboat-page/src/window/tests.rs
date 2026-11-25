use super::*;
use alloc::string::{String, ToString};
use alloc::vec;
use nativeboat_layout::{FontMetrics, TextMeasurer};
use nativeboat_native::web::{BlankPage, PRELUDE_ARGUMENTS};
use nativeboat_native::{Thrown, ops};

extern crate std;

struct Measurer;
impl TextMeasurer for Measurer {
    fn width(&self, text: &str, _: &nativeboat_ir::Font) -> f32 {
        text.chars().count() as f32 * 8.0
    }
    fn metrics(&self, _: &nativeboat_ir::Font) -> FontMetrics {
        FontMetrics {
            ascent: 12.0,
            descent: 4.0,
        }
    }
}

/// A page in a native window whose renderer interface records its calls.
struct Fixture {
    realm: Box<Realm>,
    window: Rc<PageWindow>,
    calls: Rc<RefCell<Vec<String>>>,
    host: nativeboat_native::Object,
}

const METHODS: [&str; 10] = [
    "pointer",
    "wheel",
    "key",
    "text",
    "composition",
    "scroll",
    "activate",
    "input",
    "select",
    "value",
];

impl Fixture {
    fn new() -> Fixture {
        let mut realm = Box::new(Realm::new());
        let page = Rc::new(DocumentPage::new(
            Rc::new(BlankPage {
                url: "file:///editor.html".to_string(),
                viewport: (300.0, 200.0),
            }),
            Rc::new(Measurer),
        ));
        let dom = realm.install_web(
            "<main id='editor' style='cursor: pointer; height: 50px'>text</main>",
            page.clone(),
        );
        page.set_document(dom);
        page.extend_stylesheets(vec!["body { margin: 0 }".to_string()]);
        let global = Value::Object(realm.global_object());
        let arguments = ops::get(&mut realm, &global, &Key::name(PRELUDE_ARGUMENTS)).unwrap();
        let host = ops::get(&mut realm, &arguments, &Key::Index(0)).unwrap();
        let host = host.as_object().unwrap().clone();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let web = realm.object();
        for method in METHODS.into_iter().chain(["focus"]) {
            let record = calls.clone();
            let function = realm.function(method, 0, false, move |realm, _, args| {
                let mut call = String::from(method);
                for argument in args {
                    call.push(' ');
                    let detail = match argument {
                        Value::Object(_) => ops::get(realm, argument, &Key::name("detail"))?,
                        _ => Value::Undefined,
                    };
                    let text = match detail {
                        Value::Number(detail) => alloc::format!("detail={detail}"),
                        _ => ops::to_string(realm, argument)?.to_rust_lossy(),
                    };
                    call.push_str(&text);
                }
                record.borrow_mut().push(call);
                // Keys are cancelled, as pages that handle them do.
                Ok(Value::Boolean(method == "key"))
            });
            web.define(Key::name(method), Property::data(Value::Object(function)));
        }
        host.define(Key::name("web"), Property::data(Value::Object(web)));
        let hwnd =
            nativeboat_native::app_host::create(&nativeboat_native::app_host::WindowOptions {
                size: (300.0, 200.0),
                show: false,
                ..nativeboat_native::app_host::WindowOptions::new(&[])
            })
            .unwrap();
        let realm_pointer: *mut Realm = &mut *realm;
        // SAFETY: the fixture keeps the realm alive as long as the window.
        let window =
            unsafe { PageWindow::adopt(page, realm_pointer, hwnd.raw() as *mut c_void) }.unwrap();
        Fixture {
            realm,
            window: Rc::new(window),
            calls,
            host,
        }
    }

    fn take(&self) -> Vec<String> {
        core::mem::take(&mut *self.calls.borrow_mut())
    }

    fn pointer(&self, kind: PointerKind, count: u32, x: f32, y: f32) -> bool {
        self.window.handle(&Event::Pointer {
            kind,
            button: 0,
            buttons: 1,
            count,
            x,
            y,
            modifiers: Modifiers::default(),
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let hwnd = nativeboat_win32::hwnd(self.window.id());
        if let Some(hwnd) = hwnd {
            let window = nativeboat_native::app_host::Window::from_raw(hwnd as usize);
            let _ = nativeboat_native::app_host::destroy_window(window);
        }
    }
}

#[test]
fn pointer_input_targets_the_element_under_it_and_clicks_what_was_pressed() {
    let fixture = Fixture::new();
    fixture.window.render();
    let editor = fixture
        .window
        .page()
        .element_from_point(10.0, 10.0)
        .unwrap()
        .0;
    assert!(!fixture.pointer(PointerKind::Move, 0, 10.0, 10.0));
    assert!(!fixture.pointer(PointerKind::Down, 1, 10.0, 10.0));
    assert!(!fixture.pointer(PointerKind::Up, 1, 10.0, 10.0));
    assert!(!fixture.pointer(PointerKind::Down, 2, 10.0, 10.0));
    assert!(!fixture.pointer(PointerKind::Up, 2, 10.0, 10.0));
    // A press that ends elsewhere is no click.
    assert!(!fixture.pointer(PointerKind::Down, 1, 10.0, 10.0));
    assert!(!fixture.pointer(PointerKind::Up, 1, 10.0, 150.0));
    assert!(!fixture.pointer(PointerKind::Leave, 0, 0.0, 0.0));
    let elsewhere = node(fixture.window.page().element_from_point(10.0, 150.0));
    let elsewhere = match elsewhere {
        Value::Number(id) => alloc::format!("{id}"),
        _ => "null".to_string(),
    };
    assert_eq!(
        fixture.take(),
        [
            alloc::format!("pointer move {editor} detail=0"),
            alloc::format!("pointer down {editor} detail=1"),
            alloc::format!("pointer up {editor} detail=1"),
            alloc::format!("pointer click {editor} detail=1"),
            alloc::format!("pointer down {editor} detail=2"),
            alloc::format!("pointer up {editor} detail=2"),
            alloc::format!("pointer click {editor} detail=2"),
            alloc::format!("pointer dblclick {editor} detail=2"),
            alloc::format!("pointer down {editor} detail=1"),
            alloc::format!("pointer up {elsewhere} detail=1"),
            "pointer leave null detail=0".to_string(),
        ]
    );
}

#[test]
fn keyboard_text_composition_and_wheel_input_reach_the_page() {
    let fixture = Fixture::new();
    fixture.window.render();
    let key = Event::Key {
        down: true,
        key: "a".into(),
        code: "KeyA".into(),
        repeat: false,
        modifiers: Modifiers::default(),
        control: None,
    };
    // The page cancels keys.
    assert!(fixture.window.handle(&key));
    assert!(!fixture.window.handle(&Event::Text("a".into())));
    for kind in [
        CompositionKind::Start,
        CompositionKind::Update,
        CompositionKind::End,
    ] {
        fixture.window.handle(&Event::Composition {
            kind,
            data: "ni".into(),
            cursor: 2,
        });
    }
    fixture.window.handle(&Event::Wheel {
        x: 10.0,
        y: 10.0,
        delta_x: 0.0,
        delta_y: 100.0,
        modifiers: Modifiers::default(),
    });
    let calls = fixture.take();
    assert_eq!(calls[0], "key true [object Object] false");
    assert_eq!(calls[1], "text a");
    assert_eq!(
        calls[2..5],
        [
            "composition start ni 2",
            "composition update ni 2",
            "composition end ni 2"
        ]
    );
    assert!(calls[5].starts_with("wheel "), "{calls:?}");
}

#[test]
fn scrolling_controls_and_resizing_update_the_page() {
    let fixture = Fixture::new();
    for surface in [0, 1] {
        assert!(!fixture.window.handle(&Event::Scroll {
            surface,
            x: 0.0,
            y: 5.0,
        }));
    }
    for event in [
        ControlEvent::Click,
        ControlEvent::Input("typed".into()),
        ControlEvent::Select(vec![1, 2]),
        ControlEvent::Value(0.5),
        ControlEvent::Focus,
        ControlEvent::Blur,
    ] {
        assert!(!fixture.window.handle(&Event::Control { id: 3, event }));
    }
    assert!(!fixture.window.handle(&Event::Resized {
        width: 640.0,
        height: 480.0,
    }));
    assert_eq!(fixture.window.page().viewport(), (640.0, 480.0));
    assert!(!fixture.window.handle(&Event::Close));
    assert_eq!(
        fixture.take(),
        [
            "scroll null",
            "scroll 1",
            "activate 3",
            "input 3 typed",
            "select 3 1,2",
            "value 3 0.5",
            "focus 3",
            "focus null",
        ]
    );
}

#[test]
fn input_waits_while_the_realm_runs_and_errors_are_the_pages() {
    let fixture = Fixture::new();
    fixture.window.run(|_| {
        assert!(!fixture.window.handle(&Event::Text("later".into())));
        assert!(fixture.take().is_empty());
    });
    assert_eq!(fixture.take(), ["text later"]);
    // A throwing renderer interface reports the error and cancels nothing.
    let web = fixture.realm.object();
    let failing = fixture.realm.function("text", 0, false, |_, _, _| {
        Err(Thrown(Value::str("failed")))
    });
    web.define(Key::name("text"), Property::data(Value::Object(failing)));
    fixture
        .host
        .define(Key::name("web"), Property::data(Value::Object(web)));
    assert!(!fixture.window.handle(&Event::Text("x".into())));
}

#[test]
fn rendering_applies_only_changed_scenes() {
    let fixture = Fixture::new();
    assert!(fixture.window.needs_render());
    fixture.window.render();
    assert!(!fixture.window.needs_render());
    // The same scene is not applied again.
    fixture.window.render();
    let page = fixture.window.page().clone();
    let document = page.document().unwrap();
    let editor = document
        .borrow()
        .get_element_by_id(nativeboat_dom::Dom::DOCUMENT, "editor")
        .unwrap();
    assert_eq!(page.cursor(editor), Cursor::Pointer);
    document
        .borrow_mut()
        .set_attribute(editor, "style", "height: 80px")
        .unwrap();
    // A changed page needs rendering until it is rendered.
    assert!(fixture.window.needs_render());
    fixture.window.render();
    assert!(!fixture.window.needs_render());
    assert_eq!(page.cursor(editor), Cursor::Auto);
    // A page without a document has no scene.
    let blank = Rc::new(DocumentPage::new(
        Rc::new(BlankPage {
            url: "file:///blank.html".to_string(),
            viewport: (10.0, 10.0),
        }),
        Rc::new(Measurer),
    ));
    let mut realm = Box::new(Realm::new());
    let realm: *mut Realm = &mut *realm;
    let hwnd = nativeboat_native::app_host::create(&nativeboat_native::app_host::WindowOptions {
        show: false,
        ..nativeboat_native::app_host::WindowOptions::new(&[])
    })
    .unwrap();
    // SAFETY: the realm outlives the window, which is destroyed below.
    let empty = unsafe { PageWindow::adopt(blank, realm, hwnd.raw() as *mut c_void) }.unwrap();
    empty.render();
    assert!(empty.page().document().is_none());
    // Scrolling it scrolls nothing.
    assert!(!empty.handle(&Event::Scroll {
        surface: 0,
        x: 0.0,
        y: 1.0
    }));
    let _ = nativeboat_native::app_host::destroy_window(hwnd);
}

#[test]
fn windows_route_input_to_their_pages_until_destroyed() {
    let fixture = Fixture::new();
    let windows = PageWindows::default();
    windows.add(fixture.window.clone());
    assert!(std::format!("{windows:?} {:?}", fixture.window).contains("PageWindow"));
    let mut handler = windows.clone();
    let id = fixture.window.id();
    assert!(!handler.event(id, Event::Text("routed".into())));
    assert!(!handler.event(id + 1000, Event::Text("lost".into())));
    assert_eq!(handler.idle(), None);
    assert!(!handler.event(id, Event::Destroyed));
    assert!(windows.get(id).is_none());
    assert_eq!(fixture.take(), ["text routed"]);
    let _ = nativeboat_win32::take_handler();
}

#[test]
fn input_is_traced_with_nativeboat_trace_input() {
    // SAFETY: other tests only read this variable when windows are made, and
    // tracing changes nothing but standard error.
    unsafe { std::env::set_var("NATIVEBOAT_TRACE_INPUT", "1") };
    let fixture = Fixture::new();
    // Input for a window without a page is reported too.
    let mut windows = PageWindows::default();
    assert!(!windows.event(fixture.window.id() + 1000, Event::Text("lost".into())));
    let _ = nativeboat_win32::take_handler();
    // SAFETY: as above.
    unsafe { std::env::remove_var("NATIVEBOAT_TRACE_INPUT") };
    assert!(fixture.window.traced);
    // So is input that waits for the page.
    fixture.window.run(|_| {
        assert!(!fixture.window.handle(&Event::Text("later".into())));
    });
    fixture.window.render();
    assert!(!fixture.pointer(PointerKind::Down, 1, 10.0, 10.0));
    let web = fixture.realm.object();
    let failing = fixture.realm.function("text", 0, false, |_, _, _| {
        Err(Thrown(Value::str("failed")))
    });
    web.define(Key::name("text"), Property::data(Value::Object(failing)));
    fixture
        .host
        .define(Key::name("web"), Property::data(Value::Object(web)));
    assert!(!fixture.window.handle(&Event::Text("x".into())));
}

#[test]
fn keys_of_page_windows_are_offered_to_the_main_realm() {
    let fixture = Fixture::new();
    let hwnd = nativeboat_win32::hwnd(fixture.window.id()).unwrap() as usize;
    let mut main = Realm::new();
    let context = (&mut main as *mut Realm).cast::<c_void>();
    // SAFETY: the realm outlives the calls on this thread.
    unsafe {
        assert!(!electron_before_input(
            core::ptr::null_mut(),
            hwnd,
            0x0100,
            0x41,
            0
        ));
        // Not a key, and not a window of this thread's pages.
        assert!(!electron_before_input(context, hwnd, 0x0102, 0x41, 0));
        assert!(!electron_before_input(context, 0, 0x0100, 0x41, 0));
        // The main realm has no window for it, so nothing keeps the key.
        assert!(!electron_before_input(
            context,
            hwnd,
            0x0100,
            0x41,
            0x1E << 16
        ));
    }
    assert!(fixture.take().is_empty());
}

#[test]
fn page_descriptions_catch_up_with_the_last_change() {
    let path =
        std::env::temp_dir().join(alloc::format!("nativeboat-dump-{}.txt", std::process::id()));
    let mut fixture = Fixture::new();
    Rc::get_mut(&mut fixture.window).unwrap().dump = Some(path.to_str().unwrap().into());
    fixture.window.render();
    let first = std::fs::read_to_string(&path).unwrap();
    assert!(first.contains("<main id=\"editor\""), "{first}");
    let page = fixture.window.page().clone();
    let document = page.document().unwrap();
    let editor = document
        .borrow()
        .get_element_by_id(nativeboat_dom::Dom::DOCUMENT, "editor")
        .unwrap();
    document
        .borrow_mut()
        .set_attribute(editor, "class", "changed")
        .unwrap();
    // Within its second the change waits, and the window wants rendering.
    fixture.window.render();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    assert!(fixture.window.needs_render());
    let (revision, time) = fixture.window.dumped.get().unwrap();
    fixture.window.dumped.set(Some((revision, time - 1000.0)));
    fixture.window.render();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("class=\"changed\"")
    );
    assert!(!fixture.window.needs_render());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn files_dragged_from_explorer_reach_the_page_and_bring_their_contents_when_dropped() {
    let mut fixture = Fixture::new();
    fixture.window.render();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let web = ops::get(
        &mut fixture.realm,
        &Value::Object(fixture.host.clone()),
        &Key::name("web"),
    )
    .unwrap();
    let web = web.as_object().unwrap().clone();
    for (method, files) in [("dragFiles", 2), ("dropFiles", 1)] {
        let record = seen.clone();
        let function = fixture
            .realm
            .function(method, 0, false, move |realm, _, args| {
                let file = ops::get(realm, &args[files], &Key::Index(0))?;
                let path = ops::get(realm, &file, &Key::name("path"))?;
                let bytes = ops::get(realm, &file, &Key::name("bytes"))?;
                let target = &args[files - 1];
                let mut call = String::from(method);
                for value in [target, &path, &bytes] {
                    call.push(' ');
                    call.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
                }
                record.borrow_mut().push(call);
                // The page takes the files.
                Ok(Value::Boolean(true))
            });
        web.define(Key::name(method), Property::data(Value::Object(function)));
    }
    let paths = vec![path.to_string()];
    for kind in [FileDragKind::Enter, FileDragKind::Over] {
        assert!(fixture.window.handle(&Event::FileDrag {
            kind,
            paths: paths.clone(),
            strings: Vec::new(),
            x: 10.0,
            y: 10.0,
            modifiers: Modifiers::default(),
        }));
    }
    assert!(fixture.window.handle(&Event::FilesDropped {
        paths,
        strings: Vec::new(),
        x: 10.0,
        y: 10.0,
    }));
    let contents = nativeboat_native::base64_encode(&std::fs::read(path).unwrap(), false);
    let editor = fixture
        .window
        .page()
        .document()
        .unwrap()
        .borrow()
        .get_element_by_id(nativeboat_dom::Dom::DOCUMENT, "editor")
        .unwrap();
    assert_eq!(
        *seen.borrow(),
        [
            alloc::format!("dragFiles {} {path} undefined", editor.0),
            alloc::format!("dragFiles {} {path} undefined", editor.0),
            alloc::format!("dropFiles {} {path} {contents}", editor.0),
        ]
    );
}

#[test]
fn compiled_pages_use_the_system_clipboard_and_select_text_in_controls() {
    use shimforge::{Session, mock};
    let fixture = Fixture::new();
    let page = fixture.window.page().clone();
    let mut session = Session::new();
    let read = mock!(
        session,
        nativeboat_win32::read_clipboard_contents,
        fn() -> Option<Vec<(&'static str, String)>>
    );
    read.expect()
        .returning(|| Some(vec![("text/plain", "pasted".to_string())]));
    let write = mock!(
        session,
        nativeboat_win32::write_clipboard,
        fn(Option<&str>, Option<&str>) -> Result<(), nativeboat_win32::Error>
    );
    write.expect().returning(|text, html| {
        if (text, html) == (Some("copied"), Some("<b>copied</b>")) {
            Ok(())
        } else {
            Err(nativeboat_win32::Error {
                message: "unexpected".to_string(),
            })
        }
    });
    assert_eq!(
        page.read_clipboard_contents(),
        Some(vec![("text/plain".to_string(), "pasted".to_string())])
    );
    assert!(page.write_clipboard_contents(Some("copied"), Some("<b>copied</b>")));
    assert!(!page.write_clipboard_contents(Some("other"), None));
    // A selection a script makes shows once the page renders.
    fixture.window.render();
    let node = NodeId(3);
    page.select_text(node, 1, 3);
    assert_eq!(page.text_selection(node), Some((1, 3)));
    assert!(fixture.window.needs_render());
    fixture.window.render();
    assert!(!page.selects_text());
}

#[test]
fn local_storage_is_kept_in_the_applications_file() {
    let fixture = Fixture::new();
    let page = fixture.window.page().clone();
    // Without a file nothing is kept.
    page.save_storage("{}");
    assert_eq!(page.load_storage(), None);
    let path = std::env::temp_dir().join(alloc::format!(
        "nativeboat-storage-{}.json",
        std::process::id()
    ));
    let path = path.to_string_lossy().into_owned();
    page.set_storage_file(path.clone());
    assert_eq!(page.load_storage(), None);
    page.save_storage(r#"{"side-bar-width":"280"}"#);
    assert_eq!(
        page.load_storage().as_deref(),
        Some(r#"{"side-bar-width":"280"}"#)
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn compiled_pages_draw_canvases_and_measure_their_text() {
    let fixture = Fixture::new();
    let page = fixture.window.page().clone();
    let node = NodeId(3);
    page.canvas_draw(
        node,
        4,
        4,
        r#"[{"Fill":{"path":[{"Move":[0,0]},{"Line":[4,0]},{"Line":[4,4]},{"Line":[0,4]},"Close"],
            "paint":{"Color":{"r":255,"g":0,"b":0,"a":1}},"rule":"NonZero","alpha":1,"copy":false}}]"#,
    );
    assert_eq!(page.canvas_read(node, (1, 1, 1, 1)), [255, 0, 0, 255]);
    assert_eq!(page.canvas_read(NodeId(4), (0, 0, 1, 1)), [0, 0, 0, 0]);
    assert!(
        page.canvas_url(node, false)
            .starts_with("data:image/png;base64,")
    );
    assert_eq!(page.canvas_url(NodeId(4), true), "data:,");
    page.canvas_write(node, (0, 0, 1, 1), &[0, 0, 255, 255]);
    assert_eq!(page.canvas_read(node, (0, 0, 1, 1)), [0, 0, 255, 255]);
    // Fonts parse into what scripts hand back to measure text.
    let font = page.parse_font("bold 20px Arial").unwrap();
    assert!(font.contains(r#""weight":700"#), "{font}");
    let [width, ascent, _] = page.measure_text(&font, "MMMM");
    assert!(width > 40.0 && ascent > 10.0, "{width} {ascent}");
    assert_eq!(page.parse_font("not a font"), None);
    assert_eq!(
        page.path_data("M0 0L1 2Z"),
        r#"[{"Move":[0,0]},{"Line":[1,2]},"Close"]"#
    );
}

#[test]
fn selects_the_page_draws_open_their_options_and_report_the_choice() {
    use shimforge::{Session, mock};
    let fixture = Fixture::new();
    let page = fixture.window.page().clone();
    let select = {
        let document = page.document().unwrap();
        let mut dom = document.borrow_mut();
        let select = dom.create_element("select").unwrap();
        for label in ["one", "two  & three"] {
            let option = dom.create_element("option").unwrap();
            let text = dom.create_text(label);
            dom.append_child(option, text).unwrap();
            dom.append_child(select, option).unwrap();
        }
        let editor = dom
            .get_element_by_id(nativeboat_dom::Dom::DOCUMENT, "editor")
            .unwrap();
        dom.append_child(editor, select).unwrap();
        select
    };
    fixture.window.render();
    fixture.take();
    let mut session = Session::new();
    let popup = mock!(
        session,
        nativeboat_win32::popup_menu,
        fn(Option<WindowId>, *mut c_void, &[MenuItem], Option<(f32, f32)>) -> Option<u16>
    );
    popup.expect().returning(|_, _, items, at| {
        let shown: Vec<(&str, bool)> = items
            .iter()
            .map(|item| (item.label.as_str(), item.checked))
            .collect();
        assert_eq!(shown, [("one", true), ("two && three", false)]);
        assert!(at.is_some());
        Some(2)
    });
    page.open_select(select);
    assert!(!fixture.window.handle(&Event::Text("x".into())));
    let calls = fixture.take();
    assert!(
        calls.contains(&alloc::format!("select {} 1", select.0)),
        "{calls:?}"
    );
}
