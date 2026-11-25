//! UTF-16 matching without a JavaScript engine, Rust std or a C runtime.
//! This library deals only in patterns, text and capture ranges; JavaScript
//! object/protocol behavior belongs to the native behavior compiler/library.
#![no_std]

extern crate alloc;

mod first;

use alloc::{collections::BTreeSet, string::String, vec::Vec};
use core::ops::Range;

/// Matching flags. Global/sticky/indices behavior belongs to the caller.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// Case-insensitive matching (`i`).
    pub ignore_case: bool,
    /// Line-aware anchors (`m`).
    pub multiline: bool,
    /// Dot matches line terminators (`s`).
    pub dot_all: bool,
    /// Unicode code point semantics (`u`).
    pub unicode: bool,
    /// Unicode sets syntax, which also uses code point semantics (`v`).
    pub unicode_sets: bool,
}

/// Match and capture ranges in UTF-16 code units.
#[derive(Debug)]
pub struct Match {
    /// The whole match.
    pub range: Range<usize>,
    /// Capturing groups, excluding the whole match; absent groups are None.
    pub captures: Vec<Option<Range<usize>>>,
    /// Named captures in definition order, including absent groups.
    pub groups: Vec<(String, Option<Range<usize>>)>,
    /// Names and one-based capture indices, including absent named groups.
    /// Indices associate named ranges with the same capture-pair object.
    pub group_indices: Vec<(String, usize)>,
}

/// A compiled pattern. This does not contain or execute JavaScript.
#[derive(Clone, Debug)]
pub struct Pattern {
    regex: regress::Regex,
    unicode: bool,
    /// For a pattern anchored at the input's start, the units a match can
    /// begin with: input starting otherwise fails without matching.
    first: Option<first::Units>,
}

#[derive(Clone)]
struct Points<'a> {
    source: &'a [u16],
    index: usize,
    unicode: bool,
}

impl Iterator for Points<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        let first = u32::from(*self.source.get(self.index)?);
        self.index += 1;
        if self.unicode
            && (0xD800..=0xDBFF).contains(&first)
            && let Some(&second) = self.source.get(self.index)
            && (0xDC00..=0xDFFF).contains(&second)
        {
            self.index += 1;
            return Some(0x10000 + ((first - 0xD800) << 10) + u32::from(second - 0xDC00));
        }
        Some(first)
    }
}

impl Pattern {
    /// Compiles lossless UTF-16 pattern text, returning native syntax details.
    pub fn compile(source: &[u16], flags: Flags) -> Result<Pattern, String> {
        let unicode = flags.unicode || flags.unicode_sets;
        let points = Points {
            source,
            index: 0,
            unicode,
        };
        let regex = regress::Regex::from_unicode(
            points,
            regress::Flags {
                icase: flags.ignore_case,
                multiline: flags.multiline,
                dot_all: flags.dot_all,
                unicode,
                unicode_sets: flags.unicode_sets,
                ..regress::Flags::default()
            },
        )
        .map_err(|error| alloc::format!("{error}"))?;
        let mut names = BTreeSet::new();
        for name in &regex.cr.group_names {
            if !name.is_empty() && !names.insert(name.as_ref()) {
                return Err(String::from("Duplicate capture group name"));
            }
        }
        Ok(Pattern {
            regex,
            unicode,
            first: first::first_units(source, flags),
        })
    }

    /// Finds the first match at/after a UTF-16 index. In Unicode mode a
    /// starting index inside a surrogate pair refers to that whole code point,
    /// as Node does when an observable lastIndex splits a pair.
    pub fn search(&self, text: &[u16], mut start: usize) -> Option<Match> {
        if start > text.len() {
            return None;
        }
        // An anchored pattern matches only at the start, beginning with one
        // of its first units.
        if let Some(first) = &self.first
            && (start > 0 || !text.first().is_some_and(|unit| first.contains(*unit)))
        {
            return None;
        }
        if self.unicode
            && start > 0
            && text
                .get(start)
                .is_some_and(|unit| (0xDC00..=0xDFFF).contains(unit))
            && (0xD800..=0xDBFF).contains(&text[start - 1])
        {
            start -= 1;
        }
        let matched = if self.unicode {
            self.regex.find_from_utf16(text, start).next()
        } else {
            self.regex.find_from_ucs2(text, start).next()
        }?;
        let groups = matched
            .named_groups()
            .map(|(name, range)| (String::from(name), range))
            .collect();
        let group_indices = matched
            .group_names
            .iter()
            .enumerate()
            .filter(|(_, name)| !name.is_empty())
            .map(|(index, name)| (String::from(name.as_ref()), index + 1))
            .collect();
        Some(Match {
            range: matched.range,
            captures: matched.captures,
            groups,
            group_indices,
        })
    }
}

#[cfg(test)]
mod tests;
