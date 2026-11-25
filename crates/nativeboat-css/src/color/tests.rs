use super::*;
use crate::values::parse_component_values;

fn parse(text: &str) -> Option<Color> {
    let values = without_whitespace(&parse_component_values(text));
    match parse_color(values.first()?, &SystemColors::LIGHT)? {
        SpecifiedColor::Color(color) => Some(color),
        SpecifiedColor::CurrentColor => None,
    }
}

fn close(actual: Color, expected: Color) -> bool {
    let near = |a: u8, b: u8| a.abs_diff(b) <= 1;
    near(actual.r, expected.r)
        && near(actual.g, expected.g)
        && near(actual.b, expected.b)
        && (actual.a - expected.a).abs() < 0.01
}

#[test]
fn keywords_and_names() {
    assert_eq!(parse("red"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(parse("RebeccaPurple"), Some(Color::rgb(102, 51, 153)));
    assert_eq!(parse("transparent"), Some(Color::TRANSPARENT));
    assert_eq!(parse("aliceblue"), Some(Color::rgb(240, 248, 255)));
    assert_eq!(parse("yellowgreen"), Some(Color::rgb(154, 205, 50)));
    assert_eq!(parse("notacolor"), None);
    let values = parse_component_values("currentColor");
    assert_eq!(
        parse_color(&values[0], &SystemColors::LIGHT),
        Some(SpecifiedColor::CurrentColor)
    );
    assert_eq!(parse("12px"), None);
    for window in NAMED_COLORS.windows(2) {
        assert!(window[0].0 < window[1].0, "{} not sorted", window[1].0);
    }
}

#[test]
fn system_colors() {
    let light = SystemColors::LIGHT;
    let names = [
        ("Canvas", light.canvas),
        ("window", light.canvas),
        ("background", light.canvas),
        ("CanvasText", light.canvas_text),
        ("windowtext", light.canvas_text),
        ("menutext", light.canvas_text),
        ("captiontext", light.canvas_text),
        ("infotext", light.canvas_text),
        ("LinkText", light.link_text),
        ("VisitedText", light.visited_text),
        ("ActiveText", light.active_text),
        ("ButtonFace", light.button_face),
        ("buttonhighlight", light.button_face),
        ("buttonshadow", light.button_face),
        ("threedface", light.button_face),
        ("menu", light.button_face),
        ("ButtonText", light.button_text),
        ("ButtonBorder", light.button_border),
        ("activeborder", light.button_border),
        ("inactiveborder", light.button_border),
        ("windowframe", light.button_border),
        ("Field", light.field),
        ("FieldText", light.field_text),
        ("Highlight", light.highlight),
        ("selecteditem", light.highlight),
        ("HighlightText", light.highlight_text),
        ("selecteditemtext", light.highlight_text),
        ("GrayText", light.gray_text),
        ("inactivecaptiontext", light.gray_text),
        ("Mark", light.mark),
        ("MarkText", light.mark_text),
        ("AccentColor", light.accent),
        ("AccentColorText", light.accent_text),
    ];
    for (name, expected) in names {
        assert_eq!(parse(name), Some(expected), "{name}");
    }
    assert_eq!(SystemColors::default(), SystemColors::LIGHT);
    assert_ne!(SystemColors::DARK.canvas, SystemColors::LIGHT.canvas);
}

#[test]
fn hex_colors() {
    assert_eq!(parse("#f00"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(parse("#f008"), Some(Color::rgba(255, 0, 0, 136.0 / 255.0)));
    assert_eq!(parse("#00ff00"), Some(Color::rgb(0, 255, 0)));
    assert_eq!(
        parse("#0000ff80"),
        Some(Color::rgba(0, 0, 255, 128.0 / 255.0))
    );
    assert_eq!(parse("#12345"), None);
    assert_eq!(parse("#ggg"), None);
}

#[test]
fn rgb_and_hsl() {
    assert_eq!(parse("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(
        parse("rgba(0, 0, 255, 0.5)"),
        Some(Color::rgba(0, 0, 255, 0.5))
    );
    assert_eq!(
        parse("rgb(100% 0% 0% / 50%)"),
        Some(Color::rgba(255, 0, 0, 0.5))
    );
    assert_eq!(parse("rgb(none 255 0)"), Some(Color::rgb(0, 255, 0)));
    assert_eq!(parse("rgb(300 -5 0)"), Some(Color::rgb(255, 0, 0)));
    assert!(close(
        parse("hsl(120, 100%, 50%)").unwrap(),
        Color::rgb(0, 255, 0)
    ));
    assert!(close(
        parse("hsl(240deg 100% 50%)").unwrap(),
        Color::rgb(0, 0, 255)
    ));
    assert!(close(
        parse("hsla(0.5turn, 100%, 25%, 1)").unwrap(),
        Color::rgb(0, 128, 128)
    ));
    assert!(close(
        parse("hsl(3.14159rad 100% 75%)").unwrap(),
        Color::rgb(128, 255, 255)
    ));
    assert!(close(
        parse("hsl(400grad 0% 50%)").unwrap(),
        Color::rgb(128, 128, 128)
    ));
    assert!(close(
        parse("hsl(300 100 50)").unwrap(),
        Color::rgb(255, 0, 255)
    ));
    assert!(close(
        parse("hsl(60 100% 50%)").unwrap(),
        Color::rgb(255, 255, 0)
    ));
    for invalid in [
        "rgb(1, 2)",
        "rgb(1 2 3 / 4 5)",
        "rgb(1, 2 3, 4)",
        "rgb(a, b, c)",
        "rgb(1, 2, 3, x)",
        "hsl(1foo 1% 1%)",
        "hsl(x 1% 1%)",
        "lab(1 2)",
        "rgb(1 2 3 /)",
    ] {
        assert_eq!(parse(invalid), None, "{invalid}");
    }
}

#[test]
fn hwb_colors() {
    assert!(close(parse("hwb(0 0% 0%)").unwrap(), Color::rgb(255, 0, 0)));
    assert!(close(
        parse("hwb(0 50% 50%)").unwrap(),
        Color::rgb(128, 128, 128)
    ));
    assert!(close(
        parse("hwb(120 20% 20%)").unwrap(),
        Color::rgb(51, 204, 51)
    ));
}

#[test]
fn oklab_and_oklch() {
    assert!(close(parse("oklab(1 0 0)").unwrap(), Color::WHITE));
    assert!(close(parse("oklab(0% 0% 0%)").unwrap(), Color::BLACK));
    assert!(close(
        parse("oklch(0.628 0.2577 29.23)").unwrap(),
        Color::rgb(255, 0, 0)
    ));
    assert!(close(
        parse("oklch(62.8% 64.4% 29.23deg / 0.5)").unwrap(),
        Color::rgba(255, 0, 0, 0.5)
    ));
    let [l, a, b] = srgb_to_oklab([1.0, 0.0, 0.0]);
    assert!((l - 0.628).abs() < 0.01 && (a - 0.2249).abs() < 0.01 && (b - 0.1258).abs() < 0.01);
    // CIE Lab and LCH, with Chromium's values for red.
    assert!(close(
        parse("lab(54.29 80.8 69.89)").unwrap(),
        Color::rgb(255, 0, 0)
    ));
    assert!(close(
        parse("lch(54.29% 71.2% 40.85deg / 50%)").unwrap(),
        Color::rgba(255, 0, 0, 0.5)
    ));
    assert!(close(parse("lab(100 0 0)").unwrap(), Color::WHITE));
    let [l, a, b] = srgb_to_lab([1.0, 0.0, 0.0]);
    assert!((l - 54.29).abs() < 0.05 && (a - 80.8).abs() < 0.1 && (b - 69.89).abs() < 0.1);
    assert!((linear_to_srgb(0.001) - 0.012_92).abs() < 1e-6);
    assert!((srgb_to_linear(0.01) - 0.01 / 12.92).abs() < 1e-6);
}

#[test]
fn color_mixing() {
    assert!(close(
        parse("color-mix(in srgb, red, blue)").unwrap(),
        Color::rgb(128, 0, 128)
    ));
    assert!(close(
        parse("color-mix(in srgb, red 25%, blue)").unwrap(),
        Color::rgb(64, 0, 191)
    ));
    assert!(close(
        parse("color-mix(in srgb, red, 25% blue)").unwrap(),
        Color::rgb(191, 0, 64)
    ));
    assert!(close(
        parse("color-mix(in srgb, red 20%, blue 20%)").unwrap(),
        Color::rgba(128, 0, 128, 0.4)
    ));
    assert!(close(
        parse("color-mix(in oklab, white, black)").unwrap(),
        Color::rgb(99, 99, 99)
    ));
    assert!(parse("color-mix(in oklch, white 50%, black)").is_some());
    // Alpha premultiplies the other components: a transparent color adds
    // only transparency, as Tailwind's `bg-primary/20` relies on.
    for space in [
        "srgb",
        "srgb-linear",
        "lab",
        "oklab",
        "xyz",
        "xyz-d50",
        "lch",
        "oklch",
        "hsl",
        "hwb",
    ] {
        let mixed = parse(&format!("color-mix(in {space}, blue 20%, transparent)")).unwrap();
        assert!(
            close(mixed, Color::rgba(0, 0, 255, 0.2)),
            "{space}: {mixed:?}"
        );
        let same = parse(&format!(
            "color-mix(in {space}, rgb(98, 160, 221), rgb(98, 160, 221))"
        ))
        .unwrap();
        assert!(close(same, Color::rgb(98, 160, 221)), "{space}: {same:?}");
    }
    // Hues take the shorter way unless told otherwise, and an achromatic
    // color's hue is the other color's.
    assert!(close(
        parse("color-mix(in hsl, red, blue)").unwrap(),
        Color::rgb(255, 0, 255)
    ));
    assert!(close(
        parse("color-mix(in hsl longer hue, red, blue)").unwrap(),
        Color::rgb(0, 255, 0)
    ));
    assert!(close(
        parse("color-mix(in hsl increasing hue, blue, red)").unwrap(),
        Color::rgb(255, 0, 255)
    ));
    assert!(close(
        parse("color-mix(in hsl decreasing hue, blue, red)").unwrap(),
        Color::rgb(0, 255, 0)
    ));
    assert!(close(
        parse("color-mix(in hsl, white, blue)").unwrap(),
        Color::rgb(159, 159, 223)
    ));
    for invalid in [
        "color-mix(in srgb, red)",
        "color-mix(srgb, red, blue)",
        "color-mix(in srgb longer hue, red, blue)",
        "color-mix(in hsl sideways hue, red, blue)",
        "color-mix(in display-p4, red, blue)",
        "color-mix(in srgb, red 0%, blue 0%)",
        "color-mix(in srgb, currentcolor, blue)",
        "color-mix(in srgb, red 1px, blue)",
        "color-mix(in srgb, nope, blue)",
        "color-mix(in 1, red, blue)",
    ] {
        assert_eq!(parse(invalid), None, "{invalid}");
    }
}

#[test]
fn serialization_and_helpers() {
    assert_eq!(Color::rgb(1, 2, 3).to_css(), "rgb(1, 2, 3)");
    assert_eq!(Color::rgba(1, 2, 3, 0.5).to_css(), "rgba(1, 2, 3, 0.5)");
    assert!(Color::TRANSPARENT.is_transparent());
    assert!(!Color::BLACK.is_transparent());
    assert_eq!(hex_color("fff"), Some(Color::WHITE));
    assert_eq!(named_color("nope"), None);
}
