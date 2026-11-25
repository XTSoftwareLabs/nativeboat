use super::*;

#[test]
fn only_the_first_direct_summary_is_a_disclosure_button() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<details><p id=before>before</p><summary id=first>first</summary><summary id=second>second</summary><div><summary id=nested>nested</summary></div></details><summary id=alone>alone</summary>",
    ));
    let by_id = |dom: &Dom, id: &str| dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    assert!(dom.is_disclosure_summary(by_id(&dom, "first")));
    assert!(dom.is_focusable(by_id(&dom, "first")));
    for id in ["before", "second", "nested", "alone"] {
        assert!(!dom.is_disclosure_summary(by_id(&dom, id)));
    }
    assert!(!dom.is_disclosure_summary(Dom::DOCUMENT));
    let detached = dom.create_element("summary").unwrap();
    assert!(!dom.is_disclosure_summary(detached));
}

fn element(dom: &mut Dom, name: &str) -> NodeId {
    dom.create_element(name).unwrap()
}

/// `<html><head/><body/></html>` attached to the document.
pub(crate) fn page() -> (Dom, NodeId) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(""));
    let body = dom.html_child("body").unwrap();
    (dom, body)
}

#[test]
fn errors_have_names_and_messages() {
    let cases = [
        (
            DomError::HierarchyRequest("a".into()),
            "HierarchyRequestError",
        ),
        (DomError::NotFound("a".into()), "NotFoundError"),
        (
            DomError::InvalidCharacter("a".into()),
            "InvalidCharacterError",
        ),
        (DomError::Syntax("a".into()), "SyntaxError"),
        (DomError::InvalidState("a".into()), "InvalidStateError"),
        (DomError::IndexSize("a".into()), "IndexSizeError"),
    ];
    for (error, name) in cases {
        assert_eq!(error.name(), name);
        assert_eq!(error.message(), "a");
        assert_eq!(error.to_string(), format!("{name}: a"));
    }
    assert_eq!(NodeId(7).index(), 7);
}

#[test]
fn node_kinds_types_and_names() {
    let mut dom = Dom::default();
    let div = element(&mut dom, "DIV");
    let svg = dom
        .create_element_ns(Namespace::Svg, "linearGradient")
        .unwrap();
    let text = dom.create_text("t");
    let comment = dom.create_comment("c");
    let fragment = dom.create_fragment();
    let target = dom.create_event_target();
    let cases = [
        (Dom::DOCUMENT, node_type::DOCUMENT, "#document"),
        (Dom::WINDOW, node_type::DOCUMENT, "#document"),
        (target, node_type::DOCUMENT, "#document"),
        (fragment, node_type::FRAGMENT, "#document-fragment"),
        (div, node_type::ELEMENT, "DIV"),
        (svg, node_type::ELEMENT, "linearGradient"),
        (text, node_type::TEXT, "#text"),
        (comment, node_type::COMMENT, "#comment"),
    ];
    for (node, kind, name) in cases {
        assert_eq!(dom.node_type(node), kind, "{name}");
        assert_eq!(dom.node_name(node), name);
    }
    assert_eq!(dom.local_name(div), Some("div"));
    assert_eq!(dom.local_name(text), None);
    assert!(dom.is_element(div));
    assert!(matches!(dom.kind(text), NodeKind::Text(_)));
    assert!(dom.exists(div));
    assert!(!dom.exists(NodeId(9999)));
    for invalid in ["", "1a", "a b", "a<b", "-x"] {
        assert!(
            matches!(
                dom.create_element(invalid),
                Err(DomError::InvalidCharacter(_))
            ),
            "{invalid}"
        );
    }
    assert!(dom.create_element("_x:y").is_ok());
}

