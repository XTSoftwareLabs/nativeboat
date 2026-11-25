use super::*;
use crate::tests::dom::{Node, Tree};
use nativeboat_html::parse_document;

#[test]
fn native_scrollbar_corner_and_unknown_pseudo_names_are_classified() {
    assert_eq!(
        PseudoElement::from_name("-WEBKIT-SCROLLBAR-CORNER"),
        PseudoElement::ScrollbarCorner
    );
    assert_eq!(
        PseudoElement::from_name("FUTURE-DECORATION"),
        PseudoElement::Other("future-decoration".into())
    );
}

#[test]
fn scrollbar_states_belong_to_the_part_and_keep_host_specificity() {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let node = tree.by_id("app");
    let host_hover = SelectorList::parse("a:hover::-webkit-scrollbar-thumb:hover").unwrap();
    assert!(!host_hover.0[0].matches_pseudo_state(
        &tree.by_id("link"),
        &PseudoElement::ScrollbarThumb,
        0
    ));
    assert!(host_hover.0[0].matches_pseudo_state(
        &tree.by_id("link"),
        &PseudoElement::ScrollbarThumb,
        state::HOVER
    ));
    assert!(!host_hover.0[0].matches_pseudo_state(
        &node,
        &PseudoElement::ScrollbarThumb,
        state::HOVER
    ));
    let selector = &SelectorList::parse("#app::-webkit-scrollbar-thumb:vertical:hover")
        .unwrap()
        .0[0];
    assert_eq!(selector.specificity, Specificity(1, 2, 1));
    assert!(!selector.matches_pseudo(&node, &PseudoElement::ScrollbarThumb));
    assert!(selector.matches_pseudo_state(
        &node,
        &PseudoElement::ScrollbarThumb,
        state::VERTICAL | state::HOVER
    ));
    assert!(!selector.matches_pseudo_state(
        &node,
        &PseudoElement::ScrollbarThumb,
        state::HORIZONTAL | state::HOVER
    ));
    assert!(!selector.matches_pseudo_state(
        &tree.by_id("p1"),
        &PseudoElement::ScrollbarThumb,
        state::VERTICAL | state::HOVER
    ));
    for (suffix, flags, matched) in [
        (
            ":horizontal:active",
            state::HORIZONTAL | state::ACTIVE,
            true,
        ),
        (":not(:hover)", 0, true),
        (":not(:hover)", state::HOVER, false),
        (":is(:hover,:active)", state::ACTIVE, true),
        (":where(:hover)", state::HOVER, true),
        (":has(:hover)", state::HOVER, false),
        (":is(div)", 0, false),
        (":is(.x)", 0, false),
        (":is(div span)", 0, false),
        (":is(::before)", 0, false),
        (":focus", state::FOCUS, false),
    ] {
        let list = SelectorList::parse(&format!("::-webkit-scrollbar-track{suffix}")).unwrap();
        assert_eq!(
            list.0[0].matches_pseudo_state(&node, &PseudoElement::ScrollbarTrack, flags),
            matched,
            "{suffix}"
        );
    }
    assert!(!check(":vertical", "app"));
    assert!(!check(":horizontal", "app"));
    for source in [
        "::before::after",
        "::-webkit-scrollbar.x",
        "::-webkit-scrollbar#x",
        "::-webkit-scrollbar[x]",
    ] {
        assert!(SelectorList::parse(source).is_err(), "{source}");
    }
    for name in [
        "::-webkit-scrollbar-thumb:future",
        "::-webkit-scrollbar-thumb:not(:future)",
        "::-webkit-scrollbar-thumb:is(:future)",
        "::-webkit-scrollbar-thumb:where(:future)",
        "::-webkit-scrollbar-thumb:has(:future)",
    ] {
        assert_eq!(
            SelectorList::parse(name).unwrap().unsupported(),
            [":future"]
        );
    }
    assert!(
        SelectorList::parse("::-webkit-scrollbar-thumb:vertical")
            .unwrap()
            .unsupported()
            .is_empty()
    );
}

