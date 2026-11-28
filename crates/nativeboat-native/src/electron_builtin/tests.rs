use super::*;

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn app_readiness_is_immediate_and_quit_posts_to_the_host() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let app = property(&mut realm, &exports, "app");
    assert!(matches!(
        property(&mut realm, app.as_object().unwrap(), "isPackaged"),
        Value::Boolean(true)
    ));
    let get_app_path = property(&mut realm, app.as_object().unwrap(), "getAppPath");
    assert!(matches!(
        ops::call(&mut realm, &get_app_path, app.clone(), &[]),
        Ok(Value::String(_))
    ));
    let is_ready = property(&mut realm, app.as_object().unwrap(), "isReady");
    assert!(matches!(
        ops::call(&mut realm, &is_ready, app.clone(), &[]),
        Ok(Value::Boolean(true))
    ));
    let when_ready = property(&mut realm, app.as_object().unwrap(), "whenReady");
    assert!(matches!(
        ops::call(&mut realm, &when_ready, app.clone(), &[]),
        Ok(Value::Object(_))
    ));
    let quit = property(&mut realm, app.as_object().unwrap(), "quit");
    assert!(matches!(
        ops::call(&mut realm, &quit, app, &[]),
        Ok(Value::Undefined)
    ));
    let app = property(&mut realm, &exports, "app");
    let exit = property(&mut realm, app.as_object().unwrap(), "exit");
    assert!(matches!(
        ops::call(&mut realm, &exit, app, &[Value::Number(7.0)]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        ops::call(
            &mut realm,
            &exit,
            Value::Undefined,
            &[Value::str("invalid")]
        ),
        Ok(Value::Undefined)
    ));
}

#[test]
fn app_reports_the_build_time_package_name_and_version() {
    let mut realm = Realm::new();
    realm.electron_app_version = Some("0.20.0");
    realm.electron_app_name = Some("MarkText");
    let exports = module(&mut realm);
    let app = property(&mut realm, &exports, "app");
    for (method, expected) in [("getVersion", "0.20.0"), ("getName", "MarkText")] {
        let method = property(&mut realm, app.as_object().unwrap(), method);
        assert!(matches!(
            ops::call(&mut realm, &method, app.clone(), &[]),
            Ok(Value::String(value)) if value.to_rust_lossy() == expected
        ));
    }
}

#[test]
fn app_user_data_path_uses_the_profile_default_and_accepts_an_override() {
    let mut realm = Realm::new();
    realm.electron_app_name = Some("MarkText");
    let exports = module(&mut realm);
    let app = property(&mut realm, &exports, "app");
    let get_path = property(&mut realm, app.as_object().unwrap(), "getPath");
    let default_path = ops::call(
        &mut realm,
        &get_path,
        app.clone(),
        &[Value::str("userData")],
    )
    .unwrap();
    assert!(matches!(
        default_path,
        Value::String(ref path) if path.to_rust_lossy().ends_with("MarkText")
    ));

    let set_path = property(&mut realm, app.as_object().unwrap(), "setPath");
    assert!(matches!(
        ops::call(
            &mut realm,
            &set_path,
            app.clone(),
            &[Value::str("userData"), Value::str("C:\\Temp\\MarkText")],
        ),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        ops::call(
            &mut realm,
            &get_path,
            app,
            &[Value::str("userData")],
        ),
        Ok(Value::String(path)) if path.to_rust_lossy() == "C:\\Temp\\MarkText"
    ));
}

#[test]
fn a_user_data_dir_switch_names_and_makes_the_user_data_directory() {
    let realm = Realm::new();
    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| crate::JsStr::from(*value))
            .collect::<Vec<_>>()
    };
    // Chromium's switches on Windows: any of three prefixes, a name in any
    // case, the last one counting, an empty value without `=`, and none
    // after `--`.
    for (line, expected) in [
        (&["app.exe", "--user-data-dir=C:\\a"][..], Some("C:\\a")),
        (
            &["app.exe", "/User-Data-Dir=C:\\b", "-user-data-dir=C:\\c"],
            Some("C:\\c"),
        ),
        (&["app.exe", "--", "--user-data-dir=C:\\d"], None),
        (
            &[
                "app.exe",
                "--user-data-dir=C:\\g",
                "--user-data-dir",
                "--other=1",
            ],
            Some(""),
        ),
        (&["app.exe", "user-data-dir=C:\\e", "--other=1"], None),
        (&["--user-data-dir=C:\\f"], None),
    ] {
        assert_eq!(
            switch_value(&args(line)[1..], "user-data-dir").as_deref(),
            expected,
            "{line:?}"
        );
    }
    // The directory is made absolute and created, with those above it.
    let root = std::env::temp_dir().join(format!("nb-user-data-{}", std::process::id()));
    let nested = root.join("profile").join("electron");
    let line = args(&["app.exe", &format!("--user-data-dir={}", nested.display())]);
    let found = user_data_dir(&realm, &line).unwrap().unwrap();
    assert_eq!(std::path::Path::new(&found), nested);
    assert!(nested.is_dir());
    // Asking again finds it made.
    assert_eq!(user_data_dir(&realm, &line).unwrap().unwrap(), found);
    // Without the switch, or with an empty one, the default stays.
    assert_eq!(user_data_dir(&realm, &args(&["app.exe"])).unwrap(), None);
    let empty = args(&["app.exe", "--user-data-dir="]);
    assert_eq!(user_data_dir(&realm, &empty).unwrap(), None);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn electron_exposes_native_net_fetch_and_session_proxy_configuration() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let net = property(&mut realm, &exports, "net");
    assert!(matches!(
        property(&mut realm, net.as_object().unwrap(), "fetch"),
        Value::Object(_)
    ));
    let is_online = property(&mut realm, net.as_object().unwrap(), "isOnline");
    let online = ops::call(&mut realm, &is_online, net.clone(), &[]).unwrap();
    assert!(matches!(online, Value::Boolean(_)));
    // The property agrees with the function.
    assert!(matches!(
        (property(&mut realm, net.as_object().unwrap(), "online"), online),
        (Value::Boolean(a), Value::Boolean(b)) if a == b
    ));
    let session = property(&mut realm, &exports, "session");
    let default_session = property(&mut realm, session.as_object().unwrap(), "defaultSession");
    let set_proxy = property(&mut realm, default_session.as_object().unwrap(), "setProxy");
    for mode in ["direct", "system"] {
        let options = realm.object();
        options.define(Key::name("mode"), Property::data(Value::str(mode)));
        let promise = ops::call(
            &mut realm,
            &set_proxy,
            default_session.clone(),
            &[Value::Object(options)],
        )
        .unwrap();
        assert!(matches!(promise, Value::Object(_)));
    }
    let unsupported = realm.object();
    unsupported.define(Key::name("mode"), Property::data(Value::str("pac_script")));
    assert!(
        ops::call(
            &mut realm,
            &set_proxy,
            default_session,
            &[Value::Object(unsupported)],
        )
        .is_err()
    );
}

