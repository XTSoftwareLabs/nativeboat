use super::*;
use alloc::rc::Rc;
use core::cell::RefCell;

#[derive(Default)]
struct FakeDialogs {
    files: RefCell<Vec<(usize, FileOptions, bool)>>,
    messages: RefCell<Vec<(usize, MessageOptions)>>,
    errors: RefCell<Vec<(usize, String, String)>>,
    selected_files: Vec<String>,
    selected_button: i64,
    checked: bool,
    failure: Option<u32>,
}

impl DialogBackend for FakeDialogs {
    fn choose_files(
        &self,
        owner: usize,
        options: &FileOptions,
        save: bool,
    ) -> Result<Vec<String>, u32> {
        if let Some(code) = self.failure {
            return Err(code);
        }
        self.files.borrow_mut().push((owner, options.clone(), save));
        Ok(self.selected_files.clone())
    }

    fn message(&self, owner: usize, options: &MessageOptions) -> Result<(i64, bool), u32> {
        if let Some(code) = self.failure {
            return Err(code);
        }
        self.messages.borrow_mut().push((owner, options.clone()));
        Ok((self.selected_button, self.checked))
    }

    fn error_box(&self, owner: usize, title: &str, message: &str) -> Result<(), u32> {
        self.errors
            .borrow_mut()
            .push((owner, title.to_owned(), message.to_owned()));
        Ok(())
    }
}

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

fn fulfilled(value: Value) -> Value {
    let Value::Object(promise) = value else {
        panic!("expected Promise")
    };
    let crate::object::Kind::Promise(state) = &promise.data().kind else {
        panic!("result must be a Promise")
    };
    let crate::promise_builtin::State::Fulfilled(value) = state.borrow().state.clone() else {
        panic!("Promise must be fulfilled")
    };
    value
}

fn rejected(value: Value) -> Value {
    let Value::Object(promise) = value else {
        panic!("expected Promise")
    };
    let crate::object::Kind::Promise(state) = &promise.data().kind else {
        panic!("result must be a Promise")
    };
    let crate::promise_builtin::State::Rejected(value) = state.borrow().state.clone() else {
        panic!("Promise must be rejected")
    };
    value
}

fn array(realm: &Realm, values: &[&str]) -> Object {
    let array = crate::array_builtin::array_create(realm, values.len() as u64).unwrap();
    for (index, value) in values.iter().enumerate() {
        array.define(Key::Index(index as u32), Property::data(Value::str(value)));
    }
    array
}

fn options(realm: &Realm, fields: &[(&str, Value)]) -> Object {
    let object = realm.object();
    for (name, value) in fields {
        object.define(Key::name(name), Property::data(value.clone()));
    }
    object
}

#[test]
fn module_exports_the_dialog_operations_and_their_synchronous_forms() {
    let mut realm = Realm::new();
    let dialog = module(&realm);
    for name in [
        "showOpenDialog",
        "showOpenDialogSync",
        "showSaveDialog",
        "showSaveDialogSync",
        "showMessageBox",
        "showMessageBoxSync",
        "showErrorBox",
    ] {
        assert!(matches!(
            property(&mut realm, &dialog, name),
            Value::Object(_)
        ));
    }
}

#[test]
fn open_dialog_parses_marktext_directory_and_filter_options_and_returns_promise() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs {
        selected_files: vec!["C:\\notes".to_owned()],
        ..FakeDialogs::default()
    });
    let dialog = module_with(&realm, fake.clone());
    let properties = array(&realm, &["openDirectory", "createDirectory"]);
    let extensions = array(&realm, &["md", ".txt"]);
    let filter = options(
        &realm,
        &[
            ("name", Value::str("Markdown")),
            ("extensions", Value::Object(extensions)),
        ],
    );
    let filters = crate::array_builtin::array_create(&realm, 1).unwrap();
    filters.define(Key::Index(0), Property::data(Value::Object(filter)));
    let input = options(
        &realm,
        &[
            ("title", Value::str("Open notes")),
            ("defaultPath", Value::str("C:\\notes")),
            ("properties", Value::Object(properties)),
            ("filters", Value::Object(filters)),
        ],
    );
    let open = property(&mut realm, &dialog, "showOpenDialog");
    let result = ops::call(
        &mut realm,
        &open,
        Value::Object(dialog),
        &[Value::Object(input)],
    )
    .unwrap();
    let Value::Object(result) = fulfilled(result) else {
        panic!("resolved dialog result must be an object")
    };
    assert!(matches!(
        property(&mut realm, &result, "canceled"),
        Value::Boolean(false)
    ));
    let Value::Object(paths) = property(&mut realm, &result, "filePaths") else {
        panic!("filePaths must be an array")
    };
    assert!(
        matches!(ops::get(&mut realm, &Value::Object(paths), &Key::Index(0)).unwrap(), Value::String(path) if path.to_rust_lossy() == "C:\\notes")
    );
    assert_eq!(
        fake.files.borrow().as_slice(),
        &[(
            0,
            FileOptions {
                properties: vec!["openDirectory".to_owned(), "createDirectory".to_owned()],
                title: Some("Open notes".to_owned()),
                default_path: Some("C:\\notes".to_owned()),
                filters: vec![(
                    "Markdown".to_owned(),
                    vec!["md".to_owned(), ".txt".to_owned()]
                )],
            },
            false
        )]
    );
}

