use super::*;
use crate::text::{FontMetrics, pieces};

struct Mono;

impl TextMeasurer for Mono {
    fn width(&self, text: &str, _: &Font) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    fn metrics(&self, _: &Font) -> FontMetrics {
        FontMetrics::default()
    }
}

fn segment(text: &str, white_space: WhiteSpace, line_height: f32) -> Segment {
    let mut after_space = false;
    Segment {
        pieces: pieces(text, white_space, &mut after_space),
        font: Font::default(),
        line_height,
        normal_line: false,
        color: Color::BLACK,
        underline: false,
        underline_offset: None,
        line_through: false,
        letter_spacing: 0.0,
        white_space,
        word_breaking: WordBreaking::Normal,
        node: NodeId(5),
        element: NodeId(4),
        boxes: alloc::rc::Rc::from([]),
        edge: None,
        symbol: None,
        shift: 0.0,
        tab: (64.0, 8.0),
        level: 0,
        overridden: false,
    }
}

fn breaking(text: &str, word_breaking: WordBreaking) -> InlineContent {
    content(
        vec![Segment {
            word_breaking,
            ..segment(text, WhiteSpace::Normal, 10.0)
        }],
        TextAlign::Start,
    )
}

fn words(lines: &Lines) -> Vec<String> {
    lines
        .lines
        .iter()
        .map(|line| {
            line.fragments
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect()
        })
        .collect()
}

fn content(segments: Vec<Segment>, align: TextAlign) -> InlineContent {
    InlineContent {
        strut_font: Font::default(),
        atoms: Vec::new(),
        segments,
        text_align: align,
        direction: Direction::Ltr,
        indent: 0.0,
        strut: 10.0,
        ellipsis: false,
        clamp: None,
    }
}

fn texts(lines: &Lines) -> Vec<Vec<(String, f32, f32)>> {
    lines
        .lines
        .iter()
        .map(|line| {
            line.fragments
                .iter()
                .map(|fragment| (fragment.text.clone(), fragment.x, fragment.width))
                .collect()
        })
        .collect()
}

#[test]
fn text_wraps_at_the_available_width() {
    let inline = content(
        vec![segment("aa bb cc", WhiteSpace::Normal, 12.0)],
        TextAlign::Start,
    );
    let lines = layout_lines(&inline, Some(55.0), &Mono, &[]);
    assert_eq!(
        texts(&lines),
        vec![
            vec![("aa bb".to_owned(), 0.0, 50.0)],
            vec![("cc".to_owned(), 0.0, 20.0)]
        ]
    );
    assert_eq!(lines.width, 50.0);
    assert_eq!(lines.height, 24.0);
    assert_eq!(lines.lines[1].top, 12.0);
    let unbounded = layout_lines(&inline, None, &Mono, &[]);
    assert_eq!(unbounded.lines.len(), 1);
    assert_eq!(unbounded.width, 80.0);
    let narrowest = layout_lines(&inline, Some(0.0), &Mono, &[]);
    assert_eq!(narrowest.lines.len(), 3);
    assert_eq!(narrowest.width, 20.0);
}

#[test]
fn segments_merge_and_split() {
    let inline = content(
        vec![
            segment(" one ", WhiteSpace::Normal, 10.0),
            segment("two  ", WhiteSpace::Normal, 20.0),
            segment("three", WhiteSpace::NoWrap, 10.0),
        ],
        TextAlign::Left,
    );
    let lines = layout_lines(&inline, Some(70.0), &Mono, &[]);
    assert_eq!(
        texts(&lines),
        vec![
            vec![
                ("one ".to_owned(), 0.0, 40.0),
                ("two".to_owned(), 40.0, 30.0)
            ],
            vec![("three".to_owned(), 0.0, 50.0)]
        ]
    );
    assert_eq!(lines.lines[0].height, 20.0);
}

