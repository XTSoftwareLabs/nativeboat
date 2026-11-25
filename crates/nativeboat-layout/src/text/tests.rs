use super::*;

fn split(text: &str, white_space: WhiteSpace) -> Vec<Piece> {
    let mut after_space = true;
    pieces(text, white_space, &mut after_space)
}

fn word(text: &str) -> Piece {
    Piece::Word(text.to_owned())
}

fn space(text: &str) -> Piece {
    Piece::Space(text.to_owned())
}

#[test]
fn fonts_follow_the_style() {
    let mut style = ComputedStyle::default();
    assert!(!font_of(&style).italic);
    style.font_style = FontStyle::Oblique;
    style.font_weight = nativeboat_css::types::FontWeight(700);
    let font = font_of(&style);
    assert!(font.italic);
    assert_eq!(font.weight, 700);
    assert_eq!(font.size, 16.0);
    assert_eq!(font.families, vec!["Times New Roman".to_owned()]);
}

#[test]
fn text_transforms() {
    assert_eq!(transform("Hello world", TextTransform::None), "Hello world");
    assert_eq!(transform("Hello", TextTransform::Uppercase), "HELLO");
    assert_eq!(transform("Hello", TextTransform::Lowercase), "hello");
    assert_eq!(
        transform("hello big  world-x", TextTransform::Capitalize),
        "Hello Big  World-x"
    );
}

#[test]
fn normal_white_space_collapses() {
    assert_eq!(
        split("  a \t\n b\r\u{c}c ", WhiteSpace::Normal),
        vec![
            word("a"),
            space(" "),
            word("b"),
            space(" "),
            word("c"),
            space(" ")
        ]
    );
    let mut after_space = false;
    assert_eq!(
        pieces(" x", WhiteSpace::NoWrap, &mut after_space),
        vec![space(" "), word("x")]
    );
    assert!(!after_space);
    assert_eq!(pieces(" y", WhiteSpace::NoWrap, &mut true), vec![word("y")]);
}

#[test]
fn preserved_white_space() {
    assert_eq!(
        split("a  b\tc\r\nd\n", WhiteSpace::Pre),
        vec![
            word("a"),
            space("  "),
            word("b"),
            // A tab stays one character, placed at its stop by line layout.
            space("\t"),
            word("c"),
            Piece::Break,
            word("d"),
            Piece::Break
        ]
    );
    assert_eq!(
        split("a  b\nc", WhiteSpace::PreLine),
        vec![word("a"), space(" "), word("b"), Piece::Break, word("c")]
    );
    assert_eq!(
        split(" \u{c}", WhiteSpace::PreWrap),
        vec![space(" "), word("\u{c}")]
    );
}

#[test]
fn half_leading_rounds_down_to_device_pixels() {
    struct Device(f32);
    impl TextMeasurer for Device {
        fn width(&self, _: &str, _: &Font) -> f32 {
            0.0
        }
        fn metrics(&self, _: &Font) -> FontMetrics {
            FontMetrics::default()
        }
        fn device_pixel(&self) -> f32 {
            self.0
        }
    }
    // Open Sans at 24px and 125%: a 41-pixel content area in a 42-pixel
    // line keeps no leading above; in a 60-pixel line, 9 pixels.
    let scaled = Device(0.8);
    assert_eq!(half_leading(&scaled, 33.6, 32.8), 0.0);
    assert!((half_leading(&scaled, 48.0, 32.8) - 7.2).abs() < 1e-4);
    // Negative leading rounds down too: -5.5 pixels is -6.
    assert!((half_leading(&scaled, 24.0, 32.8) + 4.8).abs() < 1e-4);
    // Without device pixels, leading is exact.
    assert!((half_leading(&Device(0.0), 33.6, 32.8) - 0.4).abs() < 1e-4);
}

#[test]
fn fonts_carry_the_opentype_features_asked_for() {
    let mut style = ComputedStyle::default();
    assert!(font_of(&style).features.is_empty());
    style.font_variant_numeric =
        nativeboat_css::types::FontVariantNumeric(vec!["tabular-nums".into()]);
    style.font_feature_settings =
        nativeboat_css::types::FontFeatureSettings(vec![(*b"tnum", 0), (*b"ss02", 1)]);
    // Settings win over the variant for the same feature.
    assert_eq!(
        font_of(&style).features,
        vec![
            (u32::from_le_bytes(*b"tnum"), 0),
            (u32::from_le_bytes(*b"ss02"), 1)
        ]
    );
    // `font-kerning: none` turns kerning off, unless the settings name it.
    let mut style = ComputedStyle {
        font_kerning: nativeboat_css::types::FontKerning::None,
        ..ComputedStyle::default()
    };
    assert_eq!(
        font_of(&style).features,
        vec![(u32::from_le_bytes(*b"kern"), 0)]
    );
    style.font_feature_settings = nativeboat_css::types::FontFeatureSettings(vec![(*b"kern", 1)]);
    assert_eq!(
        font_of(&style).features,
        vec![(u32::from_le_bytes(*b"kern"), 1)]
    );
}
