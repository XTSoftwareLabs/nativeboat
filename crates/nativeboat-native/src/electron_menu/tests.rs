use super::*;
use alloc::rc::Rc;
use core::cell::Cell;

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

fn array(values: Vec<Value>) -> Object {
    let length = values.len();
    let array = Object::new(
        None,
        crate::Kind::Array(crate::ArrayData::new(
            values.into_iter().map(Some).collect(),
        )),
    );
    array.define(
        Key::name("length"),
        Property::data(Value::Number(length as f64)),
    );
    array
}

fn item_options(realm: &Realm, id: &str, label: &str, click: Value) -> Value {
    let options = realm.object();
    options.define(Key::name("id"), Property::data(Value::str(id)));
    options.define(Key::name("label"), Property::data(Value::str(label)));
    options.define(Key::name("click"), Property::data(click));
    Value::Object(options)
}

#[test]
fn menu_template_installs_retrievable_items_and_defers_until_a_window_exists() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let menu_constructor = property(&mut realm, &exports, "Menu");
    let build = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "buildFromTemplate",
    );
    let child_template = array(vec![item_options(
        &realm,
        "nested",
        "Nested",
        Value::Undefined,
    )]);
    let submenu = realm.object();
    submenu.define(Key::name("label"), Property::data(Value::str("More")));
    submenu.define(
        Key::name("submenu"),
        Property::data(Value::Object(child_template)),
    );
    let template = array(vec![
        item_options(&realm, "open", "Open", Value::Undefined),
        Value::Object(submenu),
    ]);
    let menu = match ops::call(
        &mut realm,
        &build,
        menu_constructor.clone(),
        &[Value::Object(template)],
    )
    .unwrap()
    {
        Value::Object(menu) => menu,
        _ => panic!("buildFromTemplate did not return a Menu"),
    };
    let get_item = property(&mut realm, &menu, "getMenuItemById");
    assert!(matches!(
        ops::call(
            &mut realm,
            &get_item,
            Value::Object(menu.clone()),
            &[Value::str("nested")]
        )
        .unwrap(),
        Value::Object(_)
    ));
    let set = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "setApplicationMenu",
    );
    ops::call(
        &mut realm,
        &set,
        menu_constructor.clone(),
        &[Value::Object(menu.clone())],
    )
    .unwrap();
    let get = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "getApplicationMenu",
    );
    let current = ops::call(&mut realm, &get, menu_constructor.clone(), &[]).unwrap();
    assert!(ops::strict_equals(&current, &Value::Object(menu)));
    let unset = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "setApplicationMenu",
    );
    ops::call(&mut realm, &unset, menu_constructor.clone(), &[Value::Null]).unwrap();
    assert!(matches!(
        ops::call(&mut realm, &get, menu_constructor, &[]).unwrap(),
        Value::Null
    ));
}

#[test]
fn native_menu_dispatch_calls_item_callback_with_item_receiver_and_event() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let observed = Rc::new(Cell::new(false));
    let called = observed.clone();
    let callback = realm.function("click", 3, false, move |realm, this, args| {
        assert!(matches!(this, Value::Object(_)));
        assert!(matches!(args.get(1), Some(Value::Undefined)));
        let event = args.get(2).cloned().unwrap_or(Value::Undefined);
        let prevent = ops::get(realm, &event, &Key::name("preventDefault"))?;
        ops::call(realm, &prevent, event, &[])?;
        called.set(true);
        Ok(Value::Undefined)
    });
    let options = item_options(&realm, "copy", "Copy", Value::Object(callback));
    let item = ops::construct(&mut realm, &item_constructor, &[options]).unwrap();
    let command = private_command(&realm, &item).expect("native menu command");
    let items = realm.array(vec![item.clone()]);
    dispatch(&mut realm, command, &Value::Object(items), None, false).unwrap();
    assert!(observed.get());
    let role_options = item_options(&realm, "role-copy", "Copy", Value::Undefined);
    let Value::Object(role_options) = role_options else {
        unreachable!()
    };
    role_options.define(Key::name("role"), Property::data(Value::str("copy")));
    let role_item = ops::construct(
        &mut realm,
        &item_constructor,
        &[Value::Object(role_options)],
    )
    .unwrap();
    let role_command = private_command(&realm, &role_item).expect("role menu command");
    let role_items = Value::Object(realm.array(vec![role_item]));
    dispatch(&mut realm, role_command, &role_items, None, false).unwrap();
    let empty = Value::Object(realm.array(Vec::new()));
    assert!(dispatch(&mut realm, u16::MAX, &empty, None, false).is_ok());
}

