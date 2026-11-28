use super::*;
use crate::color::Color;
use crate::parser::{parse_declaration_list, parse_stylesheet};
use crate::tests::dom::{Node, Tree};
use crate::types::{Content, Display, Px, Size};
use crate::values::LengthPercentage;
use nativeboat_html::parse_document;

const PAGE: &str = r#"
<div id="root" class="box">
  <p id="text" class="note" style="color: green; margin-left: 3px !important">hello</p>
  <input id="field" placeholder="x">
</div>
"#;

#[test]
fn transition_cascade_resolves_variables_and_resets_components() {
    assert_eq!(
        style_of(
            "#root { transition: opacity 1s linear } #text { transition: inherit }",
            "text"
        )
        .property_value("transition-duration")
        .unwrap(),
        "1s"
    );
    let style = style_of(
        "#root { transition: opacity 1s linear } #text { --speed: 200ms; transition-delay: 5s; transition: background-color var(--speed), transform 120ms ease-in -50ms }",
        "text",
    );
    assert_eq!(
        style.property_value("transition-property").unwrap(),
        "background-color, transform"
    );
    assert_eq!(
        style.property_value("transition-duration").unwrap(),
        "0.2s, 0.12s"
    );
    assert_eq!(
        style.property_value("transition-delay").unwrap(),
        "0s, -0.05s"
    );
    assert_eq!(
        style.property_value("transition-timing-function").unwrap(),
        "ease, ease-in"
    );
    assert_eq!(
        style_of("#root { transition: opacity 1s linear }", "text")
            .property_value("transition-duration")
            .unwrap(),
        "0s"
    );
}

fn sheet(css: &str) -> Stylesheet {
    let (sheet, diagnostics) = parse_stylesheet(css);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    sheet
}

fn style_of(css: &str, id: &str) -> crate::computed::ComputedStyle {
    style_with(css, id, &StyleEnvironment::default())
}

fn style_with(
    css: &str,
    id: &str,
    environment: &StyleEnvironment,
) -> crate::computed::ComputedStyle {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let author = sheet(css);
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let mut parent = None;
    let mut chain = Vec::new();
    let mut node = Some(tree.by_id(id));
    while let Some(current) = node {
        node = current.parent_element();
        chain.push(current);
    }
    for node in chain.into_iter().rev() {
        let inline = node
            .attribute("style")
            .map(parse_declaration_list)
            .unwrap_or_default();
        let style = compute_style(&node, &sheets, &[], &inline, parent.as_ref(), environment);
        parent = Some(style);
    }
    parent.unwrap()
}

fn px(value: f32) -> Size {
    Size::Length(LengthPercentage::Px(value))
}

#[test]
fn author_backgrounds_and_borders_take_the_native_look_away() {
    let styled = |css: &str| style_of(css, "field").author_appearance;
    assert!(!styled(""));
    assert!(!styled(
        "input { color: red; padding: 4px; border-collapse: collapse }"
    ));
    assert!(styled("input { background: #fff }"));
    assert!(styled("#field { background-color: red }"));
    assert!(styled("input { background-image: none }"));
    assert!(styled("input { border: 0 }"));
    assert!(styled("input { border-radius: 4px }"));
    assert!(styled("input { -webkit-appearance: none }"));
    assert!(!styled("input { appearance: auto }"));
    // Only `appearance: none` takes away the room the native look leaves.
    let none = |css: &str| style_of(css, "field").appearance_none;
    assert!(none("input { -webkit-appearance: none }"));
    assert!(none("input { appearance: NONE }"));
    assert!(!none("input { background: #fff; border: 0 }"));
    assert!(!none("input { appearance: auto }"));
    // The style attribute is author style; presentational hints are not.
    assert!(!style_of("", "text").author_appearance);
    let document = parse_document("<button id=b style='border: 0'>x</button>");
    let tree = Tree::new(&document);
    let button = tree.by_id("b");
    let inline = parse_declaration_list("border: 0");
    let hints = parse_declaration_list("background-color: red");
    let environment = StyleEnvironment::default();
    let ua = crate::ua::user_agent_stylesheet();
    let sheets = [SheetRef {
        sheet: ua,
        origin: Origin::UserAgent,
    }];
    assert!(compute_style(&button, &sheets, &[], &inline, None, &environment).author_appearance);
    assert!(!compute_style(&button, &sheets, &hints, &[], None, &environment).author_appearance);
}

