use super::*;
use crate::serialize_children;

fn body_html(source: &str) -> String {
    serialize_children(parse_document(source).body())
}

fn fragment_html(source: &str) -> String {
    let mut root = Element::new("div");
    root.children = parse_fragment(source);
    serialize_children(&root)
}

#[test]
fn carriage_returns_become_line_feeds() {
    assert_eq!(body_html("<p>a\r\nb\rc\n</p>\r\n"), "<p>a\nb\nc\n</p>\n");
    assert_eq!(fragment_html("x\r\ny"), "x\ny");
    let document = parse_document("<p>\r\n\r\n<b>x</b>");
    let Node::Element(paragraph) = &document.body().children[0] else {
        panic!("expected a paragraph");
    };
    let Node::Element(bold) = &paragraph.children[1] else {
        panic!("expected b");
    };
    assert_eq!(bold.position.line, 3);
}

#[test]
fn implicit_document_structure() {
    let document = parse_document("<title>T</title>\n<p>Hello");
    assert_eq!(document.doctype, None);
    assert_eq!(serialize_children(document.head()), "<title>T</title>");
    assert_eq!(serialize_children(document.body()), "<p>Hello</p>");
}

#[test]
fn explicit_document_structure_merges_attributes() {
    let document = parse_document(
        "<!doctype html><!-- c --><html lang=en><head class=h><meta charset=utf-8><!-- h -->\
         </head> <body class=b>x</body></html><!-- after --><html lang=fr dir=ltr><body id=b2>",
    );
    assert_eq!(document.doctype.as_deref(), Some("html"));
    assert_eq!(document.html.attribute("lang"), Some("en"));
    assert_eq!(document.html.attribute("dir"), Some("ltr"));
    assert_eq!(document.head().attribute("class"), Some("h"));
    assert_eq!(document.body().attribute("class"), Some("b"));
    assert_eq!(document.body().attribute("id"), Some("b2"));
    assert_eq!(document.head().position.line, 1);
    assert_eq!(
        serialize_children(document.head()),
        "<meta charset=\"utf-8\"><!-- h -->"
    );
    assert_eq!(serialize_children(document.body()), "x<!-- after -->");
}

#[test]
fn second_doctype_and_late_head_are_ignored() {
    let document = parse_document("<p>a</p><!doctype x><head><title>late</title>");
    assert_eq!(document.doctype, None);
    assert_eq!(serialize_children(document.head()), "");
    assert_eq!(
        serialize_children(document.body()),
        "<p>a</p><title>late</title>"
    );
}

