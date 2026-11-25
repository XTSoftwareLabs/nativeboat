use super::*;

fn pieces(previous: Option<char>, word: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for offset in opportunities(previous, word) {
        if offset > 0 {
            parts.push(&word[start..offset]);
            start = offset;
        }
    }
    parts.push(&word[start..]);
    parts
}

#[test]
fn classes_come_from_the_table() {
    assert_eq!(class('a'), Class::Al);
    assert_eq!(class('\u{0}'), Class::Cm);
    assert_eq!(class('('), Class::Op);
    assert_eq!(class('（'), Class::Opw);
    assert_eq!(class('中'), Class::Id);
    assert_eq!(class('，'), Class::Cl);
    assert_eq!(class('7'), Class::Nu);
    assert_eq!(class('\u{10FFFF}'), Class::Al);
    assert_eq!(class('\u{1F1E6}'), Class::Ri);
}

#[test]
fn words_break_where_browsers_break_them() {
    // Markdown links break before their target and after hyphens, but
    // ASCII follows Chromium's table, which keeps slashes with what follows.
    assert_eq!(
        pieces(None, "[link](https://example.com/a-b)"),
        ["[link]", "(https://example.com/a-", "b)"]
    );
    // A hyphen breaks before digits only where it is no minus sign.
    assert_eq!(pieces(None, "ABCD-1234"), ["ABCD-", "1234"]);
    assert_eq!(pieces(None, "(-1)"), ["(-1)"]);
    assert_eq!(pieces(Some('-'), "1"), ["1"]);
    assert_eq!(pieces(None, "a?b"), ["a?", "b"]);
    assert_eq!(pieces(None, "self-evident"), ["self-", "evident"]);
    assert_eq!(pieces(None, "1,000.50"), ["1,000.50"]);
    assert_eq!(pieces(None, "$(100)"), ["$(100)"]);
    // Ideographs break between each other, but not before closing marks.
    assert_eq!(pieces(None, "中文，测试。"), ["中", "文，", "测", "试。"]);
    assert_eq!(pieces(None, "（注）"), ["（注）"]);
    assert_eq!(pieces(None, "한국어"), ["한", "국", "어"]);
    // Combining marks and joiners stay with their base; flags pair up.
    assert_eq!(pieces(None, "e\u{301}中"), ["e\u{301}", "中"]);
    assert_eq!(pieces(None, "\u{301}x"), ["\u{301}x"]);
    assert_eq!(
        pieces(None, "👩\u{200D}💻👍\u{1F3FD}"),
        ["👩\u{200D}💻", "👍\u{1F3FD}"]
    );
    assert_eq!(
        pieces(None, "\u{1F1E8}\u{1F1F3}\u{1F1FA}\u{1F1F8}"),
        ["\u{1F1E8}\u{1F1F3}", "\u{1F1FA}\u{1F1F8}"]
    );
    // No breaks around glue or word joiners, or inside Korean syllables.
    assert_eq!(pieces(None, "a\u{A0}中\u{2060}文"), ["a\u{A0}中\u{2060}文"]);
    assert_eq!(
        pieces(None, "\u{1100}\u{1161}\u{11A8}"),
        ["\u{1100}\u{1161}\u{11A8}"]
    );
}

#[test]
fn the_text_before_decides_the_first_break() {
    assert_eq!(opportunities(Some(']'), "(x"), [0]);
    assert_eq!(opportunities(Some('a'), "b"), [0usize; 0]);
    assert_eq!(opportunities(Some('中'), "文"), [0]);
    assert_eq!(opportunities(Some('\u{301}'), "b"), [0usize; 0]);
    assert_eq!(opportunities(Some('\u{200D}'), "中"), [0usize; 0]);
    assert_eq!(opportunities(Some('\u{1F1E8}'), "\u{1F1F3}"), [0usize; 0]);
    assert_eq!(opportunities(Some('\u{200B}'), "a"), [0]);
    assert_eq!(opportunities(Some(' '), "\u{301}"), [0]);
}

#[test]
fn line_ends_and_other_classes_follow_the_rules() {
    use Class::*;
    let cases = [
        (Cr, Lf, false),
        (Lf, Al, true),
        (Al, Bk, false),
        (Sp, Gl, true),
        (Al, Gl, false),
        (B2, B2, false),
        (Sp, Al, true),
        (Qu, Id, false),
        (Cb, Al, true),
        (Bb, Al, false),
        (Sy, Hl, false),
        (Pr, Id, false),
        (Id, Po, false),
        (Po, Al, false),
        (Cp, Po, false),
        (Jl, Jv, false),
        (Jv, Jt, false),
        (Jt, Jt, false),
        (H2, Po, false),
        (Pr, H3, false),
        (Is, Hl, false),
        (Cp, Nu, false),
        (Cpw, Al, true),
        (Eb, Em, false),
        (Id, Al, true),
    ];
    for (before, after, expected) in cases {
        assert_eq!(breaks(before, after, 1), expected, "{before:?} {after:?}");
    }
}
