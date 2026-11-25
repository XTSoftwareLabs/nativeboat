use super::*;

#[test]
fn windows_errors_translate_like_libuv() {
    assert_eq!(translate(2), -4058); // ERROR_FILE_NOT_FOUND: ENOENT
    assert_eq!(translate(5), -4048); // ERROR_ACCESS_DENIED: EPERM
    assert_eq!(translate(10013), -4092); // WSAEACCES: EACCES
    assert_eq!(translate(123_456), UNKNOWN);
    assert_eq!(ERRORS.len(), 84);
    assert_eq!(TRANSLATIONS.len(), 100);
}

#[test]
fn codes_have_libuv_names_and_messages() {
    assert_eq!(
        describe(-4058),
        ("ENOENT".to_owned(), "no such file or directory".to_owned())
    );
    assert_eq!(
        describe(-4095),
        ("EOF".to_owned(), "end of file".to_owned())
    );
    assert_eq!(
        describe(-1),
        (
            "Unknown system error -1".to_owned(),
            "Unknown system error -1".to_owned()
        )
    );
}

#[test]
fn failures_are_recorded_on_the_context() {
    let mut realm = Realm::new();
    let context = Value::Object(realm.object());
    collect(&mut realm, &context, -4058, "uv_os_homedir").unwrap();
    let shown = crate::inspect::inspect(&mut realm, &context, 2).unwrap();
    assert_eq!(
        shown,
        "{\n  errno: -4058,\n  code: 'ENOENT',\n  message: 'no such file or directory',\n  syscall: 'uv_os_homedir'\n}"
    );
    let frozen = realm.object();
    frozen.data_mut().extensible = false;
    let thrown = collect(&mut realm, &Value::Object(frozen), -4058, "x").unwrap_err();
    let text = crate::inspect::inspect(&mut realm, &thrown.0, 0).unwrap();
    assert!(text.contains("TypeError"), "{text}");
}
