use super::*;

fn style_rules(rules: &[Rule]) -> Vec<&StyleRule> {
    rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::Style(rule) => Some(rule),
            _ => None,
        })
        .collect()
}

fn selector_count(rule: &StyleRule) -> usize {
    rule.selectors.0.len()
}

fn declarations(rule: &StyleRule) -> Vec<(String, String, bool)> {
    rule.declarations
        .iter()
        .map(|declaration| {
            (
                declaration.name.clone(),
                declaration.value.clone(),
                declaration.important,
            )
        })
        .collect()
}

fn owned(items: &[(&str, &str, bool)]) -> Vec<(String, String, bool)> {
    items
        .iter()
        .map(|(name, value, important)| ((*name).to_owned(), (*value).to_owned(), *important))
        .collect()
}

#[test]
fn style_rules_and_declarations() {
    let (sheet, diagnostics) = parse_stylesheet(
        "<!-- .a, .b > c { COLOR: Red ; margin : 0 auto!important; --Brand: #FFF; ; width: calc(1px + (2px)) } -->\n\
         p{}",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 2);
    assert_eq!(selector_count(rules[0]), 2);
    assert_eq!(
        declarations(rules[0]),
        owned(&[
            ("color", "Red", false),
            ("margin", "0 auto", true),
            ("--Brand", "#FFF", false),
            ("width", "calc(1px + (2px))", false),
        ])
    );
    assert_eq!(rules[0].position, Position { line: 1, column: 6 });
    assert_eq!(
        rules[0].declarations[1].position,
        Position {
            line: 1,
            column: 32
        }
    );
    assert_eq!(rules[1].position, Position { line: 2, column: 1 });
    assert!(rules[1].declarations.is_empty());
}

#[test]
fn invalid_rules_produce_diagnostics() {
    let (sheet, diagnostics) =
        parse_stylesheet("a..b { color: red } p { color: blue } q { 1: x; color }\nunterminated");
    let rules = style_rules(&sheet.rules);
    assert_eq!(rules.len(), 2);
    let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        vec![
            "invalid selector `a..b`: expected a class name after `.`",
            "invalid declaration",
            "invalid declaration",
            "unexpected end of stylesheet in rule prelude",
        ]
    );
    assert_eq!(diagnostics[3].position, Position { line: 2, column: 1 });
}

#[test]
fn unclosed_blocks_run_to_the_end() {
    let (sheet, _) = parse_stylesheet("a { color: red; b: f(1");
    let rules = style_rules(&sheet.rules);
    assert_eq!(
        declarations(rules[0]),
        owned(&[("color", "red", false), ("b", "f(1", false)])
    );
}

