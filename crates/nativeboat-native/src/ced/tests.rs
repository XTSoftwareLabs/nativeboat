use super::*;
use crate::{Key, ops};
use encoding_rs::{BIG5, EUC_KR, GBK, SHIFT_JIS, UTF_8, WINDOWS_1251};

#[test]
fn detector_matches_marktext_multilingual_legacy_encoding_fixtures() {
    let cases = [
        (
            GBK,
            "这是一个用来验证字符编码检测的中文文本。这里包含多种常用词语和清晰的句子。",
            "GB",
        ),
        (
            SHIFT_JIS,
            "これは文字コードの検出を確認するための日本語の文章です。文字列を何度も繰り返して確実に検出します。",
            "SJS",
        ),
        (
            EUC_KR,
            "이 문장은 문자 인코딩 감지를 확인하기 위한 한국어 문장입니다. 다양한 글자와 문장을 반복합니다.",
            "KSC",
        ),
        (
            WINDOWS_1251,
            "Это русский текст для проверки определения кодировки. Мы повторяем разные слова и предложения много раз.",
            "CP1251",
        ),
        (
            BIG5,
            "這是一段用來確認文字編碼偵測的中文內容。我們重複文字和句子來檢查結果是否保持一致。",
            "BIG5",
        ),
    ];
    for (encoding, sample, expected) in cases {
        let text = sample.repeat(100);
        let (bytes, _, errors) = encoding.encode(&text);
        assert!(!errors);
        assert_eq!(detect(&bytes), expected);
    }
}

#[test]
fn detector_preserves_empty_ascii_utf8_and_bom_labels() {
    assert_eq!(detect(b""), "ASCII");
    assert_eq!(detect(b"plain text"), "ASCII-7-bit");
    assert_eq!(detect("Greek µ text".as_bytes()), "UTF8");
    assert_eq!(detect(&[0xef, 0xbb, 0xbf, b'A']), "UTF8");
    assert_eq!(detect(&[0xff, 0xfe, b'A', 0]), "UTF-16LE");
    assert_eq!(detect(&[0xfe, 0xff, 0, b'A']), "UTF-16BE");
    assert_eq!(detect(&[b'A', 0, b'B', 0]), "UTF-16LE");
    assert_eq!(detect(&[0, b'A', 0, b'B']), "UTF-16BE");
    let (utf8, _, errors) = UTF_8.encode("A µ κ α");
    assert!(!errors);
    assert_eq!(detect(&utf8), "UTF8");
}

#[test]
fn compact_detector_names_keep_the_legacy_ced_label_map() {
    for (name, expected) in [
        ("UTF-8", "UTF8"),
        ("Big5", "BIG5"),
        ("EUC-KR", "KSC"),
        ("GBK", "GB"),
        ("Shift_JIS", "SJS"),
        ("EUC-JP", "EUC-JP"),
        ("ISO-2022-JP", "ISO-2022-JP"),
        ("windows-1250", "CP1250"),
        ("windows-1251", "CP1251"),
        ("windows-1252", "CP1252"),
        ("windows-1253", "CP1253"),
        ("windows-1254", "CP1254"),
        ("windows-1255", "CP1255"),
        ("windows-1256", "CP1256"),
        ("windows-1257", "CP1257"),
        ("windows-874", "CP874"),
        ("IBM866", "CP866"),
        ("KOI8-R", "KOI8R"),
        ("KOI8-U", "KOI8U"),
        ("ISO-8859-2", "Latin2"),
        ("ISO-8859-4", "Latin4"),
        ("ISO-8859-5", "ISO-8859-5"),
        ("ISO-8859-6", "Arabic"),
        ("ISO-8859-7", "Greek"),
        ("ISO-8859-8", "Hebrew"),
        ("ISO-8859-13", "ISO-8859-13"),
        ("ISO-8859-15", "ISO-8859-15"),
        ("x-unknown", "Unknown"),
    ] {
        assert_eq!(ced_label(name), expected);
    }
}

#[test]
fn callable_package_accepts_only_buffer_instances_and_detects_the_view_bytes() {
    let mut realm = Realm::new();
    let buffer_prototype = realm.object();
    realm.buffer_prototype = Some(buffer_prototype.clone());
    let module = module(&mut realm);
    let buffer =
        typed_array_builtin::new_bytes(&realm, b"detector input".to_vec(), Some(buffer_prototype));
    let result = ops::call(
        &mut realm,
        &Value::Object(module.clone()),
        Value::Undefined,
        &[Value::Object(buffer)],
    )
    .unwrap();
    assert!(matches!(
        result,
        Value::String(text) if text.to_rust_lossy() == "ASCII-7-bit"
    ));

    let plain = realm.object();
    let error = ops::call(
        &mut realm,
        &Value::Object(module),
        Value::Undefined,
        &[Value::Object(plain)],
    )
    .unwrap_err();
    let name = ops::get(&mut realm, &error.0, &Key::name("name")).unwrap();
    assert!(ops::strict_equals(&name, &Value::str("TypeError")));
}

#[test]
fn callable_package_keeps_the_function_arity_and_name() {
    let mut realm = Realm::new();
    let module = module(&mut realm);
    let function = Value::Object(module);
    assert!(ops::strict_equals(
        &ops::get(&mut realm, &function, &Key::name("name")).unwrap(),
        &Value::str("ced")
    ));
    assert!(matches!(
        ops::get(&mut realm, &function, &Key::name("length")).unwrap(),
        Value::Number(1.0)
    ));
}
