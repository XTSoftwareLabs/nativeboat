use super::*;

fn parse(field: &str) -> Vec<u16> {
    let points: Vec<u32> = field
        .split(' ')
        .map(|hex| u32::from_str_radix(hex, 16).unwrap())
        .collect();
    units(&points)
}

#[test]
fn normalization_matches_the_reference_implementation() {
    let vectors = include_str!("vectors.txt");
    let mut checked = 0;
    for line in vectors.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let source = parse(fields[0]);
        for (form, expected) in [Form::Nfc, Form::Nfd, Form::Nfkc, Form::Nfkd]
            .into_iter()
            .zip(&fields[1..])
        {
            assert_eq!(
                normalize(&source, form),
                parse(expected),
                "{form:?} of {}",
                fields[0]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 12_000);
    assert_eq!(crate::unicode_tables::VERSION, "15.1.0");
}

#[test]
fn hangul_and_marks_compose_and_decompose_algorithmically() {
    // 한 decomposes into three jamo and composes back.
    let syllable = [0xD55C];
    assert_eq!(normalize(&syllable, Form::Nfd), [0x1112, 0x1161, 0x11AB]);
    assert_eq!(normalize(&[0x1112, 0x1161, 0x11AB], Form::Nfc), syllable);
    // 가 has no final consonant.
    assert_eq!(normalize(&[0xAC00], Form::Nfd), [0x1100, 0x1161]);
    // Marks reorder by combining class: dot below (220) before acute (230).
    assert_eq!(
        normalize(&[0x61, 0x301, 0x323], Form::Nfd),
        [0x61, 0x323, 0x301]
    );
    // A text starting with a mark has no starter to compose with.
    assert_eq!(normalize(&[0x301, 0x61], Form::Nfc), [0x301, 0x61]);
    // Text below U+00A0 is returned as is.
    assert_eq!(normalize(&[0x41, 0x9F], Form::Nfkc), [0x41, 0x9F]);
}

#[test]
fn code_points_keep_lone_surrogates_and_properties_classify_text() {
    let text = [0xD83D, 0xDE00, 0xD800, 0x41];
    let points = code_points(&text);
    assert_eq!(points, [0x1F600, 0xD800, 0x41]);
    assert_eq!(units(&points), text);
    assert!(case_ignorable(0x27));
    assert!(case_ignorable(0x301));
    assert!(!case_ignorable(0x41));
    assert!(punctuation(0x21));
    assert!(punctuation(0x20));
    assert!(!punctuation(0x61));
    assert_eq!(combining_class(0x301), 230);
    assert_eq!(combining_class(0x41), 0);
}
