use super::*;

#[test]
fn native_documents_follow_adoption_detachment_cloning_fragments_and_text() {
    let mut dom = Dom::new();
    assert_eq!(dom.owner_document(Dom::DOCUMENT), None);
    assert_eq!(dom.owner_document(Dom::WINDOW), None);
    let document = dom.parse_html_document("<p id=p>a<b>b</b><!--c--></p>");
    let document_fragment = dom.parse_html_fragment("<b>fragment</b>", document);
    assert_eq!(dom.owner_document(document_fragment), Some(document));
    assert_eq!(
        dom.owner_document(dom.first_child(document_fragment).unwrap()),
        Some(document)
    );
    let paragraph = dom.get_element_by_id(document, "p").unwrap();
    for node in dom.descendants(paragraph) {
        assert_eq!(dom.owner_document(node), Some(document));
    }
    dom.remove(paragraph);
    assert_eq!(dom.owner_document(paragraph), Some(document));
    let copy = dom.clone_node(paragraph, true);
    assert_eq!(dom.owner_document(copy), Some(document));
    let version = dom.version();
    dom.set_owner_document(copy, Dom::DOCUMENT).unwrap();
    dom.set_owner_document(copy, Dom::DOCUMENT).unwrap();
    assert_eq!(
        dom.version(),
        version,
        "ownership alone does not invalidate painting"
    );
    for node in dom.descendants(copy) {
        assert_eq!(dom.owner_document(node), Some(Dom::DOCUMENT));
    }
    assert_eq!(
        dom.owner_document(dom.first_child(paragraph).unwrap()),
        Some(document)
    );
    let fragment = dom.parse_html_fragment("<i>x</i><u>y</u>", paragraph);
    assert_eq!(dom.owner_document(fragment), Some(document));
    let child = dom.first_child(fragment).unwrap();
    dom.append_child(copy, fragment).unwrap();
    assert_eq!(dom.owner_document(fragment), Some(Dom::DOCUMENT));
    dom.remove(child);
    assert_eq!(dom.owner_document(child), Some(Dom::DOCUMENT));
    let empty = dom.create_fragment();
    dom.append_child(paragraph, empty).unwrap();
    assert_eq!(
        dom.owner_document(empty),
        Some(document),
        "empty fragments adopt too"
    );
    dom.set_text_content(paragraph, "abc");
    let text = dom.first_child(paragraph).unwrap();
    dom.remove(text);
    let tail = dom.split_text(text, 1).unwrap();
    assert_eq!(dom.owner_document(tail), Some(document));
    let cloned_document = dom.clone_node(document, true);
    assert_eq!(dom.owner_document(cloned_document), None);
    for node in dom.descendants(cloned_document) {
        assert_eq!(dom.owner_document(node), Some(cloned_document));
    }
    dom.set_owner_document(cloned_document, Dom::DOCUMENT)
        .unwrap();
    assert_eq!(dom.owner_document(cloned_document), None);
    for (node, owner) in [
        (copy, copy),
        (copy, NodeId(u32::MAX)),
        (NodeId(u32::MAX), document),
    ] {
        assert!(dom.set_owner_document(node, owner).is_err());
    }
}

#[test]
fn child_counts_follow_every_link_and_detach_path() {
    let mut dom = Dom::new();
    let parent = dom.create_element("div").unwrap();
    let text = dom.create_text("text");
    let first = dom.create_element("b").unwrap();
    let last = dom.create_element("i").unwrap();
    dom.append_child(parent, text).unwrap();
    dom.append_child(parent, first).unwrap();
    dom.append_child(parent, last).unwrap();
    let check = |dom: &Dom, node| {
        assert_eq!(dom.child_count(node) as usize, dom.children(node).len());
        assert_eq!(
            dom.element_child_count(node) as usize,
            dom.child_elements(node).len()
        );
    };
    check(&dom, parent);
    dom.insert_before(parent, last, Some(first)).unwrap();
    dom.insert_before(parent, first, Some(first)).unwrap();
    check(&dom, parent);
    dom.remove(text);
    dom.remove(text);
    check(&dom, parent);
    let fragment = dom.parse_html_fragment("a<!--comment--><u>child</u>", parent);
    check(&dom, fragment);
    dom.append_child(parent, fragment).unwrap();
    check(&dom, fragment);
    check(&dom, parent);
    let copy = dom.clone_node(parent, true);
    for node in dom.descendants(copy) {
        check(&dom, node);
    }
    check(&dom, copy);
    dom.set_inner_html(parent, "<span>replacement</span>");
    check(&dom, parent);
    dom.set_text_content(parent, "plain text");
    check(&dom, parent);
    dom.set_text_content(parent, "");
    check(&dom, parent);
    assert_eq!(
        (dom.child_count(parent), dom.element_child_count(parent)),
        (0, 0)
    );
}
