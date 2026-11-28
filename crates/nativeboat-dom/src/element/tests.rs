use super::*;
use crate::tests::page;

fn by_id(dom: &Dom, id: &str) -> NodeId {
    dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
}

#[test]
fn namespaces() {
    for namespace in [Namespace::Html, Namespace::Svg, Namespace::MathMl] {
        assert_eq!(Namespace::from_uri(namespace.uri()), namespace);
    }
    assert_eq!(Namespace::from_uri(""), Namespace::Html);
}

#[test]
fn inline_styles() {
    let mut style = InlineStyle::parse("color: red; margin: 1px 2px !important; --Brand: Blue");
    assert_eq!(style.len(), 3);
    assert!(!style.is_empty());
    assert_eq!(style.item(1), Some("margin"));
    assert_eq!(style.item(9), None);
    assert_eq!(style.get("color"), "red");
    assert_eq!(style.get("margin-left"), "2px");
    assert_eq!(style.get("--Brand"), "Blue");
    assert_eq!(style.get("padding"), "");
    assert_eq!(style.priority("margin"), "important");
    assert_eq!(style.priority("color"), "");
    assert_eq!(
        style.css_text(),
        "color: red; margin: 1px 2px !important; --Brand: Blue;"
    );
    style.set("MARGIN-TOP", "5px", false);
    style.set("margin", "3px", false);
    assert_eq!(style.get("margin-top"), "3px");
    assert_eq!(style.item(2), Some("margin"));
    style.set("color", "", false);
    assert_eq!(style.get("color"), "");
    assert_eq!(style.remove("margin"), "3px");
    assert_eq!(style.remove("nothing"), "");
    let declarations = style.declarations();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "--Brand");
    let border = InlineStyle::parse("border: 1px solid rgb(1, 2, 3); grid-area: 1 / 2");
    assert_eq!(border.get("border-top-color"), "rgb(1, 2, 3)");
    assert_eq!(border.get("grid-column-start"), "2");
    assert_eq!(border.get("border-width"), "1px");
    assert_eq!(border.get("border-color"), "rgb(1, 2, 3)");
    assert_eq!(border.get("border"), "1px solid rgb(1, 2, 3)");
    let sides =
        InlineStyle::parse("border: 1px solid red; border-left-width: 2px; margin-top: 1px");
    assert_eq!(sides.get("border-width"), "1px 1px 1px 2px");
    assert_eq!(sides.get("border"), "");
    assert_eq!(sides.get("margin"), "");
    let margin = InlineStyle::parse("margin: 1px 2px");
    assert_eq!(margin.get("margin"), "1px 2px");
    let longhands = InlineStyle::parse(
        "padding-top: 1px; padding-right: 2px; padding-bottom: 1px; padding-left: 2px",
    );
    assert_eq!(longhands.get("padding"), "1px 2px");
    let wrapping = InlineStyle::parse("white-space: pre; text-wrap-mode: wrap");
    assert_eq!(wrapping.get("white-space-collapse"), "preserve");
    assert_eq!(wrapping.get("white-space"), "pre-wrap");
    let wrapping =
        InlineStyle::parse("white-space-collapse: preserve-breaks; text-wrap-mode: nowrap");
    assert_eq!(wrapping.get("white-space"), "preserve-breaks nowrap");
    let wrapping = InlineStyle::parse("text-wrap-mode: nowrap; text-wrap-style: auto");
    assert_eq!(wrapping.get("text-wrap"), "nowrap");
    let wrapping = InlineStyle::parse("text-wrap-mode: wrap; text-wrap-style: balance");
    assert_eq!(wrapping.get("text-wrap"), "balance");
    assert!(InlineStyle::default().is_empty());
    use nativeboat_css::values::parse_component_values;
    assert_eq!(
        render(&parse_component_values(" (a) [b] f(c) ")),
        "(a) [b] f(c)"
    );
}