const PAGE: &str = r#"
<div id="app" class="container main" data-kind="Panel-x" lang="en-US">
  <ul id="list">
    <li id="l1" class="item">one</li>
    <li id="l2" class="item odd">two</li>
    <li id="l3" class="item">three</li>
    <li id="l4" class="item"><a id="link" href="/x" data-state="link hover">four</a></li>
  </ul>
  <p id="p1"></p>
  <span id="s1"><!-- only a comment --></span>
  <span id="s2">x</span>
  <input id="name" type="TEXT" data-state="control focus focus-visible editable required placeholder">
  <input id="check" type="checkbox" data-state="control checked">
  <input id="mixed" type="checkbox" data-state="control indeterminate">
  <button id="off" data-state="control disabled active">x</button>
  <form id="form" data-state="focus-within"><em id="em">x</em></form>
</div>
"#;

fn check(selector: &str, id: &str) -> bool {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let list = SelectorList::parse(selector).unwrap();
    list.matches(&tree.by_id(id))
}

#[test]
fn type_id_class_and_universal() {
    assert!(check("div", "app"));
    assert!(check("DIV", "app"));
    assert!(!check("span", "app"));
    assert!(check("*", "app"));
    assert!(check("#app", "app"));
    assert!(!check("#other", "app"));
    assert!(check(".container.main", "app"));
    assert!(!check(".container.missing", "app"));
    assert!(check("*.item", "l1"));
}

#[test]
fn attribute_selectors() {
    assert!(check("[data-kind]", "app"));
    assert!(!check("[missing]", "app"));
    assert!(check("[data-kind=Panel-x]", "app"));
    assert!(!check("[data-kind=panel-x]", "app"));
    assert!(check("[data-kind='panel-x' i]", "app"));
    assert!(check("[class~=main]", "app"));
    assert!(!check("[class~=mai]", "app"));
    assert!(check("[lang|=en]", "app"));
    assert!(check("[lang|=en-US]", "app"));
    assert!(!check("[lang|=e]", "app"));
    assert!(check("[data-kind^=Pan]", "app"));
    assert!(!check("[data-kind^='']", "app"));
    assert!(check("[data-kind$=-x]", "app"));
    assert!(!check("[data-kind$='']", "app"));
    assert!(check("[data-kind*=nel]", "app"));
    assert!(!check("[data-kind*='']", "app"));
    assert!(!check("[type=text s]", "name"));
}

#[test]
fn combinators() {
    assert!(check("div li", "l1"));
    assert!(check("div > ul > li", "l1"));
    assert!(!check("div > li", "l1"));
    assert!(check("li + li", "l2"));
    assert!(!check("li + li", "l1"));
    assert!(check("#l1 ~ li", "l3"));
    assert!(!check("#l3 ~ li", "l2"));
    assert!(check("ul li a", "link"));
    assert!(!check("p a", "link"));
}

#[test]
fn child_combinator_at_the_root() {
    assert!(!check("p > div", "app"));
}

#[test]
fn structural_pseudo_classes() {
    assert!(check("li:first-child", "l1"));
    assert!(!check("li:first-child", "l2"));
    assert!(check("li:last-child", "l4"));
    assert!(check("a:only-child", "link"));
    assert!(!check("li:only-child", "l1"));
    assert!(check("span:first-of-type", "s1"));
    assert!(check("span:last-of-type", "s2"));
    assert!(check("p:only-of-type", "p1"));
    assert!(!check("span:only-of-type", "s1"));
    assert!(check("li:nth-child(2n)", "l2"));
    assert!(check("li:nth-child(odd)", "l3"));
    assert!(check("li:nth-last-child(1)", "l4"));
    assert!(check("span:nth-of-type(2)", "s2"));
    assert!(check("span:nth-last-of-type(2)", "s1"));
    assert!(check("p:empty", "p1"));
    assert!(check("span:empty", "s1"));
    assert!(!check("span:empty", "s2"));
}