#[test]
fn specificity_order_and_importance() {
    let style = style_of(
        "p { width: 1px; height: 1px } .note { width: 2px } #text { width: 3px } \
         p { height: 5px } p.note { color: red !important } p { margin-left: 9px !important }",
        "text",
    );
    assert_eq!(style.width, px(3.0));
    assert_eq!(style.height, px(5.0));
    assert_eq!(style.color, Color::rgb(255, 0, 0));
    assert_eq!(
        style.margin.left,
        crate::types::LengthPercentageAuto::Length(LengthPercentage::Px(3.0))
    );
}

#[test]
fn inline_styles_beat_normal_author_rules() {
    let style = style_of("#text { color: blue }", "text");
    assert_eq!(style.color, Color::rgb(0, 128, 0));
}

#[test]
fn user_agent_rules_lose_to_author_rules_unless_important() {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let node = tree.by_id("root");
    let agent = sheet("div { width: 1px; height: 1px !important; display: block }");
    let author = sheet("div { width: 2px; height: 2px !important }");
    let sheets = [
        SheetRef {
            sheet: &author,
            origin: Origin::Author,
        },
        SheetRef {
            sheet: &agent,
            origin: Origin::UserAgent,
        },
    ];
    let style = compute_style(&node, &sheets, &[], &[], None, &StyleEnvironment::default());
    assert_eq!(style.width, px(2.0));
    assert_eq!(style.height, px(1.0));
    assert_eq!(style.display, Display::Block);
}

#[test]
fn conditional_rules() {
    let narrow = StyleEnvironment {
        media: MediaEnvironment {
            width: 300.0,
            ..MediaEnvironment::default()
        },
        ..StyleEnvironment::default()
    };
    let css = "@media (max-width: 400px) { #root { width: 1px } } \
               @supports (display: grid) { #root { height: 2px } } \
               @supports (display: nope) { #root { height: 3px } } \
               @layer base { #root { min-width: 4px } } \
               @font-face { font-family: x }";
    let style = style_with(css, "root", &narrow);
    assert_eq!(style.width, px(1.0));
    assert_eq!(style.height, px(2.0));
    assert_eq!(style.min_width, px(4.0));
    let wide = style_with(css, "root", &StyleEnvironment::default());
    assert_eq!(wide.width, Size::Auto);
}

#[test]
fn container_names_and_the_container_shorthand() {
    let style = style_with(
        "#root { container: sidebar main / size }",
        "root",
        &StyleEnvironment::default(),
    );
    assert_eq!(
        style.container_name.0,
        vec!["sidebar".to_owned(), "main".to_owned()]
    );
    assert_eq!(style.container_type, crate::types::ContainerType::Size);
    let style = style_with(
        "#root { container-type: inline-size; container: none }",
        "root",
        &StyleEnvironment::default(),
    );
    assert!(style.container_name.0.is_empty());
    assert_eq!(style.container_type, crate::types::ContainerType::Normal);
    for invalid in ["and", "a / b / size", "a /", "none none"] {
        let css = format!("#root {{ container-name: x; container: {invalid} }}");
        let style = style_with(&css, "root", &StyleEnvironment::default());
        assert_eq!(style.container_name.0, vec!["x".to_owned()], "{invalid}");
    }
    // Outside a page, rules in @container rules have no container to
    // query and do not apply.
    let style = style_with(
        "@container (min-width: 0px) { #root { width: 1px } }",
        "root",
        &StyleEnvironment::default(),
    );
    assert_eq!(style.width, Size::Auto);
}

#[test]
fn shorthands_aliases_and_prefixes() {
    let style = style_of(
        "#root { margin: 1px 2px; -webkit-user-select: none; inline-size: 7px; \
         border: 1px solid; all: bogus; unknown-prop: 1; flex: bogus; padding: 1px 2px 3px 4px 5px; color: nope }",
        "root",
    );
    assert_eq!(
        style.margin.right,
        crate::types::LengthPercentageAuto::Length(LengthPercentage::Px(2.0))
    );
    assert_eq!(style.user_select, crate::types::UserSelect::None);
    assert_eq!(style.width, px(7.0));
    assert_eq!(style.border_width.top, Px(1.0));
    assert_eq!(style.flex_grow, 0.0);
    assert_eq!(style.padding.top, LengthPercentage::Px(0.0));
}