#[test]
fn attributes() {
    let mut dom = Dom::new();
    let div = dom.create_element("div").unwrap();
    let svg = dom.create_element_ns(Namespace::Svg, "svg").unwrap();
    dom.set_attribute(div, "Data-X", "1").unwrap();
    assert_eq!(dom.get_attribute(div, "data-x"), Some("1"));
    assert_eq!(dom.get_attribute(div, "DATA-X"), Some("1"));
    dom.set_attribute(div, "data-x", "2").unwrap();
    assert_eq!(
        dom.attributes(div),
        vec![("data-x".to_owned(), "2".to_owned())]
    );
    dom.set_attribute(svg, "viewBox", "0 0 1 1").unwrap();
    assert_eq!(dom.get_attribute(svg, "viewBox"), Some("0 0 1 1"));
    assert_eq!(dom.get_attribute(svg, "viewbox"), None);
    for invalid in ["", "a b", "a\"", "a=b"] {
        assert!(matches!(
            dom.set_attribute(div, invalid, "x"),
            Err(DomError::InvalidCharacter(_))
        ));
    }
    let text = dom.create_text("t");
    assert_eq!(dom.set_attribute(text, "a", "b"), Ok(()));
    assert_eq!(dom.get_attribute(text, "a"), None);
    assert!(dom.attributes(text).is_empty());
    dom.remove_attribute(div, "data-x");
    assert!(!dom.has_attribute(div, "data-x"));
    dom.remove_attribute(div, "data-x");
    dom.remove_attribute(text, "a");
    assert_eq!(dom.toggle_attribute(div, "hidden", None), Ok(true));
    assert!(dom.has_attribute(div, "hidden"));
    assert_eq!(dom.toggle_attribute(div, "hidden", Some(true)), Ok(true));
    assert_eq!(dom.toggle_attribute(div, "hidden", None), Ok(false));
    assert_eq!(dom.toggle_attribute(div, "hidden", Some(false)), Ok(false));
    assert!(dom.toggle_attribute(div, "a b", None).is_err());
    dom.set_attribute(div, "id", "main").unwrap();
    assert_eq!(dom.element_id(div), Some("main"));
    assert_eq!(dom.namespace(svg), Some(Namespace::Svg));
    assert_eq!(dom.namespace(text), None);
}

#[test]
fn style_attribute_stays_in_sync() {
    let mut dom = Dom::new();
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "style", "color: red").unwrap();
    assert_eq!(dom.inline_style(div).get("color"), "red");
    dom.update_inline_style(div, &mut |style| style.set("width", "1px", false));
    assert_eq!(
        dom.get_attribute(div, "style"),
        Some("color: red; width: 1px;")
    );
    let version = dom.version();
    dom.update_inline_style(div, &mut |style| style.set("width", "1px", false));
    assert_eq!(dom.version(), version);
    dom.remove_attribute(div, "style");
    assert!(dom.inline_style(div).is_empty());
    let text = dom.create_text("t");
    dom.update_inline_style(text, &mut |style| style.set("color", "red", false));
    assert!(dom.inline_style(text).is_empty());
}

#[test]
fn class_lists() {
    let mut dom = Dom::new();
    let div = dom.create_element("div").unwrap();
    assert!(dom.class_list(div).is_empty());
    dom.class_remove(div, &["x"]).unwrap();
    assert!(!dom.has_attribute(div, "class"));
    dom.set_attribute(div, "class", " a  b a ").unwrap();
    assert_eq!(dom.class_list(div), vec!["a", "b"]);
    assert!(dom.has_class(div, "b"));
    assert!(!dom.has_class(div, "c"));
    dom.class_add(div, &["c", "a"]).unwrap();
    assert_eq!(dom.get_attribute(div, "class"), Some("a b c"));
    dom.class_remove(div, &["a"]).unwrap();
    assert_eq!(dom.class_list(div), vec!["b", "c"]);
    assert_eq!(dom.class_toggle(div, "d", None), Ok(true));
    assert_eq!(dom.class_toggle(div, "d", None), Ok(false));
    assert_eq!(dom.class_toggle(div, "b", Some(true)), Ok(true));
    assert_eq!(dom.class_toggle(div, "z", Some(false)), Ok(false));
    assert_eq!(dom.class_replace(div, "b", "c"), Ok(true));
    assert_eq!(dom.class_list(div), vec!["c"]);
    assert_eq!(dom.class_replace(div, "missing", "x"), Ok(false));
    assert!(matches!(
        dom.class_add(div, &[""]),
        Err(DomError::Syntax(_))
    ));
    assert!(matches!(
        dom.class_add(div, &["a b"]),
        Err(DomError::InvalidCharacter(_))
    ));
    assert!(dom.class_remove(div, &[""]).is_err());
    assert!(dom.class_toggle(div, "", None).is_err());
    assert!(dom.class_replace(div, "", "x").is_err());
    assert!(dom.class_replace(div, "c", "").is_err());
}

