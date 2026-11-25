use super::*;
use crate::tokenizer::tokens;

fn list(text: &str) -> MediaQueryList {
    parse_media_query_list(&tokens(text))
}

fn matches(text: &str, environment: &MediaEnvironment) -> bool {
    list(text).matches(environment)
}

#[test]
fn media_types() {
    let environment = MediaEnvironment::default();
    assert!(matches("", &environment));
    assert!(matches("all", &environment));
    assert!(matches("screen", &environment));
    assert!(matches("only screen", &environment));
    assert!(!matches("print", &environment));
    assert!(matches("not print", &environment));
    assert!(!matches("not screen", &environment));
    assert!(matches("print, screen", &environment));
    assert!(matches(" , screen", &environment));
}

#[test]
fn invalid_queries_never_match() {
    let environment = MediaEnvironment::default();
    for text in [
        "screen print",
        "screen and print",
        "(width",
        "screen (width: 1px)",
        "and",
        "garbage and (",
        "(width: 10foo)",
        "(width >)",
        "(width > 1px 2px)",
        "(1px <)",
        "(1px < 2px)",
        "(1px < width <= 2px 3px)",
        "(1px < width ! 2px)",
        "(: 1px)",
        "()",
        "(width: [)",
        "only not screen",
    ] {
        assert!(!matches(text, &environment), "{text:?}");
    }
}

#[test]
fn width_and_height() {
    let environment = MediaEnvironment {
        width: 800.0,
        height: 600.0,
        ..MediaEnvironment::default()
    };
    assert!(matches("(min-width: 600px)", &environment));
    assert!(!matches("(max-width: 600px)", &environment));
    assert!(matches(
        "screen and (min-width: 50em) and (max-height: 600px)",
        &environment
    ));
    assert!(matches("(width: 800px)", &environment));
    assert!(matches("(width >= 800px)", &environment));
    assert!(!matches("(width > 800px)", &environment));
    assert!(matches("(width < 801px)", &environment));
    assert!(matches("(height <= 600px)", &environment));
    assert!(matches("(width = 800px)", &environment));
    assert!(matches("(400px <= width <= 800px)", &environment));
    assert!(!matches("(400px < width < 800px)", &environment));
    assert!(matches("(900px > width)", &environment));
    assert!(matches("(800px >= width)", &environment));
    assert!(matches("(800px = width)", &environment));
    assert!(matches("(width)", &environment));
    assert!(!matches("(monochrome)", &environment));
    assert!(matches("(color)", &environment));
    assert!(!matches("(width: wide)", &environment));
}

#[test]
fn length_units() {
    let environment = MediaEnvironment {
        width: 96.0,
        ..MediaEnvironment::default()
    };
    for text in [
        "(width: 96px)",
        "(width: 6em)",
        "(width: 6rem)",
        "(width: 72pt)",
        "(width: 6pc)",
        "(width: 1in)",
        "(width: 2.54cm)",
        "(width: 25.4mm)",
    ] {
        assert!(matches(text, &environment), "{text}");
    }
}

#[test]
fn resolution_and_ratios() {
    let environment = MediaEnvironment {
        width: 1600.0,
        height: 900.0,
        resolution: 2.0,
        ..MediaEnvironment::default()
    };
    assert!(matches("(aspect-ratio: 16/9)", &environment));
    assert!(matches("(min-aspect-ratio: 4/3)", &environment));
    assert!(matches("(min-resolution: 2dppx)", &environment));
    assert!(matches("(resolution: 2x)", &environment));
    assert!(matches("(min-resolution: 192dpi)", &environment));
    assert!(matches("(max-resolution: 76dpcm)", &environment));
    assert!(matches("(resolution > 1)", &environment));
}

#[test]
fn keyword_features() {
    let light = MediaEnvironment::default();
    let dark = MediaEnvironment {
        dark: true,
        reduced_motion: true,
        high_contrast: true,
        width: 300.0,
        height: 600.0,
        ..MediaEnvironment::default()
    };
    assert!(matches("(prefers-color-scheme: light)", &light));
    assert!(matches("(prefers-color-scheme: dark)", &dark));
    assert!(matches("(prefers-reduced-motion: no-preference)", &light));
    assert!(matches("(prefers-reduced-motion: reduce)", &dark));
    assert!(!matches("(prefers-reduced-motion)", &light));
    assert!(matches("(prefers-reduced-motion)", &dark));
    assert!(matches("(prefers-contrast: no-preference)", &light));
    assert!(matches("(prefers-contrast: more)", &dark));
    assert!(matches("(forced-colors: none)", &light));
    assert!(!matches("(forced-colors)", &light));
    assert!(matches("(forced-colors: active)", &dark));
    assert!(matches("(orientation: landscape)", &light));
    assert!(matches("(orientation: portrait)", &dark));
    assert!(matches("(hover: hover)", &light));
    assert!(matches("(any-hover)", &light));
    assert!(matches("(pointer: fine)", &light));
    assert!(matches("(any-pointer: fine)", &light));
    assert!(matches("(display-mode: standalone)", &light));
    assert!(!matches("(hover: 1)", &light));
    assert!(!matches("(scripting: enabled)", &light));
}

#[test]
fn comparisons_flip_and_test() {
    let pairs = [
        (Comparison::Less, Comparison::Greater),
        (Comparison::LessOrEqual, Comparison::GreaterOrEqual),
        (Comparison::Greater, Comparison::Less),
        (Comparison::GreaterOrEqual, Comparison::LessOrEqual),
        (Comparison::Equal, Comparison::Equal),
    ];
    for (comparison, flipped) in pairs {
        assert_eq!(comparison.flip(), flipped);
    }
    assert!(Comparison::Equal.test(1.0, 1.0));
    assert!(Comparison::Less.test(1.0, 2.0));
    assert!(Comparison::LessOrEqual.test(2.0, 2.0));
    assert!(Comparison::Greater.test(3.0, 2.0));
    assert!(Comparison::GreaterOrEqual.test(2.0, 2.0));
}

#[test]
fn queries_serialize() {
    let parsed = list("not screen and (max-width: 10px), (hover)");
    let json = serde_json::to_string(&parsed).unwrap();
    assert_eq!(
        serde_json::from_str::<MediaQueryList>(&json).unwrap(),
        parsed
    );
}