#[test]
fn white_space_and_text_wrap_share_the_wrap_mode() {
    use crate::types::{TextWrapMode, TextWrapStyle, WhiteSpace, WhiteSpaceCollapse};
    let white_space = |css: &str| {
        let style = style_of(css, "text");
        let get = |name: &str| style.property_value(name).unwrap();
        (style.white_space, get("white-space"), get("text-wrap"))
    };
    assert_eq!(
        white_space("#text { white-space: pre-wrap; text-wrap: nowrap }"),
        (WhiteSpace::Pre, "pre".into(), "nowrap".into())
    );
    assert_eq!(
        white_space("#text { text-wrap: nowrap; white-space: pre-line }"),
        (WhiteSpace::PreLine, "pre-line".into(), "wrap".into())
    );
    // Either longhand inherits; `text-wrap: initial` restores wrapping.
    assert_eq!(
        white_space("#root { white-space: nowrap } #text { text-wrap: initial }"),
        (WhiteSpace::Normal, "normal".into(), "wrap".into())
    );
    assert_eq!(
        white_space(
            "#root { white-space: nowrap } #text { white-space-collapse: preserve-breaks }"
        ),
        (
            WhiteSpace::new(WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::NoWrap),
            "preserve-breaks nowrap".into(),
            "nowrap".into()
        )
    );
    assert_eq!(
        white_space("#text { white-space: nowrap preserve }").0,
        WhiteSpace::Pre
    );
    let style = style_of("#text { text-wrap: balance nowrap }", "text");
    assert_eq!(style.text_wrap_style, TextWrapStyle::Balance);
    assert_eq!(style.property_value("text-wrap").unwrap(), "nowrap balance");
    let style = style_of("#text { text-wrap: pretty }", "text");
    assert_eq!(style.property_value("text-wrap").unwrap(), "pretty");
    for invalid in ["wrap wrap", "bogus", "nowrap pre"] {
        let css =
            format!("#text {{ white-space: pre; text-wrap: {invalid}; white-space: {invalid} }}");
        assert_eq!(white_space(&css).0, WhiteSpace::Pre, "{invalid}");
    }
}

#[test]
fn css_wide_keywords() {
    let style = style_of(
        "#root { color: red; width: 5px; margin: 4px } \
         #text { color: initial; width: inherit; margin: inherit; all: unset; height: 1px }",
        "text",
    );
    assert_eq!(style.width, Size::Auto);
    assert_eq!(style.color, Color::rgb(0, 128, 0));
    assert_eq!(style.height, px(1.0));
    let style = style_of("#root { width: 5px } #text { width: inherit }", "text");
    assert_eq!(style.width, px(5.0));
    let style = style_of("#root { margin: 4px } #text { margin: inherit }", "text");
    assert_eq!(
        style.margin.top,
        crate::types::LengthPercentageAuto::Length(LengthPercentage::Px(4.0))
    );
}

#[test]
fn custom_properties_and_var() {
    let style = style_of(
        ":root { --size: 4px; --gone: 1px } \
         #root { cursor: pointer; --size: 6px; --pad: var(--size); --gone: initial; --keep: inherit; --fresh: inherit } \
         #text { width: var(--pad); height: var(--missing, calc(var(--size) * 2)); \
                 min-width: var(--gone, 9px); max-width: var(--nope); margin: var(--size) 0; \
                 cursor: var(--undefined) }",
        "text",
    );
    assert_eq!(style.width, px(6.0));
    assert_eq!(style.height, px(12.0));
    assert_eq!(style.min_width, px(9.0));
    assert_eq!(style.max_width, crate::types::MaxSize::None);
    assert_eq!(
        style.margin.top,
        crate::types::LengthPercentageAuto::Length(LengthPercentage::Px(6.0))
    );
    assert_eq!(style.cursor, crate::types::Cursor::Pointer);
    assert_eq!(style.custom.get("--size").map(String::as_str), Some("6px"));
    assert_eq!(style.custom.get("--gone"), None);
    assert_eq!(style.custom.get("--fresh"), None);
}

