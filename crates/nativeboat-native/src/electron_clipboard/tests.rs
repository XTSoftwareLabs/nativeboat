use super::*;
use alloc::collections::BTreeMap;
use alloc::string::String;
use core::cell::RefCell;

#[derive(Default)]
struct FakeClipboard {
    contents: RefCell<BTreeMap<u32, Vec<u8>>>,
    formats: RefCell<BTreeMap<String, u32>>,
}

impl ClipboardBackend for FakeClipboard {
    fn read(&self, format: u32) -> Result<Option<Vec<u8>>, u32> {
        Ok(self.contents.borrow().get(&format).cloned())
    }

    fn replace(&self, format: u32, bytes: &[u8]) -> Result<(), u32> {
        let mut contents = self.contents.borrow_mut();
        contents.clear();
        contents.insert(format, bytes.to_vec());
        Ok(())
    }

    fn register_format(&self, name: &str) -> Result<u32, u32> {
        if name.is_empty() || name.contains('\0') {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if let Some(format) = self.formats.borrow().get(name) {
            return Ok(*format);
        }
        let format = 0xc000 + self.formats.borrow().len() as u32;
        self.formats.borrow_mut().insert(name.to_owned(), format);
        Ok(format)
    }

    fn available(&self, format: u32) -> bool {
        self.contents.borrow().contains_key(&format)
    }
}

fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
    ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
}

#[test]
fn unicode_text_payload_round_trips_and_stops_at_the_first_terminator() {
    let source: Vec<u16> = "notes – 日本語 😀".encode_utf16().collect();
    let encoded = encode_utf16(&source);
    assert_eq!(decode_utf16(&encoded), source);
    let mut with_tail = encoded;
    with_tail.extend_from_slice(&[b'x', 0, 0xff]);
    assert_eq!(decode_utf16(&with_tail), source);
    assert_eq!(decode_utf16(&[b'a', 0, 0xff]), [u16::from(b'a')]);
    assert_eq!(encode_utf16(&[]), [0, 0]);
}

#[test]
fn module_exposes_only_the_supported_windows_clipboard_operations() {
    let mut realm = Realm::new();
    let clipboard = module(&realm);
    for name in ["readText", "writeText", "writeImage", "readBuffer", "has"] {
        assert!(matches!(
            property(&mut realm, &clipboard, name),
            Value::Object(function) if matches!(function.data().kind, crate::Kind::Function { .. })
        ));
    }
}

#[test]
fn write_image_converts_an_images_pixels_to_dibv5_clipboard_data() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeClipboard::default());
    let clipboard = module_with(&realm, fake.clone());
    let png = crate::electron_native_image::encode_png(1, 1, &[10, 20, 30, 255]);
    let image = crate::electron_native_image::clipboard_image(&mut realm, Some(png), None);
    let write = property(&mut realm, &clipboard, "writeImage");
    ops::call(
        &mut realm,
        &write,
        Value::Object(clipboard),
        &[Value::Object(image)],
    )
    .unwrap();
    let data = fake.contents.borrow();
    let dib = &data[&CF_DIBV5];
    assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 124);
    assert_eq!(&dib[124..], &[30, 20, 10, 255]);
}

#[test]
fn text_operations_round_trip_unicode_without_accessing_the_system_clipboard() {
    let mut realm = Realm::new();
    let clipboard = module_with(&realm, Rc::new(FakeClipboard::default()));
    let write = property(&mut realm, &clipboard, "writeText");
    let read = property(&mut realm, &clipboard, "readText");
    let has = property(&mut realm, &clipboard, "has");
    let text = "clipboard – 日本語 😀";
    ops::call(
        &mut realm,
        &write,
        Value::Object(clipboard.clone()),
        &[Value::str(text)],
    )
    .unwrap();
    let Value::String(read) =
        ops::call(&mut realm, &read, Value::Object(clipboard.clone()), &[]).unwrap()
    else {
        panic!("readText must return a string")
    };
    assert_eq!(read.to_rust_lossy(), text);
    assert!(matches!(
        ops::call(
            &mut realm,
            &has,
            Value::Object(clipboard),
            &[Value::str("text/plain")]
        ),
        Ok(Value::Boolean(true))
    ));
}

