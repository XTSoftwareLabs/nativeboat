//! The code units a match of a pattern anchored at the input's start can
//! begin with. Tokenizers try many such patterns at every position, as
//! `/^(\*\*)/.exec(rest)`, and most fail on the first unit; knowing the
//! units lets those fail without running the matcher. The analysis is
//! conservative: syntax it does not follow, or a match that can begin with
//! any unit, gives no set, and the matcher decides.

use crate::Flags;
use alloc::vec::Vec;

/// Sorted, disjoint inclusive ranges of code units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Units(Vec<(u16, u16)>);

impl Units {
    fn of(ranges: &[(u16, u16)]) -> Units {
        let mut units = Units::default();
        for &(low, high) in ranges {
            units.add(low, high);
        }
        units
    }

    fn add(&mut self, low: u16, high: u16) {
        self.0.push((low.min(high), low.max(high)));
        self.0.sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::with_capacity(self.0.len());
        for &(low, high) in &self.0 {
            match merged.last_mut() {
                Some(last) if u32::from(low) <= u32::from(last.1) + 1 => last.1 = last.1.max(high),
                _ => merged.push((low, high)),
            }
        }
        self.0 = merged;
    }

    fn join(&mut self, other: &Units) {
        for &(low, high) in &other.0 {
            self.add(low, high);
        }
    }

    /// Whether a code unit is in the set.
    pub(crate) fn contains(&self, unit: u16) -> bool {
        self.0
            .iter()
            .any(|&(low, high)| low <= unit && unit <= high)
    }
}

/// What a part of a pattern can begin with: some units, or any unit
/// (`None`), and whether it can match nothing.
#[derive(Clone, Debug)]
struct First {
    units: Option<Units>,
    empty: bool,
}

impl First {
    fn nothing() -> First {
        First {
            units: Some(Units::default()),
            empty: true,
        }
    }

    fn any(empty: bool) -> First {
        First { units: None, empty }
    }

    fn units(units: Units) -> First {
        First {
            units: Some(units),
            empty: false,
        }
    }

    /// The first units of this part followed by `next`.
    fn then(self, next: First) -> First {
        if !self.empty {
            return self;
        }
        First {
            units: union(self.units, next.units),
            empty: next.empty,
        }
    }

    /// Either this part or `other`.
    fn or(self, other: First) -> First {
        First {
            units: union(self.units, other.units),
            empty: self.empty || other.empty,
        }
    }
}

fn union(left: Option<Units>, right: Option<Units>) -> Option<Units> {
    let (mut left, right) = (left?, right?);
    left.join(&right);
    Some(left)
}

const DIGITS: &[(u16, u16)] = &[(0x30, 0x39)];
const WORD: &[(u16, u16)] = &[(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)];
const SPACE: &[(u16, u16)] = &[
    (0x09, 0x0D),
    (0x20, 0x20),
    (0xA0, 0xA0),
    (0x1680, 0x1680),
    (0x2000, 0x200A),
    (0x2028, 0x2029),
    (0x202F, 0x202F),
    (0x205F, 0x205F),
    (0x3000, 0x3000),
    (0xFEFF, 0xFEFF),
];

/// The units a match can begin with, for a pattern whose every alternative
/// starts with `^` outside multiline mode, so matches start only at the
/// input's start; `None` when that is not known.
pub(crate) fn first_units(source: &[u16], flags: Flags) -> Option<Units> {
    if flags.multiline || flags.unicode_sets {
        return None;
    }
    let mut parser = Parser {
        source,
        at: 0,
        unicode: flags.unicode,
    };
    let first = parser.disjunction(true)?;
    if parser.at != source.len() || first.empty {
        return None;
    }
    let units = first.units?;
    // Case-insensitive patterns match other letters, some outside ASCII.
    if flags.ignore_case
        && units.0.iter().any(|&(low, high)| {
            high > 0x7F || (low..=high).any(|unit| (unit as u8).is_ascii_alphabetic())
        })
    {
        return None;
    }
    // In Unicode mode a pattern's unit may begin a surrogate pair.
    if flags.unicode && units.0.iter().any(|&(_, high)| high >= 0xD800) {
        return None;
    }
    Some(units)
}