#[test]
fn head_content_and_text_transitions() {
    let document = parse_document(
        "<head>\n<style>p{}</style>\n</head>\n<link rel=x><!-- c -->\n  text<script>1</script>",
    );
    assert_eq!(
        serialize_children(document.head()),
        "<style>p{}</style><link rel=\"x\"><!-- c -->"
    );
    assert_eq!(
        serialize_children(document.body()),
        "text<script>1</script>"
    );
    match &document.body().children[0] {
        Node::Text(text) => assert_eq!(text.position, crate::Position { line: 5, column: 3 }),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn end_tags_outside_the_body_are_ignored() {
    let document = parse_document("</p></div><head></span><title>a</title></x></head></y>b");
    assert_eq!(serialize_children(document.head()), "<title>a</title>");
    assert_eq!(serialize_children(document.body()), "b");
}

#[test]
fn body_start_tag_switches_modes() {
    let document = parse_document("<body><meta name=x>");
    assert_eq!(serialize_children(document.head()), "");
    assert_eq!(serialize_children(document.body()), "<meta name=\"x\">");
}

#[test]
fn paragraphs_close_implicitly() {
    assert_eq!(body_html("<p>a<div>b</div>"), "<p>a</p><div>b</div>");
    assert_eq!(body_html("<p>a<p>b"), "<p>a</p><p>b</p>");
    assert_eq!(
        body_html("<button><p>a<div>b</div></button>"),
        "<button><p>a</p><div>b</div></button>"
    );
    assert_eq!(
        body_html("<p><button><div>b</div></button>"),
        "<p><button><div>b</div></button></p>"
    );
    assert_eq!(body_html("a</p>b"), "a<p></p>b");
}

#[test]
fn lists_and_definition_lists() {
    assert_eq!(
        body_html("<ul><li>a<li>b<ul><li>c</ul><li>d</ul>"),
        "<ul><li>a</li><li>b<ul><li>c</li></ul></li><li>d</li></ul>"
    );
    assert_eq!(
        body_html("<dl><dt>a<dd>b<dt>c</dl>"),
        "<dl><dt>a</dt><dd>b</dd><dt>c</dt></dl>"
    );
}

#[test]
fn options_and_groups() {
    assert_eq!(
        body_html("<select><option>a<option>b<optgroup><option>c<optgroup>d</select>"),
        "<select><option>a</option><option>b</option><optgroup><option>c</option></optgroup><optgroup>d</optgroup></select>"
    );
}

#[test]
fn anchors_buttons_and_headings_do_not_nest() {
    assert_eq!(body_html("<a>1<a>2"), "<a>1</a><a>2</a>");
    assert_eq!(
        body_html("<button>1<button>2"),
        "<button>1</button><button>2</button>"
    );
    assert_eq!(body_html("<h1>a<h2>b"), "<h1>a</h1><h2>b</h2>");
}

#[test]
fn tables_get_implied_sections_and_rows() {
    assert_eq!(
        body_html("<table><tr><td>a<td>b<tr><th>c</table>"),
        "<table><tbody><tr><td>a</td><td>b</td></tr><tr><th>c</th></tr></tbody></table>"
    );
    assert_eq!(
        body_html("<table><td>x</table>"),
        "<table><tbody><tr><td>x</td></tr></tbody></table>"
    );
    assert_eq!(
        body_html("<table><thead><tr><th>h<tbody><tr><td>d<tfoot><td>f</table>"),
        "<table><thead><tr><th>h</th></tr></thead><tbody><tr><td>d</td></tr></tbody><tfoot><tr><td>f</td></tr></tfoot></table>"
    );
}

#[test]
fn void_elements_and_br_end_tags() {
    assert_eq!(
        body_html("<p>a<br>b</br>c<input>d"),
        "<p>a<br>b<br>c<input>d</p>"
    );
}

#[test]
fn unmatched_end_tags_are_ignored_and_open_elements_close() {
    assert_eq!(
        body_html("<div><span>a</em>b</div>c"),
        "<div><span>ab</span></div>c"
    );
    assert_eq!(
        body_html("<div><span>a</div>b"),
        "<div><span>a</span></div>b"
    );
}

#[test]
fn svg_keeps_mixed_case_and_self_closing() {
    assert_eq!(
        body_html(
            "<svg viewbox='0 0 1 1'><lineargradient gradientunits=x/><path d=M0 /></svg><div/>x"
        ),
        "<svg viewBox=\"0 0 1 1\"><linearGradient gradientUnits=\"x/\"><path d=\"M0\"></path></linearGradient></svg><div>x</div>"
    );
    assert_eq!(body_html("<math><mi/></math>"), "<math><mi></mi></math>");
}

#[test]
fn fragments_skip_document_tags() {
    assert_eq!(
        fragment_html("<!doctype html><html><head><body><b>x</b><!--c-->y</p>"),
        "<b>x</b><!--c-->y<p></p>"
    );
    assert_eq!(fragment_html("<li>a<li>b"), "<li>a</li><li>b</li>");
}

#[test]
fn text_nodes_merge() {
    assert_eq!(parse_fragment("a&amp;b").len(), 1);
    let mut root = Element::new("div");
    root.children = parse_fragment("a</x>b");
    assert_eq!(root.children.len(), 1);
    assert_eq!(root.text_content(), "ab");
}

#[test]
fn fragments_in_svg_context_are_foreign() {
    let mut root = Element::new("svg");
    root.children = parse_fragment_in("<path/><lineargradient/>", "svg");
    assert_eq!(
        serialize_children(&root),
        "<path></path><linearGradient></linearGradient>"
    );
}

#[test]
fn vue_fragments_honor_self_closing_component_tags() {
    let nodes = super::parse_vue_fragment_in("<Icon /><span>next</span>", "div");
    let mut root = Element::new("div");
    root.children = nodes;
    assert_eq!(serialize_children(&root), "<icon></icon><span>next</span>");
}