#[test]
fn synchronous_menu_callback_exceptions_are_not_swallowed() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let callback = realm.function("click", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Error, "menu callback failed"))
    });
    let options = item_options(&realm, "bad", "Bad", Value::Object(callback));
    let item = ops::construct(&mut realm, &item_constructor, &[options]).unwrap();
    let command = private_command(&realm, &item).expect("native menu command");
    let items = Value::Object(realm.array(vec![item]));
    let error = dispatch(&mut realm, command, &items, None, false).unwrap_err();
    assert!(matches!(
        ops::get(&mut realm, &error.0, &Key::name("message")).unwrap(),
        Value::String(message) if message.units() == "menu callback failed".encode_utf16().collect::<Vec<_>>()
    ));
}

#[test]
fn window_specific_native_menu_commands_dispatch_and_release_their_menu() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let called = Rc::new(Cell::new(false));
    let callback_called = called.clone();
    let callback = realm.function("click", 3, false, move |_, _, _| {
        callback_called.set(true);
        Ok(Value::Undefined)
    });
    let options = item_options(
        &realm,
        "window-action",
        "Window action",
        Value::Object(callback),
    );
    let item = ops::construct(&mut realm, &item_constructor, &[options]).unwrap();
    let item_object = item.as_object().unwrap().clone();
    let menu = realm.object();
    menu.define(
        Key::name("items"),
        Property::data(Value::Object(realm.array(vec![item.clone()]))),
    );
    let window = app_host::create_window(&[], 320, 200).unwrap();
    set_window_menu(&mut realm, window, Value::Object(menu.clone())).unwrap();
    assert_eq!(realm.electron_window_menus.len(), 1);
    let command = private_command(&realm, &item).unwrap();
    // SAFETY: the callback receives the live Realm and the owned host HWND.
    unsafe {
        dispatch_menu_command(
            (&mut realm as *mut Realm).cast(),
            window.raw(),
            command,
            false,
        )
    };
    assert!(called.get());

    ops::set(
        &mut realm,
        &Value::Object(item_object),
        &Key::name("enabled"),
        Value::Boolean(false),
    )
    .unwrap();

    set_window_menu(&mut realm, window, Value::Null).unwrap();
    assert!(realm.electron_window_menus.is_empty());
    app_host::destroy_window(window).unwrap();
}

#[test]
fn menu_append_remove_and_template_validation_match_native_api_contract() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let menu_constructor = property(&mut realm, &exports, "Menu");
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let menu = ops::construct(&mut realm, &menu_constructor, &[]).unwrap();
    let options = item_options(&realm, "one", "One", Value::Undefined);
    let item = ops::construct(&mut realm, &item_constructor, &[options]).unwrap();
    let append = property(
        &mut realm,
        match &menu {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "append",
    );
    assert!(is_menu_item(&realm, &item));
    let appended = ops::call(&mut realm, &append, menu.clone(), &[item]);
    if let Err(Thrown(Value::Object(error))) = &appended {
        let message = ops::get(
            &mut realm,
            &Value::Object(error.clone()),
            &Key::name("message"),
        );
        panic!("append failed: {message:?}");
    }
    let remove = property(
        &mut realm,
        match &menu {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "remove",
    );
    ops::call(&mut realm, &remove, menu.clone(), &[Value::str("one")]).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &menu, &Key::name("items")).unwrap(),
        Value::Object(items) if array_length(&mut realm, &Value::Object(items.clone())).unwrap() == 0
    ));
    assert!(ops::call(&mut realm, &append, menu, &[Value::Number(3.0)],).is_err());
}