#[test]
fn auto_updaters_behave_as_without_squirrel() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let updater = property(&mut realm, &exports, "autoUpdater");
    let updater = updater.as_object().unwrap();
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let method = property(realm, updater, name);
        ops::call(realm, &method, Value::Object(updater.clone()), args)
    };
    // Without the Node.js library, listeners are accepted and events go
    // nowhere.
    assert!(matches!(
        call(&mut realm, "checkForUpdates", &[]),
        Ok(Value::Undefined)
    ));
    assert!(call(&mut realm, "setFeedURL", &[Value::Number(1.0)]).is_err());
    let options = realm.object();
    options.define(
        Key::name("url"),
        Property::data(Value::str("https://updates.example")),
    );
    call(&mut realm, "setFeedURL", &[Value::Object(options)]).unwrap();
    let url = call(&mut realm, "getFeedURL", &[]).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &url).unwrap().to_rust_lossy(),
        "https://updates.example"
    );
    call(
        &mut realm,
        "setFeedURL",
        &[Value::str("https://other.example")],
    )
    .unwrap();
    call(&mut realm, "checkForUpdates", &[]).unwrap();
    call(&mut realm, "quitAndInstall", &[]).unwrap();
    // With listeners, each call reports its error.
    let errors = Rc::new(RefCell::new(Vec::new()));
    let seen = errors.clone();
    let listener = realm.function("listener", 1, false, move |realm, _, args| {
        let message = ops::get_named(realm, &args[0], "message")?;
        seen.borrow_mut()
            .push(ops::to_string(realm, &message)?.to_rust_lossy());
        Ok(Value::Undefined)
    });
    updater.define(
        Key::name("emit"),
        Property::data(Value::Object(realm.function(
            "emit",
            2,
            false,
            move |realm, _, args| {
                ops::call(
                    realm,
                    &Value::Object(listener.clone()),
                    Value::Undefined,
                    &args[1..],
                )?;
                Ok(Value::Boolean(true))
            },
        ))),
    );
    call(&mut realm, "checkForUpdates", &[]).unwrap();
    call(&mut realm, "quitAndInstall", &[]).unwrap();
    call(&mut realm, "setFeedURL", &[Value::str("")]).unwrap();
    call(&mut realm, "checkForUpdates", &[]).unwrap();
    assert_eq!(
        *errors.borrow(),
        [
            "Can not find Squirrel",
            "No update available, can't quit and install",
            "Update URL is not set"
        ]
    );
}

#[test]
fn net_requests_come_from_the_node_library_and_partitions_share_the_default_session() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let session = property(&mut realm, &exports, "session");
    let session = session.as_object().unwrap();
    let default_session = property(&mut realm, session, "defaultSession");
    let from_partition = property(&mut realm, session, "fromPartition");
    let shared = ops::call(
        &mut realm,
        &from_partition,
        Value::Object(session.clone()),
        &[Value::str("persist:updates")],
    )
    .unwrap();
    assert!(ops::strict_equals(&shared, &default_session));
    assert!(ops::call(&mut realm, &from_partition, Value::Undefined, &[]).is_err());
    let net = property(&mut realm, &exports, "net");
    let request = property(&mut realm, net.as_object().unwrap(), "request");
    let message = |realm: &mut Realm, request: &Value| {
        let Err(crate::Thrown(error)) =
            ops::call(realm, request, Value::Undefined, &[Value::str("http://x")])
        else {
            panic!("net.request did not throw");
        };
        let message = ops::get_named(realm, &error, "message").unwrap();
        ops::to_string(realm, &message).unwrap().to_rust_lossy()
    };
    // Without the Node.js library, or a library without the module, the
    // request reports what is missing.
    assert!(message(&mut realm, &request).contains("is not linked"));
    realm.node_library = Some(|_, _| Ok(Value::Undefined));
    assert!(message(&mut realm, &request).contains("needs the http and https modules"));
    realm.node_library = Some(|realm, name| {
        assert_eq!(name, "internal/electron/net");
        let module = realm.object();
        let create = realm.function("request", 3, false, |realm, _, args| {
            assert!(matches!(args[1], Value::Undefined));
            let agent = ops::to_string(realm, &args[2])?.to_rust_lossy();
            let url = ops::to_string(realm, &args[0])?.to_rust_lossy();
            Ok(Value::Boolean(
                url == "http://x" && agent.contains("Chrome/"),
            ))
        });
        module.define(Key::name("request"), Property::data(Value::Object(create)));
        Ok(Value::Object(module))
    });
    let created = ops::call(
        &mut realm,
        &request,
        Value::Undefined,
        &[Value::str("http://x")],
    )
    .unwrap();
    assert!(matches!(created, Value::Boolean(true)));
}

#[test]
fn electron_exposes_context_bridge_zoom_and_file_path_apis() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);

    let bridge = property(&mut realm, &exports, "contextBridge");
    let expose = property(&mut realm, bridge.as_object().unwrap(), "exposeInMainWorld");
    let api = realm.object();
    api.define(Key::name("ready"), Property::data(Value::Boolean(true)));
    ops::call(
        &mut realm,
        &expose,
        bridge.clone(),
        &[Value::str("electron"), Value::Object(api.clone())],
    )
    .unwrap();
    assert_eq!(realm.electron_bridge_exports.len(), 1);
    assert_eq!(realm.electron_bridge_exports[0].0, "electron");
    let global = realm.global.clone();
    assert!(matches!(
        property(&mut realm, &global, "electron"),
        Value::Object(_)
    ));

    let frame = property(&mut realm, &exports, "webFrame");
    for (setter, getter, value) in [
        ("setZoomFactor", "getZoomFactor", 1.25),
        ("setZoomLevel", "getZoomLevel", 2.0),
    ] {
        let setter = property(&mut realm, frame.as_object().unwrap(), setter);
        ops::call(&mut realm, &setter, frame.clone(), &[Value::Number(value)]).unwrap();
        let getter = property(&mut realm, frame.as_object().unwrap(), getter);
        assert!(matches!(
            ops::call(&mut realm, &getter, frame.clone(), &[]),
            Ok(Value::Number(actual)) if (actual - value).abs() < 1e-9
        ));
    }

    // Visual zoom limits take two numbers, as Electron's binding does.
    let limits = property(
        &mut realm,
        frame.as_object().unwrap(),
        "setVisualZoomLevelLimits",
    );
    let numbers = [Value::Number(1.0), Value::Number(1.0)];
    assert!(matches!(
        ops::call(&mut realm, &limits, frame.clone(), &numbers),
        Ok(Value::Undefined)
    ));
    let error = ops::call(&mut realm, &limits, frame.clone(), &[Value::Number(1.0)]).unwrap_err();
    let message = ops::get_named(&mut realm, &error.0, "message").unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &message)
            .unwrap()
            .to_rust_lossy(),
        "Error processing argument at index 1, conversion failure from undefined"
    );

    let utils = property(&mut realm, &exports, "webUtils");
    let get_path = property(&mut realm, utils.as_object().unwrap(), "getPathForFile");
    let file = realm.object();
    file.define(
        Key::name("path"),
        Property::data(Value::str("C:\\notes\\draft.md")),
    );
    assert!(matches!(
        ops::call(
            &mut realm,
            &get_path,
            utils,
            &[Value::Object(file)]
        ),
        Ok(Value::String(path)) if path.to_rust_lossy() == "C:\\notes\\draft.md"
    ));
}