#[test]
fn trailing_spaces_are_trimmed() {
    let inline = content(
        vec![
            segment("ab", WhiteSpace::Normal, 10.0),
            segment("   ", WhiteSpace::PreWrap, 10.0),
        ],
        TextAlign::Start,
    );
    let lines = layout_lines(&inline, None, &Mono, &[]);
    assert_eq!(
        texts(&lines),
        vec![vec![
            ("ab".to_owned(), 0.0, 20.0),
            ("   ".to_owned(), 20.0, 30.0)
        ]]
    );
    let collapsed = content(
        vec![
            segment("ab ", WhiteSpace::Normal, 10.0),
            segment(" ", WhiteSpace::Normal, 10.0),
        ],
        TextAlign::Start,
    );
    let lines = layout_lines(&collapsed, None, &Mono, &[]);
    assert_eq!(texts(&lines), vec![vec![("ab".to_owned(), 0.0, 20.0)]]);
}

#[test]
fn forced_breaks_and_empty_content() {
    let inline = content(
        vec![segment("a\n\nb\n", WhiteSpace::Pre, 10.0)],
        TextAlign::Start,
    );
    let lines = layout_lines(&inline, None, &Mono, &[]);
    assert_eq!(lines.lines.len(), 3);
    assert!(lines.lines[1].fragments.is_empty());
    assert_eq!(lines.height, 30.0);
    let empty = content(Vec::new(), TextAlign::Start);
    let lines = layout_lines(&empty, Some(10.0), &Mono, &[]);
    assert_eq!(lines.lines.len(), 1);
    assert_eq!(lines.height, 10.0);
    assert!(empty.is_blank());
    assert!(content(vec![segment("  ", WhiteSpace::Pre, 1.0)], TextAlign::Start).is_blank());
    assert!(!inline.is_blank());
}

#[test]
fn alignment_and_indent() {
    let line = |align: TextAlign| {
        let inline = content(vec![segment("ab", WhiteSpace::Normal, 10.0)], align);
        layout_lines(&inline, Some(100.0), &Mono, &[]).lines[0].fragments[0].x
    };
    assert_eq!(line(TextAlign::Right), 80.0);
    assert_eq!(line(TextAlign::End), 80.0);
    assert_eq!(line(TextAlign::Center), 40.0);
    assert_eq!(line(TextAlign::Justify), 0.0);
    let mut inline = content(
        vec![segment("ab", WhiteSpace::Normal, 10.0)],
        TextAlign::Center,
    );
    assert_eq!(
        layout_lines(&inline, None, &Mono, &[]).lines[0].fragments[0].x,
        0.0
    );
    inline.indent = 15.0;
    inline.text_align = TextAlign::Start;
    assert_eq!(
        layout_lines(&inline, None, &Mono, &[]).lines[0].fragments[0].x,
        15.0
    );
    let mut spaced = segment("ab", WhiteSpace::Normal, 10.0);
    spaced.letter_spacing = 2.0;
    let lines = layout_lines(&content(vec![spaced], TextAlign::Start), None, &Mono, &[]);
    assert_eq!(lines.width, 24.0);
}