#[test]
fn message_box_preserves_localized_buttons_detail_and_selected_response() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs {
        selected_button: 2,
        checked: true,
        ..FakeDialogs::default()
    });
    let dialog = module_with(&realm, fake.clone());
    let buttons = array(&realm, &["Discard", "Cancel", "Save"]);
    let input = options(
        &realm,
        &[
            ("type", Value::str("warning")),
            ("message", Value::str("Unsaved changes")),
            ("detail", Value::str("Save before closing?")),
            ("buttons", Value::Object(buttons)),
            ("defaultId", Value::Number(2.0)),
            ("cancelId", Value::Number(1.0)),
        ],
    );
    let method = property(&mut realm, &dialog, "showMessageBox");
    let result = ops::call(
        &mut realm,
        &method,
        Value::Object(dialog),
        &[Value::Object(input)],
    )
    .unwrap();
    let Value::Object(result) = fulfilled(result) else {
        panic!("resolved message result must be an object")
    };
    assert!(matches!(
        property(&mut realm, &result, "response"),
        Value::Number(2.0)
    ));
    assert!(matches!(
        property(&mut realm, &result, "checkboxChecked"),
        Value::Boolean(true)
    ));
    assert_eq!(
        fake.messages.borrow().as_slice(),
        &[(
            0,
            MessageOptions {
                title: None,
                message: "Unsaved changes".to_owned(),
                detail: Some("Save before closing?".to_owned()),
                kind: "warning".to_owned(),
                buttons: vec!["Discard".to_owned(), "Cancel".to_owned(), "Save".to_owned()],
                default_id: 2,
                cancel_id: 1,
                no_link: false,
                checkbox_label: None,
                checkbox_checked: false,
            }
        )]
    );
}

#[test]
fn parented_message_box_uses_the_browser_window_native_handle() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs::default());
    let dialog = module_with(&realm, fake.clone());
    let window = realm.object();
    window.define(
        Key::Symbol(realm.electron_window_key.clone()),
        Property::hidden(Value::Number(0x1234 as f64)),
    );
    let input = options(&realm, &[("message", Value::str("Confirm"))]);
    let method = property(&mut realm, &dialog, "showMessageBox");
    ops::call(
        &mut realm,
        &method,
        Value::Object(dialog),
        &[Value::Object(window), Value::Object(input)],
    )
    .unwrap();
    assert_eq!(fake.messages.borrow()[0].0, 0x1234);
}

#[test]
fn canceled_file_picker_resolves_with_electron_canceled_shape() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs::default());
    let dialog = module_with(&realm, fake);
    let method = property(&mut realm, &dialog, "showOpenDialog");
    let result = fulfilled(ops::call(&mut realm, &method, Value::Object(dialog), &[]).unwrap());
    let Value::Object(result) = result else {
        panic!("resolved open result must be an object")
    };
    assert!(matches!(
        property(&mut realm, &result, "canceled"),
        Value::Boolean(true)
    ));
    let Value::Object(paths) = property(&mut realm, &result, "filePaths") else {
        panic!("filePaths must be an array")
    };
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(paths), &Key::name("length")).unwrap(),
        Value::Number(0.0)
    ));
}

#[test]
fn operating_system_dialog_failures_reject_the_electron_promise() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs {
        failure: Some(5),
        ..FakeDialogs::default()
    });
    let dialog = module_with(&realm, fake);
    let method = property(&mut realm, &dialog, "showOpenDialog");
    let error = rejected(ops::call(&mut realm, &method, Value::Object(dialog), &[]).unwrap());
    let Value::Object(error) = error else {
        panic!("the rejection reason must be an Error")
    };
    assert!(
        matches!(ops::get(&mut realm, &Value::Object(error), &Key::name("message")).unwrap(), Value::String(message) if message.to_rust_lossy().contains("Windows error 5"))
    );
}

