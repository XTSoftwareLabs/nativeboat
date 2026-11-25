//! Expected bytes are Node 22's `Buffer.from(text, encoding)` results.

use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn encoding_names_ignore_case() {
    for (name, expected) in [
        ("UTF8", Some(Encoding::Utf8)),
        ("Utf-8", Some(Encoding::Utf8)),
        ("ucs2", Some(Encoding::Utf16Le)),
        ("UCS-2", Some(Encoding::Utf16Le)),
        ("utf16le", Some(Encoding::Utf16Le)),
        ("UTF-16LE", Some(Encoding::Utf16Le)),
        ("binary", Some(Encoding::Latin1)),
        ("Latin1", Some(Encoding::Latin1)),
        ("ascii", Some(Encoding::Ascii)),
        ("base64", Some(Encoding::Base64)),
        ("base64URL", Some(Encoding::Base64Url)),
        ("HEX", Some(Encoding::Hex)),
        ("buffer", None),
        ("utf-16", None),
        ("latin-1", None),
        ("", None),
    ] {
        assert_eq!(named(&units(name)), expected, "{name}");
    }
}

#[test]
fn strings_encode_like_node_buffers() {
    let unpaired = [0xe9, 0x20ac, 0xd83d];
    for (text, encoding, expected) in [
        (units("aGVsbG8="), Encoding::Base64, "68656c6c6f"),
        (units("aGVs bG8"), Encoding::Base64, "68656c6c6f"),
        (units("a-_b"), Encoding::Base64, "6befdb"),
        (units("a+/b"), Encoding::Base64Url, "6befdb"),
        (units("aG!VsbG8"), Encoding::Base64, "68656c6c6f"),
        (units("aGVsbG8=aGk="), Encoding::Base64, "68656c6c6f"),
        (units("a"), Encoding::Base64, ""),
        (units("abc"), Encoding::Base64, "69b7"),
        (units("=abc"), Encoding::Base64, ""),
        (units("a\u{e9}b"), Encoding::Base64, "69"),
        (units("ab\ncd"), Encoding::Base64, "69b71d"),
        (units("zz12"), Encoding::Hex, ""),
        (units("12zz34"), Encoding::Hex, "12"),
        (units("123"), Encoding::Hex, "12"),
        (units("1g"), Encoding::Hex, ""),
        (units("ABcd"), Encoding::Hex, "abcd"),
        (units("\u{e9}1"), Encoding::Hex, ""),
        (vec![0xd800, u16::from(b'1')], Encoding::Hex, ""),
        (unpaired.to_vec(), Encoding::Latin1, "e9ac3d"),
        (unpaired[..2].to_vec(), Encoding::Ascii, "e9ac"),
        (unpaired.to_vec(), Encoding::Utf16Le, "e900ac203dd8"),
        (
            vec![0xd83d, 0xde00, 0xd83d, u16::from(b'!')],
            Encoding::Utf8,
            "f09f9880efbfbd21",
        ),
    ] {
        assert_eq!(
            hex(&encode(&text, encoding)),
            expected,
            "{text:?} {encoding:?}"
        );
    }
}

#[test]
fn numbers_name_node_encodings() {
    let expected = [
        Some(Encoding::Ascii),
        Some(Encoding::Utf8),
        Some(Encoding::Base64),
        Some(Encoding::Utf16Le),
        Some(Encoding::Latin1),
        Some(Encoding::Hex),
        None,
        Some(Encoding::Base64Url),
        None,
    ];
    for (number, encoding) in expected.into_iter().enumerate() {
        assert_eq!(numbered(number as i32), encoding, "{number}");
    }
}

#[test]
fn writes_fit_their_capacity_like_node() {
    let text = units("\u{e9}\u{20ac}");
    assert_eq!(hex(&write(&text, Encoding::Utf8, 5)), "c3a9e282ac");
    assert_eq!(hex(&write(&text, Encoding::Utf8, 4)), "c3a9");
    assert_eq!(hex(&write(&text, Encoding::Utf8, 1)), "");
    assert_eq!(hex(&write(&units("abc"), Encoding::Utf16Le, 3)), "6100");
    assert_eq!(hex(&write(&units("abcd"), Encoding::Hex, 1)), "ab");
    assert_eq!(hex(&write(&units("abc"), Encoding::Latin1, 9)), "616263");
    assert_eq!(
        utf8_length(&[0xd800, 0xe9, u16::from(b'a'), 0xd83d, 0xde00]),
        10
    );
}

#[test]
fn bytes_decode_like_buffer_to_string() {
    let bytes = [0xe9, 0x41, 0xff, 0x80, 0x7f, 0xc3, 0xa9, 0xe2, 0x82];
    let text = |encoding| String::from_utf16_lossy(&decode(&bytes, encoding));
    assert_eq!(
        text(Encoding::Utf8),
        "\u{fffd}A\u{fffd}\u{fffd}\u{7f}\u{e9}\u{fffd}"
    );
    assert_eq!(text(Encoding::Ascii), "iA\u{7f}\u{0}\u{7f}C)b\u{2}");
    assert_eq!(decode(&bytes, Encoding::Latin1)[0], 0xe9);
    assert_eq!(text(Encoding::Hex), "e941ff807fc3a9e282");
    assert_eq!(text(Encoding::Base64), "6UH/gH/DqeKC");
    assert_eq!(text(Encoding::Base64Url), "6UH_gH_DqeKC");
    assert_eq!(decode(&bytes, Encoding::Utf16Le).len(), 4);
    assert_eq!(base64_encode(b"a", false), "YQ==");
    assert_eq!(base64_encode(b"ab", false), "YWI=");
    assert_eq!(base64_encode(b"a", true), "YQ");
}

#[test]
fn atob_decodes_forgiving_base64() {
    for (text, expected) in [
        ("YQ", Ok("61")),
        ("YQ==", Ok("61")),
        (" YQ = = ", Ok("61")),
        ("YWJj\n", Ok("616263")),
        ("", Ok("")),
        ("YQ=", Err(-2)),
        ("YQ===", Err(-2)),
        ("Y=Q=", Err(-2)),
        ("YW-j", Err(-2)),
        ("\u{100}A==", Err(-2)),
        ("Y", Err(-1)),
    ] {
        let result = forgiving_base64(&units(text)).map(|bytes| hex(&bytes));
        assert_eq!(result, expected.map(str::to_owned), "{text:?}");
    }
}
