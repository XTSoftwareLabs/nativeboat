use super::*;
use crate::tests::page;
use nativeboat_css::selector::state;

#[test]
fn custom_element_names_need_a_hyphen_a_lowercase_start_and_no_reserved_name() {
    for name in ["x-a", "my-element", "a-", "x-\u{e9}l\u{e9}ment", "x-1.2_3"] {
        assert!(is_custom_element_name(name), "{name}");
    }
    for name in [
        "div",
        "-x",
        "1-x",
        "X-a",
        "x-A",
        "font-face",
        "missing-glyph",
        "x-a b",
        "",
    ] {
        assert!(!is_custom_element_name(name), "{name}");
    }
}

#[test]
fn custom_elements_are_undefined_until_their_definition_upgrades_them() {
    let (mut dom, body) = page();
    let custom = dom.create_element("x-widget").unwrap();
    dom.append_child(body, custom).unwrap();
    let svg = dom.create_element_ns(Namespace::Svg, "x-widget").unwrap();
    dom.append_child(body, svg).unwrap();
    assert!(dom.is_undefined_custom_element(custom));
    assert_ne!(dom.element_state(custom) & state::UNDEFINED, 0);
    assert!(!dom.is_undefined_custom_element(body));
    assert!(!dom.is_undefined_custom_element(svg));
    let version = dom.version();
    dom.set_custom_defined(custom);
    assert!(dom.version() > version);
    assert!(!dom.is_undefined_custom_element(custom));
    assert_eq!(dom.element_state(custom) & state::UNDEFINED, 0);
    // Built-in elements are always defined, and marking them changes
    // nothing.
    let version = dom.version();
    dom.set_custom_defined(body);
    assert_eq!(dom.version(), version);
    assert!(dom.matches(custom, ":defined").unwrap());
    assert!(dom.matches(body, ":defined").unwrap());
}

#[test]
fn candidates_are_custom_named_and_is_elements_of_a_subtree_in_tree_order() {
    let (mut dom, body) = page();
    let outer = dom.create_element("x-outer").unwrap();
    let plain = dom.create_element("p").unwrap();
    let button = dom.create_element("button").unwrap();
    dom.set_attribute(button, "is", "x-button").unwrap();
    let inner = dom.create_element("x-inner").unwrap();
    dom.append_child(body, outer).unwrap();
    dom.append_child(outer, plain).unwrap();
    dom.append_child(plain, button).unwrap();
    dom.append_child(outer, inner).unwrap();
    assert_eq!(dom.custom_element_candidates(body), [outer, button, inner]);
    assert_eq!(dom.custom_element_candidates(outer), [outer, button, inner]);
    assert_eq!(dom.custom_element_candidates(plain), [button]);
}