#[test]
fn preload_ipc_sync_and_invoke_dispatch_into_the_main_realm() {
    let mut main = Realm::new();
    let main_exports = module(&mut main);
    assert!(matches!(
        ops::get(
            &mut main,
            &Value::Object(main_exports.clone()),
            &Key::name("ipcRenderer")
        ),
        Ok(Value::Undefined)
    ));
    let ipc_main = property(&mut main, &main_exports, "ipcMain");
    let on = property(&mut main, ipc_main.as_object().unwrap(), "on");
    let sync_listener = main.function("syncListener", 2, false, |realm, _, args| {
        let event = args.first().cloned().unwrap_or(Value::Undefined);
        let value = args.get(1).cloned().unwrap_or(Value::Undefined);
        ops::set(realm, &event, &Key::name("returnValue"), value)?;
        Ok(Value::Undefined)
    });
    ops::call(
        &mut main,
        &on,
        ipc_main.clone(),
        &[Value::str("sync"), Value::Object(sync_listener)],
    )
    .unwrap();
    let handle = property(&mut main, ipc_main.as_object().unwrap(), "handle");
    let async_handler = main.function("asyncHandler", 2, false, |_, _, args| {
        Ok(args.get(1).cloned().unwrap_or(Value::Undefined))
    });
    ops::call(
        &mut main,
        &handle,
        ipc_main,
        &[Value::str("invoke"), Value::Object(async_handler)],
    )
    .unwrap();

    let mut preload = Realm::new();
    preload.electron_ipc_main = (&mut main as *mut Realm) as usize;
    let exports = module(&mut preload);
    let renderer = property(&mut preload, &exports, "ipcRenderer");
    let on = property(&mut preload, renderer.as_object().unwrap(), "on");
    let received = Rc::new(RefCell::new(false));
    let received_by_listener = received.clone();
    let listener = preload.function("listener", 1, false, move |_, _, _| {
        *received_by_listener.borrow_mut() = true;
        Ok(Value::Undefined)
    });
    assert!(matches!(
        ops::call(
            &mut preload,
            &on,
            renderer.clone(),
            &[Value::str("message"), Value::Object(listener.clone())]
        ),
        Ok(Value::Object(_))
    ));
    let remove = property(
        &mut preload,
        renderer.as_object().unwrap(),
        "removeListener",
    );
    assert!(matches!(
        ops::call(
            &mut preload,
            &remove,
            renderer.clone(),
            &[Value::str("message"), Value::Object(listener)]
        ),
        Ok(Value::Object(_))
    ));
    assert!(!*received.borrow());
    let send_sync = property(&mut preload, renderer.as_object().unwrap(), "sendSync");
    assert!(matches!(
        ops::call(
            &mut preload,
            &send_sync,
            renderer.clone(),
            &[Value::str("sync"), Value::Number(37.0)]
        ),
        Ok(Value::Number(37.0))
    ));
    let payload = preload.object();
    payload.define(
        Key::name("nested"),
        Property::data(Value::Object(preload.array(vec![Value::Number(73.0)]))),
    );
    let cloned = ops::call(
        &mut preload,
        &send_sync,
        renderer.clone(),
        &[Value::str("sync"), Value::Object(payload)],
    )
    .unwrap();
    let nested = property(&mut preload, cloned.as_object().unwrap(), "nested");
    let value = ops::get(&mut preload, &nested, &Key::Index(0)).unwrap();
    assert!(matches!(value, Value::Number(73.0)));

    let invoke = property(&mut preload, renderer.as_object().unwrap(), "invoke");
    let promise = ops::call(
        &mut preload,
        &invoke,
        renderer,
        &[Value::str("invoke"), Value::Number(42.0)],
    )
    .unwrap();
    let observed = Rc::new(RefCell::new(None));
    let observed_value = observed.clone();
    let then = property(&mut preload, promise.as_object().unwrap(), "then");
    let callback = preload.function("observe", 1, false, move |_, _, args| {
        *observed_value.borrow_mut() = args.first().cloned();
        Ok(Value::Undefined)
    });
    ops::call(&mut preload, &then, promise, &[Value::Object(callback)]).unwrap();
    assert_eq!(preload.run_main(|_| Ok(())), 0);
    assert!(matches!(*observed.borrow(), Some(Value::Number(42.0))));
}

#[test]
fn app_ready_listener_runs_after_module_evaluation_jobs() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let app = property(&mut realm, &exports, "app");
    let listener = realm.function("ready", 0, false, |realm, this, _| {
        ops::set(
            realm,
            &this,
            &Key::name("readyListenerRan"),
            Value::Boolean(true),
        )?;
        Ok(Value::Undefined)
    });
    let on = property(&mut realm, app.as_object().unwrap(), "on");
    assert!(matches!(
        ops::call(
            &mut realm,
            &on,
            app.clone(),
            &[Value::str("ready"), Value::Object(listener)]
        ),
        Ok(Value::Object(_))
    ));
    assert!(matches!(
        property(&mut realm, app.as_object().unwrap(), "readyListenerRan"),
        Value::Undefined
    ));
    realm.run_jobs().unwrap();
    assert!(matches!(
        property(&mut realm, app.as_object().unwrap(), "readyListenerRan"),
        Value::Boolean(true)
    ));
    let on = property(&mut realm, app.as_object().unwrap(), "on");
    ops::call(
        &mut realm,
        &on,
        app,
        &[Value::str("activate"), Value::Undefined],
    )
    .unwrap();
    realm.run_jobs().unwrap();
}

#[test]
fn browser_window_constructs_and_exposes_native_lifecycle_methods() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    options.define(
        Key::name("title"),
        Property::data(Value::str("Native MarkText test")),
    );
    options.define(Key::name("width"), Property::data(Value::Number(640.0)));
    options.define(Key::name("height"), Property::data(Value::Number(480.0)));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let id = property(&mut realm, window.as_object().unwrap(), "id");
    assert!(matches!(&id, Value::Number(id) if *id > 0.0));
    let from_id = property(&mut realm, constructor.as_object().unwrap(), "fromId");
    assert!(matches!(
        ops::call(
            &mut realm,
            &from_id,
            constructor.clone(),
            core::slice::from_ref(&id)
        ),
        Ok(Value::Object(_))
    ));
    let get_all_windows = property(
        &mut realm,
        constructor.as_object().unwrap(),
        "getAllWindows",
    );
    let all_windows = ops::call(&mut realm, &get_all_windows, constructor.clone(), &[]).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &all_windows, &Key::name("length")),
        Ok(Value::Number(1.0))
    ));
    // The webContents module finds the window's contents.
    let contents = property(&mut realm, window.as_object().unwrap(), "webContents");
    let module = property(&mut realm, &exports, "webContents");
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let method = property(realm, module.as_object().unwrap(), name);
        ops::call(realm, &method, module.clone(), args).unwrap()
    };
    let all = call(&mut realm, "getAllWebContents", &[]);
    let first = ops::get(&mut realm, &all, &Key::Index(0)).unwrap();
    assert!(crate::same_value(&first, &contents));
    let contents_id = property(&mut realm, contents.as_object().unwrap(), "id");
    let found = call(&mut realm, "fromId", &[contents_id]);
    assert!(crate::same_value(&found, &contents));
    assert!(matches!(
        call(&mut realm, "fromId", &[Value::Number(-1.0)]),
        Value::Undefined
    ));
    assert!(matches!(
        call(&mut realm, "fromFrame", &[Value::Null]),
        Value::Undefined
    ));
    assert!(matches!(
        call(&mut realm, "getFocusedWebContents", &[]),
        Value::Null | Value::Object(_)
    ));
    let set_menu = property(&mut realm, window.as_object().unwrap(), "setMenu");
    assert!(matches!(
        ops::call(&mut realm, &set_menu, window.clone(), &[Value::Null]),
        Ok(Value::Undefined)
    ));
    let remove_menu = property(&mut realm, window.as_object().unwrap(), "removeMenu");
    assert!(matches!(
        ops::call(&mut realm, &remove_menu, window.clone(), &[]),
        Ok(Value::Undefined)
    ));
    let is_visible = property(&mut realm, window.as_object().unwrap(), "isVisible");
    let destroyed = property(&mut realm, window.as_object().unwrap(), "isDestroyed");
    assert!(matches!(
        ops::call(&mut realm, &destroyed, window.clone(), &[]),
        Ok(Value::Boolean(false))
    ));
    assert!(matches!(
        ops::call(&mut realm, &is_visible, window.clone(), &[]),
        Ok(Value::Boolean(true))
    ));
    let hide = property(&mut realm, window.as_object().unwrap(), "hide");
    assert!(matches!(
        ops::call(&mut realm, &hide, window.clone(), &[]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        ops::call(&mut realm, &is_visible, window.clone(), &[]),
        Ok(Value::Boolean(false))
    ));
    let show = property(&mut realm, window.as_object().unwrap(), "show");
    assert!(matches!(
        ops::call(&mut realm, &show, window.clone(), &[]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        ops::call(&mut realm, &is_visible, window.clone(), &[]),
        Ok(Value::Boolean(true))
    ));
    let load_file = property(&mut realm, window.as_object().unwrap(), "loadFile");
    assert!(matches!(
        ops::call(
            &mut realm,
            &load_file,
            window.clone(),
            &[Value::str("index.html")]
        ),
        Ok(Value::Object(_))
    ));
    let load_url = property(&mut realm, window.as_object().unwrap(), "loadURL");
    assert!(matches!(
        ops::call(
            &mut realm,
            &load_url,
            window.clone(),
            &[Value::str("https://example.invalid")]
        ),
        Ok(Value::Object(_))
    ));
    let loads = realm.take_electron_window_loads();
    // A relative file is in the application folder, the executable's.
    let executable = std::env::current_exe().unwrap();
    let folder = executable.parent().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        loads
            .iter()
            .map(|(_, target)| target.as_str())
            .collect::<Vec<_>>(),
        [
            format!("{folder}\\index.html").as_str(),
            "https://example.invalid"
        ]
    );
    assert_eq!(loads[0].0, loads[1].0);
    let contents = property(&mut realm, window.as_object().unwrap(), "webContents");
    let contents_on = property(&mut realm, contents.as_object().unwrap(), "on");
    assert!(matches!(
        ops::call(
            &mut realm,
            &contents_on,
            contents.clone(),
            &[Value::str("did-finish-load"), Value::Undefined]
        ),
        Ok(Value::Object(_))
    ));
    let set_handler = property(
        &mut realm,
        contents.as_object().unwrap(),
        "setWindowOpenHandler",
    );
    assert!(matches!(
        ops::call(&mut realm, &set_handler, contents, &[Value::Undefined]),
        Ok(Value::Undefined)
    ));
    // `close` asks the window to close, which the message loop handles
    // later; `destroy` closes it at once.
    let close = property(&mut realm, window.as_object().unwrap(), "close");
    assert!(matches!(
        ops::call(&mut realm, &close, window.clone(), &[]),
        Ok(Value::Undefined)
    ));
    let destroyed = property(&mut realm, window.as_object().unwrap(), "isDestroyed");
    assert!(matches!(
        ops::call(&mut realm, &destroyed, window.clone(), &[]),
        Ok(Value::Boolean(false))
    ));
    let destroy = property(&mut realm, window.as_object().unwrap(), "destroy");
    assert!(matches!(
        ops::call(&mut realm, &destroy, window.clone(), &[]),
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        ops::call(&mut realm, &destroyed, window, &[]),
        Ok(Value::Boolean(true))
    ));
    let all_windows = ops::call(&mut realm, &get_all_windows, constructor.clone(), &[]).unwrap();
    assert!(matches!(
        ops::get(&mut realm, &all_windows, &Key::name("length")),
        Ok(Value::Number(0.0))
    ));
    assert!(matches!(
        ops::call(&mut realm, &from_id, constructor, &[id]),
        Ok(Value::Null)
    ));
}

