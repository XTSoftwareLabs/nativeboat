use super::*;

fn units(text: &str, level: u8) -> Vec<(u8, Class)> {
    text.chars()
        .map(|character| (level, class(character)))
        .collect()
}

/// The characters of text in visual order, left to right.
fn visual(text: &str, paragraph: u8) -> String {
    let characters: Vec<char> = text.chars().collect();
    let levels = resolve(&units(text, paragraph), paragraph);
    visual_order(&levels)
        .into_iter()
        .map(|index| characters[index])
        .collect()
}

#[test]
fn characters_are_classed_coarsely() {
    assert_eq!(class('a'), Class::Left);
    assert_eq!(class('\u{5d0}'), Class::Right);
    assert_eq!(class('\u{627}'), Class::Right);
    assert_eq!(class('7'), Class::Number);
    assert_eq!(class(' '), Class::Space);
    assert_eq!(class('/'), Class::Neutral);
    assert_eq!(class('\u{4e2d}'), Class::Left);
    assert!(has_right("ab \u{5d0}"));
    assert!(!has_right("abc"));
    assert_eq!(first_strong("12 \u{5d0}b"), Some(true));
    assert_eq!(first_strong("- b\u{5d0}"), Some(false));
    assert_eq!(first_strong("123"), None);
}

#[test]
fn embeddings_open_the_next_level_of_their_direction() {
    assert_eq!(embed(0, true), 1);
    assert_eq!(embed(0, false), 2);
    assert_eq!(embed(1, false), 2);
    assert_eq!(embed(1, true), 3);
    assert_eq!(embed(125, true), 125);
}

#[test]
fn right_to_left_text_reverses_and_numbers_keep_their_order() {
    // Hebrew letters in a left-to-right paragraph are reversed as a run,
    // with the space between them.
    assert_eq!(
        visual("ab \u{5d0}\u{5d1} \u{5d2} cd", 0),
        "ab \u{5d2} \u{5d1}\u{5d0} cd"
    );
    // A number after Hebrew stays left to right.
    assert_eq!(visual("\u{5d0} 12", 0), "12 \u{5d0}");
    // Latin text in a right-to-left paragraph keeps its order; the period
    // ending it takes the paragraph's direction and goes to the left.
    assert_eq!(visual("ab cd.", 1), ".ab cd");
    // Trailing spaces stay at the end of the line, which is the left.
    assert_eq!(visual("\u{5d0}\u{5d1}  ", 1), "  \u{5d1}\u{5d0}");
}

#[test]
fn isolated_runs_resolve_by_their_own_level() {
    // A left-to-right isolate inside a right-to-left paragraph: its
    // neutrals stay with it and it is placed as one unit.
    let mut line = units("x/", 1);
    line.extend(units("ab/cd", 2));
    let levels = resolve(&line, 1);
    assert_eq!(levels, [2, 1, 2, 2, 2, 2, 2]);
    // The paragraph runs right to left: the isolate, which follows, is
    // on the left, its own text in order.
    assert_eq!(visual_order(&levels), [2, 3, 4, 5, 6, 1, 0]);
}
