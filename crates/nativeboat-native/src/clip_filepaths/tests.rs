use super::*;

// These tests never touch the system clipboard: they check the formats the
// module writes and reads, and its argument checks.

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

#[test]
fn file_lists_round_trip_through_dropfiles_blocks() {
    let paths = strings(&["C:\\a b\\one.png", "D:\\\u{56fe}.jpg"]);
    let block = drop_files(&paths);
    assert_eq!(&block[..4], &(DROPFILES as u32).to_le_bytes());
    assert_eq!(&block[16..20], &1u32.to_le_bytes());
    assert!(block.ends_with(&[0, 0, 0, 0]));
    assert_eq!(dropped_paths(&block), paths);
    // ANSI lists, as older programs write them.
    let mut ansi = Vec::new();
    ansi.extend((DROPFILES as u32).to_le_bytes());
    ansi.extend([0; 16]);
    ansi.extend(b"C:\\x.txt\0C:\\y.txt\0\0");
    assert_eq!(dropped_paths(&ansi), strings(&["C:\\x.txt", "C:\\y.txt"]));
    assert!(dropped_paths(&[1, 2]).is_empty());
}

#[test]
fn text_form_lists_file_urls_against_the_working_directory() {
    let paths = strings(&["C:\\pics\\a.png", "b.png", "\\\\server\\share\\c.png"]);
    assert_eq!(
        file_urls(&paths, "D:\\work\\"),
        "file:///C:/pics/a.png\nfile:///D:/work/b.png\nfile://///server/share/c.png"
    );
}

#[test]
fn writing_checks_every_path_before_touching_the_clipboard() {
    let realm = &mut Realm::new();
    let exports = Value::Object(module(realm));
    let write = ops::get_named(realm, &exports, "writeClipboardFilePaths").unwrap();
    let paths = realm.array(alloc::vec![Value::str("Z:\\nativeboat-missing\\file.png")]);
    let error = ops::call(realm, &write, Value::Undefined, &[Value::Object(paths)]).unwrap_err();
    let message = ops::get_named(realm, &error.0, "message").unwrap();
    let message = ops::to_string(realm, &message).unwrap().to_rust_lossy();
    assert!(
        message.starts_with(
            "windows clipboard error: Some paths could not be processed: Failed to canonicalize path Z:\\nativeboat-missing\\file.png: "
        ),
        "{message}"
    );
    assert!(message.contains("(os error 3)"), "{message}");
    assert!(ops::call(realm, &write, Value::Undefined, &[]).is_err());
    let hello = ops::get_named(realm, &exports, "helloWorld").unwrap();
    let greeting = ops::call(realm, &hello, Value::Undefined, &[]).unwrap();
    assert!(crate::same_value(
        &greeting,
        &Value::str("Hello from Rust 0.3.0 on Windows!")
    ));
}
