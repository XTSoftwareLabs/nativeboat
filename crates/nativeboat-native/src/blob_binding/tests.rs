use super::*;

fn accessor() -> Property {
    Property {
        slot: Slot::Accessor {
            get: None,
            set: None,
        },
        enumerable: false,
        configurable: true,
    }
}

#[test]
fn blob_private_readers_and_handles_cover_invalid_and_accessor_states() {
    let mut realm = Realm::new();
    let reader = Value::Object(realm.object());
    let bytes_key = Symbol::private("reader-bytes");
    let offset_key = Symbol::private("reader-offset");
    assert!(reader_bytes(&realm, &reader, &bytes_key).is_err());
    assert_eq!(reader_offset(&mut realm, &reader, &offset_key).unwrap(), 0);

    reader
        .as_object()
        .unwrap()
        .set_private(&bytes_key, accessor());
    reader
        .as_object()
        .unwrap()
        .set_private(&offset_key, accessor());
    assert!(reader_bytes(&realm, &reader, &bytes_key).is_err());
    assert_eq!(reader_offset(&mut realm, &reader, &offset_key).unwrap(), 0);

    let bytes = Value::Object(typed_array_builtin::new_bytes(&realm, vec![4, 5, 6], None));
    reader
        .as_object()
        .unwrap()
        .set_private(&bytes_key, Property::data(bytes.clone()));
    reader
        .as_object()
        .unwrap()
        .set_private(&offset_key, Property::data(Value::Number(2.0)));
    assert_eq!(
        reader_bytes(&realm, &reader, &bytes_key).unwrap(),
        [4, 5, 6]
    );
    assert_eq!(reader_offset(&mut realm, &reader, &offset_key).unwrap(), 2);

    let handle_key = Symbol::private("handle-bytes");
    let handle = Value::Object(handle(&mut realm, vec![1, 2], &handle_key));
    assert_eq!(handle_bytes(&realm, &handle, &handle_key).unwrap(), [1, 2]);
    handle
        .as_object()
        .unwrap()
        .set_private(&handle_key, accessor());
    assert!(handle_bytes(&realm, &handle, &handle_key).is_err());
    assert!(contents(&Value::Undefined, &handle_key).is_none());
}

#[test]
fn blob_file_paths_reject_non_strings_nulls_and_missing_files() {
    assert!(file_bytes(&Value::Number(1.0)).is_none());
    assert!(file_bytes(&Value::str("C:\\bad\0path")).is_none());
    assert!(file_bytes(&Value::str("N:\\nativeboat-missing-blob")).is_none());
}

#[test]
fn blob_array_buffer_copies_native_bytes() {
    let mut realm = Realm::new();
    let result = array_buffer(&mut realm, vec![7, 8, 9]).unwrap();
    assert_eq!(
        typed_array_builtin::buffer_of(&result)
            .unwrap()
            .borrow()
            .as_slice(),
        &[7, 8, 9]
    );
}

#[test]
fn blob_concat_rejects_parts_that_are_not_array_buffers() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let concat = ops::get(&mut realm, &exports, &Key::name("concat")).unwrap();
    let parts = Value::Object(realm.array(vec![Value::Null]));
    assert!(ops::call(&mut realm, &concat, Value::Undefined, &[parts]).is_err());
}