#[test]
fn save_dialog_returns_electron_file_path_and_cancellation_fields() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs {
        selected_files: vec!["C:\\notes\\new.md".to_owned()],
        ..FakeDialogs::default()
    });
    let dialog = module_with(&realm, fake);
    let input = options(&realm, &[("defaultPath", Value::str("C:\\notes\\new.md"))]);
    let save = property(&mut realm, &dialog, "showSaveDialog");
    let result = fulfilled(
        ops::call(
            &mut realm,
            &save,
            Value::Object(dialog),
            &[Value::Object(input)],
        )
        .unwrap(),
    );
    let Value::Object(result) = result else {
        panic!("resolved save result must be an object")
    };
    assert!(matches!(
        property(&mut realm, &result, "canceled"),
        Value::Boolean(false)
    ));
    assert!(
        matches!(property(&mut realm, &result, "filePath"), Value::String(path) if path.to_rust_lossy() == "C:\\notes\\new.md")
    );
}

#[test]
fn error_box_is_synchronous_and_preserves_title_and_stack() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeDialogs::default());
    let dialog = module_with(&realm, fake.clone());
    let method = property(&mut realm, &dialog, "showErrorBox");
    assert!(matches!(
        ops::call(
            &mut realm,
            &method,
            Value::Object(dialog),
            &[Value::str("Fatal"), Value::str("at main.js:12")]
        ),
        Ok(Value::Undefined)
    ));
    assert_eq!(
        fake.errors.borrow().as_slice(),
        &[(0, "Fatal".to_owned(), "at main.js:12".to_owned())]
    );
}

#[test]
fn file_filter_and_multiselect_helpers_preserve_windows_common_dialog_contract() {
    let filter = filter_string(&[(
        "Markdown".to_owned(),
        vec!["md".to_owned(), ".txt".to_owned()],
    )]);
    assert_eq!(
        String::from_utf16_lossy(&filter),
        "Markdown\0*.md;*.txt\0\0"
    );
    let parsed = parse_multi_path(&[
        b'C' as u16,
        b':' as u16,
        b'\\' as u16,
        b'd' as u16,
        0,
        b'a' as u16,
        b'.' as u16,
        b'm' as u16,
        b'd' as u16,
        0,
        b'b' as u16,
        b'.' as u16,
        b'm' as u16,
        b'd' as u16,
        0,
        0,
    ]);
    assert_eq!(parsed, ["C:\\d\\a.md", "C:\\d\\b.md"]);
}

#[test]
fn invalid_options_and_button_indexes_return_javascript_errors() {
    let mut realm = Realm::new();
    assert!(file_options(&mut realm, &Value::Number(1.0)).is_err());
    let invalid = |realm: &mut Realm, name: &str, value: Value| {
        let options = realm.object();
        options.define(Key::name(name), Property::data(value));
        message_options(realm, &Value::Object(options)).is_err()
    };
    assert!(invalid(&mut realm, "defaultId", Value::str("first")));
    assert!(invalid(&mut realm, "type", Value::str("fancy")));
    assert!(invalid(&mut realm, "noLink", Value::Number(1.0)));
    assert!(invalid(&mut realm, "checkboxChecked", Value::Boolean(true)));
    // Electron's own default, -1, names no default button.
    assert!(!invalid(&mut realm, "defaultId", Value::Number(-1.0)));
}

#[test]
fn message_options_take_electrons_defaults() {
    let mut realm = Realm::new();
    realm.electron_app_name = Some("ytdownloader");
    let parse = |realm: &mut Realm, buttons: &[&str], fields: &[(&str, Value)]| {
        let mut fields = fields.to_vec();
        let list = array(realm, buttons);
        fields.push(("buttons", Value::Object(list)));
        let input = options(realm, &fields);
        message_options(realm, &Value::Object(input)).unwrap()
    };
    let plain = parse(
        &mut realm,
        &[],
        &[("title", Value::str("")), ("detail", Value::str(""))],
    );
    // An empty title shows the application's name, and an empty detail none.
    assert_eq!(plain.title.as_deref(), Some("ytdownloader"));
    assert_eq!(plain.detail, None);
    assert!(plain.buttons.is_empty());
    assert_eq!((plain.default_id, plain.cancel_id), (-1, 0));
    // "Cancel" or "No" cancels, else the button after a first default.
    let cancel = |realm: &mut Realm, buttons: &[&str], default: Option<f64>| {
        let fields: Vec<(&str, Value)> = default
            .map(|id| ("defaultId", Value::Number(id)))
            .into_iter()
            .collect();
        parse(realm, buttons, &fields).cancel_id
    };
    assert_eq!(cancel(&mut realm, &["Save", "NO", "Cancel"], None), 1);
    assert_eq!(cancel(&mut realm, &["Clear", "Keep"], Some(0.0)), 1);
    assert_eq!(cancel(&mut realm, &["Clear", "Keep"], None), 0);
    assert_eq!(cancel(&mut realm, &["Clear"], Some(0.0)), 0);
    let checked = parse(
        &mut realm,
        &["OK"],
        &[
            ("checkboxLabel", Value::str("Remember")),
            ("checkboxChecked", Value::Number(1.0)),
            ("noLink", Value::Boolean(true)),
            ("cancelId", Value::Number(3.0)),
        ],
    );
    assert_eq!(checked.checkbox_label.as_deref(), Some("Remember"));
    assert!(checked.checkbox_checked && checked.no_link);
    assert_eq!(checked.cancel_id, 3);
}

