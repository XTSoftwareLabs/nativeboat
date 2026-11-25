use super::*;
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet};

/// The counter texts of each element's `::before`, by id, in document order.
fn befores(html: &str, css: &str) -> Vec<(String, Vec<String>)> {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let (author, _) = parse_stylesheet(css);
    let sheets = [
        SheetRef {
            sheet: user_agent_stylesheet(),
            origin: Origin::UserAgent,
        },
        SheetRef {
            sheet: &author,
            origin: Origin::Author,
        },
    ];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let resolved = resolve(&dom, &styles);
    let mut found: Vec<(NodeId, String, Vec<String>)> = resolved
        .iter()
        .filter(|((_, key), _)| *key == 0)
        .map(|((node, _), texts)| {
            let id = dom.get_attribute(*node, "id").unwrap_or("").to_owned();
            (*node, id, texts.clone())
        })
        .collect();
    found.sort_by_key(|(node, ..)| *node);
    found
        .into_iter()
        .map(|(_, id, texts)| (id, texts))
        .collect()
}

fn texts(found: &[(String, Vec<String>)]) -> Vec<&str> {
    found
        .iter()
        .flat_map(|(_, texts)| texts.iter().map(String::as_str))
        .collect()
}

#[test]
fn line_numbers_count_their_rows() {
    // Prism's line numbers: a reset on the rows, an increment per row.
    let found = befores(
        "<pre><span class=rows><span id=a></span><span id=b></span><span id=c></span></span></pre>\
         <pre><span class=rows><span id=d></span></span></pre>",
        ".rows { counter-reset: line } .rows > span { counter-increment: line } \
         .rows > span::before { content: counter(line) }",
    );
    assert_eq!(texts(&found), ["1", "2", "3", "1"]);
}

#[test]
fn nested_resets_nest_and_sibling_resets_replace() {
    let found = befores(
        "<ol id=x><li id=a>a<ol><li id=b>b</li><li id=c>c</li></ol></li><li id=d>d</li></ol>",
        "ol { counter-reset: item 0; list-style: none } li { counter-increment: item; display: block } \
         li::before { content: counters(item, '.') ' ' counter(item, upper-roman) }",
    );
    assert_eq!(
        texts(&found),
        ["1", "I", "1.1", "I", "1.2", "II", "2", "II"]
    );
    // A sibling's reset starts the count again rather than nesting.
    let found = befores(
        "<div><h2 id=a></h2><p id=b></p><h2 id=c></h2><p id=d></p></div>",
        "h2 { counter-reset: sub 4 } p { counter-increment: sub 2 } p::before { content: counters(sub, '-') }",
    );
    assert_eq!(texts(&found), ["6", "6"]);
}

#[test]
fn sets_follow_increments_and_hidden_elements_count_nothing() {
    let found = befores(
        "<div id=a></div><div id=b style='display:none'></div><div id=c></div><div id=d></div>",
        "body { counter-reset: n } div { counter-increment: n } #d { counter-set: n 10 } \
         div::before { content: counter(n, decimal-leading-zero) }",
    );
    assert_eq!(texts(&found), ["01", "02", "10"]);
}

#[test]
fn counters_without_resets_start_at_zero_and_list_items_count_themselves() {
    let found = befores(
        "<p id=a></p><ul><li id=b></li><li id=c></li></ul>",
        "p::before { content: counter(missing, lower-alpha) } \
         li::before { content: counter(list-item) '/' counter(list-item, lower-alpha) }",
    );
    assert_eq!(texts(&found), ["0", "1", "a", "2", "b"]);
}

#[test]
fn counter_styles_fall_back_to_decimal_out_of_range() {
    assert_eq!(counter_text(ListStyleType::UpperRoman, 4000), "4000");
    assert_eq!(counter_text(ListStyleType::LowerAlpha, 28), "ab");
    assert_eq!(counter_text(ListStyleType::LowerAlpha, -1), "-1");
    assert_eq!(counter_text(ListStyleType::DecimalLeadingZero, -3), "-03");
    assert_eq!(counter_text(ListStyleType::Square, 3), "\u{25AA}");
    assert_eq!(counter_text(ListStyleType::None, 3), "");
}