#[test]
fn appending_inserting_and_removing() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, "div");
    let a = element(&mut dom, "a");
    let b = element(&mut dom, "b");
    let c = element(&mut dom, "c");
    let version = dom.version();
    dom.append_child(parent, a).unwrap();
    dom.append_child(parent, c).unwrap();
    dom.insert_before(parent, b, Some(c)).unwrap();
    assert!(dom.version() > version);
    assert_eq!(dom.children(parent), vec![a, b, c]);
    assert_eq!(dom.first_child(parent), Some(a));
    assert_eq!(dom.last_child(parent), Some(c));
    assert_eq!(dom.previous_sibling(b), Some(a));
    assert_eq!(dom.next_sibling(b), Some(c));
    assert_eq!(dom.parent(b), Some(parent));
    dom.insert_before(parent, c, Some(a)).unwrap();
    assert_eq!(dom.children(parent), vec![c, a, b]);
    dom.insert_before(parent, a, Some(a)).unwrap();
    assert_eq!(dom.children(parent), vec![c, a, b]);
    assert_eq!(dom.remove_child(parent, a), Ok(a));
    assert_eq!(dom.children(parent), vec![c, b]);
    assert!(matches!(
        dom.remove_child(parent, a),
        Err(DomError::NotFound(_))
    ));
    dom.remove(b);
    assert_eq!(dom.children(parent), vec![c]);
    dom.remove(b);
    dom.remove(c);
    assert_eq!(dom.first_child(parent), None);
    assert_eq!(dom.last_child(parent), None);
}

#[test]
fn replacing_children() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, "div");
    let a = element(&mut dom, "a");
    let b = element(&mut dom, "b");
    let c = element(&mut dom, "c");
    dom.append_child(parent, a).unwrap();
    dom.append_child(parent, b).unwrap();
    assert_eq!(dom.replace_child(parent, c, a), Ok(a));
    assert_eq!(dom.children(parent), vec![c, b]);
    assert_eq!(dom.replace_child(parent, b, c), Ok(c));
    assert_eq!(dom.children(parent), vec![b]);
    assert_eq!(dom.replace_child(parent, b, b), Ok(b));
    assert!(matches!(
        dom.replace_child(parent, c, a),
        Err(DomError::NotFound(_))
    ));
    dom.append_child(parent, a).unwrap();
    assert!(matches!(
        dom.replace_child(parent, parent, a),
        Err(DomError::HierarchyRequest(_))
    ));
    assert_eq!(dom.children(parent), vec![b, a]);
}

#[test]
fn fragments_move_their_children() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, "div");
    let fragment = dom.create_fragment();
    let a = element(&mut dom, "a");
    let b = element(&mut dom, "b");
    dom.append_child(fragment, a).unwrap();
    dom.append_child(fragment, b).unwrap();
    assert_eq!(dom.append_child(parent, fragment), Ok(fragment));
    assert_eq!(dom.children(parent), vec![a, b]);
    assert!(dom.children(fragment).is_empty());
}

#[test]
fn hierarchy_is_validated() {
    let mut dom = Dom::new();
    let div = element(&mut dom, "div");
    let span = element(&mut dom, "span");
    let other = element(&mut dom, "p");
    let text = dom.create_text("t");
    dom.append_child(div, span).unwrap();
    let hierarchy =
        |result: Result<NodeId, DomError>| matches!(result, Err(DomError::HierarchyRequest(_)));
    assert!(hierarchy(dom.append_child(text, div)));
    assert!(hierarchy(dom.append_child(span, div)));
    assert!(hierarchy(dom.append_child(div, div)));
    assert!(hierarchy(dom.append_child(div, Dom::DOCUMENT)));
    assert!(hierarchy(dom.append_child(div, Dom::WINDOW)));
    let target = dom.create_event_target();
    assert!(hierarchy(dom.append_child(div, target)));
    assert!(hierarchy(dom.append_child(Dom::DOCUMENT, text)));
    assert!(matches!(
        dom.insert_before(div, other, Some(other)),
        Err(DomError::NotFound(_))
    ));
    dom.append_child(Dom::DOCUMENT, div).unwrap();
    assert!(hierarchy(dom.append_child(Dom::DOCUMENT, other)));
    dom.append_child(Dom::DOCUMENT, div).unwrap();
    let comment = dom.create_comment("c");
    dom.append_child(Dom::DOCUMENT, comment).unwrap();
}