#[test]
fn installed_menu_item_updates_resynchronize_native_enabled_checked_and_visibility_state() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let menu_constructor = property(&mut realm, &exports, "Menu");
    let build = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "buildFromTemplate",
    );
    let options = realm.object();
    options.define(Key::name("id"), Property::data(Value::str("open")));
    options.define(Key::name("label"), Property::data(Value::str("Open")));
    options.define(Key::name("checked"), Property::data(Value::Boolean(false)));
    options.define(Key::name("type"), Property::data(Value::str("radio")));
    options.define(
        Key::name("accelerator"),
        Property::data(Value::str("Ctrl+O")),
    );
    let hidden = realm.object();
    hidden.define(Key::name("label"), Property::data(Value::str("Hidden")));
    hidden.define(Key::name("visible"), Property::data(Value::Boolean(false)));
    let role = realm.object();
    role.define(Key::name("role"), Property::data(Value::str("copy")));
    let template = array(vec![
        Value::Object(options),
        Value::Object(hidden),
        Value::Object(role),
    ]);
    let menu = ops::call(
        &mut realm,
        &build,
        menu_constructor.clone(),
        &[Value::Object(template)],
    )
    .unwrap();
    let set = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "setApplicationMenu",
    );
    ops::call(
        &mut realm,
        &set,
        menu_constructor,
        core::slice::from_ref(&menu),
    )
    .unwrap();
    let items = ops::get(&mut realm, &menu, &Key::name("items")).unwrap();
    let item = find_item(&mut realm, &items, &Value::str("open"));
    let item = item.unwrap().unwrap();
    ops::set(
        &mut realm,
        &Value::Object(item.clone()),
        &Key::name("enabled"),
        Value::Boolean(false),
    )
    .unwrap();
    ops::set(
        &mut realm,
        &Value::Object(item),
        &Key::name("checked"),
        Value::Boolean(true),
    )
    .unwrap();
    let items = ops::get(&mut realm, &menu, &Key::name("items")).unwrap();
    let native = native_items(&mut realm, &items).unwrap();
    assert_eq!(native.len(), 2);
    // A role names its unnamed item and shows its accelerator, which the
    // menu does not register.
    let units = |text: &str| text.encode_utf16().collect::<Vec<_>>();
    assert_eq!(native[1].label, units("Copy\0"));
    assert_eq!(native[1].shortcut, Some(units("Ctrl+C")));
    assert_eq!(native[1].accelerator, None);
    assert_eq!(native[0].shortcut, Some(units("Ctrl+O")));
    assert!(!native[0].enabled);
    assert!(native[0].checked);
    assert!(native[0].radio);
    let expected_accelerator = "Ctrl+O".encode_utf16().collect::<Vec<_>>();
    assert_eq!(
        native[0].accelerator.as_deref(),
        Some(expected_accelerator.as_slice())
    );
}

#[test]
fn template_builder_rejects_non_arrays_and_menu_index_mutators_keep_order() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let menu_constructor = property(&mut realm, &exports, "Menu");
    let build = property(
        &mut realm,
        match &menu_constructor {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "buildFromTemplate",
    );
    assert!(ops::call(&mut realm, &build, menu_constructor.clone(), &[Value::Null]).is_err());
    let menu = ops::construct(&mut realm, &menu_constructor, &[]).unwrap();
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let mut items = Vec::new();
    for id in ["first", "third"] {
        let options = item_options(&realm, id, id, Value::Undefined);
        items.push(ops::construct(&mut realm, &item_constructor, &[options]).unwrap());
    }
    let append = property(
        &mut realm,
        match &menu {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "append",
    );
    for item in items {
        ops::call(&mut realm, &append, menu.clone(), &[item]).unwrap();
    }
    let insert = property(
        &mut realm,
        match &menu {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "insert",
    );
    let middle_options = item_options(&realm, "middle", "middle", Value::Undefined);
    let middle = ops::construct(&mut realm, &item_constructor, &[middle_options]).unwrap();
    ops::call(
        &mut realm,
        &insert,
        menu.clone(),
        &[Value::Number(1.0), middle],
    )
    .unwrap();
    let remove_at = property(
        &mut realm,
        match &menu {
            Value::Object(object) => object,
            _ => panic!(),
        },
        "removeAt",
    );
    ops::call(&mut realm, &remove_at, menu.clone(), &[Value::Number(0.0)]).unwrap();
    let items = ops::get(&mut realm, &menu, &Key::name("items")).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &items, &Key::Index(0)).unwrap(),
        Value::Object(item) if matches!(ops::get(&mut realm, &Value::Object(item.clone()), &Key::name("id")).unwrap(), Value::String(ref id) if id.units() == "middle".encode_utf16().collect::<Vec<_>>())
    ));
}