#[test]
fn root_and_scope() {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let root = Node {
        tree: &tree,
        index: 0,
    };
    assert!(SelectorList::parse(":root").unwrap().matches(&root));
    assert!(SelectorList::parse(":scope").unwrap().matches(&root));
    assert!(
        !SelectorList::parse(":root")
            .unwrap()
            .matches(&tree.by_id("app"))
    );
}

#[test]
fn state_pseudo_classes() {
    assert!(check("a:hover", "link"));
    assert!(check("a:link", "link"));
    assert!(check("a:any-link", "link"));
    assert!(!check("a:visited", "link"));
    assert!(!check(":target", "link"));
    assert!(check("button:active", "off"));
    assert!(check("input:focus", "name"));
    assert!(check("input:focus-visible", "name"));
    assert!(check("form:focus-within", "form"));
    assert!(check(":disabled", "off"));
    assert!(!check(":enabled", "off"));
    assert!(check(":enabled", "check"));
    assert!(!check(":enabled", "app"));
    assert!(check(":checked", "check"));
    assert!(check(":indeterminate", "mixed"));
    assert!(check(":required", "name"));
    assert!(!check(":optional", "name"));
    assert!(check(":optional", "check"));
    assert!(!check(":optional", "app"));
    assert!(check(":read-write", "name"));
    assert!(check(":read-only", "check"));
    assert!(check(":placeholder-shown", "name"));
}

#[test]
fn logical_pseudo_classes() {
    assert!(check("li:not(.odd)", "l1"));
    assert!(!check("li:not(.odd, #l3)", "l3"));
    assert!(check(":is(p, li.odd)", "l2"));
    assert!(check(":where(#l2)", "l2"));
    assert!(check(":matches(#l2)", "l2"));
    assert!(check(":-webkit-any(#l2)", "l2"));
    assert!(check(":is(.item, ., #l1)", "l1"));
    assert!(check(":has(a)", "l4"));
    assert!(!check(":lang(en)", "app"));
    assert!(!check(":future", "app"));
}

#[test]
fn pseudo_elements() {
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    let node = tree.by_id("name");
    let before = &SelectorList::parse("input::before").unwrap().0[0];
    assert!(!before.matches(&node));
    assert!(before.matches_pseudo(&node, &PseudoElement::Before));
    assert!(!before.matches_pseudo(&node, &PseudoElement::After));
    let cases = [
        ("a:before", PseudoElement::Before),
        ("a:after", PseudoElement::After),
        ("a::placeholder", PseudoElement::Placeholder),
        ("a::selection", PseudoElement::Selection),
        ("a:first-line", PseudoElement::FirstLine),
        ("a::first-letter", PseudoElement::FirstLetter),
        ("a::marker", PseudoElement::Marker),
        ("a::-webkit-scrollbar", PseudoElement::Scrollbar),
        ("a::part(x)", PseudoElement::Other("part".to_owned())),
        ("a::slotted(b)", PseudoElement::Slotted),
    ];
    for (text, expected) in cases {
        let list = SelectorList::parse(text).unwrap();
        assert_eq!(list.0[0].pseudo_element, Some(expected), "{text}");
    }
}

#[test]
fn specificity() {
    let of = |text: &str| SelectorList::parse(text).unwrap().0[0].specificity;
    assert_eq!(of("*"), Specificity(0, 0, 0));
    assert_eq!(of("li"), Specificity(0, 0, 1));
    assert_eq!(of("ul li.item"), Specificity(0, 1, 2));
    assert_eq!(of("#a.b[c]:hover"), Specificity(1, 3, 0));
    assert_eq!(of("a::before"), Specificity(0, 0, 2));
    assert_eq!(of(":not(#a, .b)"), Specificity(1, 0, 0));
    assert_eq!(of(":is(.a, p)"), Specificity(0, 1, 0));
    assert_eq!(of(":where(#a)"), Specificity(0, 0, 0));
    assert_eq!(of(":nth-child(2)"), Specificity(0, 1, 0));
    assert_eq!(of(":is()"), Specificity(0, 0, 0));
    assert_eq!(of("div:has(> #a, .b)"), Specificity(1, 0, 1));
}