#[test]
fn traversal_helpers() {
    let (mut dom, body) = page();
    let a = element(&mut dom, "a");
    let text = dom.create_text("x");
    let b = element(&mut dom, "b");
    for node in [a, text, b] {
        dom.append_child(body, node).unwrap();
    }
    let html = dom.document_element().unwrap();
    assert_eq!(dom.local_name(html), Some("html"));
    assert_eq!(
        dom.html_child("head").map(|head| dom.local_name(head)),
        Some(Some("head"))
    );
    assert_eq!(dom.html_child("nope"), None);
    assert_eq!(dom.child_elements(body), vec![a, b]);
    assert_eq!(dom.next_element_sibling(a), Some(b));
    assert_eq!(dom.previous_element_sibling(b), Some(a));
    assert_eq!(dom.next_element_sibling(b), None);
    assert_eq!(dom.previous_element_sibling(a), None);
    assert_eq!(dom.parent_element(a), Some(body));
    assert_eq!(dom.parent_element(html), None);
    assert!(dom.contains(body, text));
    assert!(dom.contains(text, text));
    assert!(!dom.contains(a, body));
    assert!(dom.is_connected(a));
    let detached = element(&mut dom, "div");
    assert!(!dom.is_connected(detached));
    assert_eq!(dom.descendants(body), vec![a, text, b]);
    assert_eq!(Dom::new().document_element(), None);
    assert_eq!(Dom::new().html_child("body"), None);
}

#[test]
fn text_content_and_data() {
    let mut dom = Dom::new();
    let div = element(&mut dom, "div");
    let span = element(&mut dom, "span");
    let text = dom.create_text("a");
    let inner = dom.create_text("b");
    let comment = dom.create_comment("c");
    dom.append_child(div, text).unwrap();
    dom.append_child(div, span).unwrap();
    dom.append_child(span, inner).unwrap();
    dom.append_child(div, comment).unwrap();
    assert_eq!(dom.text_content(div).as_deref(), Some("ab"));
    assert_eq!(dom.text_content(comment).as_deref(), Some("c"));
    assert_eq!(dom.text_content(Dom::DOCUMENT), None);
    assert_eq!(dom.data(text), Some("a"));
    assert_eq!(dom.data(div), None);
    dom.set_data(text, "z");
    assert_eq!(dom.data(text), Some("z"));
    dom.set_data(div, "ignored");
    dom.set_text_content(comment, "cc");
    assert_eq!(dom.data(comment), Some("cc"));
    dom.set_text_content(div, "new");
    assert_eq!(dom.children(div).len(), 1);
    assert_eq!(dom.text_content(div).as_deref(), Some("new"));
    dom.set_text_content(div, "");
    assert!(dom.children(div).is_empty());
    let fragment = dom.create_fragment();
    dom.set_text_content(fragment, "f");
    assert_eq!(dom.text_content(fragment).as_deref(), Some("f"));
    dom.set_text_content(Dom::DOCUMENT, "ignored");
    assert!(dom.children(Dom::DOCUMENT).is_empty());
}

#[test]
fn cloning() {
    let mut dom = Dom::new();
    let div = element(&mut dom, "div");
    dom.set_attribute(div, "id", "a").unwrap();
    dom.set_scroll(div, 3.0, 4.0);
    let text = dom.create_text("x");
    dom.append_child(div, text).unwrap();
    let shallow = dom.clone_node(div, false);
    assert_eq!(dom.get_attribute(shallow, "id"), Some("a"));
    assert!(dom.children(shallow).is_empty());
    assert_eq!(dom.scroll(shallow), (0.0, 0.0));
    let deep = dom.clone_node(div, true);
    assert_eq!(dom.text_content(deep).as_deref(), Some("x"));
    let text_copy = dom.clone_node(text, false);
    assert_eq!(dom.data(text_copy), Some("x"));
}