#[test]
fn long_words_break_as_overflow_wrap_and_word_break_say() {
    // Ten pixels per character.
    let normal = breaking("ab abcdefgh", WordBreaking::Normal);
    assert_eq!(
        words(&layout_lines(&normal, Some(35.0), &Mono, &[])),
        ["ab", "abcdefgh"]
    );
    for kind in [WordBreaking::Overflowing, WordBreaking::Anywhere] {
        let text = breaking("ab abcdefgh", kind);
        assert_eq!(
            words(&layout_lines(&text, Some(35.0), &Mono, &[])),
            ["ab", "abc", "def", "gh"]
        );
        // Narrower than a character, each line still takes one.
        assert_eq!(
            words(&layout_lines(&breaking("xyz", kind), Some(5.0), &Mono, &[])),
            ["x", "y", "z"]
        );
    }
    // Only `anywhere` lets the breaks shrink the minimum content width.
    let overflowing = breaking("ab abcdefgh", WordBreaking::Overflowing);
    assert_eq!(
        break_lines(&overflowing, Some(0.0), true, &Mono, &[]).width,
        80.0
    );
    let anywhere = breaking("ab abcdefgh", WordBreaking::Anywhere);
    assert_eq!(
        break_lines(&anywhere, Some(0.0), true, &Mono, &[]).width,
        10.0
    );
    // `break-all` fills the rest of the line first.
    let all = breaking("ab abcdefgh", WordBreaking::All);
    assert_eq!(
        words(&layout_lines(&all, Some(55.0), &Mono, &[])),
        ["ab ab", "cdefg", "h"]
    );
    // A word following another without a space starts a new line when
    // nothing of it fits.
    let joined = content(
        vec![
            segment("abcd", WhiteSpace::Normal, 10.0),
            Segment {
                word_breaking: WordBreaking::Anywhere,
                ..segment("efgh", WhiteSpace::Normal, 10.0)
            },
        ],
        TextAlign::Start,
    );
    assert_eq!(
        words(&layout_lines(&joined, Some(40.0), &Mono, &[])),
        ["abcd", "efgh"]
    );
    // Words that fit are left whole.
    assert_eq!(
        words(&layout_lines(&anywhere, None, &Mono, &[])),
        ["ab abcdefgh"]
    );
}

#[test]
fn words_break_where_unicode_allows() {
    // Ten pixels per character. A Markdown link breaks before its target
    // even across spans, as CodeMirror's tokens are, and its URL stays whole.
    let link = content(
        vec![
            segment("see [link]", WhiteSpace::Normal, 10.0),
            segment("(https://example.com)", WhiteSpace::Normal, 10.0),
        ],
        TextAlign::Start,
    );
    assert_eq!(
        words(&layout_lines(&link, Some(120.0), &Mono, &[])),
        ["see [link]", "(https://example.com)"]
    );
    // Ideographs wrap anywhere but before closing punctuation, and set
    // the minimum width; nothing breaks inside words with `nowrap`.
    let chinese = breaking("中文的段落，没有空格。", WordBreaking::Normal);
    assert_eq!(
        words(&layout_lines(&chinese, Some(45.0), &Mono, &[])),
        ["中文的段", "落，没有", "空格。"]
    );
    assert_eq!(
        break_lines(&chinese, Some(0.0), true, &Mono, &[]).width,
        20.0
    );
    let kept = content(
        vec![segment("中文的段落", WhiteSpace::NoWrap, 10.0)],
        TextAlign::Start,
    );
    assert_eq!(
        words(&layout_lines(&kept, Some(25.0), &Mono, &[])),
        ["中文的段落"]
    );
    // A part longer than the line still overflows as `overflow-wrap` says.
    let long = breaking("a-bcdefgh", WordBreaking::Normal);
    assert_eq!(
        words(&layout_lines(&long, Some(35.0), &Mono, &[])),
        ["a-", "bcdefgh"]
    );
    let wrapped = breaking("a-bcdefgh", WordBreaking::Overflowing);
    assert_eq!(
        words(&layout_lines(&wrapped, Some(35.0), &Mono, &[])),
        ["a-", "bcd", "efg", "h"]
    );
}

/// Ten pixels a character, less four where a space comes before a `T`, as
/// a face kerns that pair.
struct Kerned;

impl TextMeasurer for Kerned {
    fn width(&self, text: &str, _: &Font) -> f32 {
        text.chars().count() as f32 * 10.0 - text.matches(" T").count() as f32 * 4.0
    }

    fn metrics(&self, _: &Font) -> FontMetrics {
        FontMetrics::default()
    }
}

