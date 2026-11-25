const SAMPLE: fn() -> &'static str = nativeboat_conceal::text!("sample.txt");

#[test]
fn text_reads_back_the_file() {
    assert_eq!(SAMPLE(), include_str!("sample.txt"));
    assert!(std::ptr::eq(SAMPLE(), SAMPLE()));
}

/// A concealed text's accessor.
type Text = fn() -> &'static str;

#[test]
fn texts_fit_in_constant_tables() {
    const TABLE: &[(&str, Text)] = &[("sample", nativeboat_conceal::text!("sample.txt"))];
    assert!(TABLE[0].1().starts_with("Text with é"));
}
