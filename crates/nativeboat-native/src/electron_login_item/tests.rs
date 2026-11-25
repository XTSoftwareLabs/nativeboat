use super::*;

/// Login item keys of the test's own, removed afterwards; the real `Run`
/// key is never touched.
struct Scratch {
    base: String,
    keys: Keys,
}

impl Scratch {
    fn new() -> Self {
        let base = alloc::format!(
            "Software\\NativeBoatTests-login-{:x}",
            crate::platform::random()
        );
        let keys = Keys {
            root: HKEY_CURRENT_USER,
            classes: alloc::format!("{base}\\Classes"),
            run: alloc::format!("{base}\\Run"),
            approved: alloc::format!("{base}\\Approved"),
            machine_run: None,
        };
        Self { base, keys }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let path = wide(&self.base);
        // SAFETY: the path is NUL-terminated and names the test's own key.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
    }
}

fn enabled_with(args: &[&str]) -> Settings {
    Settings {
        open_at_login: true,
        args: args.iter().map(|arg| String::from(*arg)).collect(),
        enabled: true,
        ..Settings::default()
    }
}

#[test]
fn login_items_are_run_values_marked_when_disabled_and_removed_when_off() {
    let scratch = Scratch::new();
    let keys = &scratch.keys;
    let exe = "C:\\Apps\\My App\\app.exe";
    let settings = enabled_with(&["--hidden"]);
    set_login_item(keys, "com.example.app", exe, &settings);
    let run = RegKey::open(keys.root, &keys.run, false).unwrap();
    let (kind, data) = run.get("com.example.app").unwrap();
    assert_eq!(kind, REG_SZ);
    assert_eq!(
        string_value(&data),
        "\"C:\\Apps\\My App\\app.exe\" --hidden"
    );
    let state = login_item(keys, "com.example.app", exe, &settings);
    assert!(state.open_at_login);
    assert_eq!(state.launch_items.len(), 1);
    let item = &state.launch_items[0];
    assert_eq!(
        (
            item.name.as_str(),
            item.path.as_str(),
            item.scope,
            item.enabled
        ),
        ("com.example.app", exe, "user", true)
    );
    assert_eq!(item.args, ["--hidden"]);
    // Other arguments make another command.
    assert!(!login_item(keys, "com.example.app", exe, &enabled_with(&[])).open_at_login);

    let disabled = Settings {
        enabled: false,
        ..enabled_with(&["--hidden"])
    };
    set_login_item(keys, "com.example.app", exe, &disabled);
    let approved = RegKey::open(keys.root, &keys.approved, false).unwrap();
    assert_eq!(approved.get("com.example.app").unwrap().1[0], 3);
    assert!(!login_item(keys, "com.example.app", exe, &settings).launch_items[0].enabled);

    // Another program's entries are not this app's.
    RegKey::open(keys.root, &keys.run, true)
        .unwrap()
        .set_string("other", "\"C:\\Other\\other.exe\" --x");
    assert_eq!(
        login_item(keys, "com.example.app", exe, &settings)
            .launch_items
            .len(),
        1
    );

    set_login_item(keys, "com.example.app", exe, &Settings::default());
    assert!(run.get("com.example.app").is_none());
    assert!(approved.get("com.example.app").is_none());
    assert!(run.get("other").is_some());
}

#[test]
fn commands_split_as_windows_does() {
    assert_eq!(
        split_command("\"C:\\A B\\app.exe\" --one \"two three\""),
        ["C:\\A B\\app.exe", "--one", "two three"]
    );
    assert_eq!(command_line("C:\\app.exe", &[]), "\"C:\\app.exe\"");
}