#[test]
fn has_matches_related_elements() {
    // Descendants, at any depth.
    assert!(check("div:has(a)", "app"));
    assert!(check("ul:has(li a[href])", "list"));
    assert!(!check("ul:has(button)", "list"));
    assert!(!check("ul:has(div a)", "list"));
    // Children only.
    assert!(check("ul:has(> li.odd)", "list"));
    assert!(!check("div:has(> li)", "app"));
    assert!(check("div:has(> ul > li > a)", "app"));
    assert!(!check("div:has(> li > a)", "app"));
    // The next sibling, then any later sibling and what they contain.
    assert!(check("li:has(+ .odd)", "l1"));
    assert!(!check("li:has(+ .odd)", "l2"));
    assert!(check("li:has(~ li a)", "l1"));
    assert!(!check("li:has(~ li a)", "l4"));
    assert!(check("p:has(+ span ~ form em)", "p1"));
    assert!(!check("p:has(+ form)", "p1"));
    // Lists, nesting with other pseudo-classes, and leaf elements.
    assert!(check("ul:has(button, .odd)", "list"));
    assert!(check("div:not(:has(video))", "app"));
    assert!(!check("p:has(*)", "p1"));
    assert!(!check("li:has(~ li)", "l4"));
    assert!(SelectorList::parse("a:has()").is_err());
    assert!(SelectorList::parse("a:has(> )").is_err());
}

#[test]
fn invalid_selectors() {
    for text in [
        "",
        "a,",
        ".",
        ". a",
        "a >",
        "[",
        "[1]",
        "[a",
        "[a=]",
        "[a~b]",
        "[a=b c]",
        "[a=b",
        "[a=b i x]",
        ":",
        "::",
        "::1",
        ":1",
        ":nth-child(x)",
        ":nth-child(2n+x)",
        ":nth-of-type(an)",
        ":nth-last-child(1.5)",
        ":nth-last-of-type()",
        ":not(.)",
        "a::before b",
        "a)",
        "#123",
        "a|b",
    ] {
        assert!(SelectorList::parse(text).is_err(), "{text:?} should fail");
    }
    assert_eq!(
        SelectorList::parse("a)").unwrap_err(),
        "unexpected `)` in selector"
    );
    assert_eq!(SelectorList::parse("").unwrap_err(), "expected a selector");
}

#[test]
fn attribute_values_can_be_numbers_and_flags_are_optional() {
    let list = SelectorList::parse("[ data-x = 1 ]").unwrap();
    let SimpleSelector::Attribute(attribute) = &list.0[0].compounds[0].simple[0] else {
        panic!("expected an attribute selector");
    };
    assert_eq!(
        attribute.matcher,
        Some((AttributeOperator::Equals, "1".to_owned()))
    );
    assert!(!attribute.case_insensitive);
}

#[test]
fn nth_arguments() {
    let nth = |text: &str| parse_nth(&tokens(text));
    assert_eq!(nth("odd"), Some(Nth { a: 2, b: 1 }));
    assert_eq!(nth(" EVEN "), Some(Nth { a: 2, b: 0 }));
    assert_eq!(nth("3"), Some(Nth { a: 0, b: 3 }));
    assert_eq!(nth("n"), Some(Nth { a: 1, b: 0 }));
    assert_eq!(nth("+n"), Some(Nth { a: 1, b: 0 }));
    assert_eq!(nth("-n+3"), Some(Nth { a: -1, b: 3 }));
    assert_eq!(nth("2n - 1"), Some(Nth { a: 2, b: -1 }));
    assert_eq!(nth("2n + 1"), Some(Nth { a: 2, b: 1 }));
    assert_eq!(nth("x"), None);
    assert!(Nth { a: -1, b: 3 }.matches(3));
    assert!(!Nth { a: -1, b: 3 }.matches(4));
    assert!(Nth { a: 2, b: 1 }.matches(5));
    assert!(!Nth { a: 2, b: 1 }.matches(4));
}