#[test]
fn registered_custom_properties_take_initial_values_and_may_not_inherit() {
    let css = "@property --shadow { syntax: '*'; inherits: false; initial-value: 0 0 #0000 }                @property --tone { syntax: '<color>'; inherits: true; initial-value: red }                @media print { @property --tone { syntax: '*'; inherits: true; initial-value: blue } }                #root { --shadow: 1px; --bad: 2px; --tone: green }                #text { width: var(--bad, 7px); --tone: unset }";
    let root = style_of(css, "root");
    assert_eq!(root.custom.get("--shadow").map(String::as_str), Some("1px"));
    let text = style_of(css, "text");
    assert_eq!(
        text.custom.get("--shadow").map(String::as_str),
        Some("0 0 #0000")
    );
    // `unset` inherits a property that inherits.
    assert_eq!(text.custom.get("--tone").map(String::as_str), Some("green"));
    // Without a registration, a property stays an ordinary inherited one.
    assert_eq!(text.width, px(2.0));
    // A registration without an initial value is dropped.
    let (sheet, diagnostics) =
        parse_stylesheet("@property --bad { syntax: '<length>'; inherits: false }");
    assert!(sheet.rules.is_empty());
    assert_eq!(diagnostics[0].message, "invalid @property --bad");
    let plain = style_of(
        "@property --tone { syntax: '*'; inherits: false; initial-value: red } #root { --tone: initial }",
        "text",
    );
    assert_eq!(plain.custom.get("--tone").map(String::as_str), Some("red"));
}

#[test]
fn cascade_layers_order_by_declaration_and_lose_to_unlayered_rules() {
    let css = "@layer base, utilities; \
               @layer utilities { #root #text { width: 3px; height: 5px !important } } \
               @layer base { #root #text { width: 2px; min-width: 4px; height: 6px !important } } \
               #text { min-width: 7px } \
               @layer outer { #text { max-width: 9px } @layer inner { #root #text { max-width: 8px } } } \
               @layer { #text { min-height: 1px } } @layer { #text { min-height: 2px } }";
    let style = style_of(css, "text");
    let value = |name: &str| style.property_value(name).unwrap();
    // A later layer wins over an earlier one whatever their source order.
    assert_eq!(value("width"), "3px");
    // Rules outside any layer win over layered ones of any specificity.
    assert_eq!(value("min-width"), "7px");
    // Important declarations of earlier layers win.
    assert_eq!(value("height"), "6px");
    // A layer's own rules win over its sublayers'.
    assert_eq!(value("max-width"), "9px");
    // Anonymous layers are separate layers, in order.
    assert_eq!(value("min-height"), "2px");
}

#[test]
fn custom_properties_inherit_explicitly() {
    let style = style_of(
        ":root { --a: 1px } #root { --a: inherit; width: var(--a) }",
        "root",
    );
    assert_eq!(style.width, px(1.0));
}

#[test]
fn invalid_values_after_substitution_unset_the_property() {
    let style = style_of(
        "#root { visibility: hidden; --bad: 12px } #text { visibility: visible; visibility: var(--bad); width: var(--bad) var(--bad) }",
        "text",
    );
    assert_eq!(style.visibility, crate::types::Visibility::Hidden);
    assert_eq!(style.width, Size::Auto);
}

#[test]
fn substitution() {
    let mut custom = BTreeMap::new();
    custom.insert("--a".to_owned(), "1px".to_owned());
    custom.insert("--loop".to_owned(), "var(--loop)".to_owned());
    assert_eq!(substitute("var(--a)", &custom).as_deref(), Some("1px"));
    assert_eq!(
        substitute("var( --a ) 2px", &custom).as_deref(),
        Some("1px 2px")
    );
    assert_eq!(
        substitute("calc(var(--a) + var(--b, 2px))", &custom).as_deref(),
        Some("calc(1px + 2px)")
    );
    assert_eq!(
        substitute("var(--b, var(--a))", &custom).as_deref(),
        Some("1px")
    );
    assert_eq!(
        substitute("var(--b, f(x))", &custom).as_deref(),
        Some("f(x)")
    );
    assert_eq!(substitute("var(--b)", &custom), None);
    assert_eq!(substitute("var(--b, var(--c))", &custom), None);
    assert_eq!(substitute("var(--loop)", &custom), None);
    assert_eq!(substitute("var(x)", &custom), None);
    assert_eq!(substitute("var(--a", &custom).as_deref(), Some("1px"));
    assert_eq!(
        substitute("rgb(1 2 3)", &custom).as_deref(),
        Some("rgb(1 2 3)")
    );
}