struct Parser<'a> {
    source: &'a [u16],
    at: usize,
    unicode: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.source.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u16> {
        self.source.get(self.at + offset).copied()
    }

    fn eat(&mut self, unit: u8) -> bool {
        if self.peek() == Some(u16::from(unit)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Alternatives up to a `)` or the end; at the top, each must start
    /// with `^`.
    fn disjunction(&mut self, top: bool) -> Option<First> {
        let mut first = self.alternative(top)?;
        while self.eat(b'|') {
            first = first.or(self.alternative(top)?);
        }
        Some(first)
    }

    fn alternative(&mut self, top: bool) -> Option<First> {
        if top && !self.eat(b'^') {
            return None;
        }
        let mut first = First::nothing();
        while let Some(unit) = self.peek() {
            if unit == u16::from(b'|') || unit == u16::from(b')') {
                break;
            }
            let term = self.term()?;
            first = first.then(term);
        }
        Some(first)
    }

    /// An assertion, or an atom and its quantifier.
    fn term(&mut self) -> Option<First> {
        let unit = self.peek()?;
        match unit as u8 {
            _ if unit > 0x7F => {
                self.at += 1;
                self.quantified(First::units(Units::of(&[(unit, unit)])))
            }
            b'^' | b'$' => {
                self.at += 1;
                Some(First::nothing())
            }
            b'\\' if matches!(self.peek_at(1), Some(0x62 | 0x42)) => {
                // `\b` and `\B` match between units.
                self.at += 2;
                Some(First::nothing())
            }
            b'(' => {
                self.at += 1;
                if self.eat(b'?') {
                    let look = match self.peek().map(|unit| unit as u8) {
                        Some(b'=' | b'!') => {
                            self.at += 1;
                            true
                        }
                        Some(b'<') if matches!(self.peek_at(1), Some(0x3D | 0x21)) => {
                            self.at += 2;
                            true
                        }
                        Some(b':') => {
                            self.at += 1;
                            false
                        }
                        Some(b'<') => {
                            // A named group.
                            while self.peek()? != u16::from(b'>') {
                                self.at += 1;
                            }
                            self.at += 1;
                            false
                        }
                        _ => return None,
                    };
                    let inner = self.disjunction(false)?;
                    if !self.eat(b')') {
                        return None;
                    }
                    if look {
                        // Lookarounds consume nothing; quantifying one is
                        // allowed only outside Unicode mode and changes
                        // nothing a match begins with.
                        self.quantifier()?;
                        return Some(First::nothing());
                    }
                    return self.quantified(inner);
                }
                let inner = self.disjunction(false)?;
                if !self.eat(b')') {
                    return None;
                }
                self.quantified(inner)
            }
            b'[' => {
                let class = self.class()?;
                self.quantified(class)
            }
            b'.' => {
                self.at += 1;
                self.quantified(First::any(false))
            }
            b'\\' => {
                self.at += 1;
                let escape = self.escape(false)?;
                self.quantified(escape)
            }
            // A quantifier with nothing to repeat, or in Unicode mode a
            // lone bracket or brace, is a syntax error the compiler reports.
            b'*' | b'+' | b'?' => None,
            b']' | b'{' | b'}' if self.unicode => None,
            _ => {
                self.at += 1;
                self.quantified(First::units(Units::of(&[(unit, unit)])))
            }
        }
    }

    /// An atom's first units under the quantifier that follows it.
    fn quantified(&mut self, atom: First) -> Option<First> {
        let least = self.quantifier()?;
        Some(if least == 0 {
            First {
                empty: true,
                ..atom
            }
        } else {
            atom
        })
    }

    /// The least repetitions the quantifier after an atom allows: one when
    /// there is none.
    fn quantifier(&mut self) -> Option<u32> {
        let least = match self.peek().map(|unit| unit as u8) {
            Some(b'*' | b'?') => {
                self.at += 1;
                0
            }
            Some(b'+') => {
                self.at += 1;
                1
            }
            Some(b'{') => {
                let start = self.at;
                self.at += 1;
                let mut least = 0u32;
                let mut digits = 0;
                while let Some(digit) = self.peek().filter(|unit| (0x30..=0x39).contains(unit)) {
                    least = least
                        .saturating_mul(10)
                        .saturating_add(u32::from(digit - 0x30));
                    digits += 1;
                    self.at += 1;
                }
                if self.eat(b',') {
                    while self
                        .peek()
                        .is_some_and(|unit| (0x30..=0x39).contains(&unit))
                    {
                        self.at += 1;
                    }
                }
                if digits == 0 || !self.eat(b'}') {
                    // Outside Unicode mode a brace that starts no quantifier
                    // is a literal, which `term` reads next.
                    if self.unicode {
                        return None;
                    }
                    self.at = start;
                    return Some(1);
                }
                least
            }
            _ => return Some(1),
        };
        self.eat(b'?');
        Some(least)
    }

    /// An escape after its backslash: a character, a character class, or a
    /// back-reference, which can match nothing.
    fn escape(&mut self, in_class: bool) -> Option<First> {
        let unit = self.peek()?;
        self.at += 1;
        let single = |unit: u16| First::units(Units::of(&[(unit, unit)]));
        Some(match unit as u8 {
            _ if unit > 0x7F => single(unit),
            b'd' => First::units(Units::of(DIGITS)),
            b'w' => First::units(Units::of(WORD)),
            b's' => First::units(Units::of(SPACE)),
            b'D' | b'W' | b'S' | b'p' | b'P' => {
                if matches!(unit as u8, b'p' | b'P') && self.eat(b'{') {
                    while self.peek()? != u16::from(b'}') {
                        self.at += 1;
                    }
                    self.at += 1;
                }
                First::any(false)
            }
            b'n' => single(0x0A),
            b'r' => single(0x0D),
            b't' => single(0x09),
            b'v' => single(0x0B),
            b'f' => single(0x0C),
            b'b' if in_class => single(0x08),
            b'0' if !self
                .peek()
                .is_some_and(|unit| (0x30..=0x39).contains(&unit)) =>
            {
                single(0)
            }
            b'1'..=b'9' | b'k' | b'0' => {
                // A back-reference (or, outside Unicode mode, an octal or
                // literal escape): anything, possibly nothing.
                if unit == u16::from(b'k') && self.eat(b'<') {
                    while self.peek()? != u16::from(b'>') {
                        self.at += 1;
                    }
                    self.at += 1;
                } else {
                    while self
                        .peek()
                        .is_some_and(|unit| (0x30..=0x39).contains(&unit))
                    {
                        self.at += 1;
                    }
                }
                First::any(true)
            }
            b'c' => match self.peek() {
                Some(letter) if (letter as u8).is_ascii_alphabetic() && letter < 0x80 => {
                    self.at += 1;
                    single(letter % 32)
                }
                _ => First::any(false),
            },
            b'x' => match self.hex(2) {
                Some(value) => single(value),
                None => First::any(false),
            },
            b'u' => {
                if self.unicode && self.eat(b'{') {
                    let mut value = 0u32;
                    while let Some(digit) = self
                        .peek()
                        .and_then(|unit| char::from_u32(u32::from(unit))?.to_digit(16))
                    {
                        value = value.saturating_mul(16).saturating_add(digit);
                        self.at += 1;
                    }
                    if !self.eat(b'}') {
                        return None;
                    }
                    match u16::try_from(value) {
                        Ok(value) => single(value),
                        Err(_) => First::any(false),
                    }
                } else {
                    match self.hex(4) {
                        Some(value) => single(value),
                        None => First::any(false),
                    }
                }
            }
            _ => single(unit),
        })
    }

    fn hex(&mut self, count: usize) -> Option<u16> {
        let mut value = 0u16;
        for offset in 0..count {
            let digit = char::from_u32(u32::from(self.peek_at(offset)?))?.to_digit(16)?;
            value = value * 16 + digit as u16;
        }
        self.at += count;
        Some(value)
    }

    /// A character class: its units, or any unit when negated or holding
    /// what the analysis does not follow.
    fn class(&mut self) -> Option<First> {
        self.at += 1;
        let negated = self.eat(b'^');
        let mut units = Some(Units::default());
        loop {
            let unit = self.peek()?;
            if unit == u16::from(b']') {
                self.at += 1;
                break;
            }
            let low = self.class_atom()?;
            // A range: `a-z`.
            if self.peek() == Some(u16::from(b'-'))
                && self.peek_at(1).is_some_and(|next| next != u16::from(b']'))
            {
                self.at += 1;
                let high = self.class_atom()?;
                match (low, high) {
                    (ClassAtom::Unit(low), ClassAtom::Unit(high)) if low <= high => {
                        if let Some(units) = &mut units {
                            units.add(low, high);
                        }
                    }
                    _ => units = None,
                }
                continue;
            }
            match low {
                ClassAtom::Unit(unit) => {
                    if let Some(units) = &mut units {
                        units.add(unit, unit);
                    }
                }
                ClassAtom::Set(set) => {
                    units = union(units, set);
                }
            }
        }
        if negated {
            return Some(First::any(false));
        }
        Some(match units {
            Some(units) => First::units(units),
            None => First::any(false),
        })
    }

    fn class_atom(&mut self) -> Option<ClassAtom> {
        let unit = self.peek()?;
        self.at += 1;
        if unit != u16::from(b'\\') {
            return Some(ClassAtom::Unit(unit));
        }
        let escape = self.escape(true)?;
        Some(match escape.units {
            Some(units) if units.0.len() == 1 && units.0[0].0 == units.0[0].1 => {
                ClassAtom::Unit(units.0[0].0)
            }
            units => ClassAtom::Set(units),
        })
    }
}

enum ClassAtom {
    Unit(u16),
    Set(Option<Units>),
}

#[cfg(test)]
mod tests;