#[test]
fn task_dialog_buttons_follow_electrons_layout() {
    let labels = |names: &[&str]| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    // Buttons labelled as the dialog's own are those; the others are custom.
    let plan = button_plan(&labels(&["ok", "Discard", "CANCEL"]), false);
    assert_eq!(plan.common, 0x01 | 0x08);
    assert_eq!(plan.common_ids, [(1, 0), (2, 2)]);
    assert_eq!(plan.custom, [1]);
    assert_eq!(response(&plan, 1, 9), 0);
    assert_eq!(response(&plan, 2, 9), 2);
    assert_eq!(response(&plan, 101, 9), 1);
    // Closed without a button: the cancel ID.
    assert_eq!(response(&plan, 0, 9), 9);
    // `noLink` keeps every button the application's own.
    let plan = button_plan(&labels(&["Yes", "No", "Retry", "Close"]), true);
    assert_eq!((plan.common, plan.custom.len()), (0, 4));
    let plan = button_plan(&labels(&["Yes", "No", "Retry", "Close"]), false);
    assert_eq!(plan.common, 0x02 | 0x04 | 0x10 | 0x20);
    assert_eq!(plan.common_ids, [(6, 0), (7, 1), (4, 2), (8, 3)]);
}

#[test]
fn synchronous_dialogs_return_their_results() {
    let mut realm = Realm::new();
    let call = |realm: &mut Realm, fake: FakeDialogs, name: &str| {
        let dialog = module_with(realm, Rc::new(fake));
        let method = property(realm, &dialog, name);
        ops::call(realm, &method, Value::Object(dialog), &[])
    };
    let chosen = || FakeDialogs {
        selected_files: vec!["C:\\a.md".to_owned()],
        selected_button: 1,
        ..FakeDialogs::default()
    };
    assert!(matches!(
        call(&mut realm, chosen(), "showMessageBoxSync").unwrap(),
        Value::Number(1.0)
    ));
    assert!(matches!(
        call(&mut realm, chosen(), "showSaveDialogSync").unwrap(),
        Value::String(path) if path.to_rust_lossy() == "C:\\a.md"
    ));
    let Value::Object(paths) = call(&mut realm, chosen(), "showOpenDialogSync").unwrap() else {
        panic!("chosen paths must be an array")
    };
    assert!(matches!(
        ops::get(&mut realm, &Value::Object(paths), &Key::Index(0)).unwrap(),
        Value::String(path) if path.to_rust_lossy() == "C:\\a.md"
    ));
    // Canceled: no paths, and an empty path.
    assert!(matches!(
        call(&mut realm, FakeDialogs::default(), "showOpenDialogSync").unwrap(),
        Value::Undefined
    ));
    assert!(matches!(
        call(&mut realm, FakeDialogs::default(), "showSaveDialogSync").unwrap(),
        Value::String(path) if path.to_rust_lossy().is_empty()
    ));
    // Failures throw.
    for name in [
        "showMessageBoxSync",
        "showOpenDialogSync",
        "showSaveDialogSync",
    ] {
        let failing = FakeDialogs {
            failure: Some(5),
            ..FakeDialogs::default()
        };
        assert!(call(&mut realm, failing, name).is_err());
    }
}

#[test]
fn task_dialog_config_matches_the_packed_windows_layout() {
    // `TASKDIALOGCONFIG` is declared with one-byte packing; on x64 it is 160
    // bytes, and the verification flag is a call parameter, not a field.
    assert_eq!(core::mem::size_of::<TaskDialogConfig>(), 160);
    assert_eq!(core::mem::size_of::<TaskDialogButton>(), 12);
}

#[test]
fn message_titles_default_to_the_application_name() {
    let mut realm = Realm::new();
    let options = Value::Object(realm.object());
    assert_eq!(message_options(&mut realm, &options).unwrap().title, None);
    realm.electron_app_name = Some("Notes");
    assert_eq!(
        message_options(&mut realm, &options)
            .unwrap()
            .title
            .as_deref(),
        Some("Notes")
    );
}