#[test]
fn supports_conditions() {
    for (condition, expected) in [
        ("(display: grid)", true),
        ("(display: nope)", false),
        ("not (display: nope)", true),
        ("(display: grid) and (color: red)", true),
        ("(display: grid) and (color: 1px)", false),
        ("(display: nope) or (display: flex)", true),
        ("((display: grid))", true),
        ("selector(a > b)", true),
        ("selector(a >)", false),
        ("selector(a:is(b, (c)) [d])", true),
        ("(--x: y)", true),
        ("(-webkit-user-select: none)", true),
        ("(-webkit-appearance: none)", false),
        ("(margin: 1px 2px)", true),
        ("(display grid)", false),
        ("foo(x)", false),
        ("", false),
        ("(1px: 2)", false),
        ("(a b: c)", false),
        ("(display: grid) xor (color: red)", false),
        ("(display: grid) and 1", false),
        ("(display: grid) and", false),
        ("(display: grid) and (oops)", false),
        // `var()` makes any value of a known property supported.
        ("(backdrop-filter: var(--tw))", true),
        ("(color: calc(var(--x) * 2) red)", true),
        ("(margin: var(--x))", true),
        ("(no-such-property: var(--tw))", false),
    ] {
        assert_eq!(supports(condition), expected, "{condition}");
    }
    // Tailwind nests the condition in the rule it guards.
    let style = style_of(
        "#root { @supports (backdrop-filter: var(--tw)) { --tw-blur: blur(4px); backdrop-filter: var(--tw-blur); } }",
        "root",
    );
    assert!(
        !style.backdrop_filter.0.is_empty(),
        "{:?}",
        style.backdrop_filter
    );
}

#[test]
fn pseudo_elements() {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let node = tree.by_id("field");
    let author = sheet(
        "input::placeholder { color: red } p::before { content: \"*\"; color: blue } \
         p::after { color: green } input { color: black }",
    );
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let element_style = compute_style(&node, &sheets, &[], &[], None, &environment);
    let placeholder = compute_pseudo_style(
        &node,
        &PseudoElement::Placeholder,
        &sheets,
        &element_style,
        &environment,
    )
    .unwrap();
    assert_eq!(placeholder.color, Color::rgb(255, 0, 0));
    assert!(
        compute_pseudo_style(
            &node,
            &PseudoElement::Before,
            &sheets,
            &element_style,
            &environment
        )
        .is_none()
    );
    let paragraph: Node<'_> = tree.by_id("text");
    let paragraph_style = compute_style(&paragraph, &sheets, &[], &[], None, &environment);
    let before = compute_pseudo_style(
        &paragraph,
        &PseudoElement::Before,
        &sheets,
        &paragraph_style,
        &environment,
    )
    .unwrap();
    assert_eq!(
        before.content,
        Content::Items(vec![crate::types::ContentItem::Text("*".to_owned())])
    );
    assert!(
        compute_pseudo_style(
            &paragraph,
            &PseudoElement::After,
            &sheets,
            &paragraph_style,
            &environment
        )
        .is_none()
    );
    let declarations = matching_declarations(
        &paragraph,
        &sheets,
        &[],
        &[],
        Some(&PseudoElement::Before),
        &environment.media,
    );
    assert_eq!(declarations.len(), 2);
}

#[test]
fn presentational_hints_rank_below_author_rules() {
    use crate::types::Paint;
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let agent = sheet("p { fill: blue; stroke: blue }");
    let author = sheet("* { stroke: red }");
    let sheets = [
        SheetRef {
            sheet: &agent,
            origin: Origin::UserAgent,
        },
        SheetRef {
            sheet: &author,
            origin: Origin::Author,
        },
    ];
    let hints = parse_declaration_list("fill: green; stroke: green; stroke-width: 3");
    let node = tree.by_id("text");
    let style = compute_style(
        &node,
        &sheets,
        &hints,
        &[],
        None,
        &StyleEnvironment::default(),
    );
    assert_eq!(style.fill, Paint::Color(Color::rgb(0, 128, 0)));
    assert_eq!(style.stroke, Paint::Color(Color::rgb(255, 0, 0)));
    assert_eq!(style.stroke_width, Px(3.0));
}