#[test]
fn scrolling() {
    let mut dom = Dom::new();
    let div = dom.create_element("div").unwrap();
    let text = dom.create_text("t");
    let version = dom.version();
    dom.set_scroll(div, 0.0, 0.0);
    assert_eq!(dom.version(), version);
    dom.set_scroll(div, 1.0, 2.0);
    assert_eq!(dom.scroll(div), (1.0, 2.0));
    dom.set_scroll(text, 1.0, 2.0);
    assert_eq!(dom.scroll(text), (0.0, 0.0));
}

#[test]
fn text_values() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<input id=t value=initial><textarea id=a>default text</textarea><button id=b value=v></button><div id=d></div>",
    );
    let input = by_id(&dom, "t");
    assert_eq!(dom.value(input), "initial");
    assert!(!dom.is_value_dirty(input));
    dom.set_value(input, "typed");
    assert_eq!(dom.value(input), "typed");
    assert!(dom.is_value_dirty(input));
    let version = dom.version();
    dom.set_value(input, "typed");
    assert_eq!(dom.version(), version);
    assert_eq!(dom.get_attribute(input, "value"), Some("initial"));
    let area = by_id(&dom, "a");
    assert_eq!(dom.value(area), "default text");
    dom.set_value(area, "new");
    assert_eq!(dom.value(area), "new");
    assert_eq!(dom.value(by_id(&dom, "b")), "v");
    assert_eq!(dom.value(by_id(&dom, "d")), "");
    let text = dom.create_text("t");
    assert_eq!(dom.value(text), "");
    dom.set_value(text, "x");
    assert!(!dom.is_value_dirty(text));
}

#[test]
fn checkboxes_and_radio_groups() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<input id=c type=checkbox checked><input id=u type=CHECKBOX>\
         <form><input id=r1 type=radio name=g checked><input id=r2 type=radio name=g><input id=x type=radio name=other></form>\
         <input id=r3 type=radio name=g checked><input id=n type=radio><input id=t value=1>",
    );
    let checkbox = by_id(&dom, "c");
    assert!(dom.is_checkable(checkbox));
    assert!(!dom.is_checkable(by_id(&dom, "t")));
    assert!(!dom.is_checkable(body));
    assert!(dom.checked(checkbox));
    assert!(!dom.checked(by_id(&dom, "u")));
    assert_eq!(dom.value(checkbox), "on");
    dom.set_value(checkbox, "yes");
    assert_eq!(dom.value(checkbox), "yes");
    dom.set_checked(checkbox, false);
    assert!(!dom.checked(checkbox));
    assert!(dom.has_attribute(checkbox, "checked"));
    dom.set_checked(by_id(&dom, "r2"), true);
    assert!(!dom.checked(by_id(&dom, "r1")));
    assert!(dom.checked(by_id(&dom, "r2")));
    assert!(dom.checked(by_id(&dom, "r3")));
    dom.set_checked(by_id(&dom, "r3"), true);
    assert!(dom.checked(by_id(&dom, "r2")));
    assert_eq!(dom.form_owner(by_id(&dom, "r3")), None);
    assert!(dom.form_owner(by_id(&dom, "r1")).is_some());
    let form = dom.form_owner(by_id(&dom, "r1")).unwrap();
    dom.set_attribute(form, "id", "owner").unwrap();
    let outside = by_id(&dom, "r3");
    dom.set_attribute(outside, "form", "owner").unwrap();
    assert_eq!(dom.form_owner(outside), Some(form));
    dom.set_attribute(outside, "form", "c").unwrap();
    assert_eq!(dom.form_owner(outside), None);
    dom.remove_attribute(outside, "form");
    dom.set_checked(by_id(&dom, "n"), true);
    assert!(dom.checked(by_id(&dom, "n")));
    let text = dom.create_text("t");
    dom.set_checked(text, true);
    assert!(!dom.checked(text));
    assert!(!dom.indeterminate(checkbox));
    dom.set_indeterminate(checkbox, true);
    assert!(dom.indeterminate(checkbox));
    dom.set_indeterminate(text, true);
    assert!(!dom.indeterminate(text));
}