#[test]
fn fragments_take_the_width_of_their_whole_text() {
    let mut second = segment("x", WhiteSpace::Normal, 10.0);
    second.node = NodeId(7);
    let mut atom = segment("", WhiteSpace::Normal, 10.0);
    atom.pieces = vec![crate::text::Piece::Atom(0)];
    let mut inline = content(
        vec![segment("a To", WhiteSpace::Normal, 10.0), second, atom],
        TextAlign::Start,
    );
    inline.atoms.push(Atom {
        node: 9,
        align: VerticalAlign::Baseline,
    });
    let size = AtomSize {
        width: 5.0,
        height: 5.0,
        baseline: 5.0,
    };
    let lines = layout_lines(&inline, None, &Kerned, &[size]);
    // The space and the word kern as one text; what follows moves back.
    assert_eq!(
        texts(&lines),
        vec![vec![
            ("a To".to_owned(), 0.0, 36.0),
            ("x".to_owned(), 36.0, 10.0)
        ]]
    );
    assert_eq!(lines.lines[0].atoms[0].x, 46.0);
}

/// Text with a "W" falls back to a font reaching further from the baseline.
struct Fallbacks;

impl TextMeasurer for Fallbacks {
    fn width(&self, text: &str, _: &Font) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    fn metrics(&self, _: &Font) -> FontMetrics {
        FontMetrics::default()
    }

    fn normal_extent(&self, text: &str, _: &Font) -> Option<(f32, f32)> {
        Some(if text.contains('W') {
            (9.0, 3.0)
        } else {
            (5.0, 5.0)
        })
    }
}

#[test]
fn fallback_fonts_make_lines_of_normal_height_taller() {
    let normal = |text: &str, normal_line: bool| {
        let lines = layout_lines(
            &content(
                vec![Segment {
                    normal_line,
                    ..segment(text, WhiteSpace::Normal, 10.0)
                }],
                TextAlign::Start,
            ),
            Some(100.0),
            &Fallbacks,
            &[],
        );
        (lines.lines[0].height, lines.lines[0].baseline)
    };
    assert_eq!(normal("ab", true), (10.0, 5.0));
    // The fallback font's ascent and the primary font's descent.
    assert_eq!(normal("ab W", true), (14.0, 9.0));
    // A set line height ignores the fonts.
    assert_eq!(normal("ab W", false), (10.0, 5.0));
    assert_eq!(
        crate::text::normal_line_height(&Fallbacks, &Font::default()),
        10.0
    );
    assert_eq!(
        crate::text::normal_line_height(
            &Mono,
            &Font {
                size: 10.0,
                ..Font::default()
            }
        ),
        12.0
    );
}

/// Right-to-left content, whose text is at least at level 1.
fn rtl(segments: Vec<Segment>, align: TextAlign) -> InlineContent {
    let segments = segments
        .into_iter()
        .map(|segment| Segment {
            level: segment.level.max(1),
            ..segment
        })
        .collect();
    InlineContent {
        direction: Direction::Rtl,
        ..content(segments, align)
    }
}

fn placed(lines: &Lines) -> Vec<(String, f32, u8)> {
    lines.lines[0]
        .fragments
        .iter()
        .map(|fragment| (fragment.text.clone(), fragment.x, fragment.level))
        .collect()
}

#[test]
fn right_to_left_lines_start_on_the_right_and_overflow_on_the_left() {
    let lay = |content: &InlineContent| layout_lines(content, Some(100.0), &Mono, &[]);
    // Left-to-right text in a right-to-left paragraph keeps its order.
    let start = lay(&rtl(
        vec![segment("aa bb", WhiteSpace::Normal, 10.0)],
        TextAlign::Start,
    ));
    assert_eq!(placed(&start), [("aa bb".to_owned(), 50.0, 2)]);
    // `end` is the left.
    let end = lay(&rtl(
        vec![segment("aa bb", WhiteSpace::Normal, 10.0)],
        TextAlign::End,
    ));
    assert_eq!(placed(&end)[0].1, 0.0);
    // A line too long overflows its end, the left, whatever the alignment.
    for align in [TextAlign::Start, TextAlign::Center, TextAlign::Left] {
        let long = lay(&rtl(
            vec![segment("abcdefghijkl", WhiteSpace::NoWrap, 10.0)],
            align,
        ));
        assert_eq!(placed(&long)[0].1, -20.0);
    }
    // Right-to-left text runs from the right: the second segment is left
    // of the first.
    let hebrew = lay(&rtl(
        vec![
            segment("\u{5d0} ", WhiteSpace::Normal, 10.0),
            segment("\u{5d1}", WhiteSpace::Normal, 10.0),
        ],
        TextAlign::Start,
    ));
    assert_eq!(
        placed(&hebrew),
        [
            ("\u{5d0} ".to_owned(), 80.0, 1),
            ("\u{5d1}".to_owned(), 70.0, 1)
        ]
    );
}