#[test]
fn unsupported_pseudo_classes_are_reported() {
    let list = SelectorList::parse(
        "a:has(b:hover-ish), :not(:state(en)), :is(:popover-open), :where(.x), p:has(> q), :lang(en), :dir(rtl)",
    )
    .unwrap();
    assert_eq!(
        list.unsupported(),
        vec![":hover-ish", ":state", ":popover-open"]
    );
}

#[test]
fn selector_lists_serialize() {
    let list = SelectorList::parse("a > b").unwrap();
    let json = serde_json::to_string(&list).unwrap();
    assert_eq!(serde_json::from_str::<SelectorList>(&json).unwrap(), list);
}

#[test]
fn only_selectors_with_has_are_relational() {
    assert!(SelectorList::parse("a:has(b)").unwrap().is_relational());
    assert!(
        SelectorList::parse("a:not(:has(b))")
            .unwrap()
            .is_relational()
    );
    assert!(
        SelectorList::parse("a:is(b, c:has(d))")
            .unwrap()
            .is_relational()
    );
    assert!(
        !SelectorList::parse("a > b + c:nth-child(2):not(.x)")
            .unwrap()
            .is_relational()
    );
}

#[test]
fn relations_name_what_compounds_left_of_combinators_test() {
    fn names(set: &alloc::collections::BTreeSet<String>) -> Vec<&str> {
        set.iter().map(String::as_str).collect()
    }
    let mut relations = Relations::default();
    for selector in [
        ".a + .b",
        "#x ~ [data-y] .c",
        "input:checked ~ label",
        ":is(.p + .q)",
        "li:first-child + li:not(.r, :empty)",
        ".s:not(.t) ~ u",
        ".plain > .child .descendant:hover",
        ".menu:focus-within :is(.open .item)",
    ] {
        for selector in SelectorList::parse(selector).unwrap().0 {
            relations.add(&selector);
        }
    }
    let Relations {
        siblings,
        ancestors,
        structure,
    } = &relations;
    assert_eq!(
        *structure,
        super::structure::PRECEDING | super::structure::FIRST
    );
    assert_eq!(names(&siblings.classes), ["a", "p", "s", "t"]);
    assert!(siblings.attributes.is_empty());
    assert!(siblings.ids && siblings.states == state::CHECKED && !siblings.empty);
    assert_eq!(
        names(&ancestors.classes),
        ["child", "menu", "open", "plain"]
    );
    assert_eq!(names(&ancestors.attributes), ["data-y"]);
    assert!(!ancestors.ids && ancestors.states == state::FOCUS_WITHIN);
    let mut other = Relations::default();
    for selector in SelectorList::parse("[data-y] + a, x + :not(:empty), b:hover")
        .unwrap()
        .0
    {
        other.add(&selector);
    }
    assert_eq!(names(&other.siblings.attributes), ["data-y"]);
    assert!(!other.siblings.ids && other.siblings.states == 0 && !other.siblings.empty);
    assert_eq!(other.ancestors, RelatedFeatures::default());
    let mut places = Relations::default();
    for selector in SelectorList::parse(
        "li:only-child, :not(p:nth-last-child(2n)) a, b:nth-of-type(2), i:only-of-type, q:nth-last-of-type(1)",
    )
    .unwrap()
    .0
    {
        places.add(&selector);
    }
    use super::structure::*;
    assert_eq!(
        places.structure,
        FIRST
            | LAST
            | INDEX_FROM_LAST
            | INDEX_OF_TYPE
            | FIRST_OF_TYPE
            | LAST_OF_TYPE
            | INDEX_OF_TYPE_FROM_LAST
    );
    assert_eq!(
        super::structure_of(&PseudoClass::NthChild(Nth { a: 0, b: 1 })),
        INDEX
    );
}

