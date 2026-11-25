//! Line break opportunities inside words, as Chromium finds them: between
//! two printable ASCII characters from its own table, else as Unicode's
//! line breaking algorithm (UAX #14) finds them: between ideographs, after
//! hyphens, before opening brackets and so on. Spaces and forced breaks are
//! split off before; these rules decide the breaks between the other
//! characters. The `Line_Break` classes come from a generated table.

use crate::prelude::*;

mod ascii;
mod table;

/// A `Line_Break` class after UAX #14's LB1 resolutions; `Opw` and `Cpw`
/// are opening and closing punctuation of East Asian width F, W or H.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Al,
    B2,
    Ba,
    Bb,
    Bk,
    Cb,
    Cl,
    Cm,
    Cp,
    Cpw,
    Cr,
    Eb,
    Em,
    Ex,
    Gl,
    H2,
    H3,
    Hl,
    Hy,
    Id,
    In,
    Is,
    Jl,
    Jt,
    Jv,
    Lf,
    Nl,
    Ns,
    Nu,
    Op,
    Opw,
    Po,
    Pr,
    Qu,
    Ri,
    Sp,
    Sy,
    Wj,
    Zw,
    Zwj,
}

const CLASSES: [Class; 40] = {
    use Class::*;
    [
        Al, B2, Ba, Bb, Bk, Cb, Cl, Cm, Cp, Cpw, Cr, Eb, Em, Ex, Gl, H2, H3, Hl, Hy, Id, In, Is,
        Jl, Jt, Jv, Lf, Nl, Ns, Nu, Op, Opw, Po, Pr, Qu, Ri, Sp, Sy, Wj, Zw, Zwj,
    ]
};

/// The line breaking class of a character.
pub(crate) fn class(character: char) -> Class {
    let key = (u32::from(character) << 8) | 0xFF;
    let index = table::RANGES.partition_point(|range| *range <= key);
    let entry = table::RANGES[index.saturating_sub(1)];
    CLASSES[(entry & 0xFF) as usize]
}

/// Whether a line may break between two characters whose classes are
/// `before` and `after`; `regional` counts the regional indicators ending
/// at `before`.
fn breaks(before: Class, after: Class, regional: usize) -> bool {
    use Class::*;
    match (before, after) {
        // LB4 to LB6: breaks after and only after line ends.
        (Cr, Lf) => false,
        (Bk | Cr | Lf | Nl, _) => true,
        (_, Bk | Cr | Lf | Nl) => false,
        // LB7, LB8 and LB11 to LB12a.
        (_, Sp | Zw) => false,
        (Zw, _) => true,
        (Wj, _) | (_, Wj) | (Gl, _) => false,
        (Sp | Ba | Hy, Gl) => true,
        (_, Gl) => false,
        // LB13 to LB17: no breaks before closing punctuation or after
        // opening punctuation.
        (_, Cl | Cp | Cpw | Ex | Is | Sy) => false,
        (Op | Opw, _) => false,
        (Qu, Op | Opw) => false,
        (Cl | Cp | Cpw, Ns) => false,
        (B2, B2) => false,
        // LB18 to LB22.
        (Sp, _) => true,
        (Qu, _) | (_, Qu) => false,
        (Cb, _) | (_, Cb) => true,
        (_, Ba | Hy | Ns | In) | (Bb, _) | (Sy, Hl) => false,
        // LB23 to LB25: letters, numbers and their prefixes and suffixes.
        (Al | Hl, Nu) | (Nu, Al | Hl) => false,
        (Pr, Id | Eb | Em) | (Id | Eb | Em, Po) => false,
        (Pr | Po, Al | Hl) | (Al | Hl, Pr | Po) => false,
        (Cl | Cp | Cpw | Nu, Po | Pr) | (Po | Pr, Op | Opw | Nu) | (Hy | Is | Sy | Nu, Nu) => false,
        // LB26 and LB27: Korean syllables.
        (Jl, Jl | Jv | H2 | H3) | (Jv | H2, Jv | Jt) | (Jt | H3, Jt) => false,
        (Jl | Jv | Jt | H2 | H3, Po) | (Pr, Jl | Jv | Jt | H2 | H3) => false,
        // LB28 to LB30b.
        (Al | Hl, Al | Hl) | (Is, Al | Hl) => false,
        (Al | Hl | Nu, Op) | (Cp, Al | Hl | Nu) => false,
        (Ri, Ri) => regional.is_multiple_of(2),
        (Eb, Em) => false,
        // LB31: everywhere else.
        _ => true,
    }
}

/// Whether a line may break between two characters by Chromium's rules for
/// ASCII, `None` when they are not both printable ASCII: a `-` breaks
/// before a digit only after a letter or digit, where it is no minus sign,
/// and other pairs follow Chromium's table.
fn ascii_breaks(last_last: Option<char>, last: char, character: char) -> Option<bool> {
    if last == '-' && character.is_ascii_digit() {
        return Some(last_last.is_some_and(|before| before.is_ascii_alphanumeric()));
    }
    let index = |character: char| {
        u32::from(character)
            .checked_sub(u32::from(ascii::FIRST))
            .filter(|index| *index < 94)
    };
    let (row, column) = (index(last)?, index(character)?);
    Some(ascii::ROWS[row as usize] >> column & 1 == 1)
}

/// Byte offsets in `word` at which a line may break, ascending. Offset 0
/// is included when the line may break between `previous`, the last
/// character of the text before, and the word.
pub(crate) fn opportunities(previous: Option<char>, word: &str) -> Vec<usize> {
    use Class::*;
    let mut found = Vec::new();
    // The class characters compare as: combining marks take their base's
    // (LB9), or are letters without one (LB10).
    let mut before = previous.map(|character| match class(character) {
        Cm | Zwj => Al,
        other => other,
    });
    let mut joined = previous == Some('\u{200D}');
    let mut regional = usize::from(before == Some(Ri));
    let (mut last_last, mut last) = (None, previous);
    for (offset, character) in word.char_indices() {
        let current = class(character);
        let attaches = matches!(current, Cm | Zwj)
            && before.is_some_and(|before| !matches!(before, Bk | Cr | Lf | Nl | Sp | Zw));
        if let Some(before) = before
            && !joined
            && !attaches
            && last
                .and_then(|last| ascii_breaks(last_last, last, character))
                .unwrap_or_else(|| breaks(before, current, regional))
        {
            found.push(offset);
        }
        (last_last, last) = (last, Some(character));
        joined = current == Zwj;
        if !attaches {
            regional = if current == Ri { regional + 1 } else { 0 };
            before = Some(if matches!(current, Cm | Zwj) {
                Al
            } else {
                current
            });
        }
    }
    found
}

#[cfg(test)]
mod tests;
