use super::*;
use alloc::format;

fn compile(pattern: &str, flags: Flags) -> Pattern {
    Pattern::compile(&pattern.encode_utf16().collect::<Vec<_>>(), flags).unwrap()
}

#[test]
fn code_unit_and_code_point_modes_preserve_surrogates_and_start_positions() {
    let text = [0xD83D, 0xDE00, 0xD800, 0xDC00, 0xD800, 0x61, 0xDC00];
    for unicode in [false, true] {
        let pattern = compile(
            ".",
            Flags {
                unicode,
                ..Flags::default()
            },
        );
        assert_eq!(
            pattern.search(&text, 0).unwrap().range,
            if unicode { 0..2 } else { 0..1 }
        );
        assert_eq!(
            pattern.search(&text, 1).unwrap().range,
            if unicode { 0..2 } else { 1..2 }
        );
        assert_eq!(pattern.search(&text, 4).unwrap().range, 4..5);
        assert_eq!(pattern.search(&text, 5).unwrap().range, 5..6);
        assert_eq!(pattern.search(&text, 6).unwrap().range, 6..7);
        assert!(pattern.search(&text, 7).is_none());
        assert!(pattern.search(&text, 8).is_none());
        for source in [&text[0..2], &text[4..5], &text[5..7]] {
            let literal = Pattern::compile(
                source,
                Flags {
                    unicode,
                    ..Flags::default()
                },
            )
            .unwrap();
            assert_eq!(literal.search(source, 0).unwrap().range, 0..source.len());
        }
    }
    let literal = Pattern::compile(
        &[0xD800, 0x61],
        Flags {
            unicode: true,
            ..Flags::default()
        },
    )
    .unwrap();
    assert_eq!(literal.search(&[0xD800, 0x61], 0).unwrap().range, 0..2);
    let empty = compile("", Flags::default());
    assert_eq!(
        empty.search(&text, text.len()).unwrap().range,
        text.len()..text.len()
    );
    let sets = compile(
        "[\\p{ASCII}&&[a-z]]+",
        Flags {
            unicode_sets: true,
            ..Flags::default()
        },
    );
    assert_eq!(sets.search(&[0x61, 0x62, 0x31], 0).unwrap().range, 0..2);
    assert!(format!("{sets:?}").contains("Pattern"));
    assert!(format!("{:?}", sets.search(&[0x61], 0).unwrap()).contains("Match"));
}

#[test]
fn matching_flags_captures_backreferences_and_lookaround_are_native() {
    let pattern = compile(
        "(?<word>[a-z]+)\\s+\\k<word>(?<optional>x)?",
        Flags::default(),
    );
    let text = "word word".encode_utf16().collect::<Vec<_>>();
    let matched = pattern.search(&text, 0).unwrap();
    assert_eq!(matched.range, 0..9);
    assert_eq!(matched.captures, [Some(0..4), None]);
    assert_eq!(
        matched.groups,
        [
            (String::from("word"), Some(0..4)),
            (String::from("optional"), None)
        ]
    );
    assert_eq!(
        matched.group_indices,
        [(String::from("word"), 1), (String::from("optional"), 2)]
    );
    let plain = compile("(a)|(b)", Flags::default());
    let matched = plain.search(&[0x62], 0).unwrap();
    assert_eq!(matched.captures, [None, Some(0..1)]);
    assert!(matched.groups.is_empty());
    assert!(matched.group_indices.is_empty());
    for (source, text, flags, expected) in [
        ("(?<=a)b", "ab", Flags::default(), 1..2),
        ("a(?!b)", "acab", Flags::default(), 0..1),
        (
            "^a.b$",
            "A\nB",
            Flags {
                ignore_case: true,
                dot_all: true,
                ..Flags::default()
            },
            0..3,
        ),
        (
            "^b$",
            "a\nb\nc",
            Flags {
                multiline: true,
                ..Flags::default()
            },
            2..3,
        ),
    ] {
        let pattern = compile(source, flags);
        assert_eq!(
            pattern
                .search(&text.encode_utf16().collect::<Vec<_>>(), 0)
                .unwrap()
                .range,
            expected
        );
    }
    assert!(Pattern::compile(&[0x28], Flags::default()).is_err());
    let cloned = pattern.clone();
    assert_eq!(cloned.search(&text, 0).unwrap().range, 0..9);
}

#[test]
fn node22_syntax_and_named_indices_use_unicode16_metadata() {
    for source in ["(?<x>a)|(?<x>b)", "(?<\\u0078>a)|(?<x>b)"] {
        assert_eq!(
            Pattern::compile(&source.encode_utf16().collect::<Vec<_>>(), Flags::default())
                .unwrap_err(),
            "Duplicate capture group name"
        );
    }
    let pattern = compile("((?<name>a))(b)", Flags::default());
    let matched = pattern.search(&[0x61, 0x62], 0).unwrap();
    assert_eq!(matched.captures, [Some(0..1), Some(0..1), Some(1..2)]);
    assert_eq!(matched.group_indices, [(String::from("name"), 2)]);
    assert_eq!(matched.groups, [(String::from("name"), Some(0..1))]);
    let flags = Flags {
        unicode: true,
        ..Flags::default()
    };
    // Unicode 16's Kirat Rai is supported; Unicode 17's Beria Erfe is not.
    let kirat = compile("\\p{Script=Kirat_Rai}", flags);
    let text = "\u{16D40}".encode_utf16().collect::<Vec<_>>();
    assert_eq!(kirat.search(&text, 0).unwrap().range, 0..2);
    assert!(
        Pattern::compile(
            &"\\p{Script=Beria_Erfe}".encode_utf16().collect::<Vec<_>>(),
            flags
        )
        .is_err()
    );
    let assigned = compile("\\p{Assigned}", flags);
    assert!(
        assigned
            .search(&"\u{16EA0}".encode_utf16().collect::<Vec<_>>(), 0)
            .is_none()
    );
}

#[test]
fn start_anchored_patterns_match_only_at_the_start_of_the_input() {
    let units = |text: &str| text.encode_utf16().collect::<Vec<_>>();
    let anchored = compile(r"^\*\*([^*]+)\*\*|^`x`", Flags::default());
    assert_eq!(
        anchored
            .search(&units("**bold** and **more**"), 0)
            .unwrap()
            .range,
        0..8
    );
    assert!(anchored.search(&units("plain **bold**"), 0).is_none());
    // A start past the input's start cannot match `^`.
    assert!(anchored.search(&units("**a****b**"), 5).is_none());
    // In multiline mode `^` matches after line breaks, so the search scans.
    let multiline = compile(
        "^b",
        Flags {
            multiline: true,
            ..Flags::default()
        },
    );
    assert_eq!(multiline.search(&units("a\nb"), 0).unwrap().range, 2..3);
}