#[test]
fn browser_window_defaults_options_and_rejects_invalid_options() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    options.define(Key::name("title"), Property::data(Value::Number(12.0)));
    options.define(Key::name("width"), Property::data(Value::Number(0.0)));
    options.define(Key::name("height"), Property::data(Value::str("large")));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let prototype = ops::get(&mut realm, &constructor, &Key::name("prototype")).unwrap();
    let on = property(&mut realm, prototype.as_object().unwrap(), "on");
    assert!(matches!(
        ops::call(
            &mut realm,
            &on,
            window.clone(),
            &[Value::str("focus"), Value::Undefined]
        ),
        Ok(Value::Object(_))
    ));
    let destroy = property(&mut realm, window.as_object().unwrap(), "destroy");
    ops::call(&mut realm, &destroy, window.clone(), &[]).unwrap();
    let show = property(&mut realm, window.as_object().unwrap(), "show");
    assert!(ops::call(&mut realm, &show, window.clone(), &[]).is_err());
    let hide = property(&mut realm, window.as_object().unwrap(), "hide");
    assert!(ops::call(&mut realm, &hide, window, &[]).is_err());
    assert!(ops::construct(&mut realm, &constructor, &[Value::str("invalid")]).is_err());
    let default_window = ops::construct(&mut realm, &constructor, &[Value::Null]).unwrap();
    let destroy = property(&mut realm, default_window.as_object().unwrap(), "destroy");
    ops::call(&mut realm, &destroy, default_window, &[]).unwrap();
}

fn invoke(realm: &mut Realm, window: &Value, name: &str, args: &[Value]) -> Value {
    let method = property(realm, window.as_object().unwrap(), name);
    ops::call(realm, &method, window.clone(), args).unwrap()
}

fn numbers(realm: &mut Realm, value: &Value, keys: &[&str]) -> Vec<f64> {
    keys.iter()
        .map(
            |key| match ops::get(realm, value, &Key::name(key)).unwrap() {
                Value::Number(number) => number,
                other => panic!("{key}: {other:?}"),
            },
        )
        .collect()
}