#[test]
fn menu_constructors_static_helpers_and_window_menu_validation_report_errors() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let menu_constructor = property(&mut realm, &exports, "Menu");
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    assert!(ops::call(&mut realm, &menu_constructor, Value::Undefined, &[]).is_err());
    assert!(ops::call(&mut realm, &item_constructor, Value::Undefined, &[]).is_err());
    assert!(ops::construct(&mut realm, &item_constructor, &[Value::Number(1.0)]).is_err());

    let get_application_menu = property(
        &mut realm,
        menu_constructor.as_object().unwrap(),
        "getApplicationMenu",
    );
    assert!(matches!(
        ops::call(
            &mut realm,
            &get_application_menu,
            menu_constructor.clone(),
            &[]
        ),
        Ok(Value::Null)
    ));
    let set_application_menu = property(
        &mut realm,
        menu_constructor.as_object().unwrap(),
        "setApplicationMenu",
    );
    assert!(
        ops::call(
            &mut realm,
            &set_application_menu,
            menu_constructor.clone(),
            &[Value::Number(1.0)],
        )
        .is_err()
    );
    let send_action = property(
        &mut realm,
        menu_constructor.as_object().unwrap(),
        "sendActionToFirstResponder",
    );
    assert!(matches!(
        ops::call(
            &mut realm,
            &send_action,
            menu_constructor,
            &[Value::str("paste:")],
        ),
        Ok(Value::Undefined)
    ));
    assert!(set_window_menu(&mut realm, app_host::Window::from_raw(0), Value::Null).is_err());
}

#[test]
fn menu_item_changes_rebuild_the_native_menus_once_per_host_step() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let make_item = |realm: &mut Realm, id: &str| {
        let options = item_options(realm, id, id, Value::Undefined);
        ops::construct(realm, &item_constructor, &[options]).unwrap()
    };
    let application_item = make_item(&mut realm, "application");
    let window_item = make_item(&mut realm, "window");
    let menu = |realm: &mut Realm, item: &Value| {
        let menu = realm.object();
        menu.define(
            Key::name("items"),
            Property::data(Value::Object(realm.array(vec![item.clone()]))),
        );
        menu
    };
    let application_menu = menu(&mut realm, &application_item);
    let window_menu = menu(&mut realm, &window_item);
    realm.electron_application_menu = Some(application_menu);
    let window = app_host::create_window(&[], 320, 200).unwrap();
    set_window_menu(&mut realm, window, Value::Object(window_menu)).unwrap();
    let set_enabled = |realm: &mut Realm, item: &Value, enabled: bool| {
        ops::set(realm, item, &Key::name("enabled"), Value::Boolean(enabled)).unwrap();
    };
    // Changes only mark the menus; the flush installs both, once.
    set_enabled(&mut realm, &application_item, false);
    set_enabled(&mut realm, &window_item, false);
    assert!(realm.electron_menus_changed);
    flush_menus(&mut realm).unwrap();
    assert!(!realm.electron_menus_changed);
    assert_eq!(realm.electron_installed_menus.len(), 2);
    assert!(!realm.electron_installed_menus[1].1[0].enabled);
    // Nothing changed: nothing is rebuilt.
    flush_menus(&mut realm).unwrap();
    // Setting the same state, or a click handler, changes no menu.
    set_enabled(&mut realm, &window_item, false);
    ops::set(&mut realm, &window_item, &Key::name("click"), Value::Null).unwrap();
    assert!(!realm.electron_menus_changed);
    flush_menus(&mut realm).unwrap();
    assert_eq!(realm.electron_installed_menus.len(), 2);
    // Only the window menu changed.
    set_enabled(&mut realm, &window_item, true);
    flush_menus(&mut realm).unwrap();
    assert!(realm.electron_installed_menus[1].1[0].enabled);
    // A window that is gone fails the rebuild, as an uncaught error of the
    // host step.
    app_host::destroy_window(window).unwrap();
    set_enabled(&mut realm, &window_item, false);
    assert!(flush_menus(&mut realm).is_err());
    // The failed menu was not installed, so the next change tries again.
    ops::set(
        &mut realm,
        &window_item,
        &Key::name("label"),
        Value::str("Again"),
    )
    .unwrap();
    let mut session = shimforge::Session::new();
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = written.clone();
    shimforge::mock!(session, crate::platform::write_stderr, fn(&str) -> bool)
        .expect()
        .returning(move |text| {
            sink.lock().unwrap().push(text.to_owned());
            true
        });
    assert_eq!(realm.step_host().unwrap_err(), 1);
    let written = written.lock().unwrap();
    assert!(
        written[0].contains("native menu failed (Win32 error 1400)"),
        "{written:?}"
    );
}