#[test]
fn conditional_group_rules() {
    let (sheet, diagnostics) = parse_stylesheet(
        "@media screen and (max-width: 600px) { .a { color: red } }\n\
         @supports (display: grid) { .b { display: grid } }\n\
         @layer base { .c { margin: 0 } }\n\
         @layer one, two;\n\
         @charset \"utf-8\";\n\
         @namespace svg url(x);",
    );
    assert!(diagnostics.is_empty());
    assert_eq!(sheet.rules.len(), 4);
    assert_eq!(
        sheet.rules[3],
        Rule::LayerOrder(vec!["one".to_owned(), "two".to_owned()])
    );
    match &sheet.rules[0] {
        Rule::Media { query, rules } => {
            assert_eq!(query.0.len(), 1);
            assert_eq!(style_rules(rules).len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[1] {
        Rule::Supports { condition, rules } => {
            assert_eq!(condition, "(display: grid)");
            assert_eq!(style_rules(rules).len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[2] {
        Rule::Layer { name, rules } => {
            assert_eq!(name, "base");
            assert_eq!(style_rules(rules).len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn imports() {
    let (sheet, diagnostics) = parse_stylesheet(
        "@import \"a.css\";@import url(b.css) print;@import url( \"c.css\" ) screen;@import url(\"d.css\");@import 1;@import url(\"bad\n);",
    );
    let imports: Vec<(String, usize)> = sheet
        .rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::Import { url, media, .. } => Some((url.clone(), media.0.len())),
            _ => None,
        })
        .collect();
    assert_eq!(
        imports,
        vec![
            ("a.css".to_owned(), 0),
            ("b.css".to_owned(), 1),
            ("c.css".to_owned(), 1),
            ("d.css".to_owned(), 0),
        ]
    );
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].message, "invalid @import");
}

#[test]
fn font_faces_and_keyframes() {
    let (sheet, _) = parse_stylesheet(
        "@font-face { font-family: X; src: url(x.woff2) }\n\
         @keyframes spin { from { opacity: 0 } 50%, 75% { opacity: .5 } to { opacity: 1 } }\n\
         @-webkit-keyframes \"fade\" { 0% { opacity: 0 } junk }\n\
         @keyframes { }",
    );
    match &sheet.rules[0] {
        Rule::FontFace(declarations) => assert_eq!(declarations.len(), 2),
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[1] {
        Rule::Keyframes { name, keyframes } => {
            assert_eq!(name, "spin");
            let offsets: Vec<Vec<f64>> = keyframes
                .iter()
                .map(|frame| frame.offsets.clone())
                .collect();
            assert_eq!(offsets, vec![vec![0.0], vec![0.5, 0.75], vec![1.0]]);
            assert_eq!(keyframes[1].declarations[0].value, "0.5");
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[2] {
        Rule::Keyframes { name, keyframes } => {
            assert_eq!(name, "fade");
            assert_eq!(keyframes.len(), 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    match &sheet.rules[3] {
        Rule::Keyframes { name, keyframes } => {
            assert!(name.is_empty());
            assert!(keyframes.is_empty());
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn unknown_at_rules_are_kept() {
    let (sheet, _) = parse_stylesheet(
        "@counter-style thumbs { system: cyclic }\n@font-feature-values Font { @swash { fancy: 1 } }\n@media print;",
    );
    let names: Vec<(String, String, u32)> = sheet
        .rules
        .iter()
        .map(|rule| match rule {
            Rule::Unknown {
                name,
                prelude,
                position,
            } => (name.clone(), prelude.clone(), position.line),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        vec![
            ("counter-style".to_owned(), "thumbs".to_owned(), 1),
            ("font-feature-values".to_owned(), "Font".to_owned(), 2),
            ("media".to_owned(), "print".to_owned(), 3),
        ]
    );
}

#[test]
fn container_rules_keep_their_query_and_rules() {
    let (sheet, _) = parse_stylesheet(
        "@container sidebar (min-width: 4px) { a { color: red } }\n@container a b (width > 1px) { b {} }\n.card { @container (width < 9px) { color: blue } }",
    );
    let containers: Vec<_> = sheet
        .rules
        .iter()
        .filter_map(|rule| match rule {
            Rule::Container { query, rules } => Some((query.name.clone(), rules.len())),
            _ => None,
        })
        .collect();
    // The invalid prelude drops its rule; the nested one keeps the parent
    // selector.
    assert_eq!(containers, vec![(Some("sidebar".to_owned()), 1), (None, 1)]);
}

#[test]
fn nested_rules_are_flattened() {
    let (sheet, diagnostics) = parse_stylesheet(
        ".card { color: red; &:hover { color: blue } .title, & > .icon { margin: 0 } \
         @media (max-width: 10px) { padding: 0; .x { gap: 1px } } }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let top = style_rules(&sheet.rules);
    assert_eq!(top.len(), 3);
    assert_eq!(declarations(top[0]), owned(&[("color", "red", false)]));
    assert_eq!(declarations(top[1]), owned(&[("color", "blue", false)]));
    assert_eq!(
        top[1].selectors,
        crate::SelectorList::parse(":is(.card):hover").unwrap()
    );
    assert_eq!(
        top[2].selectors,
        crate::SelectorList::parse(":is(.card) .title, :is(.card) > .icon").unwrap()
    );
    match &sheet.rules[3] {
        Rule::Media { rules, .. } => {
            let nested = style_rules(rules);
            assert_eq!(nested.len(), 2);
            assert_eq!(declarations(nested[0]), owned(&[("padding", "0", false)]));
            assert_eq!(
                nested[1].selectors,
                crate::SelectorList::parse(":is(.card) .x").unwrap()
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn nested_media_without_declarations_adds_no_parent_rule() {
    let (sheet, _) = parse_stylesheet(".a { @media print { .b { color: red } } }");
    match &sheet.rules[1] {
        Rule::Media { rules, .. } => assert_eq!(style_rules(rules).len(), 1),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn invalid_nested_selectors_are_reported() {
    let (sheet, diagnostics) = parse_stylesheet(".a { & .. { color: red } color: blue }");
    assert_eq!(style_rules(&sheet.rules).len(), 1);
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn declaration_lists() {
    let declarations =
        parse_declaration_list("color: red; .x { a: b } ; Font-Size:12PX !IMPORTANT");
    let names: Vec<(&str, &str, bool)> = declarations
        .iter()
        .map(|declaration| {
            (
                declaration.name.as_str(),
                declaration.value.as_str(),
                declaration.important,
            )
        })
        .collect();
    assert_eq!(
        names,
        vec![("color", "red", false), ("font-size", "12PX", true)]
    );
    assert!(parse_declaration_list("").is_empty());
    assert!(parse_declaration_list(":x").is_empty());
}

#[test]
fn stylesheets_serialize() {
    let (sheet, _) =
        parse_stylesheet("@media (hover) { a:hover { color: red } } @font-face { x: y }");
    let json = serde_json::to_string(&sheet).unwrap();
    assert_eq!(serde_json::from_str::<Stylesheet>(&json).unwrap(), sheet);
}

#[test]
fn positions_on_long_lines_count_characters_from_the_line_start() {
    // A minified sheet is one long line; columns count characters, so the
    // multi-byte content before a rule counts once per character.
    let (sheet, _) = parse_stylesheet("a{x:é}b{y:1}c{z:2}\nd{w:3}");
    let rules = style_rules(&sheet.rules);
    let positions: Vec<_> = rules.iter().map(|rule| rule.position).collect();
    assert_eq!(
        positions,
        [
            Position { line: 1, column: 1 },
            Position { line: 1, column: 7 },
            Position {
                line: 1,
                column: 13
            },
            Position { line: 2, column: 1 },
        ]
    );
    // A position before the last one found is counted from the line start.
    let (_, diagnostics) = parse_stylesheet("a{b:c}d{1:x}");
    assert_eq!(diagnostics[0].position, Position { line: 1, column: 9 });
}
