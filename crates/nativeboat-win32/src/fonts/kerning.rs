//! Pair kerning from a face's tables, as Chromium's shaper applies it by
//! default: the lookups of the OpenType `kern` feature in `GPOS`, or the
//! legacy `kern` table of faces without one.

#[allow(unused_imports)]
use crate::prelude::*;

/// How far a glyph's advance changes before another, in design units.
#[derive(Debug, Default)]
pub(crate) struct Kerning {
    /// `GPOS` lookups in order, each its subtables in order.
    lookups: Vec<Vec<PairTable>>,
    /// Legacy pairs, sorted, when the face has no `GPOS` kerning.
    legacy: Vec<(u32, i16)>,
}

/// One pair adjustment subtable.
#[derive(Debug)]
enum PairTable {
    /// Pairs listed by first glyph: the first glyph's coverage index picks a
    /// sorted list of second glyphs and adjustments.
    Glyphs {
        coverage: Coverage,
        sets: Vec<Vec<(u16, i16)>>,
    },
    /// Pairs by classes of glyphs.
    Classes {
        coverage: Coverage,
        first: ClassDef,
        second: ClassDef,
        seconds: usize,
        values: Vec<i16>,
    },
}

/// The glyphs a subtable covers, in coverage order.
#[derive(Debug)]
enum Coverage {
    Glyphs(Vec<u16>),
    /// Ranges of glyphs: start, end and the coverage index of the start.
    Ranges(Vec<(u16, u16, u16)>),
}

/// Glyph classes: ranges of glyphs with a class each; others are class 0.
#[derive(Debug, Default)]
struct ClassDef(Vec<(u16, u16, u16)>);