#[test]
fn named_buffers_preserve_file_name_wide_bytes_and_custom_format_presence() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeClipboard::default());
    let clipboard = module_with(&realm, fake.clone());
    let format = fake.register_format("FileNameW").unwrap();
    fake.contents.borrow_mut().insert(
        format,
        "C:\\资料\\笔记.md\0D:\\second.md\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    );
    let read = property(&mut realm, &clipboard, "readBuffer");
    let result = ops::call(
        &mut realm,
        &read,
        Value::Object(clipboard.clone()),
        &[Value::str("FileNameW")],
    )
    .unwrap();
    let bytes = crate::typed_array_builtin::view_bytes(&result).unwrap();
    assert_eq!(bytes, fake.contents.borrow()[&format]);
    let has = property(&mut realm, &clipboard, "has");
    assert!(matches!(
        ops::call(
            &mut realm,
            &has,
            Value::Object(clipboard),
            &[Value::str("FileNameW")]
        ),
        Ok(Value::Boolean(true))
    ));
}

#[test]
fn text_write_surfaces_native_clipboard_errors_as_javascript_errors() {
    struct FailingClipboard;
    impl ClipboardBackend for FailingClipboard {
        fn read(&self, _: u32) -> Result<Option<Vec<u8>>, u32> {
            Ok(None)
        }
        fn replace(&self, _: u32, _: &[u8]) -> Result<(), u32> {
            Err(5)
        }
        fn register_format(&self, _: &str) -> Result<u32, u32> {
            Err(5)
        }
        fn available(&self, _: u32) -> bool {
            false
        }
    }
    let mut realm = Realm::new();
    let clipboard = module_with(&realm, Rc::new(FailingClipboard));
    let write = property(&mut realm, &clipboard, "writeText");
    let error = ops::call(
        &mut realm,
        &write,
        Value::Object(clipboard),
        &[Value::str("text")],
    )
    .unwrap_err();
    let Value::Object(error) = error.0 else {
        panic!("native clipboard failures must be Error objects")
    };
    let Value::String(message) =
        ops::get(&mut realm, &Value::Object(error), &Key::name("message")).unwrap()
    else {
        panic!("native clipboard error must include its Windows status")
    };
    assert!(message.to_rust_lossy().contains("Windows error 5"));
}

#[test]
fn clipboard_read_buffer_returns_a_native_uint8_view() {
    let mut realm = Realm::new();
    let result = buffer_value(&mut realm, vec![0, 0x80, 0xff]).unwrap();
    let Value::Object(view) = result else {
        panic!("Buffer result must be an object")
    };
    assert_eq!(
        crate::typed_array_builtin::view_bytes(&Value::Object(view)),
        Some(vec![0, 0x80, 0xff])
    );
}

#[test]
fn invalid_named_formats_reject_empty_or_nul_names_before_registration() {
    for name in ["", "FileNameW\0other"] {
        let result = register_format(name);
        assert!(result.is_err());
    }
}

#[test]
fn read_image_takes_the_png_form_before_the_bitmap() {
    let mut realm = Realm::new();
    let fake = Rc::new(FakeClipboard::default());
    let clipboard = module_with(&realm, fake.clone());
    let read = property(&mut realm, &clipboard, "readImage");
    let pixels = |realm: &mut Realm| {
        let image = ops::call(realm, &read, Value::Undefined, &[]).unwrap();
        let to_bitmap = ops::get(realm, &image, &Key::name("toBitmap")).unwrap();
        let bitmap = ops::call(realm, &to_bitmap, image, &[]).unwrap();
        crate::typed_array_builtin::view_bytes(&bitmap).unwrap()
    };
    assert!(pixels(&mut realm).is_empty());
    let mut dib = alloc::vec![0u8; 40];
    dib[0] = 40;
    dib[4] = 1;
    dib[8] = 1;
    dib[14] = 32;
    dib.extend([0, 0, 255, 0]);
    fake.contents.borrow_mut().insert(CF_DIB, dib);
    // Premultiplied BGRA, as Electron's `toBitmap` gives pixels.
    assert_eq!(pixels(&mut realm), [0, 0, 255, 255]);
    let png = fake.register_format("PNG").unwrap();
    fake.contents.borrow_mut().insert(
        png,
        crate::electron_native_image::encode_png(1, 1, &[0, 255, 0, 255]),
    );
    assert_eq!(pixels(&mut realm), [0, 255, 0, 255]);
}
