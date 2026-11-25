use super::*;

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn shell_exports_windows_operations_and_validates_required_strings() {
    let mut realm = Realm::new();
    let shell = module(&realm);
    for name in ["openExternal", "openPath", "showItemInFolder", "trashItem"] {
        assert!(matches!(
            property(&mut realm, &shell, name),
            Value::Object(_)
        ));
    }
    let open_external = property(&mut realm, &shell, "openExternal");
    assert!(ops::call(&mut realm, &open_external, Value::Undefined, &[]).is_err());
    assert!(
        ops::call(
            &mut realm,
            &open_external,
            Value::Undefined,
            &[Value::Number(7.0)]
        )
        .is_err()
    );
    let open_path = property(&mut realm, &shell, "openPath");
    assert!(ops::call(&mut realm, &open_path, Value::Undefined, &[]).is_err());
    let reveal = property(&mut realm, &shell, "showItemInFolder");
    assert!(ops::call(&mut realm, &reveal, Value::Undefined, &[]).is_err());
    let trash = property(&mut realm, &shell, "trashItem");
    assert!(ops::call(&mut realm, &trash, Value::Undefined, &[]).is_err());
}

#[test]
fn shell_encodes_unicode_paths_and_reports_shell_error_messages() {
    assert_eq!(
        wide("D:\\笔记.md"),
        [68, 58, 92, 31508, 35760, 46, 109, 100, 0]
    );
    assert!(shell_error(2).contains("not found"));
    assert!(shell_error(3).contains("not found"));
    assert!(shell_error(5).contains("denied"));
    assert!(shell_error(8).contains("memory"));
    assert!(shell_error(31).contains("associated"));
    assert!(shell_error(87).contains("87"));
}

#[test]
fn shell_open_external_validates_url_options_without_launching_processes() {
    let mut realm = Realm::new();
    let long_url = "x".repeat(2082);
    let shell = module(&realm);
    let open_external = property(&mut realm, &shell, "openExternal");
    assert!(
        ops::call(
            &mut realm,
            &open_external,
            Value::Undefined,
            &[Value::str(&long_url)]
        )
        .is_err()
    );
    let args = [Value::str("https://example.invalid"), Value::str("invalid")];
    assert!(option_string(&mut realm, &args, "workingDirectory").is_err());
    let options = realm.object();
    options.define(
        Key::name("workingDirectory"),
        Property::data(Value::str("C:\\Workspace")),
    );
    let args = [
        Value::str("https://example.invalid"),
        Value::Object(options),
    ];
    assert_eq!(
        option_string(&mut realm, &args, "workingDirectory").unwrap(),
        Some("C:\\Workspace".to_owned())
    );
}
