use super::*;

/// Big-endian words.
fn words(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

/// A pair adjustment subtable listing pairs by first glyph, with x advances
/// only, or x placements before them when `placed`.
fn glyph_pairs(firsts: &[u16], sets: &[&[(u16, i16)]], placed: bool) -> Vec<u8> {
    let format = if placed { 0x0005 } else { 0x0004 };
    let header = 10 + 2 * sets.len();
    let mut bodies = Vec::new();
    let mut offsets = Vec::new();
    for set in sets {
        offsets.push((header + bodies.len()) as u16);
        bodies.extend(words(&[set.len() as u16]));
        for (glyph, value) in *set {
            bodies.extend(words(&[*glyph]));
            if placed {
                bodies.extend(words(&[7]));
            }
            bodies.extend(words(&[*value as u16]));
        }
    }
    let coverage = (header + bodies.len()) as u16;
    let mut data = words(&[1, coverage, format, 0, sets.len() as u16]);
    data.extend(words(&offsets));
    data.extend(bodies);
    data.extend(words(&[1, firsts.len() as u16]));
    data.extend(words(firsts));
    data
}

/// A pair adjustment subtable by classes, its coverage given as ranges.
fn class_pairs(
    ranges: &[(u16, u16, u16)],
    firsts: &[(u16, u16, u16)],
    seconds: &[(u16, u16, u16)],
    matrix: &[&[i16]],
) -> Vec<u8> {
    let class_def = |ranges: &[(u16, u16, u16)]| {
        let mut data = words(&[2, ranges.len() as u16]);
        for (start, end, class) in ranges {
            data.extend(words(&[*start, *end, *class]));
        }
        data
    };
    let values: Vec<u16> = matrix
        .iter()
        .flat_map(|row| row.iter().map(|value| *value as u16))
        .collect();
    let mut coverage = words(&[2, ranges.len() as u16]);
    for (start, end, index) in ranges {
        coverage.extend(words(&[*start, *end, *index]));
    }
    let first = class_def(firsts);
    let second = class_def(seconds);
    let start = 16 + values.len() * 2;
    let mut data = words(&[
        2,
        start as u16,
        0x0004,
        0,
        (start + coverage.len()) as u16,
        (start + coverage.len() + first.len()) as u16,
        matrix.len() as u16,
        matrix[0].len() as u16,
    ]);
    data.extend(words(&values));
    data.extend(coverage);
    data.extend(first);
    data.extend(second);
    data
}

/// A lookup of a type with subtables, each wrapped in an extension when
/// `extended`.
fn lookup(subtables: &[Vec<u8>], extended: bool) -> Vec<u8> {
    let header = 6 + 2 * subtables.len();
    let mut body = Vec::new();
    let mut offsets = Vec::new();
    for subtable in subtables {
        offsets.push((header + body.len()) as u16);
        if extended {
            body.extend(words(&[1, 2]));
            body.extend(8u32.to_be_bytes());
        }
        body.extend(subtable);
    }
    let mut data = words(&[if extended { 9 } else { 2 }, 0, subtables.len() as u16]);
    data.extend(words(&offsets));
    data.extend(body);
    data
}

/// A `GPOS` table with features naming lookups.
fn gpos(features: &[(&[u8; 4], &[u16])], lookups: &[Vec<u8>]) -> Vec<u8> {
    let mut feature_list = words(&[features.len() as u16]);
    let mut tables = Vec::new();
    let records = 2 + 6 * features.len();
    for (tag, indices) in features {
        feature_list.extend(*tag);
        feature_list.extend(words(&[(records + tables.len()) as u16]));
        tables.extend(words(&[0, indices.len() as u16]));
        tables.extend(words(indices));
    }
    feature_list.extend(tables);
    let mut lookup_list = words(&[lookups.len() as u16]);
    let mut bodies: Vec<u8> = Vec::new();
    for lookup in lookups {
        lookup_list.extend(words(&[(2 + 2 * lookups.len() + bodies.len()) as u16]));
        bodies.extend(lookup);
    }
    lookup_list.extend(bodies);
    let mut data = words(&[1, 0, 0, 10, (10 + feature_list.len()) as u16]);
    data.extend(feature_list);
    data.extend(lookup_list);
    data
}

#[test]
fn kern_feature_lookups_add_their_first_matching_subtables() {
    let first = lookup(
        &[
            glyph_pairs(&[10, 20], &[&[(12, -30), (11, -50)], &[(21, 40)]], false),
            class_pairs(
                &[(10, 10, 0), (30, 30, 1)],
                &[(10, 10, 1), (30, 30, 2)],
                &[(13, 13, 1)],
                &[&[0, 0], &[0, -7], &[0, -9]],
            ),
        ],
        false,
    );
    let second = lookup(&[glyph_pairs(&[10], &[&[(11, -5)]], true)], true);
    let ligatures = lookup(&[glyph_pairs(&[10], &[&[(11, -1000)]], false)], false);
    let table = gpos(
        &[(b"liga", &[2]), (b"kern", &[1, 0])],
        &[first, second, ligatures],
    );
    let kerning = Kerning::parse(Some(&table), None);
    assert!(!kerning.is_empty());
    // Both kern lookups adjust the pair; the ligature feature's does not.
    assert_eq!(kerning.pair(10, 11), -55);
    assert_eq!(kerning.pair(10, 12), -30);
    assert_eq!(kerning.pair(20, 21), 40);
    // A pair the list lacks falls to the class subtable.
    assert_eq!(kerning.pair(10, 13), -7);
    assert_eq!(kerning.pair(30, 13), -9);
    assert_eq!(kerning.pair(10, 14), 0);
    assert_eq!(kerning.pair(11, 10), 0);
    assert_eq!(kerning.pair(40, 13), 0);
}

#[test]
fn faces_without_gpos_kerning_use_their_kern_table() {
    let mut kern = words(&[0, 2]);
    // A cross-stream subtable is skipped; the horizontal one counts.
    kern.extend(words(&[0, 20, 0x0005, 1, 6, 0, 0]));
    kern.extend(words(&[3, 4, 99u16]));
    kern.extend(words(&[0, 26, 0x0001, 2, 12, 1, 0]));
    kern.extend(words(&[5, 6, (-20i16) as u16]));
    kern.extend(words(&[3, 4, (-80i16) as u16]));
    let empty = gpos(&[(b"liga", &[])], &[]);
    let kerning = Kerning::parse(Some(&empty), Some(&kern));
    assert_eq!(kerning.pair(3, 4), -80);
    assert_eq!(kerning.pair(5, 6), -20);
    assert_eq!(kerning.pair(4, 3), 0);
    // A face with GPOS kerning leaves its kern table alone.
    let lookups = [lookup(&[glyph_pairs(&[3], &[&[(4, -1)]], false)], false)];
    let both = Kerning::parse(Some(&gpos(&[(b"kern", &[0])], &lookups)), Some(&kern));
    assert_eq!(both.pair(3, 4), -1);
    assert_eq!(both.pair(5, 6), 0);
    // Tables cut short or of unknown formats kern nothing.
    assert!(Kerning::parse(None, None).is_empty());
    assert!(Kerning::parse(Some(&table_prefix(&empty)), Some(&kern[..9])).is_empty());
    let unknown = lookup(&[words(&[3, 0, 0, 0])], false);
    assert!(Kerning::parse(Some(&gpos(&[(b"kern", &[0])], &[unknown])), None).is_empty());
}

fn table_prefix(table: &[u8]) -> Vec<u8> {
    table[..7].to_vec()
}