#[test]
fn browser_window_options_shape_the_native_window_in_device_independent_pixels() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    for (name, value) in [
        ("x", Value::Number(30.0)),
        ("y", Value::Number(40.0)),
        ("width", Value::Number(500.0)),
        ("height", Value::Number(400.0)),
        ("minWidth", Value::Number(300.0)),
        ("minHeight", Value::Number(200.0)),
        ("maxWidth", Value::Number(900.0)),
        ("maxHeight", Value::Number(800.0)),
        ("useContentSize", Value::Boolean(true)),
        ("frame", Value::Number(0.0)),
        ("show", Value::Boolean(false)),
        ("backgroundColor", Value::str("#123")),
        ("transparent", Value::Boolean(true)),
        ("alwaysOnTop", Value::Boolean(true)),
        ("skipTaskbar", Value::Boolean(true)),
    ] {
        options.define(Key::name(name), Property::data(value));
    }
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let near = |actual: &[f64], expected: &[f64]| {
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() <= 1.0)
    };
    let visible = invoke(&mut realm, &window, "isVisible", &[]);
    assert!(matches!(visible, Value::Boolean(false)));
    let top = invoke(&mut realm, &window, "isAlwaysOnTop", &[]);
    assert!(matches!(top, Value::Boolean(true)));
    invoke(
        &mut realm,
        &window,
        "setSkipTaskbar",
        &[Value::Boolean(false)],
    );
    let keys = ["x", "y", "width", "height"];
    let bounds = invoke(&mut realm, &window, "getBounds", &[]);
    let bounds = numbers(&mut realm, &bounds, &keys);
    assert!(near(&bounds, &[30.0, 40.0, 500.0, 400.0]), "{bounds:?}");
    // Frameless windows are all content.
    let content = invoke(&mut realm, &window, "getContentBounds", &[]);
    assert_eq!(numbers(&mut realm, &content, &keys), bounds);
    let normal = invoke(&mut realm, &window, "getNormalBounds", &[]);
    assert_eq!(numbers(&mut realm, &normal, &keys), bounds);
    let size = invoke(&mut realm, &window, "getSize", &[]);
    assert_eq!(numbers(&mut realm, &size, &["0", "1"]), bounds[2..]);
    let size = invoke(&mut realm, &window, "getContentSize", &[]);
    assert_eq!(numbers(&mut realm, &size, &["0", "1"]), bounds[2..]);
    let minimum = invoke(&mut realm, &window, "getMinimumSize", &[]);
    assert_eq!(numbers(&mut realm, &minimum, &["0", "1"]), [300.0, 200.0]);
    let maximum = invoke(&mut realm, &window, "getMaximumSize", &[]);
    assert_eq!(numbers(&mut realm, &maximum, &["0", "1"]), [900.0, 800.0]);

    let size = [Value::Number(320.0), Value::Number(210.0)];
    invoke(&mut realm, &window, "setMinimumSize", &size);
    let size = [Value::Number(1000.0), Value::Number(700.0)];
    invoke(&mut realm, &window, "setMaximumSize", &size);
    let minimum = invoke(&mut realm, &window, "getMinimumSize", &[]);
    assert_eq!(numbers(&mut realm, &minimum, &["0", "1"]), [320.0, 210.0]);
    let maximum = invoke(&mut realm, &window, "getMaximumSize", &[]);
    assert_eq!(numbers(&mut realm, &maximum, &["0", "1"]), [1000.0, 700.0]);

    let partial = realm.object();
    partial.define(Key::name("width"), Property::data(Value::Number(640.0)));
    partial.define(Key::name("x"), Property::data(Value::str("left")));
    invoke(&mut realm, &window, "setBounds", &[Value::Object(partial)]);
    let bounds = invoke(&mut realm, &window, "getBounds", &[]);
    let bounds = numbers(&mut realm, &bounds, &keys);
    assert!(near(&bounds, &[30.0, 40.0, 640.0, 400.0]), "{bounds:?}");
    let size = [Value::Number(600.0), Value::Number(500.0)];
    invoke(&mut realm, &window, "setSize", &size);
    let size = invoke(&mut realm, &window, "getSize", &[]);
    assert!(near(
        &numbers(&mut realm, &size, &["0", "1"]),
        &[600.0, 500.0]
    ));
    let size = [Value::Number(450.0), Value::Number(350.0)];
    invoke(&mut realm, &window, "setContentSize", &size);
    let size = invoke(&mut realm, &window, "getContentSize", &[]);
    assert!(near(
        &numbers(&mut realm, &size, &["0", "1"]),
        &[450.0, 350.0]
    ));
    invoke(&mut realm, &window, "center", &[]);

    assert!(matches!(
        invoke(&mut realm, &window, "isResizable", &[]),
        Value::Boolean(true)
    ));
    invoke(
        &mut realm,
        &window,
        "setResizable",
        &[Value::Boolean(false)],
    );
    assert!(matches!(
        invoke(&mut realm, &window, "isResizable", &[]),
        Value::Boolean(false)
    ));
    invoke(&mut realm, &window, "destroy", &[]);

    // Content-sized framed windows exclude the frame and menu bar.
    let options = realm.object();
    options.define(
        Key::name("useContentSize"),
        Property::data(Value::Boolean(true)),
    );
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    options.define(
        Key::name("backgroundColor"),
        Property::data(Value::Number(1.0)),
    );
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let size = invoke(&mut realm, &window, "getContentSize", &[]);
    assert!(near(
        &numbers(&mut realm, &size, &["0", "1"]),
        &[800.0, 600.0]
    ));
    let size = invoke(&mut realm, &window, "getSize", &[]);
    assert!(numbers(&mut realm, &size, &["0", "1"])[1] > 600.0);
    invoke(&mut realm, &window, "destroy", &[]);
}

#[test]
fn background_colors_accept_electrons_hex_forms() {
    assert_eq!(hex_color("#123"), Some([0x11, 0x22, 0x33]));
    assert_eq!(hex_color("#a0b1c2"), Some([0xa0, 0xb1, 0xc2]));
    assert_eq!(hex_color("#80a0b1c2"), Some([0xa0, 0xb1, 0xc2]));
    assert_eq!(hex_color("#12"), None);
    assert_eq!(hex_color("#xyz"), None);
    assert_eq!(hex_color("white"), None);
}

#[test]
fn browser_window_call_without_new_throws() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    assert!(ops::call(&mut realm, &constructor, Value::Undefined, &[]).is_err());
}

#[test]
fn native_theme_reads_system_mode_and_emits_updated_when_source_changes() {
    use alloc::rc::Rc;
    use core::cell::Cell;

    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let theme = property(&mut realm, &exports, "nativeTheme");
    assert!(matches!(
        property(&mut realm, theme.as_object().unwrap(), "themeSource"),
        Value::String(_)
    ));
    assert!(matches!(
        property(
            &mut realm,
            theme.as_object().unwrap(),
            "shouldUseDarkColors"
        ),
        Value::Boolean(_)
    ));

    let calls = Rc::new(Cell::new(0));
    let on_calls = calls.clone();
    let on_listener = realm.function("onUpdated", 0, false, move |_, _, _| {
        on_calls.set(on_calls.get() + 1);
        Ok(Value::Undefined)
    });
    let on = property(&mut realm, theme.as_object().unwrap(), "on");
    ops::call(
        &mut realm,
        &on,
        theme.clone(),
        &[Value::str("updated"), Value::Object(on_listener)],
    )
    .unwrap();
    let once_calls = calls.clone();
    let once_listener = realm.function("onceUpdated", 0, false, move |_, _, _| {
        once_calls.set(once_calls.get() + 1);
        Ok(Value::Undefined)
    });
    let once = property(&mut realm, theme.as_object().unwrap(), "once");
    ops::call(
        &mut realm,
        &once,
        theme.clone(),
        &[Value::str("updated"), Value::Object(once_listener)],
    )
    .unwrap();

    ops::set(
        &mut realm,
        &theme,
        &Key::name("themeSource"),
        Value::str("system"),
    )
    .unwrap();
    assert_eq!(calls.get(), 0);

    ops::set(
        &mut realm,
        &theme,
        &Key::name("themeSource"),
        Value::str("dark"),
    )
    .unwrap();
    assert!(matches!(
        property(
            &mut realm,
            theme.as_object().unwrap(),
            "shouldUseDarkColors"
        ),
        Value::Boolean(true)
    ));
    assert_eq!(calls.get(), 2);
    ops::set(
        &mut realm,
        &theme,
        &Key::name("themeSource"),
        Value::str("light"),
    )
    .unwrap();
    assert!(matches!(
        property(
            &mut realm,
            theme.as_object().unwrap(),
            "shouldUseDarkColors"
        ),
        Value::Boolean(false)
    ));
    assert_eq!(calls.get(), 3);
    assert!(
        ops::set(
            &mut realm,
            &theme,
            &Key::name("themeSource"),
            Value::str("sepia"),
        )
        .is_err()
    );
}

#[test]
fn native_theme_propagates_updated_listener_exceptions() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let theme = property(&mut realm, &exports, "nativeTheme");
    let listener = realm.function("throwOnUpdated", 0, false, |realm, _, _| {
        Err(realm.throw(crate::ErrorKind::Type, "theme listener failed"))
    });
    let on = property(&mut realm, theme.as_object().unwrap(), "on");
    ops::call(
        &mut realm,
        &on,
        theme.clone(),
        &[Value::str("updated"), Value::Object(listener)],
    )
    .unwrap();
    assert!(
        ops::set(
            &mut realm,
            &theme,
            &Key::name("themeSource"),
            Value::str("dark"),
        )
        .is_err()
    );
}

