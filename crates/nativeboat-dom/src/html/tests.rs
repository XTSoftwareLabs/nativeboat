use super::*;
use crate::tests::page;

#[test]
fn parsed_detached_documents_share_the_arena_without_replacing_the_page() {
    let (mut dom, body) = page();
    dom.set_inner_html(body, "<p>keep</p>");
    let root = dom.document_element();
    let detached = dom.parse_html_document("<title>T</title><p id=p>A &amp; B</p>");
    assert_eq!(dom.node_type(detached), 9);
    assert_eq!(dom.document_element(), root);
    assert_eq!(dom.inner_html(body), "<p>keep</p>");
    let paragraph = dom.get_element_by_id(detached, "p").unwrap();
    assert_eq!(dom.text_content(paragraph).as_deref(), Some("A & B"));
    assert!(!dom.is_connected(paragraph));
    assert!(dom.contains(detached, paragraph));
    dom.insert_before(body, paragraph, None).unwrap();
    assert!(dom.is_connected(paragraph));
}

#[test]
fn documents_are_imported_with_namespaces() {
    let document = nativeboat_html::parse_document(
        "<!doctype html><title>T</title><p class=a>x<!--c--></p>\
         <svg viewBox='0 0 1 1'><foreignObject><div>html</div></foreignObject><circle/></svg><math><mi>x</mi></math>",
    );
    let dom = Dom::from_document(&document);
    let html = dom.document_element().unwrap();
    assert_eq!(dom.local_name(html), Some("html"));
    let svg = dom.get_elements_by_tag_name(Dom::DOCUMENT, "svg")[0];
    assert_eq!(dom.namespace(svg), Some(Namespace::Svg));
    assert_eq!(dom.get_attribute(svg, "viewBox"), Some("0 0 1 1"));
    let foreign = dom.get_elements_by_tag_name(Dom::DOCUMENT, "foreignObject")[0];
    // An SVG element whose children are HTML, as browsers parse it.
    assert_eq!(dom.namespace(foreign), Some(Namespace::Svg));
    let div = dom.get_elements_by_tag_name(Dom::DOCUMENT, "div")[0];
    assert_eq!(dom.namespace(div), Some(Namespace::Html));
    let circle = dom.get_elements_by_tag_name(Dom::DOCUMENT, "circle")[0];
    assert_eq!(dom.namespace(circle), Some(Namespace::Svg));
    let mi = dom.get_elements_by_tag_name(Dom::DOCUMENT, "mi")[0];
    assert_eq!(dom.namespace(mi), Some(Namespace::MathMl));
    let paragraph = dom.get_elements_by_tag_name(Dom::DOCUMENT, "p")[0];
    assert_eq!(dom.outer_html(paragraph), "<p class=\"a\">x<!--c--></p>");
}

#[test]
fn invalid_attribute_names_are_skipped() {
    let dom = Dom::from_document(&nativeboat_html::parse_document("<p a\"b=1 ok=2>x</p>"));
    let paragraph = dom.get_elements_by_tag_name(Dom::DOCUMENT, "p")[0];
    assert_eq!(
        dom.attributes(paragraph),
        vec![("ok".to_owned(), "2".to_owned())]
    );
}

#[test]
fn inner_html_round_trips() {
    let (mut dom, body) = page();
    dom.set_inner_html(body, "<b>bold</b> &amp; <i>it</i>");
    assert_eq!(dom.inner_html(body), "<b>bold</b> &amp; <i>it</i>");
    assert_eq!(dom.child_elements(body).len(), 2);
    dom.set_inner_html(body, "");
    assert!(dom.children(body).is_empty());
    let text = dom.create_text("t");
    dom.set_inner_html(text, "<b>x</b>");
    assert_eq!(dom.outer_html(text), "t");
    assert_eq!(
        dom.inner_html(Dom::DOCUMENT),
        "<html><head></head><body></body></html>"
    );
    let fragment = dom.create_fragment();
    let comment = dom.create_comment("c");
    dom.append_child(fragment, comment).unwrap();
    assert_eq!(dom.inner_html(fragment), "<!--c-->");
    let svg = dom.create_element_ns(Namespace::Svg, "svg").unwrap();
    let parsed = dom.parse_html_fragment("<path/><g></g>", svg);
    let children = dom.children(parsed);
    assert_eq!(children.len(), 2);
    assert_eq!(dom.namespace(children[0]), Some(Namespace::Svg));
    let orphan = dom.create_text("t");
    let parsed = dom.parse_html_fragment("<p></p>", orphan);
    assert_eq!(
        dom.namespace(dom.children(parsed)[0]),
        Some(Namespace::Html)
    );
}

#[test]
fn math_fragments_keep_the_namespace() {
    let mut dom = Dom::new();
    let math = dom.create_element_ns(Namespace::MathMl, "math").unwrap();
    let parsed = dom.parse_html_fragment("<mi/><mo></mo>", math);
    assert_eq!(dom.children(parsed).len(), 2);
}

#[test]
fn integration_points_parse_their_content_as_html() {
    let mut dom = Dom::new();
    let svg = dom.create_element_ns(Namespace::Svg, "svg").unwrap();
    let object = dom
        .create_element_ns(Namespace::Svg, "foreignObject")
        .unwrap();
    dom.append_child(svg, object).unwrap();
    dom.set_inner_html(object, "<div>hi</div>");
    let div = dom.children(object)[0];
    assert_eq!(dom.namespace(div), Some(Namespace::Html));
    dom.set_inner_html(svg, "<g></g>");
    assert_eq!(dom.namespace(dom.children(svg)[0]), Some(Namespace::Svg));
    let math = dom.create_element_ns(Namespace::MathMl, "math").unwrap();
    let mi = dom.create_element_ns(Namespace::MathMl, "mi").unwrap();
    dom.append_child(math, mi).unwrap();
    dom.set_inner_html(mi, "<b>x</b>");
    assert_eq!(dom.namespace(dom.children(mi)[0]), Some(Namespace::Html));
    // Imported documents follow the same rule.
    let parsed = Dom::from_document(&nativeboat_html::parse_document(
        "<svg><desc><p>d</p></desc></svg>",
    ));
    let p = parsed.get_elements_by_tag_name(Dom::DOCUMENT, "p")[0];
    assert_eq!(parsed.namespace(p), Some(Namespace::Html));
}