#[test]
fn focus_rules() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<input id=i><input id=h type=hidden><button id=b disabled></button><button id=e></button>\
         <select id=s></select><textarea id=t></textarea><a id=l href=x></a><a id=n></a>\
         <div id=d tabindex=0></div><div id=p tabindex=x></div><div id=c contenteditable></div>\
         <div id=f contenteditable=false></div><iframe id=r></iframe>",
    );
    let by_id = |dom: &Dom, id: &str| dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    let focusable: Vec<(&str, bool)> = [
        "i", "h", "b", "e", "s", "t", "l", "n", "d", "p", "c", "f", "r",
    ]
    .into_iter()
    .map(|id| (id, dom.is_focusable(by_id(&dom, id))))
    .collect();
    assert_eq!(
        focusable,
        vec![
            ("i", true),
            ("h", false),
            ("b", false),
            ("e", true),
            ("s", true),
            ("t", true),
            ("l", true),
            ("n", false),
            ("d", true),
            ("p", false),
            ("c", true),
            ("f", false),
            ("r", true),
        ]
    );
    let text = dom.create_text("x");
    assert!(!dom.is_focusable(text));
    let input = by_id(&dom, "i");
    assert_eq!(dom.set_focus(Some(input), true), Some(None));
    assert_eq!(dom.focused(), Some(input));
    assert_eq!(dom.set_focus(Some(input), true), None);
    assert_eq!(dom.set_focus(Some(by_id(&dom, "n")), false), None);
    let detached = element(&mut dom, "button");
    assert_eq!(dom.set_focus(Some(detached), false), None);
    assert_eq!(dom.set_focus(None, false), Some(Some(input)));
    assert_eq!(dom.focused(), None);
}

#[test]
fn editing_hosts_are_the_outermost_editable_elements() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<div id=host contenteditable=TRUE><p id=p>a<span id=inner contenteditable>b</span></p>\
         <span id=odd contenteditable=odd>c</span><div id=fixed contenteditable=false>\
         <b id=nested contenteditable=plaintext-only>d</b></div></div><p id=plain>e</p>",
    );
    let by_id = |dom: &Dom, id: &str| dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
    let host = by_id(&dom, "host");
    let nested = by_id(&dom, "nested");
    let text = dom.first_child(by_id(&dom, "inner")).unwrap();
    let hosts: Vec<Option<NodeId>> = ["host", "p", "inner", "odd", "fixed", "nested", "plain"]
        .into_iter()
        .map(|id| dom.editing_host(by_id(&dom, id)))
        .collect();
    assert_eq!(
        hosts,
        [
            Some(host),
            Some(host),
            Some(host),
            Some(host),
            None,
            Some(nested),
            None
        ]
    );
    assert_eq!(dom.editing_host(text), Some(host));
    let focusable: Vec<bool> = ["host", "p", "inner", "nested"]
        .into_iter()
        .map(|id| dom.is_focusable(by_id(&dom, id)))
        .collect();
    assert_eq!(focusable, [true, false, false, true]);
}

#[test]
fn removing_nodes_clears_interaction_state() {
    let (mut dom, body) = page();
    let div = element(&mut dom, "div");
    let button = element(&mut dom, "button");
    dom.append_child(body, div).unwrap();
    dom.append_child(div, button).unwrap();
    dom.set_focus(Some(button), false);
    dom.set_hovered(Some(button));
    dom.set_active(Some(button));
    assert_eq!(dom.hovered(), Some(button));
    assert_eq!(dom.active(), Some(button));
    let version = dom.version();
    dom.set_hovered(Some(button));
    dom.set_active(Some(button));
    assert_eq!(dom.version(), version);
    dom.set_scrollbar_parts((Some((div, 1)), None));
    assert_eq!(dom.scrollbar_parts(), (Some((div, 1)), None));
    assert_eq!(dom.version(), version + 1);
    dom.set_scrollbar_parts((Some((div, 1)), None));
    assert_eq!(dom.version(), version + 1);
    dom.remove(div);
    assert_eq!(dom.focused(), None);
    assert_eq!(dom.hovered(), None);
    assert_eq!(dom.active(), None);
    dom.set_hovered(Some(body));
    dom.set_active(Some(body));
    dom.remove(div);
    assert_eq!(dom.hovered(), Some(body));
}

#[test]
fn canvases_remember_their_bitmaps() {
    let mut dom = Dom::new();
    let canvas = element(&mut dom, "canvas");
    assert_eq!(dom.canvas(canvas), None);
    let version = dom.version();
    dom.canvas_drawn(canvas, 7);
    dom.canvas_drawn(canvas, 7);
    assert_eq!(dom.canvas(canvas), Some((7, 2)));
    assert_eq!(dom.version(), version + 2);
}