#[test]
fn native_theme_listener_methods_add_remove_count_and_emit_events() {
    use alloc::rc::Rc;
    use core::cell::Cell;

    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let theme = property(&mut realm, &exports, "nativeTheme");
    let calls = Rc::new(Cell::new(0));
    let listener_calls = calls.clone();
    let callback = realm.function("listener", 0, false, move |_, _, _| {
        listener_calls.set(listener_calls.get() + 1);
        Ok(Value::Undefined)
    });

    let add = property(&mut realm, theme.as_object().unwrap(), "addListener");
    ops::call(
        &mut realm,
        &add,
        theme.clone(),
        &[Value::str("probe"), Value::Object(callback.clone())],
    )
    .unwrap();
    ops::call(
        &mut realm,
        &add,
        theme.clone(),
        &[Value::str("probe"), Value::Object(callback.clone())],
    )
    .unwrap();
    let count = property(&mut realm, theme.as_object().unwrap(), "listenerCount");
    assert!(matches!(
        ops::call(&mut realm, &count, theme.clone(), &[Value::str("probe")]).unwrap(),
        Value::Number(2.0)
    ));
    let emit = property(&mut realm, theme.as_object().unwrap(), "emit");
    assert!(matches!(
        ops::call(&mut realm, &emit, theme.clone(), &[Value::str("probe")]).unwrap(),
        Value::Boolean(true)
    ));
    assert_eq!(calls.get(), 2);

    let off = property(&mut realm, theme.as_object().unwrap(), "off");
    ops::call(
        &mut realm,
        &off,
        theme.clone(),
        &[Value::str("probe"), Value::Object(callback.clone())],
    )
    .unwrap();
    assert!(matches!(
        ops::call(&mut realm, &emit, theme.clone(), &[Value::str("probe")]).unwrap(),
        Value::Boolean(true)
    ));
    assert_eq!(calls.get(), 3);
    ops::call(
        &mut realm,
        &off,
        theme.clone(),
        &[Value::str("probe"), Value::Object(callback.clone())],
    )
    .unwrap();
    assert!(matches!(
        ops::call(&mut realm, &emit, theme.clone(), &[Value::str("probe")]).unwrap(),
        Value::Boolean(false)
    ));

    let on = property(&mut realm, theme.as_object().unwrap(), "on");
    ops::call(
        &mut realm,
        &on,
        theme.clone(),
        &[Value::str("first"), Value::Object(callback.clone())],
    )
    .unwrap();
    ops::call(
        &mut realm,
        &on,
        theme.clone(),
        &[Value::str("second"), Value::Object(callback)],
    )
    .unwrap();
    let remove_all = property(&mut realm, theme.as_object().unwrap(), "removeAllListeners");
    ops::call(
        &mut realm,
        &remove_all,
        theme.clone(),
        &[Value::str("first")],
    )
    .unwrap();
    assert!(matches!(
        ops::call(&mut realm, &count, theme.clone(), &[Value::str("first")]).unwrap(),
        Value::Number(0.0)
    ));
    ops::call(&mut realm, &remove_all, theme.clone(), &[]).unwrap();
    assert!(matches!(
        ops::call(&mut realm, &count, theme.clone(), &[Value::str("second")]).unwrap(),
        Value::Number(0.0)
    ));
    assert!(
        ops::call(
            &mut realm,
            &on,
            theme,
            &[Value::str("invalid"), Value::Number(1.0)],
        )
        .is_err()
    );
}

#[test]
fn sessions_record_registered_preload_scripts() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let session = property(&mut realm, &exports, "session");
    let default_session = property(&mut realm, session.as_object().unwrap(), "defaultSession");
    let target = default_session.as_object().unwrap().clone();
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let function = property(realm, &target, name);
        ops::call(realm, &function, default_session.clone(), args)
    };
    let script = |realm: &Realm, id: Option<&str>, file: &str| {
        let script = realm.object();
        script.define(Key::name("filePath"), Property::data(Value::str(file)));
        if let Some(id) = id {
            script.define(Key::name("id"), Property::data(Value::str(id)));
            script.define(
                Key::name("type"),
                Property::data(Value::str("service-worker")),
            );
        }
        Value::Object(script)
    };
    let text =
        |realm: &mut Realm, value: Value| ops::to_string(realm, &value).unwrap().to_rust_lossy();

    let first = script(&realm, Some("log"), "C:/log.js");
    let id = call(
        &mut realm,
        "registerPreloadScript",
        std::slice::from_ref(&first),
    )
    .unwrap();
    assert_eq!(text(&mut realm, id), "log");
    assert!(call(&mut realm, "registerPreloadScript", &[first]).is_err());
    assert!(call(&mut realm, "registerPreloadScript", &[Value::Number(1.0)]).is_err());
    let second = script(&realm, None, "C:/other.js");
    let generated = call(&mut realm, "registerPreloadScript", &[second]).unwrap();
    assert_eq!(text(&mut realm, generated), "nativeboat-preload-2");

    let files = realm.array(vec![Value::str("C:/a.js"), Value::str("C:/b.js")]);
    call(&mut realm, "setPreloads", &[Value::Object(files)]).unwrap();
    let files = realm.array(vec![Value::str("C:/c.js")]);
    call(&mut realm, "setPreloads", &[Value::Object(files)]).unwrap();
    let preloads = call(&mut realm, "getPreloads", &[]).unwrap();
    assert_eq!(text(&mut realm, preloads), "C:/c.js");

    call(
        &mut realm,
        "unregisterPreloadScript",
        &[Value::str("nativeboat-preload-2")],
    )
    .unwrap();
    let scripts = call(&mut realm, "getPreloadScripts", &[]).unwrap();
    let length = ops::get(&mut realm, &scripts, &Key::name("length")).unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(2.0)));
    let entry = ops::get(&mut realm, &scripts, &Key::Index(0)).unwrap();
    let kind = ops::get(&mut realm, &entry, &Key::name("type")).unwrap();
    assert_eq!(text(&mut realm, kind), "service-worker");
    let file = ops::get(&mut realm, &entry, &Key::name("filePath")).unwrap();
    assert_eq!(text(&mut realm, file), "C:/log.js");
}

#[test]
fn new_windows_and_their_web_contents_are_announced_on_the_app() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let app = realm.electron_app.clone().unwrap();
    let seen = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new()));
    let record = seen.clone();
    let emit = realm.function("emit", 3, false, move |realm, _, args| {
        let name = ops::to_string(realm, &args[0])?.to_rust_lossy();
        record.borrow_mut().push((name, args[2].clone()));
        Ok(Value::Boolean(true))
    });
    app.define(Key::name("emit"), Property::data(Value::Object(emit)));
    let options = realm.object();
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let contents = property(&mut realm, window.as_object().unwrap(), "webContents");
    let seen = seen.borrow();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "web-contents-created");
    assert!(ops::strict_equals(&seen[0].1, &contents));
    assert_eq!(seen[1].0, "browser-window-created");
    assert!(ops::strict_equals(&seen[1].1, &window));
    let destroy = property(&mut realm, window.as_object().unwrap(), "destroy");
    ops::call(&mut realm, &destroy, window.clone(), &[]).unwrap();
}

static PRINTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

unsafe fn record_print(
    _: *mut core::ffi::c_void,
    _: usize,
    request: &crate::app_host::PrintRequest,
) -> Result<crate::app_host::PrintOutcome, String> {
    PRINTED.lock().unwrap().push(format!("{request:?}"));
    match &request.target {
        crate::app_host::PrintTarget::Pdf => {
            Ok(crate::app_host::PrintOutcome::Pdf(b"%PDF".to_vec()))
        }
        crate::app_host::PrintTarget::Dialog => Ok(crate::app_host::PrintOutcome::Cancelled),
        crate::app_host::PrintTarget::Printer(name) if name == "broken" => Err("failed".into()),
        crate::app_host::PrintTarget::Printer(_) => Ok(crate::app_host::PrintOutcome::Printed),
    }
}

