//! Bidirectional text: the levels of a line's pieces of text and the
//! visual order they give, after the Unicode Bidirectional Algorithm.
//!
//! The algorithm is followed in a simplified form, as far as Latin, Hebrew
//! and Arabic text with numbers and punctuation need it: characters are
//! strong left-to-right, strong right-to-left, European or Arabic numbers,
//! or neutral; runs at one embedding level resolve their numbers and
//! neutrals by the strong text around them, starting and ending in the
//! direction of their level; and the pieces of a line are reversed from the
//! highest level to the lowest odd one. Mirrored glyphs and the finer weak
//! types are left out.

use crate::prelude::*;

/// The bidirectional class of a character, coarsely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Strong left-to-right: Latin, Greek, CJK and most letters.
    Left,
    /// Strong right-to-left: Hebrew, Arabic and other right-to-left scripts.
    Right,
    /// A digit, which runs left to right but takes the direction of the
    /// text before it for the neutrals around it.
    Number,
    /// Punctuation and symbols.
    Neutral,
    /// White space, a neutral that takes the paragraph's level at the end
    /// of a line.
    Space,
}

/// The class of a character.
pub fn class(character: char) -> Class {
    match character {
        '0'..='9' | '\u{660}'..='\u{669}' | '\u{6f0}'..='\u{6f9}' => Class::Number,
        '\u{590}'..='\u{8ff}'
        | '\u{fb1d}'..='\u{fdff}'
        | '\u{fe70}'..='\u{fefe}'
        | '\u{10800}'..='\u{10fff}'
        | '\u{1e800}'..='\u{1efff}'
        | '\u{200f}' => Class::Right,
        '\u{200e}' => Class::Left,
        character if character.is_whitespace() => Class::Space,
        character if character.is_alphabetic() => Class::Left,
        // Marks, ideographs and symbols outside the letters: letters of
        // scripts such as Thai and CJK are alphabetic, so the rest is
        // neutral.
        _ => Class::Neutral,
    }
}

/// Whether text has right-to-left characters.
pub fn has_right(text: &str) -> bool {
    !text.is_ascii()
        && text
            .chars()
            .any(|character| class(character) == Class::Right)
}

/// The direction of the first strong character of text, `true` for right
/// to left, as `dir="auto"` decides it.
pub fn first_strong(text: &str) -> Option<bool> {
    text.chars().find_map(|character| match class(character) {
        Class::Left => Some(false),
        Class::Right => Some(true),
        _ => None,
    })
}

/// The lowest level above `level` with the parity of a direction: the level
/// an embedding or isolate in that direction opens.
pub fn embed(level: u8, rtl: bool) -> u8 {
    let next = level + 1;
    let next = if (next % 2 == 1) == rtl {
        next
    } else {
        next + 1
    };
    next.min(125)
}

/// Resolves the levels of a line's units, given in logical order as their
/// embedding levels and classes, in a paragraph of a level: units at one
/// embedding level form a run whose numbers and neutrals resolve by the
/// strong units around them, and whose ends take the run's own direction.
pub fn resolve(units: &[(u8, Class)], paragraph: u8) -> Vec<u8> {
    let mut levels = vec![0; units.len()];
    let mut start = 0;
    while start < units.len() {
        let level = units[start].0;
        let end = units[start..]
            .iter()
            .position(|(other, _)| *other != level)
            .map_or(units.len(), |length| start + length);
        resolve_run(&units[start..end], level, &mut levels[start..end]);
        start = end;
    }
    // Spaces ending a line take the paragraph's level.
    for (index, (_, class)) in units.iter().enumerate().rev() {
        if *class != Class::Space {
            break;
        }
        levels[index] = paragraph;
    }
    levels
}

/// The levels of units of one run at an embedding level.
fn resolve_run(units: &[(u8, Class)], level: u8, levels: &mut [u8]) {
    let edge = if level % 2 == 1 {
        Class::Right
    } else {
        Class::Left
    };
    // Numbers after left-to-right text, or at the start of a
    // left-to-right run, are left to right.
    let mut classes: Vec<Class> = Vec::with_capacity(units.len());
    let mut strong = edge;
    for (_, class) in units {
        let class = match class {
            Class::Number if strong == Class::Left => Class::Left,
            Class::Space => Class::Neutral,
            other => *other,
        };
        if matches!(class, Class::Left | Class::Right) {
            strong = class;
        }
        classes.push(class);
    }
    // Neutrals between text of one direction take it, numbers counting as
    // right to left; the others take the run's direction.
    let direction = |class: Class| match class {
        Class::Number => Class::Right,
        other => other,
    };
    let mut index = 0;
    while index < classes.len() {
        if classes[index] != Class::Neutral {
            index += 1;
            continue;
        }
        let end = classes[index..]
            .iter()
            .position(|class| *class != Class::Neutral)
            .map_or(classes.len(), |length| index + length);
        let before = if index == 0 {
            edge
        } else {
            direction(classes[index - 1])
        };
        let after = classes.get(end).map_or(edge, |class| direction(*class));
        let resolved = if before == after { before } else { edge };
        for class in &mut classes[index..end] {
            *class = resolved;
        }
        index = end;
    }
    for (resolved, class) in levels.iter_mut().zip(classes) {
        *resolved = match (level % 2 == 1, class) {
            (false, Class::Right) => level + 1,
            (false, Class::Number) => level + 2,
            (true, Class::Left | Class::Number) => level + 1,
            _ => level,
        };
    }
}

/// The visual order of units by their levels, left to right: from the
/// highest level to the lowest odd one, each run of units at that level or
/// above is reversed.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(&highest) = levels.iter().max() else {
        return order;
    };
    let lowest_odd = levels
        .iter()
        .copied()
        .filter(|level| level % 2 == 1)
        .min()
        .unwrap_or(highest + 1);
    let mut level = highest;
    while level >= lowest_odd && level > 0 {
        let mut index = 0;
        while index < order.len() {
            if levels[order[index]] < level {
                index += 1;
                continue;
            }
            let end = order[index..]
                .iter()
                .position(|unit| levels[*unit] < level)
                .map_or(order.len(), |length| index + length);
            order[index..end].reverse();
            index = end;
        }
        level -= 1;
    }
    order
}

/// Splits text into runs of characters with one class, as byte ranges with
/// their classes.
pub fn class_runs(text: &str) -> Vec<(core::ops::Range<usize>, Class)> {
    let mut runs: Vec<(core::ops::Range<usize>, Class)> = Vec::new();
    for (at, character) in text.char_indices() {
        let class = class(character);
        let end = at + character.len_utf8();
        match runs.last_mut() {
            Some((range, last)) if *last == class => range.end = end,
            _ => runs.push((at..end, class)),
        }
    }
    runs
}

#[cfg(test)]
mod tests;