#[test]
fn select_elements() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<select id=s><option id=o1 disabled>One</option><optgroup><option id=o2 value=two>  Two  items </option></optgroup><option id=o3>Three</option></select>\
         <select id=m multiple><option id=m1 selected>a</option><option id=m2 selected>b</option></select>\
         <select id=z size=3><option id=z1>a</option></select>\
         <select id=p><option id=p1 selected>a</option><option id=p2 selected>b</option></select>\
         <select id=e></select><option id=orphan selected>o</option>",
    );
    let select = by_id(&dom, "s");
    assert_eq!(dom.options(select).len(), 3);
    assert_eq!(dom.selected_options(select), vec![by_id(&dom, "o2")]);
    assert_eq!(dom.selected_index(select), 1);
    assert_eq!(dom.value(select), "two");
    assert_eq!(dom.value(by_id(&dom, "o3")), "Three");
    assert_eq!(dom.value(by_id(&dom, "o1")), "One");
    assert!(dom.option_selected(by_id(&dom, "o2")));
    dom.set_value(select, "Three");
    assert_eq!(dom.selected_index(select), 2);
    dom.set_value(select, "missing");
    assert_eq!(dom.selected_index(select), -1);
    assert_eq!(dom.value(select), "");
    dom.set_selected_index(select, 0);
    assert_eq!(dom.value(select), "One");
    dom.set_option_selected(by_id(&dom, "o3"), true);
    assert_eq!(dom.selected_options(select), vec![by_id(&dom, "o3")]);
    dom.set_value(by_id(&dom, "o3"), "3");
    assert_eq!(dom.value(select), "3");
    let multiple = by_id(&dom, "m");
    assert_eq!(dom.selected_options(multiple).len(), 2);
    dom.set_option_selected(by_id(&dom, "m1"), false);
    assert_eq!(dom.selected_options(multiple), vec![by_id(&dom, "m2")]);
    dom.set_option_selected(by_id(&dom, "m1"), true);
    assert_eq!(dom.selected_options(multiple).len(), 2);
    assert!(dom.selected_options(by_id(&dom, "z")).is_empty());
    assert_eq!(
        dom.selected_options(by_id(&dom, "p")),
        vec![by_id(&dom, "p2")]
    );
    assert_eq!(dom.selected_index(by_id(&dom, "e")), -1);
    let orphan = by_id(&dom, "orphan");
    assert!(dom.option_selected(orphan));
    dom.set_option_selected(orphan, false);
    assert!(!dom.option_selected(orphan));
}

#[test]
fn attribute_names_match_case_insensitively_only_on_html_elements() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=d data-Name=x></div><svg><rect id=r viewBox='0 0 1 1'/></svg>",
    ));
    let div = by_id(&dom, "d");
    assert_eq!(dom.get_attribute(div, "DATA-name"), Some("x"));
    assert_eq!(dom.get_attribute(div, "data-nam"), None);
    assert_eq!(dom.get_attribute(div, "data-namf"), None);
    let rect = by_id(&dom, "r");
    assert_eq!(dom.get_attribute(rect, "viewBox"), Some("0 0 1 1"));
    assert_eq!(dom.get_attribute(rect, "viewbox"), None);
}