#[test]
fn chosen_items_flip_their_check_and_receive_the_window_and_how_they_were_chosen() {
    let _host_lock = app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let item_constructor = property(&mut realm, &exports, "MenuItem");
    let instance = realm.object();
    realm.register_electron_window(77, instance.clone(), realm.object());
    let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
    let record = seen.clone();
    let callback = realm.function("click", 3, false, move |realm, _, args| {
        let window = args.get(1).cloned().unwrap_or_default();
        let event = args.get(2).cloned().unwrap_or_default();
        let accelerator = ops::get(realm, &event, &Key::name("triggeredByAccelerator"))?;
        let ctrl = ops::get(realm, &event, &Key::name("ctrlKey"))?;
        record.borrow_mut().push((
            matches!(window, Value::Object(_)),
            ops::to_boolean(&accelerator),
            matches!(ctrl, Value::Boolean(_)),
        ));
        Ok(Value::Undefined)
    });
    let make = |realm: &mut Realm, id: &str, kind: &str, checked: bool| {
        let options = item_options(realm, id, id, Value::Object(callback.clone()));
        let Value::Object(object) = &options else {
            unreachable!()
        };
        object.define(Key::name("type"), Property::data(Value::str(kind)));
        object.define(
            Key::name("checked"),
            Property::data(Value::Boolean(checked)),
        );
        ops::construct(realm, &item_constructor, &[options]).unwrap()
    };
    let items = vec![
        make(&mut realm, "box", "checkbox", false),
        make(&mut realm, "one", "radio", true),
        make(&mut realm, "two", "radio", false),
        make(&mut realm, "gap", "separator", false),
        make(&mut realm, "three", "radio", true),
    ];
    let commands: Vec<u16> = items
        .iter()
        .map(|item| private_command(&realm, item).unwrap_or(0))
        .collect();
    let list = Value::Object(realm.array(items.clone()));
    let checked = |realm: &mut Realm| -> Vec<bool> {
        items
            .iter()
            .map(|item| ops::to_boolean(&ops::get(realm, item, &Key::name("checked")).unwrap()))
            .collect()
    };
    dispatch(&mut realm, commands[0], &list, Some(77), true).unwrap();
    assert_eq!(checked(&mut realm), [true, true, false, false, true]);
    dispatch(&mut realm, commands[0], &list, None, false).unwrap();
    assert_eq!(checked(&mut realm), [false, true, false, false, true]);
    // A radio item checks itself and unchecks the rest of its group only.
    dispatch(&mut realm, commands[2], &list, Some(5), false).unwrap();
    assert_eq!(checked(&mut realm), [false, false, true, false, true]);
    dispatch(&mut realm, commands[2], &list, Some(77), false).unwrap();
    assert_eq!(checked(&mut realm), [false, false, true, false, true]);
    assert_eq!(
        *seen.borrow(),
        [
            (true, true, true),
            (false, false, true),
            (false, false, true),
            (true, false, true)
        ]
    );
}

#[test]
fn popups_find_the_window_a_browser_window_records() {
    let mut realm = Realm::new();
    // BrowserWindow keeps its handle under the private window symbol.
    let window = realm.object();
    window.define(
        Key::Symbol(realm.electron_window_key.clone()),
        Property::hidden(Value::Number(4242.0)),
    );
    let options = realm.object();
    options.define(Key::name("window"), Property::data(Value::Object(window)));
    let found = window_option(&mut realm, &Value::Object(options)).unwrap();
    assert_eq!(found.map(app_host::Window::raw), Some(4242));
    // Options without a window, and no options, name none.
    let bare = Value::Object(realm.object());
    assert!(window_option(&mut realm, &bare).unwrap().is_none());
    assert!(
        window_option(&mut realm, &Value::Undefined)
            .unwrap()
            .is_none()
    );
}

