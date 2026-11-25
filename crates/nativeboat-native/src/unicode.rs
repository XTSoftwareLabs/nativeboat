//! Unicode algorithms over the generated character data: normalization forms
//! (UAX #15), case-ignorable and punctuation properties, and conversion
//! between UTF-16 code units and code points (lone surrogates are kept as
//! their own code points, as JavaScript strings may contain them).

#[allow(unused_imports)]
use crate::prelude::*;

use crate::unicode_tables::{
    CANONICAL, CASE_IGNORABLE, COMBINING_CLASSES, COMPATIBILITY, COMPOSITIONS, DECOMPOSITION_DATA,
    PUNCTUATION,
};

const HANGUL_BASE: u32 = 0xAC00;
const LEAD_BASE: u32 = 0x1100;
const VOWEL_BASE: u32 = 0x1161;
const TRAIL_BASE: u32 = 0x11A7;
const VOWELS: u32 = 21;
const TRAILS: u32 = 28;
const SYLLABLES: u32 = 11_172;

/// A normalization form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

/// The code points of UTF-16 text; unpaired surrogates stay code points.
pub fn code_points(units: &[u16]) -> Vec<u32> {
    char::decode_utf16(units.iter().copied())
        .map(|decoded| match decoded {
            Ok(character) => u32::from(character),
            Err(error) => u32::from(error.unpaired_surrogate()),
        })
        .collect()
}

/// The UTF-16 code units of code points.
pub fn units(points: &[u32]) -> Vec<u16> {
    let mut units = Vec::with_capacity(points.len());
    for &point in points {
        match char::from_u32(point) {
            Some(character) => {
                let mut buffer = [0; 2];
                units.extend_from_slice(character.encode_utf16(&mut buffer));
            }
            // A lone surrogate.
            None => units.push(point as u16),
        }
    }
    units
}

fn in_ranges(ranges: &[(u32, u32)], point: u32) -> bool {
    ranges
        .binary_search_by(|&(start, end)| {
            if end < point {
                core::cmp::Ordering::Less
            } else if start > point {
                core::cmp::Ordering::Greater
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Whether a code point is case-ignorable (for the Final_Sigma rule).
pub fn case_ignorable(point: u32) -> bool {
    in_ranges(&CASE_IGNORABLE, point)
}

/// Whether a code point is whitespace, punctuation, a symbol or a control.
pub fn punctuation(point: u32) -> bool {
    in_ranges(&PUNCTUATION, point)
}

/// The canonical combining class.
pub fn combining_class(point: u32) -> u8 {
    COMBINING_CLASSES
        .binary_search_by_key(&point, |&(code, _)| code)
        .map_or(0, |index| COMBINING_CLASSES[index].1)
}

fn decomposition(table: &[(u32, u16, u8)], point: u32) -> Option<&'static [u32]> {
    let index = table
        .binary_search_by_key(&point, |&(code, _, _)| code)
        .ok()?;
    let (_, start, length) = table[index];
    Some(&DECOMPOSITION_DATA[usize::from(start)..usize::from(start) + usize::from(length)])
}

/// Appends the full canonical (and, for compatibility forms, compatibility)
/// decomposition of a code point.
fn decompose(point: u32, compatibility: bool, out: &mut Vec<u32>) {
    let syllable = point.wrapping_sub(HANGUL_BASE);
    if syllable < SYLLABLES {
        out.push(LEAD_BASE + syllable / (VOWELS * TRAILS));
        out.push(VOWEL_BASE + (syllable % (VOWELS * TRAILS)) / TRAILS);
        if !syllable.is_multiple_of(TRAILS) {
            out.push(TRAIL_BASE + syllable % TRAILS);
        }
        return;
    }
    let found = decomposition(&CANONICAL, point).or_else(|| {
        if compatibility {
            decomposition(&COMPATIBILITY, point)
        } else {
            None
        }
    });
    match found {
        Some(parts) => {
            for &part in parts {
                decompose(part, compatibility, out);
            }
        }
        None => out.push(point),
    }
}

/// Puts each run of combining marks in canonical order (a stable sort by
/// combining class).
fn reorder(points: &mut [u32]) {
    let mut start = 0;
    while start < points.len() {
        if combining_class(points[start]) == 0 {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < points.len() && combining_class(points[end]) != 0 {
            end += 1;
        }
        points[start..end].sort_by_key(|&point| combining_class(point));
        start = end;
    }
}

/// The canonical composite of two code points, if any.
fn compose_pair(first: u32, second: u32) -> Option<u32> {
    let lead = first.wrapping_sub(LEAD_BASE);
    let vowel = second.wrapping_sub(VOWEL_BASE);
    if lead < 19 && vowel < VOWELS {
        return Some(HANGUL_BASE + (lead * VOWELS + vowel) * TRAILS);
    }
    let syllable = first.wrapping_sub(HANGUL_BASE);
    let trail = second.wrapping_sub(TRAIL_BASE);
    if syllable < SYLLABLES && syllable.is_multiple_of(TRAILS) && trail > 0 && trail < TRAILS {
        return Some(first + trail);
    }
    COMPOSITIONS
        .binary_search_by_key(&(first, second), |&(a, b, _)| (a, b))
        .ok()
        .map(|index| COMPOSITIONS[index].2)
}

/// Canonical composition of decomposed, reordered code points.
fn compose(points: &[u32]) -> Vec<u32> {
    let mut result: Vec<u32> = Vec::with_capacity(points.len());
    // The last starter and the highest combining class seen since it.
    let mut starter: Option<usize> = None;
    let mut last_class = 0u8;
    for &point in points {
        let class = combining_class(point);
        if let Some(index) = starter {
            // A mark composes unless a mark of the same or higher class
            // stands between it and the starter (it is blocked).
            let blocked = result.len() - 1 != index && last_class >= class;
            if let (false, Some(composite)) = (blocked, compose_pair(result[index], point)) {
                result[index] = composite;
                continue;
            }
        }
        if class == 0 {
            starter = Some(result.len());
            last_class = 0;
        } else {
            last_class = class;
        }
        result.push(point);
    }
    result
}

/// Normalizes UTF-16 text.
pub fn normalize(text: &[u16], form: Form) -> Vec<u16> {
    // Text below U+00A0 is unchanged by every form.
    if text.iter().all(|&unit| unit < 0xA0) {
        return text.to_vec();
    }
    let compatibility = matches!(form, Form::Nfkc | Form::Nfkd);
    let mut points = Vec::with_capacity(text.len());
    for point in code_points(text) {
        decompose(point, compatibility, &mut points);
    }
    reorder(&mut points);
    if matches!(form, Form::Nfc | Form::Nfkc) {
        points = compose(&points);
    }
    units(&points)
}

#[cfg(test)]
mod tests;