#[test]
fn app_methods_name_items_after_the_model_id_and_return_electron_shapes() {
    let scratch = Scratch::new();
    let mut realm = Realm::new();
    realm.electron_app_name = Some("Login Test");
    let app = realm.object();
    install_with(&realm, &app, scratch.keys.clone());
    let method = |realm: &mut Realm, name: &'static str| {
        ops::get_named(realm, &Value::Object(app.clone()), name).unwrap()
    };
    let options = |realm: &mut Realm, open: bool| {
        let options = realm.object();
        options.define(
            Key::name("openAtLogin"),
            Property::data(Value::Boolean(open)),
        );
        options.define(
            Key::name("args"),
            Property::data(strings(realm, &[String::from("--tray")]).unwrap()),
        );
        Value::Object(options)
    };
    let run = || RegKey::open(scratch.keys.root, &scratch.keys.run, false).unwrap();

    // Without a model id, Electron's default names the item.
    let set = method(&mut realm, "setLoginItemSettings");
    let on = options(&mut realm, true);
    ops::call(
        &mut realm,
        &set,
        Value::Undefined,
        std::slice::from_ref(&on),
    )
    .unwrap();
    assert!(run().get("electron.app.Login Test").is_some());
    let off = options(&mut realm, false);
    ops::call(
        &mut realm,
        &set,
        Value::Undefined,
        std::slice::from_ref(&off),
    )
    .unwrap();
    assert!(run().get("electron.app.Login Test").is_none());

    let set_id = method(&mut realm, "setAppUserModelId");
    ops::call(
        &mut realm,
        &set_id,
        Value::Undefined,
        &[Value::str("com.example.login")],
    )
    .unwrap();
    ops::call(
        &mut realm,
        &set,
        Value::Undefined,
        std::slice::from_ref(&on),
    )
    .unwrap();
    let executable = crate::platform::executable_path().unwrap().to_rust_lossy();
    assert_eq!(
        string_value(&run().get("com.example.login").unwrap().1),
        alloc::format!("\"{executable}\" --tray")
    );
    let get = method(&mut realm, "getLoginItemSettings");
    let state = ops::call(&mut realm, &get, Value::Undefined, &[on]).unwrap();
    let flag = |realm: &mut Realm, name: &'static str| {
        ops::to_boolean(&ops::get_named(realm, &state, name).unwrap())
    };
    assert!(flag(&mut realm, "openAtLogin"));
    assert!(flag(&mut realm, "executableWillLaunchAtLogin"));
    assert!(!flag(&mut realm, "wasOpenedAtLogin"));
    let items = ops::get_named(&mut realm, &state, "launchItems").unwrap();
    let item = ops::get(&mut realm, &items, &Key::Index(0)).unwrap();
    let name = ops::get_named(&mut realm, &item, "name").unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &name).unwrap().to_rust_lossy(),
        "com.example.login"
    );
    ops::call(&mut realm, &set, Value::Undefined, &[off]).unwrap();
    assert!(run().get("com.example.login").is_none());
}

/// A scheme's handler is the quoted executable with its arguments and the
/// quoted URL, under the scheme's `URL:` key. Removing it leaves another
/// program's handler alone and cleans up the program's own.
#[test]
fn protocol_clients_are_registered_checked_and_removed_as_electron_keeps_them() {
    let scratch = Scratch::new();
    let keys = &scratch.keys;
    let exe = "C:\\Apps\\My App\\app.exe";
    let command = protocol_command(exe, &[String::from("--open-url"), String::from("--")]);
    assert_eq!(
        command,
        "\"C:\\Apps\\My App\\app.exe\" --open-url -- \"%1\""
    );
    assert_eq!(
        protocol_command(exe, &[]),
        "\"C:\\Apps\\My App\\app.exe\" \"%1\""
    );
    assert!(!is_protocol_client(keys, "nativeboat-test", &command));
    assert!(set_protocol_client(keys, "nativeboat-test", &command));
    assert!(is_protocol_client(keys, "nativeboat-test", &command));
    assert!(!is_protocol_client(
        keys,
        "nativeboat-test",
        "\"other.exe\" \"%1\""
    ));
    let path = alloc::format!("{}\\nativeboat-test", keys.classes);
    let key = RegKey::open(keys.root, &path, false).unwrap();
    assert_eq!(string_value(&key.get("").unwrap().1), "URL:nativeboat-test");
    assert_eq!(string_value(&key.get("URL Protocol").unwrap().1), "");
    drop(key);
    assert!(remove_protocol_client(
        keys,
        "nativeboat-test",
        "\"other.exe\" \"%1\""
    ));
    assert!(is_protocol_client(keys, "nativeboat-test", &command));
    assert!(remove_protocol_client(keys, "nativeboat-test", &command));
    assert!(!is_protocol_client(keys, "nativeboat-test", &command));
    assert!(RegKey::open(keys.root, &path, false).is_none());
    assert!(!set_protocol_client(keys, "", &command));
}