#[test]
fn popups_drop_bracketed_mnemonics_as_chromium_shows_them() {
    let item = |label: &str, children: Vec<app_host::NativeMenuItem>| {
        let mut units: Vec<u16> = label.encode_utf16().collect();
        units.push(0);
        app_host::NativeMenuItem {
            id: 0,
            label: units,
            separator: false,
            submenu: !children.is_empty(),
            enabled: true,
            checked: false,
            radio: false,
            accelerator: None,
            shortcut: None,
            children,
        }
    };
    let mut items = vec![
        item("主题(&T)", vec![item("深色(&D)", Vec::new())]),
        item("&File", Vec::new()),
        item("(&)", Vec::new()),
    ];
    without_bracketed_mnemonics(&mut items);
    let text = |item: &app_host::NativeMenuItem| {
        String::from_utf16_lossy(&item.label[..item.label.len() - 1])
    };
    assert_eq!(text(&items[0]), "主题");
    assert_eq!(text(&items[0].children[0]), "深色");
    // Other mnemonics stay for the menu to underline.
    assert_eq!(text(&items[1]), "&File");
    assert_eq!(text(&items[2]), "(&)");
}

#[test]
fn shortcuts_show_as_chromium_shows_them_and_roles_have_defaults() {
    let zh = |accelerator: &str| shortcut_text(accelerator, "zh-CN");
    let en = |accelerator: &str| shortcut_text(accelerator, "en-US");
    // Modifiers in Chromium's order, whatever the page's.
    assert_eq!(zh("CmdOrCtrl+Shift+N"), "Ctrl+Shift+N");
    assert_eq!(zh("Shift+Ctrl+Alt+k"), "Alt+Ctrl+Shift+K");
    assert_eq!(zh("Super+K"), "Win+K");
    assert_eq!(zh("Alt+F4"), "Alt+F4");
    assert_eq!(zh("F11"), "F11");
    // Key names of the application locale, as Electron showed them.
    assert_eq!(zh("Ctrl+Plus"), "Ctrl++");
    assert_eq!(zh("Ctrl++"), "Ctrl++");
    assert_eq!(zh("Ctrl+-"), "Ctrl+-");
    assert_eq!(zh("Ctrl+,"), "Ctrl+逗号");
    assert_eq!(en("Ctrl+."), "Ctrl+Period");
    assert_eq!(zh("Space"), "空格键");
    assert_eq!(en("Space"), "Space");
    assert_eq!(zh("Tab"), "Tab 键");
    assert_eq!(zh("PageUp"), "向上翻页");
    assert_eq!(en("PageDown"), "Page Down");
    assert_eq!(zh("Up"), "向上箭头");
    assert_eq!(en("Left"), "Left Arrow");
    assert_eq!(zh("Delete"), "Del");
    assert_eq!(zh("Insert"), "Ins");
    assert_eq!(zh("Esc"), "Esc");
    assert_eq!(zh("Return"), "Enter");
    assert_eq!(zh("Backspace"), "退格");
    assert_eq!(zh("Home"), "首页");
    assert_eq!(en("End"), "End");
    assert_eq!(en("Right"), "Right Arrow");
    assert_eq!(en("Down"), "Down Arrow");
    assert_eq!(zh("Ctrl+/"), "Ctrl+/");
    assert_eq!(zh("num1"), "num1");
    // Roles' labels and accelerators on Windows.
    assert_eq!(role_defaults("cut"), ("Cut", "CommandOrControl+X"));
    assert_eq!(
        role_defaults("selectAll"),
        ("Select All", "CommandOrControl+A")
    );
    assert_eq!(shortcut_text(role_defaults("redo").1, "en-US"), "Ctrl+Y");
    assert_eq!(shortcut_text(role_defaults("zoomIn").1, "en-US"), "Ctrl++");
    assert_eq!(role_defaults("quit"), ("Exit", ""));
    assert_eq!(role_defaults("unknown"), ("", ""));
    for role in [
        "undo",
        "copy",
        "paste",
        "pasteAndMatchStyle",
        "delete",
        "minimize",
        "close",
        "reload",
        "forceReload",
        "toggleDevTools",
        "togglefullscreen",
        "resetZoom",
        "zoomOut",
        "toggleSpellChecker",
        "about",
        "help",
        "window",
        "fileMenu",
        "editMenu",
        "viewMenu",
    ] {
        assert!(!role_defaults(role).0.is_empty(), "{role}");
    }
}