#[test]
fn embedded_right_to_left_text_is_split_and_reversed() {
    let lines = layout_lines(
        &content(
            vec![
                segment("ab \u{5d0}\u{5d1} ", WhiteSpace::Normal, 10.0),
                segment("\u{5d2} cd", WhiteSpace::Normal, 10.0),
            ],
            TextAlign::Start,
        ),
        Some(200.0),
        &Mono,
        &[],
    );
    // The Hebrew run, across both segments, reads from the right.
    assert_eq!(
        placed(&lines),
        [
            ("ab ".to_owned(), 0.0, 0),
            ("\u{5d0}\u{5d1} ".to_owned(), 40.0, 1),
            ("\u{5d2}".to_owned(), 30.0, 1),
            (" cd".to_owned(), 70.0, 0),
        ]
    );
    // An isolate in the other direction stays whole and in order.
    let isolated = layout_lines(
        &rtl(
            vec![
                segment("x/", WhiteSpace::Normal, 10.0),
                Segment {
                    level: 2,
                    ..segment("ab/cd", WhiteSpace::Normal, 10.0)
                },
            ],
            TextAlign::Start,
        ),
        Some(100.0),
        &Mono,
        &[],
    );
    assert_eq!(
        placed(&isolated),
        [
            ("x".to_owned(), 90.0, 2),
            ("/".to_owned(), 80.0, 1),
            ("ab/cd".to_owned(), 30.0, 2),
        ]
    );
}

#[test]
fn right_to_left_ellipses_cut_text_on_the_left() {
    let content = rtl(
        vec![segment("abcdefghijkl", WhiteSpace::NoWrap, 10.0)],
        TextAlign::Start,
    );
    let lines = layout_lines(&content, Some(100.0), &Mono, &[]);
    let (kept, mark) = ellipsize(&content, &lines.lines[0].fragments, 100.0, &Mono);
    assert_eq!(
        kept.iter()
            .map(|fragment| (fragment.text.as_str(), fragment.x))
            .collect::<Vec<_>>(),
        [("defghijkl", 10.0)]
    );
    assert_eq!(mark.map(|mark| (mark.x, mark.width)), Some((0.0, 10.0)));
}

#[test]
fn reordered_lines_keep_the_gaps_of_inline_boxes() {
    let gap = Segment {
        pieces: vec![crate::text::Piece::Gap(5.0)],
        ..segment("", WhiteSpace::Normal, 10.0)
    };
    let lines = layout_lines(
        &content(
            vec![
                segment("ab ", WhiteSpace::Normal, 10.0),
                gap,
                segment("cd \u{5d0}", WhiteSpace::Normal, 10.0),
            ],
            TextAlign::Start,
        ),
        Some(200.0),
        &Mono,
        &[],
    );
    // The padding before `cd` stays where it was laid out.
    assert_eq!(
        placed(&lines),
        [
            ("ab ".to_owned(), 0.0, 0),
            ("".to_owned(), 30.0, 0),
            ("cd ".to_owned(), 35.0, 0),
            ("\u{5d0}".to_owned(), 65.0, 1),
        ]
    );
}