#[test]
fn web_contents_print_their_pages_as_electron_options_ask() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let contents = property(&mut realm, window.as_object().unwrap(), "webContents");
    let contents_object = contents.as_object().unwrap().clone();
    let print_to_pdf = property(&mut realm, &contents_object, "printToPDF");
    let print = property(&mut realm, &contents_object, "print");
    let settled = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = |realm: &mut Realm,
                  settled: &std::rc::Rc<std::cell::RefCell<Vec<String>>>,
                  label: &'static str| {
        let settled = settled.clone();
        Value::Object(realm.function("", 2, false, move |realm, _, args| {
            let mut entry = String::from(label);
            for value in args {
                entry.push(' ');
                entry.push_str(&ops::to_string(realm, value)?.to_rust_lossy());
            }
            settled.borrow_mut().push(entry);
            Ok(Value::Undefined)
        }))
    };
    let pdf = |realm: &mut Realm, options: Value| {
        let promise = ops::call(realm, &print_to_pdf, contents.clone(), &[options]).unwrap();
        let then = ops::get(realm, &promise, &Key::name("then")).unwrap();
        let fulfilled = record(realm, &settled, "pdf");
        let rejected = record(realm, &settled, "error");
        ops::call(realm, &then, promise, &[fulfilled, rejected]).unwrap();
        realm.run_jobs().unwrap();
    };
    // Without a printer the promise rejects.
    crate::app_host::set_print_handler(core::ptr::null_mut(), None);
    pdf(&mut realm, Value::Undefined);
    crate::app_host::set_print_handler(core::ptr::null_mut(), Some(record_print));
    let size = realm.object();
    size.define(Key::name("width"), Property::data(Value::Number(100_000.0)));
    size.define(
        Key::name("height"),
        Property::data(Value::Number(200_000.0)),
    );
    let margins = realm.object();
    margins.define(Key::name("top"), Property::data(Value::Number(1.0)));
    margins.define(Key::name("left"), Property::data(Value::Number(-1.0)));
    let options = realm.object();
    options.define(Key::name("pageSize"), Property::data(Value::Object(size)));
    options.define(Key::name("landscape"), Property::data(Value::Boolean(true)));
    options.define(Key::name("margins"), Property::data(Value::Object(margins)));
    pdf(&mut realm, Value::Object(options));
    let named = realm.object();
    named.define(Key::name("pageSize"), Property::data(Value::str("A4")));
    pdf(&mut realm, Value::Object(named));
    // Printing: the dialog, which the user cancels, then named printers.
    let callback = record(&mut realm, &settled, "print");
    ops::call(
        &mut realm,
        &print,
        contents.clone(),
        &[Value::Undefined, callback.clone()],
    )
    .unwrap();
    for device in ["", "broken"] {
        let silent = realm.object();
        silent.define(Key::name("silent"), Property::data(Value::Boolean(true)));
        silent.define(Key::name("deviceName"), Property::data(Value::str(device)));
        ops::call(
            &mut realm,
            &print,
            contents.clone(),
            &[Value::Object(silent), callback.clone()],
        )
        .unwrap();
    }
    let quiet = realm.object();
    quiet.define(Key::name("silent"), Property::data(Value::Boolean(true)));
    quiet.define(Key::name("pageSize"), Property::data(Value::str("Unknown")));
    ops::call(
        &mut realm,
        &print,
        contents.clone(),
        &[Value::Object(quiet)],
    )
    .unwrap();
    crate::app_host::set_print_handler(core::ptr::null_mut(), None);
    let settled = settled.borrow().clone();
    assert!(
        settled[0].starts_with("error Error: printing is not available"),
        "{settled:?}"
    );
    assert_eq!(settled[1], "pdf 37,80,68,70");
    assert_eq!(
        &settled[3..],
        ["print false cancelled", "print true ", "print false failed"]
    );
    let printed = PRINTED.lock().unwrap().clone();
    assert!(
        printed[0].contains(
            "paper: Some((100.0, 200.0)), landscape: true, margins: [Some(96.0), None, None, None]"
        ),
        "{printed:?}"
    );
    assert!(
        printed[1].contains("paper: Some((210.0, 297.0)), landscape: false"),
        "{printed:?}"
    );
    assert!(
        printed[2].contains("target: Dialog, paper: None"),
        "{printed:?}"
    );
    assert!(
        printed[5].contains("target: Printer(\"\"), paper: None"),
        "{printed:?}"
    );
    let destroy = property(&mut realm, window.as_object().unwrap(), "destroy");
    ops::call(&mut realm, &destroy, window.clone(), &[]).unwrap();
}

#[test]
fn load_file_targets_are_absolute_only_with_a_drive_or_share() {
    assert!(absolute_path("C:\\app\\index.html"));
    assert!(absolute_path("c:/app/index.html"));
    assert!(absolute_path("\\\\server\\share\\index.html"));
    assert!(absolute_path("//server/share/index.html"));
    assert!(!absolute_path("html/index.html"));
    assert!(!absolute_path("..\\index.html"));
    assert!(!absolute_path("1:"));
}

/// A class a native application cannot provide is exported, and
/// constructing it throws, saying why.
#[test]
fn unavailable_classes_throw_when_constructed() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let view = property(&mut realm, &exports, "WebContentsView");
    let error = ops::construct(&mut realm, &view, &[]).unwrap_err();
    let message = ops::get_named(&mut realm, &error.0, "message").unwrap();
    let message = ops::to_string(&mut realm, &message)
        .unwrap()
        .to_rust_lossy();
    assert!(message.starts_with("WebContentsView is not available in native applications"));
}

fn call_method(realm: &mut Realm, object: &Value, name: &str, args: &[Value]) -> Value {
    let method = ops::get(realm, object, &Key::name(name)).unwrap();
    ops::call(realm, &method, object.clone(), args).unwrap()
}

/// The `app` members VS Code's main process calls on Windows.
#[test]
fn app_reports_metrics_gpu_status_jump_lists_and_accessibility() {
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let app = property(&mut realm, &exports, "app");
    assert!(matches!(
        call_method(&mut realm, &app, "enableSandbox", &[]),
        Value::Undefined
    ));
    call_method(
        &mut realm,
        &app,
        "setAppLogsPath",
        &[Value::str("C:\\Logs\\app")],
    );
    let logs = call_method(&mut realm, &app, "getPath", &[Value::str("logs")]);
    assert_eq!(
        ops::to_string(&mut realm, &logs).unwrap().to_rust_lossy(),
        "C:\\Logs\\app"
    );
    let metrics = call_method(&mut realm, &app, "getAppMetrics", &[]);
    let first = ops::get(&mut realm, &metrics, &Key::Index(0)).unwrap();
    let pid = ops::get(&mut realm, &first, &Key::name("pid")).unwrap();
    assert!(matches!(pid, Value::Number(pid) if pid == f64::from(crate::platform::process_id())));
    let kind = ops::get(&mut realm, &first, &Key::name("type")).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &kind).unwrap().to_rust_lossy(),
        "Browser"
    );
    let memory = ops::get(&mut realm, &first, &Key::name("memory")).unwrap();
    let working = ops::get(&mut realm, &memory, &Key::name("workingSetSize")).unwrap();
    assert!(matches!(working, Value::Number(size) if size > 0.0));
    let created = ops::get(&mut realm, &first, &Key::name("creationTime")).unwrap();
    // After 2020 and not in the far future.
    assert!(matches!(created, Value::Number(ms) if ms > 1.6e12 && ms < 4.0e12));
    let status = call_method(&mut realm, &app, "getGPUFeatureStatus", &[]);
    let compositing = ops::get(&mut realm, &status, &Key::name("gpu_compositing")).unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &compositing)
            .unwrap()
            .to_rust_lossy(),
        "disabled_software"
    );
    let jump = call_method(&mut realm, &app, "getJumpListSettings", &[]);
    let slots = ops::get(&mut realm, &jump, &Key::name("minItems")).unwrap();
    assert!(matches!(slots, Value::Number(10.0)));
    call_method(
        &mut realm,
        &app,
        "setAccessibilitySupportEnabled",
        &[Value::Boolean(true)],
    );
    assert!(matches!(
        call_method(&mut realm, &app, "isAccessibilitySupportEnabled", &[]),
        Value::Boolean(true)
    ));
    ops::set(
        &mut realm,
        &app,
        &Key::name("accessibilitySupportEnabled"),
        Value::Boolean(false),
    )
    .unwrap();
    assert!(matches!(
        ops::get(&mut realm, &app, &Key::name("accessibilitySupportEnabled")).unwrap(),
        Value::Boolean(false)
    ));
    for name in [
        "setProxy",
        "resolveProxy",
        "focus",
        "getFileIcon",
        "getGPUInfo",
    ] {
        let method = ops::get(&mut realm, &app, &Key::name(name)).unwrap();
        assert!(crate::array_builtin::is_callable(&method), "{name}");
    }
}

