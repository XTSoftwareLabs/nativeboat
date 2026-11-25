use super::*;

#[test]
fn element_helpers() {
    let document = parse_document("<div id=a>x<span>y</span><!--z--><b>w</b></div>");
    let div = document.body().child_elements().next().unwrap();
    assert_eq!(div.attribute("id"), Some("a"));
    assert_eq!(div.attribute("missing"), None);
    assert_eq!(div.text_content(), "xyw");
    let names: Vec<&str> = div
        .child_elements()
        .map(|child| child.name.as_str())
        .collect();
    assert_eq!(names, vec!["span", "b"]);
    assert_eq!(Node::Comment(String::new()).as_element(), None);
}

#[test]
fn document_helpers() {
    let document = parse_document("<title>One</title><title>Two</title><p>x");
    assert_eq!(document.title().as_deref(), Some("One"));
    let names: Vec<&str> = document
        .elements()
        .into_iter()
        .map(|element| element.name.as_str())
        .collect();
    assert_eq!(names, vec!["html", "head", "title", "title", "body", "p"]);
    assert_eq!(parse_document("").title(), None);
}

#[test]
fn element_categories() {
    assert!(is_void_element("img"));
    assert!(!is_void_element("div"));
    assert!(is_raw_text_element("script"));
    assert!(!is_raw_text_element("textarea"));
    assert!(is_escapable_raw_text_element("textarea"));
    assert!(!is_escapable_raw_text_element("p"));
}

#[test]
fn entity_decoding() {
    assert!(matches!(
        decode_entities("plain"),
        std::borrow::Cow::Borrowed("plain")
    ));
    assert_eq!(decode_entities("&lt;&#65;&#x42;&#X43;&nbsp;"), "<ABC\u{a0}");
    assert_eq!(decode_entities("&#0;&#xD800;"), "\u{fffd}\u{fffd}");
    assert_eq!(
        decode_entities("a & b &bogus; &#xZZ; &#; &amp"),
        "a & b &bogus; &#xZZ; &#; &amp"
    );
    assert_eq!(decode_entities("&copy;&euro;&hellip;"), "©€…");
}

#[test]
fn serialization_escapes() {
    assert_eq!(escape_text("a<b>&\u{a0}\""), "a&lt;b&gt;&amp;&nbsp;\"");
    assert_eq!(escape_attribute("a\"&<\u{a0}"), "a&quot;&amp;<&nbsp;");
    let nodes = parse_fragment("<p class='a\"b'>x &amp; y<br><!--c--></p><script>a<b</script>");
    let html: Vec<String> = nodes.iter().map(serialize).collect();
    assert_eq!(
        html,
        vec![
            "<p class=\"a&quot;b\">x &amp; y<br><!--c--></p>".to_owned(),
            "<script>a<b</script>".to_owned()
        ]
    );
}

#[test]
fn ir_round_trips_through_serde() {
    let document = parse_document("<p a=b>x</p>");
    let json = serde_json::to_string(&document).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, document);
}

#[test]
fn a_newline_after_pre_and_textarea_start_tags_is_dropped() {
    let nodes = parse_fragment(
        "<pre>\na\n</pre><textarea>\n\nb</textarea><listing>\nc</listing><div>\nd</div>",
    );
    let html: Vec<String> = nodes.iter().map(serialize::serialize).collect();
    assert_eq!(
        html,
        [
            "<pre>a\n</pre>",
            "<textarea>\nb</textarea>",
            "<listing>c</listing>",
            "<div>\nd</div>"
        ]
    );
    // A newline alone leaves no text.
    let nodes = parse_fragment("<pre>\n</pre>");
    assert_eq!(serialize::serialize(&nodes[0]), "<pre></pre>");
}

#[test]
fn raw_text_ends_survive_characters_after_a_closing_slash() {
    // Exported pages put CJK text in style and script elements.
    let nodes = parse_fragment(
        "<style>a::after{content:'</中文'}</style><script>x='</导出'</script><p>ok</p>",
    );
    let html: Vec<String> = nodes.iter().map(serialize::serialize).collect();
    assert_eq!(
        html,
        [
            "<style>a::after{content:'</中文'}</style>",
            "<script>x='</导出'</script>",
            "<p>ok</p>"
        ]
    );
}
