use super::*;
use crate::tokenizer::tokens;

fn query(text: &str) -> Option<ContainerQuery> {
    parse_container_query(&tokens(text))
}

fn holds(text: &str, width: Option<f32>, height: Option<f32>) -> bool {
    query(text)
        .unwrap_or_else(|| panic!("{text} did not parse"))
        .matches(&QueryContainer { width, height })
}

#[test]
fn preludes_name_their_containers() {
    assert_eq!(query("(width > 1px)").unwrap().name, None);
    assert_eq!(
        query("sidebar (max-width: 240px)").unwrap().name.as_deref(),
        Some("sidebar")
    );
    assert_eq!(query("not (width > 1px)").unwrap().name, None);
    for invalid in [
        "",
        "sidebar",
        "(width > 1px) and",
        "(a) and (b) or (c)",
        "a b (width)",
    ] {
        assert_eq!(query(invalid), None, "{invalid}");
    }
}

#[test]
fn size_features_test_the_container() {
    let inline = (Some(300.0), None);
    assert!(holds("(max-width: 300px)", inline.0, inline.1));
    assert!(!holds("(max-width: 299px)", inline.0, inline.1));
    assert!(holds("(min-inline-size: 18em)", inline.0, inline.1));
    assert!(holds("(200px < width <= 300px)", inline.0, inline.1));
    assert!(!holds("(200px < width < 300px)", inline.0, inline.1));
    // An inline-size container has no height to query.
    assert!(!holds("(height > 0px)", inline.0, inline.1));
    assert!(!holds("not (height > 0px)", inline.0, inline.1));
    let size = (Some(300.0), Some(100.0));
    assert!(holds("(height: 100px)", size.0, size.1));
    assert!(holds("(block-size < 200px)", size.0, size.1));
    assert!(holds("(orientation: landscape)", size.0, size.1));
    assert!(holds("(aspect-ratio > 2)", size.0, size.1));
    assert!(holds("(aspect-ratio: 3/1)", size.0, size.1));
    // A container laid out at no size yet satisfies nothing.
    assert!(!holds("(width >= 0px)", None, None));
}

#[test]
fn conditions_combine_and_unknown_tests_never_hold() {
    let (width, height) = (Some(500.0), Some(100.0));
    assert!(holds("(width > 400px) and (height < 200px)", width, height));
    assert!(!holds(
        "(width > 600px) and (height < 200px)",
        width,
        height
    ));
    assert!(holds("(width > 600px) or (height < 200px)", width, height));
    assert!(holds("not (width > 600px)", width, height));
    assert!(holds("((width > 600px) or (height: 100px))", width, height));
    assert!(holds(
        "(not (width > 600px)) and (width > 1px)",
        width,
        height
    ));
    // Style queries are not evaluated: unknown, so neither they nor their
    // negation hold, while `or` with a true test does.
    assert!(!holds("style(--compact: true)", width, height));
    assert!(!holds("not style(--compact: true)", width, height));
    assert!(holds(
        "style(--compact: true) or (width > 1px)",
        width,
        height
    ));
    assert!(!holds("(unknown-feature: 1)", width, height));
}