/// `:nth-child(An+B of S)` and `:nth-last-child(An+B of S)` count only the
/// siblings `S` matches, and the element must match `S`; their specificity
/// adds the most specific of `S`.
#[test]
fn nth_child_of_selectors_count_matching_siblings() {
    // `.item` siblings: l1..l4; `:not(.odd)` ones: l1, l3, l4.
    assert!(check("li:nth-child(2 of :not(.odd))", "l3"));
    assert!(!check("li:nth-child(2 of :not(.odd))", "l2"));
    assert!(check("li:nth-last-child(1 of :not([hidden]))", "l4"));
    assert!(check("li:nth-last-child(3 of .item:not(.odd))", "l1"));
    assert!(check("li:nth-child(odd of .item)", "l3"));
    assert!(!check("li:nth-child(1 of .odd)", "l1"));
    assert!(check("li:nth-child(1 of .odd)", "l2"));
    assert_eq!(
        SelectorList::parse(":nth-child(1 of #a, .b)").unwrap().0[0].specificity,
        Specificity(1, 1, 0)
    );
    assert!(SelectorList::parse(":nth-child(1 of)").is_err());
    assert!(SelectorList::parse(":nth-child(x of .a)").is_err());
}

/// Lists inside `:has()` and `:is()` split only at their own commas, so
/// nested lists such as `:has(.a:is(.b, .c))` parse.
#[test]
fn nested_selector_lists_keep_their_commas() {
    assert!(check("ul:has(li:is(.odd, .missing))", "list"));
    assert!(check(
        "ul:has(> li:is(.none, .item) a:is([href], .x))",
        "list"
    ));
    assert!(!check("ul:has(li:is(.none, .missing))", "list"));
    assert!(check(":is(:is(li, p) > a, em)", "link"));
    assert!(check(":is(:is(li, p) > a, em)", "em"));
}

#[test]
fn shadow_selectors_parse_with_their_specificity() {
    let list =
        SelectorList::parse(":host, :host(.a#b), :host-context(.dark) .menu, slot::slotted(b.c)")
            .unwrap();
    let [host, host_of, context, slotted] = &list.0[..] else {
        panic!("{list:?}");
    };
    assert_eq!(
        host.compounds[0].simple,
        [SimpleSelector::PseudoClass(PseudoClass::Host(None))]
    );
    assert_eq!(host.specificity, Specificity(0, 1, 0));
    assert_eq!(host_of.specificity, Specificity(1, 2, 0));
    assert_eq!(context.specificity, Specificity(0, 3, 0));
    assert_eq!(slotted.pseudo_element, Some(PseudoElement::Slotted));
    assert_eq!(slotted.specificity, Specificity(0, 1, 3));
    assert_eq!(
        slotted.slotted.as_ref().unwrap().0[0].compounds[0]
            .tag
            .as_deref(),
        Some("b")
    );
    for invalid in [
        ":host(.a .b)",
        ":host-context(a > b)",
        "::slotted(a b)",
        "::slotted(a::before)",
    ] {
        assert!(SelectorList::parse(invalid).is_err(), "{invalid}");
    }
    // Outside a shadow tree's own rules, nothing is a host.
    let document = parse_document(PAGE);
    let tree = Tree::new(&document);
    assert!(
        !SelectorList::parse(":host")
            .unwrap()
            .matches(&tree.by_id("app"))
    );
    assert!(
        !SelectorList::parse(":host-context(*)")
            .unwrap()
            .matches(&tree.by_id("app"))
    );
}