fn word(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn long(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl Coverage {
    fn parse(data: &[u8], at: usize) -> Option<Coverage> {
        let count = usize::from(word(data, at + 2)?);
        match word(data, at)? {
            1 => (0..count)
                .map(|index| word(data, at + 4 + index * 2))
                .collect::<Option<Vec<_>>>()
                .map(Coverage::Glyphs),
            2 => (0..count)
                .map(|index| {
                    let record = at + 4 + index * 6;
                    Some((
                        word(data, record)?,
                        word(data, record + 2)?,
                        word(data, record + 4)?,
                    ))
                })
                .collect::<Option<Vec<_>>>()
                .map(Coverage::Ranges),
            _ => None,
        }
    }

    fn index(&self, glyph: u16) -> Option<usize> {
        match self {
            Coverage::Glyphs(glyphs) => glyphs.binary_search(&glyph).ok(),
            Coverage::Ranges(ranges) => {
                let at = ranges.partition_point(|(_, end, _)| *end < glyph);
                let (start, end, first) = *ranges.get(at)?;
                (start <= glyph && glyph <= end)
                    .then(|| usize::from(first) + usize::from(glyph - start))
            }
        }
    }
}

impl ClassDef {
    fn parse(data: &[u8], at: usize) -> Option<ClassDef> {
        match word(data, at)? {
            1 => {
                let start = word(data, at + 2)?;
                let count = word(data, at + 4)?;
                let mut ranges = Vec::new();
                for index in 0..count {
                    let class = word(data, at + 6 + usize::from(index) * 2)?;
                    let glyph = start.checked_add(index)?;
                    if class != 0 {
                        ranges.push((glyph, glyph, class));
                    }
                }
                Some(ClassDef(ranges))
            }
            2 => {
                let count = usize::from(word(data, at + 2)?);
                let mut ranges = (0..count)
                    .map(|index| {
                        let record = at + 4 + index * 6;
                        Some((
                            word(data, record)?,
                            word(data, record + 2)?,
                            word(data, record + 4)?,
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                ranges.sort_unstable();
                Some(ClassDef(ranges))
            }
            _ => None,
        }
    }

    fn class(&self, glyph: u16) -> usize {
        let at = self.0.partition_point(|(_, end, _)| *end < glyph);
        match self.0.get(at) {
            Some((start, end, class)) if *start <= glyph && glyph <= *end => usize::from(*class),
            _ => 0,
        }
    }
}

/// The size of a value record of a format, and where its x advance is.
fn value_layout(format: u16) -> (usize, Option<usize>) {
    let size = format.count_ones() as usize * 2;
    let advance = (format & 0x0004 != 0).then(|| (format & 0x0003).count_ones() as usize * 2);
    (size, advance)
}

impl PairTable {
    fn parse(data: &[u8], at: usize) -> Option<PairTable> {
        let format = word(data, at)?;
        let coverage = Coverage::parse(data, at + usize::from(word(data, at + 2)?))?;
        let (size1, advance) = value_layout(word(data, at + 4)?);
        let (size2, _) = value_layout(word(data, at + 6)?);
        let read = |record: usize| -> Option<i16> {
            match advance {
                Some(offset) => word(data, record + offset).map(|value| value as i16),
                None => Some(0),
            }
        };
        match format {
            1 => {
                let count = usize::from(word(data, at + 8)?);
                let mut sets = Vec::with_capacity(count);
                for index in 0..count {
                    let set = at + usize::from(word(data, at + 10 + index * 2)?);
                    let pairs = usize::from(word(data, set)?);
                    let stride = 2 + size1 + size2;
                    let mut entries = Vec::with_capacity(pairs);
                    for pair in 0..pairs {
                        let record = set + 2 + pair * stride;
                        entries.push((word(data, record)?, read(record + 2)?));
                    }
                    entries.sort_unstable_by_key(|(glyph, _)| *glyph);
                    sets.push(entries);
                }
                Some(PairTable::Glyphs { coverage, sets })
            }
            2 => {
                let first = ClassDef::parse(data, at + usize::from(word(data, at + 8)?))?;
                let second = ClassDef::parse(data, at + usize::from(word(data, at + 10)?))?;
                let firsts = usize::from(word(data, at + 12)?);
                let seconds = usize::from(word(data, at + 14)?);
                let stride = size1 + size2;
                let mut values = Vec::with_capacity(firsts * seconds);
                for index in 0..firsts * seconds {
                    values.push(read(at + 16 + index * stride)?);
                }
                Some(PairTable::Classes {
                    coverage,
                    first,
                    second,
                    seconds,
                    values,
                })
            }
            _ => None,
        }
    }

    /// The adjustment of a pair, when this subtable has it: a list of pairs
    /// leaves pairs it lacks to the next subtable, as HarfBuzz does, while a
    /// class matrix decides every pair its first glyph's coverage holds.
    fn pair(&self, left: u16, right: u16) -> Option<i16> {
        match self {
            PairTable::Glyphs { coverage, sets } => {
                let set = sets.get(coverage.index(left)?)?;
                set.binary_search_by_key(&right, |(glyph, _)| *glyph)
                    .ok()
                    .map(|at| set[at].1)
            }
            PairTable::Classes {
                coverage,
                first,
                second,
                seconds,
                values,
            } => {
                coverage.index(left)?;
                let column = second.class(right);
                (column < *seconds).then(|| {
                    values
                        .get(first.class(left) * seconds + column)
                        .copied()
                        .unwrap_or(0)
                })
            }
        }
    }
}

impl Kerning {
    /// Kerning from a face's `GPOS` table, else its `kern` table.
    pub(crate) fn parse(gpos: Option<&[u8]>, kern: Option<&[u8]>) -> Kerning {
        let lookups = gpos.and_then(gpos_lookups).unwrap_or_default();
        let legacy = if lookups.is_empty() {
            kern.and_then(legacy_pairs).unwrap_or_default()
        } else {
            Vec::new()
        };
        Kerning { lookups, legacy }
    }

    /// Whether the face kerns no pairs.
    pub(crate) fn is_empty(&self) -> bool {
        self.lookups.is_empty() && self.legacy.is_empty()
    }

    /// How far `left`'s advance changes before `right`, in design units:
    /// each lookup's first subtable with the first glyph decides.
    pub(crate) fn pair(&self, left: u16, right: u16) -> i32 {
        if !self.legacy.is_empty() {
            let key = (u32::from(left) << 16) | u32::from(right);
            return self
                .legacy
                .binary_search_by_key(&key, |(pair, _)| *pair)
                .map_or(0, |at| i32::from(self.legacy[at].1));
        }
        self.lookups
            .iter()
            .filter_map(|lookup| lookup.iter().find_map(|table| table.pair(left, right)))
            .map(i32::from)
            .sum()
    }
}

/// The pair adjustment subtables of the `kern` feature's lookups.
fn gpos_lookups(data: &[u8]) -> Option<Vec<Vec<PairTable>>> {
    let features = usize::from(word(data, 6)?);
    let lookups = usize::from(word(data, 8)?);
    let mut indices: Vec<u16> = Vec::new();
    for index in 0..usize::from(word(data, features)?) {
        let record = features + 2 + index * 6;
        if data.get(record..record + 4)? != b"kern" {
            continue;
        }
        let feature = features + usize::from(word(data, record + 4)?);
        for lookup in 0..usize::from(word(data, feature + 2)?) {
            indices.push(word(data, feature + 4 + lookup * 2)?);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    let mut found = Vec::new();
    for index in indices {
        let lookup = lookups + usize::from(word(data, lookups + 2 + usize::from(index) * 2)?);
        let kind = word(data, lookup)?;
        let mut tables = Vec::new();
        for subtable in 0..usize::from(word(data, lookup + 4)?) {
            let mut at = lookup + usize::from(word(data, lookup + 6 + subtable * 2)?);
            // Extension subtables point to the real ones.
            let mut subtable_kind = kind;
            if kind == 9 {
                subtable_kind = word(data, at + 2)?;
                at += long(data, at + 4)? as usize;
            }
            if subtable_kind == 2
                && let Some(table) = PairTable::parse(data, at)
            {
                tables.push(table);
            }
        }
        if !tables.is_empty() {
            found.push(tables);
        }
    }
    Some(found)
}

/// The horizontal pairs of a legacy `kern` table's format 0 subtables.
fn legacy_pairs(data: &[u8]) -> Option<Vec<(u32, i16)>> {
    let mut pairs = Vec::new();
    let mut at = 4;
    for _ in 0..word(data, 2)? {
        let length = usize::from(word(data, at + 2)?);
        let coverage = word(data, at + 4)?;
        // Format 0, horizontal, not minimum or cross-stream values.
        if coverage >> 8 == 0 && coverage & 0x0007 == 0x0001 {
            for pair in 0..usize::from(word(data, at + 6)?) {
                let record = at + 14 + pair * 6;
                pairs.push((long(data, record)?, word(data, record + 4)? as i16));
            }
        }
        at += length.max(6);
    }
    pairs.sort_unstable_by_key(|(pair, _)| *pair);
    Some(pairs)
}

#[cfg(test)]
mod tests;
