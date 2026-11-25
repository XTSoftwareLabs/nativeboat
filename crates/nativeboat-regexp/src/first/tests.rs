use super::*;
use crate::{Flags, Pattern};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn first(pattern: &str, flags: Flags) -> Option<Units> {
    first_units(&units(pattern), flags)
}

fn set(ranges: &[(char, char)]) -> Option<Units> {
    Some(Units::of(
        &ranges
            .iter()
            .map(|&(low, high)| (low as u16, high as u16))
            .collect::<Vec<_>>(),
    ))
}

#[test]
fn anchored_patterns_begin_with_their_first_atoms_units() {
    let plain = Flags::default();
    // Tokenizer rules: literals, groups, alternatives and lookarounds.
    assert_eq!(
        first(r"^(\*\*|__)(?=\S)", plain),
        set(&[('*', '*'), ('_', '_')])
    );
    assert_eq!(first(r"^(!\[)(.*?)\]", plain), set(&[('!', '!')]));
    assert_eq!(first(r"^(\n)(?!\n)", plain), set(&[('\n', '\n')]));
    assert_eq!(first(r"^( {2,})(\n)", plain), set(&[(' ', ' ')]));
    assert_eq!(
        first(r"^(?!\$(?!\$)x)(\$\$(?!\$)|\$(?=\S))(y)", plain),
        set(&[('$', '$')])
    );
    assert_eq!(
        first(r"^(`{3,})([^`]*)$|^(~{3,})(.*)$", plain),
        set(&[('`', '`'), ('~', '~')])
    );
    assert_eq!(
        first(r"^(?:\[[^\]]*\])?x", plain),
        set(&[('[', '['), ('x', 'x')])
    );
    assert_eq!(
        first(r"^[a-c\d_]+", plain),
        set(&[('0', '9'), ('_', '_'), ('a', 'c')])
    );
    assert_eq!(first(r"^\s", plain), Some(Units::of(SPACE)));
    assert_eq!(first(r"^\x41B\cJ\0", plain), set(&[('A', 'A')]));
    assert_eq!(first(r"^(?<name>a)\k<name>", plain), set(&[('a', 'a')]));
    assert_eq!(first(r"^\bx", plain), set(&[('x', 'x')]));
    assert_eq!(first(r"^a{,}", plain), set(&[('a', 'a')]));
    assert_eq!(first(r"^中", plain), set(&[('中', '中')]));
    assert_eq!(first(r"^[\b]", plain), set(&[('\u{8}', '\u{8}')]));
    assert_eq!(
        first(
            r"^\u{41}",
            Flags {
                unicode: true,
                ..plain
            }
        ),
        set(&[('A', 'A')])
    );
    // Digits and punctuation stay known when case does not matter.
    let insensitive = Flags {
        ignore_case: true,
        ..plain
    };
    assert_eq!(first(r"^\d", insensitive), set(&[('0', '9')]));
}

#[test]
fn unanchored_optional_or_unknown_starts_give_no_units() {
    let plain = Flags::default();
    for pattern in [
        "a",
        "^a|b",
        "^a*",
        "^(a)?",
        "^.",
        r"^\S",
        r"^[^a]",
        r"^\1a",
        "^(?=a)",
        r"^\p{L}",
        r"^[\w-z]",
        "^(?x)",
        "^*",
        "^(a",
        "^[a",
        r"^\x4",
        r"^\c1",
        "^a{2,3}?|^b{1}",
    ] {
        let found = first(pattern, plain);
        if pattern == "^a{2,3}?|^b{1}" {
            assert_eq!(found, set(&[('a', 'b')]), "{pattern}");
        } else {
            assert_eq!(found, None, "{pattern}");
        }
    }
    // Multiline anchors, case-insensitive letters, set syntax and pairs.
    assert_eq!(
        first(
            "^a",
            Flags {
                multiline: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            "^a",
            Flags {
                ignore_case: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            "^a",
            Flags {
                unicode_sets: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            r"^\u{1F600}",
            Flags {
                unicode: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            r"^\uD83D",
            Flags {
                unicode: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            "^{",
            Flags {
                unicode: true,
                ..plain
            }
        ),
        None
    );
    assert_eq!(
        first(
            r"^\u{41",
            Flags {
                unicode: true,
                ..plain
            }
        ),
        None
    );
}

#[test]
fn filtered_searches_find_what_the_matcher_finds() {
    let patterns = [
        r"^(\*\*|__)(?=\S)([\s\S]*?\S)\1",
        r"^(`{1,3})([^`]+|.{2,})\1",
        r"^(\\)([\\`*{}[\]()#+\-.!_>~:|<$])",
        r"^(:)([a-z_\d+-]+)\1",
        r"^( {2,})(\n)(?!\n)",
        r"^(\[\^)([^^[\]\s]+)(?<!\\)\]",
        r"^(~{2})(?=\S)([\s\S]*?\S)(\\*)\1",
        r"^[{}]x",
        r"^a{2}|^\d+",
    ];
    let inputs = [
        "",
        "**bold** rest",
        "__x__",
        "* no",
        "`code`",
        "``a``",
        r"\*",
        ":smile: x",
        "  \nnext",
        "[^1] note",
        "~~del~~",
        "{x",
        "}x",
        "aa",
        "a",
        "123",
        "x**y**",
        " **",
        "~~ x~~",
    ];
    for flags in [
        Flags::default(),
        Flags {
            unicode: true,
            ..Flags::default()
        },
    ] {
        for pattern in patterns {
            let filtered = Pattern::compile(&units(pattern), flags).unwrap();
            let plain = Pattern {
                first: None,
                ..filtered.clone()
            };
            assert!(filtered.first.is_some(), "{pattern}");
            for input in inputs {
                let text = units(input);
                for start in 0..=text.len() {
                    let (a, b) = (filtered.search(&text, start), plain.search(&text, start));
                    assert_eq!(
                        a.map(|found| (found.range, found.captures)),
                        b.map(|found| (found.range, found.captures)),
                        "{pattern} on {input:?} from {start}"
                    );
                }
            }
        }
    }
}