/// Every window's contents share the default session, which has the
/// handlers and request hooks programs install.
#[test]
fn web_contents_share_the_default_session_and_its_handlers() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let session = property(&mut realm, &exports, "session");
    let default_session = ops::get(&mut realm, &session, &Key::name("defaultSession")).unwrap();
    let from_path = call_method(
        &mut realm,
        &session,
        "fromPath",
        &[Value::str("C:\\Profiles\\one")],
    );
    assert!(
        matches!((&from_path, &default_session), (Value::Object(a), Value::Object(b)) if a == b)
    );
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let contents = ops::get(&mut realm, &window, &Key::name("webContents")).unwrap();
    let shared = ops::get(&mut realm, &contents, &Key::name("session")).unwrap();
    assert!(matches!((&shared, &default_session), (Value::Object(a), Value::Object(b)) if a == b));
    for name in [
        "setPermissionRequestHandler",
        "setDisplayMediaRequestHandler",
        "setCodeCachePath",
        "clearData",
        "fetch",
        "setProxy",
        "resolveProxy",
        "setSpellCheckerLanguages",
    ] {
        let method = ops::get(&mut realm, &shared, &Key::name(name)).unwrap();
        assert!(crate::array_builtin::is_callable(&method), "{name}");
    }
    let requests = ops::get(&mut realm, &shared, &Key::name("webRequest")).unwrap();
    let hook = ops::get(&mut realm, &requests, &Key::name("onHeadersReceived")).unwrap();
    assert!(crate::array_builtin::is_callable(&hook));
    assert!(matches!(
        call_method(&mut realm, &shared, "isPersistent", &[]),
        Value::Boolean(true)
    ));
    // The contents' process, title and states are the window's.
    let pid = call_method(&mut realm, &contents, "getOSProcessId", &[]);
    assert!(matches!(pid, Value::Number(pid) if pid == f64::from(crate::platform::process_id())));
    call_method(
        &mut realm,
        &window,
        "setTitle",
        &[Value::str("Shared session")],
    );
    let title = call_method(&mut realm, &contents, "getTitle", &[]);
    assert_eq!(
        ops::to_string(&mut realm, &title).unwrap().to_rust_lossy(),
        "Shared session"
    );
    assert!(matches!(
        call_method(&mut realm, &contents, "isCrashed", &[]),
        Value::Boolean(false)
    ));
    let first = call_method(&mut realm, &contents, "findInPage", &[Value::str("x")]);
    let second = call_method(&mut realm, &contents, "findInPage", &[Value::str("x")]);
    assert!(matches!(
        (first, second),
        (Value::Number(1.0), Value::Number(2.0))
    ));
    let load = ops::get(&mut realm, &contents, &Key::name("loadURL")).unwrap();
    let window_load = ops::get(&mut realm, &window, &Key::name("loadURL")).unwrap();
    assert!(matches!((&load, &window_load), (Value::Object(a), Value::Object(b)) if a == b));
    // The handle is the window's HWND, in a Buffer of its bytes.
    let handle = call_method(&mut realm, &window, "getNativeWindowHandle", &[]);
    let bytes = crate::typed_array_builtin::view_bytes(&handle).unwrap();
    assert_eq!(bytes.len(), 8);
    let id = property(&mut realm, window.as_object().unwrap(), "id");
    let raw = u64::from_le_bytes(bytes.try_into().unwrap()) as f64;
    assert!(matches!(id, Value::Number(id) if id == raw));
    call_method(&mut realm, &window, "destroy", &[]);
}

/// `webPreferences.additionalArguments` are the window's renderer
/// arguments, which a page realm's `process.argv` gives after the
/// executable and the renderer's type switch, as Electron gives them.
#[test]
fn additional_arguments_reach_the_window_renderer_process_argv() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let preferences = realm.object();
    let additional = realm.array(alloc::vec![
        Value::str("--vscode-window-config=vscode:42"),
        Value::str("--extra"),
    ]);
    preferences.define(
        Key::name("additionalArguments"),
        Property::data(Value::Object(additional)),
    );
    let options = realm.object();
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    options.define(
        Key::name("webPreferences"),
        Property::data(Value::Object(preferences)),
    );
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    let id = match property(&mut realm, window.as_object().unwrap(), "id") {
        Value::Number(id) => id as usize,
        other => panic!("{other:?}"),
    };
    let arguments = realm.electron_window_arguments(id);
    assert_eq!(arguments, ["--vscode-window-config=vscode:42", "--extra"]);

    let mut page = Realm::new();
    page.set_page_arguments(arguments);
    let argv = crate::process_builtin::argv(&mut page).unwrap();
    let length = ops::get(&mut page, &argv, &Key::name("length")).unwrap();
    assert!(matches!(length, Value::Number(4.0)));
    let mut texts = Vec::new();
    for index in 1..4 {
        let item = ops::get(&mut page, &argv, &Key::Index(index)).unwrap();
        texts.push(ops::to_string(&mut page, &item).unwrap().to_rust_lossy());
    }
    assert_eq!(
        texts,
        [
            "--type=renderer",
            "--vscode-window-config=vscode:42",
            "--extra"
        ]
    );
    call_method(&mut realm, &window, "destroy", &[]);
}

/// Electron's macOS window states exist on Windows, as VS Code reads them
/// on every platform: a window is never in simple full screen, has no
/// represented file and is not marked edited.
#[test]
fn macos_window_states_exist_on_windows() {
    let _host_lock = crate::app_host::lock_host_tests();
    let mut realm = Realm::new();
    let exports = module(&mut realm);
    let constructor = property(&mut realm, &exports, "BrowserWindow");
    let options = realm.object();
    options.define(Key::name("show"), Property::data(Value::Boolean(false)));
    let window = ops::construct(&mut realm, &constructor, &[Value::Object(options)]).unwrap();
    call_method(
        &mut realm,
        &window,
        "setSimpleFullScreen",
        &[Value::Boolean(true)],
    );
    for name in ["isSimpleFullScreen", "isDocumentEdited"] {
        assert!(
            matches!(
                call_method(&mut realm, &window, name, &[]),
                Value::Boolean(false)
            ),
            "{name}"
        );
    }
    let file = call_method(&mut realm, &window, "getRepresentedFilename", &[]);
    assert_eq!(
        ops::to_string(&mut realm, &file).unwrap().to_rust_lossy(),
        ""
    );
    call_method(&mut realm, &window, "destroy", &[]);
}

/// A realm has one `electron` module, as a process has in Electron: every
/// importer shares its `ipcMain` handlers and `app`.
#[test]
fn a_realm_has_one_electron_module_and_ipc_registry() {
    let mut realm = Realm::new();
    let first = module(&mut realm);
    let ipc = property(&mut realm, &first, "ipcMain");
    let handle = ops::get(&mut realm, &ipc, &Key::name("handle")).unwrap();
    let handler = realm.function("handler", 0, false, |_, _, _| Ok(Value::Undefined));
    ops::call(
        &mut realm,
        &handle,
        ipc.clone(),
        &[Value::str("probe:early"), Value::Object(handler)],
    )
    .unwrap();
    let second = module(&mut realm);
    assert!(first == second);
    let registry = realm.electron_ipc_registry().unwrap();
    assert!(registry.borrow_mut().take_handler("probe:early").is_some());
}

#[test]
fn loaded_files_are_reported_as_file_urls() {
    assert_eq!(
        file_url(r"C:\My App\pages\index#1.html"),
        "file:///C:/My%20App/pages/index%231.html"
    );
}