#[test]
fn one_rule_map_styles_many_elements() {
    let document = parse_document(
        r#"<div id="a" class="x y"><p id="b" class="y">t</p><span id="c" style="--tone: blue">s</span><i id="d" style="bogus: 1; width: nope; height: 6px">i</i></div><svg><clipPath id="clip"></clipPath></svg>"#,
    );
    let tree = Tree::new(&document);
    let author = sheet(
        ".x { color: red } .y { color: green } div#a, .x { width: 1px } \
         p { width: 2px } * { height: 3px } .missing { color: black } clippath { width: 7px } \
         span { color: var(--tone) } p.y { color: var(--tone, purple); cursor: 1; unknown: 2 } \
         @media (min-width: 99999px) { p { height: 9px } } \
         @supports (display: grid) { span { width: 4px } } @layer base { span { height: 5px } }",
    );
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let map = RuleMap::new(&sheets, &environment.media);
    assert!(!map.has_pseudo_elements());
    let style = |id: &str, parent: Option<&ComputedStyle>| {
        let node = tree.by_id(id);
        let inline = node
            .attribute("style")
            .map(parse_declaration_list)
            .unwrap_or_default();
        map.compute_style(&node, &[], &inline, parent, &environment)
    };
    // Later rules win among equals, whichever bucket found them; a rule
    // matched by two selectors counts once.
    let a = style("a", None);
    assert_eq!(a.color, Color::rgb(0, 128, 0));
    assert_eq!((a.width.clone(), a.height.clone()), (px(1.0), px(3.0)));
    // Values are worked out once and reused; `var()` values per element.
    for _ in 0..2 {
        let b = style("b", Some(&a));
        assert_eq!(b.color, Color::rgb(128, 0, 128));
        assert_eq!((b.width.clone(), b.height.clone()), (px(2.0), px(3.0)));
        assert_eq!(b.cursor, crate::types::Cursor::Auto);
        let c = style("c", Some(&a));
        assert_eq!(c.color, Color::rgb(0, 0, 255));
        // The unlayered `*` rule wins over the layered `span` rule.
        assert_eq!((c.width.clone(), c.height.clone()), (px(4.0), px(3.0)));
    }
    assert_eq!(style("clip", Some(&a)).width, px(7.0));
    // Inline declarations of unknown properties or with invalid values do
    // nothing.
    let d = style("d", Some(&a));
    assert_eq!((d.width.clone(), d.height.clone()), (Size::Auto, px(6.0)));
    let pseudo = sheet("p::before { content: 'x' }");
    let sheets = [SheetRef {
        sheet: &pseudo,
        origin: Origin::Author,
    }];
    assert!(RuleMap::new(&sheets, &environment.media).has_pseudo_elements());
    assert!(format!("{map:?}").starts_with("RuleMap"));
}

#[test]
fn rules_with_has_match_apart_and_merge_into_cascade_order() {
    let document =
        parse_document(r#"<div id="a" class="x"><p id="b">t</p></div><SPAN id="s"></SPAN>"#);
    let tree = Tree::new(&document);
    let author = sheet(
        ".x, div:has(p) { color: red } div:has(p) { width: 1px } p { width: 2px } \
         div:has(p)::before { content: 'x' } span { color: blue }",
    );
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let map = RuleMap::new(&sheets, &environment.media);
    assert!(map.is_relational());
    assert!(map.has_pseudo_elements());
    let a = tree.by_id("a");
    // The first rule matches through both of its selectors; it counts once,
    // with the higher specificity.
    let local = map.matching_local(&a, None);
    let relational = map.matching_relational(&a, None);
    assert_eq!(local.iter().map(|(rule, _)| *rule).collect::<Vec<_>>(), [0]);
    assert_eq!(
        relational.iter().map(|(rule, _)| *rule).collect::<Vec<_>>(),
        [0, 1]
    );
    let all = map.matching(&a, None);
    assert_eq!(all, merge_matches(&local, &relational));
    assert_eq!(
        all.iter().map(|(rule, _)| *rule).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(all[0].1, local[0].1.max(relational[0].1));
    assert_eq!(merge_matches(&local, &[]), local);
    // Known matches give the same style as matching anew.
    let matched = map.compute_matched_style(&a, &all, &[], &[], None, &environment);
    assert_eq!(matched, map.compute_style(&a, &[], &[], None, &environment));
    assert_eq!(matched.width, px(1.0));
    // Upper-case element names find their type rules.
    let s = tree.by_id("s");
    assert_eq!(
        map.compute_style(&s, &[], &[], None, &environment).color,
        Color::rgb(0, 0, 255)
    );
    let plain = sheet("p { width: 2px }");
    let sheets = [SheetRef {
        sheet: &plain,
        origin: Origin::Author,
    }];
    let map = RuleMap::new(&sheets, &environment.media);
    assert!(!map.is_relational());
    assert!(map.matching_relational(&a, None).is_empty());
}
