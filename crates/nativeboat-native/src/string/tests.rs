use super::*;

#[test]
fn strings_keep_utf16_units_and_share_storage() {
    let face = JsStr::from("a😀");
    assert_eq!(face.len(), 3);
    assert_eq!(face.units(), &[0x61, 0xD83D, 0xDE00]);
    let lone = JsStr::from_units(&[0xD83D]);
    assert_eq!(lone.len(), 1);
    assert_eq!(lone.to_rust_lossy(), "\u{FFFD}");
    assert_eq!(format!("{lone}"), "\u{FFFD}");
    assert_eq!(format!("{face:?}"), "\"a😀\"");
    let joined = lone.concat(&JsStr::from_units(&[0xDE00]));
    assert_eq!(joined.to_rust_lossy(), "😀");
    let empty = JsStr::default();
    assert!(empty.is_empty());
    assert!(face.concat(&empty).ptr_eq(&face));
    assert!(empty.concat(&face).ptr_eq(&face));
    assert!(!face.ptr_eq(&JsStr::from("a😀")));
    assert_eq!(face, JsStr::from("a😀"));
}

#[test]
fn white_space_matches_the_language_grammar() {
    for unit in [
        0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x20, 0xA0, 0x1680, 0x2000, 0x200A, 0x2028, 0x2029, 0x202F,
        0x205F, 0x3000, 0xFEFF,
    ] {
        assert!(is_space(unit), "{unit:#x}");
    }
    for unit in [0x08, 0x41, 0x180E, 0x200B] {
        assert!(!is_space(unit), "{unit:#x}");
    }
}

#[test]
fn substrings_share_long_buffers_and_compare_by_content() {
    use core::hash::BuildHasher;
    let text = JsStr::from("the quick brown fox jumps over the lazy dog");
    // A long slice views the same buffer; a short one is its own copy.
    let long = text.slice(4, 30);
    assert_eq!(long.to_rust_lossy(), "quick brown fox jumps over");
    assert_eq!(long.buffer, text.buffer);
    let short = text.slice(4, 9);
    assert_eq!(short.to_rust_lossy(), "quick");
    assert_ne!(short.buffer, text.buffer);
    // Slicing all of a string is the string; slices of slices stay views.
    assert!(text.slice(0, text.len()).ptr_eq(&text));
    let nested = long.slice(6, 26);
    assert_eq!(nested.to_rust_lossy(), "brown fox jumps over");
    assert_eq!(nested.buffer, text.buffer);
    // Views compare, hash and order by their units.
    let copy = JsStr::from("brown fox jumps over");
    assert_eq!(nested, copy);
    assert!(!nested.ptr_eq(&copy));
    let state = std::collections::hash_map::RandomState::new();
    let hash_with = |value: &JsStr| state.hash_one(value);
    assert_eq!(hash_with(&nested), hash_with(&copy));
    assert!(JsStr::from("a") < JsStr::from("b"));
    assert_eq!(
        nested.concat(&JsStr::from("!")).to_rust_lossy(),
        "brown fox jumps over!"
    );
    assert!(nested.concat(&JsStr::default()).ptr_eq(&nested));
    assert!(JsStr::default().concat(&nested).ptr_eq(&nested));
    // The empty string needs no buffer.
    let empty = text.slice(3, 3);
    assert!(empty.is_empty() && empty.units().is_empty());
    assert_eq!(JsStr::from_units(&[]).buffer, None);
    // Non-ASCII text keeps its code units.
    assert_eq!(JsStr::from("é😀").units(), [0xE9, 0xD83D, 0xDE00]);
    drop(text);
    // Views outlive the string they were cut from.
    assert_eq!(long.to_rust_lossy(), "quick brown fox jumps over");
}

#[test]
fn static_strings_are_shared_without_counting() {
    static ASCII: StaticStr<3> = StaticStr::new("pos");
    static WIDE: StaticStr<5> = StaticStr::new("é中😀x");
    static LONE: StaticStr<2> = StaticStr::from_units([0xd800, 0x61]);
    static LONG: StaticStr<20> = StaticStr::new("abcdefghijklmnopqrst");
    assert_eq!(ASCII.header.str(), JsStr::from("pos"));
    assert_eq!(WIDE.header.str(), JsStr::from("é中😀x"));
    assert_eq!(LONE.header.str().units(), [0xd800, 0x61]);
    assert_eq!(ASCII.header.key(), crate::Key::name("pos"));
    assert!(matches!(ASCII.header.value(), crate::Value::String(text) if text.len() == 3));
    // Handles neither count nor free the static header.
    let handle = LONG.header.str();
    let copies: Vec<JsStr> = (0..3).map(|_| handle.clone()).collect();
    drop(copies);
    assert_eq!(LONG.header.0.count.get(), 0);
    let view = handle.slice(2, 19);
    assert_eq!(view.to_rust_lossy(), "cdefghijklmnopqrs");
    assert_eq!(view.buffer, handle.buffer);
    drop(view);
    assert_eq!(LONG.header.0.count.get(), 0);
    assert_eq!(handle.concat(&ASCII.header.str()).len(), 23);
}

#[test]
fn appending_grows_strings_in_place_without_changing_other_views() {
    let text = |value: &JsStr| String::from_utf16_lossy(value.units());
    // A loop of `pending += character` builds one buffer.
    let mut pending = JsStr::from("a");
    let mut kept = Vec::new();
    for character in "bcdefghijklmnopqrstuvwxyz".chars() {
        kept.push(pending.clone());
        pending = pending.concat(&JsStr::from(character.to_string().as_str()));
    }
    assert_eq!(text(&pending), "abcdefghijklmnopqrstuvwxyz");
    assert_eq!(text(&kept[0]), "a");
    assert_eq!(text(&kept[3]), "abcd");
    assert!(kept[24].buffer == pending.buffer);
    // An older view appends to a copy: the newer one keeps its units.
    let branch = kept[3].concat(&JsStr::from("!"));
    assert_eq!(text(&branch), "abcd!");
    assert_eq!(text(&pending), "abcdefghijklmnopqrstuvwxyz");
    assert!(branch.buffer != pending.buffer);
    // The whole buffer's hash follows its units.
    let before = JsStr::from_units(pending.units());
    assert_eq!(pending.fingerprint(), before.fingerprint());
    let longer = pending.concat(&JsStr::from("!"));
    assert_eq!(
        longer.fingerprint(),
        JsStr::from_units(longer.units()).fingerprint()
    );
    assert_eq!(pending.fingerprint(), before.fingerprint());
    // Static strings are never written.
    static STATIC: StaticStr<3> = StaticStr::new("abc");
    let grown = STATIC.header.str().concat(&JsStr::from("d"));
    assert_eq!(text(&grown), "abcd");
    assert_eq!(text(&STATIC.header.str()), "abc");
    // Long additions leave no room.
    let long = JsStr::from("x").concat(&JsStr::from_units(&[0x61; 100]));
    assert_eq!(long.len(), 101);
}
