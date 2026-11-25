use super::*;

fn kinds(source: &str) -> Vec<TokenKind> {
    tokenize(source)
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn start(name: &str, attributes: &[(&str, &str)], self_closing: bool) -> TokenKind {
    TokenKind::StartTag {
        name: name.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| Attribute {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
        self_closing,
    }
}

fn text(value: &str) -> TokenKind {
    TokenKind::Text(value.to_owned())
}

fn end(name: &str) -> TokenKind {
    TokenKind::EndTag(name.to_owned())
}

#[test]
fn tags_attributes_and_text() {
    assert_eq!(
        kinds(r#"<DIV Class="a &amp; b" id='x' data-v=1 hidden>Hi &lt;3</div>"#),
        vec![
            start(
                "div",
                &[
                    ("class", "a & b"),
                    ("id", "x"),
                    ("data-v", "1"),
                    ("hidden", "")
                ],
                false
            ),
            text("Hi <3"),
            end("div"),
        ]
    );
}

#[test]
fn self_closing_and_stray_slashes() {
    assert_eq!(
        kinds("<br/><img src=a.png /><x a/b / >"),
        vec![
            start("br", &[], true),
            start("img", &[("src", "a.png")], true),
            start("x", &[("a", ""), ("b", "")], false),
        ]
    );
}

#[test]
fn duplicate_attributes_keep_the_first() {
    assert_eq!(
        kinds("<a x=1 X=2 y = 'q'>"),
        vec![start("a", &[("x", "1"), ("y", "q")], false)]
    );
}

#[test]
fn unterminated_tags_and_values_end_at_eof() {
    assert_eq!(
        kinds("<a href=\"x"),
        vec![start("a", &[("href", "x")], false)]
    );
    assert_eq!(kinds("<a b"), vec![start("a", &[("b", "")], false)]);
    assert_eq!(kinds("<a b="), vec![start("a", &[("b", "")], false)]);
}

#[test]
fn comments_doctype_and_bogus_markup() {
    assert_eq!(
        kinds("<!DOCTYPE html><!-- hi --><?xml x?><!x><!doctype><!--open"),
        vec![
            TokenKind::Doctype("html".to_owned()),
            TokenKind::Comment(" hi ".to_owned()),
            TokenKind::Comment("xml x?".to_owned()),
            TokenKind::Comment("x".to_owned()),
            TokenKind::Doctype(String::new()),
            TokenKind::Comment("open".to_owned()),
        ]
    );
    assert_eq!(kinds("<!x"), vec![TokenKind::Comment("x".to_owned())]);
}

#[test]
fn end_tags_and_odd_closers() {
    assert_eq!(
        kinds("</P class=x></><//x></3>"),
        vec![
            end("p"),
            TokenKind::Comment("/x".to_owned()),
            TokenKind::Comment("3".to_owned()),
        ]
    );
    assert_eq!(kinds("</a/b"), vec![end("a")]);
}

#[test]
fn literal_less_than_signs_are_text() {
    assert_eq!(kinds("a < b <"), vec![text("a < b <")]);
    assert_eq!(kinds("x</"), vec![text("x</")]);
    assert_eq!(kinds("1 <2"), vec![text("1 <2")]);
}

#[test]
fn raw_text_elements_keep_markup() {
    assert_eq!(
        kinds("<script>if (a < b && c) x('</div>');</script ><p>"),
        vec![
            start("script", &[], false),
            text("if (a < b && c) x('</div>');"),
            end("script"),
            start("p", &[], false),
        ]
    );
    assert_eq!(
        kinds("<style>a>b{}</styles></STYLE>"),
        vec![
            start("style", &[], false),
            text("a>b{}</styles>"),
            end("style")
        ]
    );
    assert_eq!(
        kinds("<script>never closed"),
        vec![start("script", &[], false), text("never closed")]
    );
    assert_eq!(
        kinds("<script></script"),
        vec![start("script", &[], false), end("script")]
    );
    assert_eq!(
        kinds("<script></script/>"),
        vec![start("script", &[], false), end("script")]
    );
    assert_eq!(
        kinds("<script></sc"),
        vec![start("script", &[], false), text("</sc")]
    );
}

#[test]
fn escapable_raw_text_decodes_references() {
    assert_eq!(
        kinds("<textarea>a &amp; <b></textarea>"),
        vec![
            start("textarea", &[], false),
            text("a & <b>"),
            end("textarea")
        ]
    );
    assert_eq!(
        kinds("<title></title>"),
        vec![start("title", &[], false), end("title")]
    );
}

#[test]
fn offsets_point_at_token_starts() {
    let tokens = tokenize("ab<p>cd</p>");
    let offsets: Vec<usize> = tokens.iter().map(|token| token.offset).collect();
    assert_eq!(offsets, vec![0, 2, 5, 7]);
}

#[test]
fn line_index_counts_characters() {
    let source = "é\nab\n\nc";
    let index = LineIndex::new(source);
    assert_eq!(index.position(0), Position { line: 1, column: 1 });
    assert_eq!(index.position(2), Position { line: 1, column: 2 });
    assert_eq!(index.position(4), Position { line: 2, column: 2 });
    assert_eq!(index.position(7), Position { line: 4, column: 1 });
}

#[test]
fn text_around_ignored_markup_merges() {
    assert_eq!(kinds("a</>b"), vec![text("ab")]);
}
