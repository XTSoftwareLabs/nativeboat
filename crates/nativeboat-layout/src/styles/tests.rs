use super::*;
use nativeboat_css::{Color, Origin, parse_stylesheet};

#[test]
fn retained_styles_refresh_matches_inheritance_reparenting_and_environment() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=a><p id=p>text</p><input id=i placeholder=x></div><div id=b><svg><path id=s fill=red /></svg></div>",
    ));
    let (sheet, diagnostics) = parse_stylesheet(
        "#a {color:red;--tone:red} #b {color:blue} p {background:var(--tone)} #a:has(.flag) p {color:green} p:empty {opacity:.5} p::before {content:'x';color:inherit} p.on::after {content:'y'} ::placeholder {color:gray}",
    );
    assert!(diagnostics.is_empty());
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let mut environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let p = id(&dom, "p");
    let a = id(&dom, "a");
    let b = id(&dom, "b");
    let s = id(&dom, "s");
    let first = Styles::compute(&dom, &sheets, &environment);
    dom.set_data(dom.first_child(p).unwrap(), "typing");
    let retained = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert_eq!(first.selector_state, retained.selector_state);
    let mut presentation = retained.clone();
    presentation.get_mut(p).unwrap().opacity = 0.3;
    let authored = Styles::compute_reusing(&dom, &sheets, &environment, Some(&presentation));
    assert_eq!(authored.get(p).unwrap().opacity, 1.0);
    for (node, _) in first.iter() {
        assert!(Rc::ptr_eq(
            first.get(node).unwrap(),
            retained.get(node).unwrap()
        ));
    }
    assert!(Rc::ptr_eq(
        first.pseudo(p, &PseudoElement::Before).unwrap(),
        retained.pseudo(p, &PseudoElement::Before).unwrap()
    ));
    assert!(retained.pseudo(p, &PseudoElement::After).is_none());
    assert!(Rc::ptr_eq(
        first
            .pseudo(id(&dom, "i"), &PseudoElement::Placeholder)
            .unwrap(),
        retained
            .pseudo(id(&dom, "i"), &PseudoElement::Placeholder)
            .unwrap()
    ));
    dom.set_attribute(p, "class", "flag on").unwrap();
    let changed = Styles::compute_reusing(&dom, &sheets, &environment, Some(&retained));
    assert_eq!(changed.get(p).unwrap().color, Color::rgb(0, 128, 0));
    assert!(changed.pseudo(p, &PseudoElement::After).is_some());
    assert!(Rc::ptr_eq(
        changed.get(b).unwrap(),
        retained.get(b).unwrap()
    ));
    dom.set_attribute(a, "style", "--tone:purple;color:purple")
        .unwrap();
    let inherited = Styles::compute_reusing(&dom, &sheets, &environment, Some(&changed));
    assert_eq!(
        inherited.get(p).unwrap().background_color,
        Color::rgb(128, 0, 128)
    );
    dom.remove_attribute(p, "class");
    dom.append_child(b, p).unwrap();
    let moved = Styles::compute_reusing(&dom, &sheets, &environment, Some(&inherited));
    assert_eq!(moved.get(p).unwrap().color, Color::rgb(0, 0, 255));
    dom.set_text_content(p, "");
    dom.set_attribute(s, "fill", "green").unwrap();
    let empty = Styles::compute_reusing(&dom, &sheets, &environment, Some(&moved));
    assert_eq!(empty.get(p).unwrap().opacity, 0.5);
    assert_ne!(empty.get(s).unwrap().fill, moved.get(s).unwrap().fill);
    environment.media.width += 10.0;
    let resized = Styles::compute_reusing(&dom, &sheets, &environment, Some(&empty));
    assert!(!Rc::ptr_eq(resized.get(p).unwrap(), empty.get(p).unwrap()));
    dom.remove_child(b, p).unwrap();
    let removed = Styles::compute_reusing(&dom, &sheets, &environment, Some(&resized));
    assert!(removed.get(p).is_none());
    assert!(!removed.retained.contains_key(&p));
    assert_eq!(removed.retained.len(), removed.len());
}

#[test]
fn scrollbar_parts_compute_variables_axes_and_independent_interaction_states() {
    let (dom, styles) = styles_for(
        "<div id=a></div><div id=b></div>",
        r#"
        #a { overflow:auto; --thumb:red; color:blue }
        ::-webkit-scrollbar { background:#eee }
        #a::-webkit-scrollbar:vertical { width:12px }
        #a::-webkit-scrollbar:horizontal { height:7px }
        #a::-webkit-scrollbar-thumb { background:var(--thumb); border:1px solid currentcolor; border-radius:50%; margin:2px; opacity:.5 }
        #a::-webkit-scrollbar-thumb:hover { background:green }
        #a::-webkit-scrollbar-thumb:active { background:blue }
        #a::-webkit-scrollbar-track { background:yellow }
        #a::-webkit-scrollbar-corner { background:black }
    "#,
    );
    let a = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let b = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    assert!(styles.scrollbars(b).is_none());
    let bars = styles.scrollbars(a).unwrap();
    assert_eq!(
        bars.axes[0].thickness,
        nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(7.0))
    );
    assert_eq!(
        bars.axes[1].thickness,
        nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(12.0))
    );
    let thumb = &bars.axes[1].thumb.0;
    assert_eq!(
        thumb[0].background,
        Color {
            a: 0.5,
            ..Color::rgb(255, 0, 0)
        }
    );
    assert_eq!(
        thumb[1].background,
        Color {
            a: 0.5,
            ..Color::rgb(0, 128, 0)
        }
    );
    assert_eq!(
        thumb[2].background,
        Color {
            a: 0.5,
            ..Color::rgb(0, 0, 255)
        }
    );
    assert_eq!(thumb[0].widths, [1.0; 4]);
    assert_eq!(
        thumb[0].colors,
        [Color {
            a: 0.5,
            ..Color::rgb(0, 0, 255)
        }; 4]
    );
    assert_eq!(bars.corner.0[0].background, Color::BLACK);
    assert!(styles.scrollbars(dom.document_element().unwrap()).is_some());
}

#[test]
fn standard_scrollbars_override_parts_without_changing_scrollability() {
    let (dom, styles) = styles_for(
        "<div id=a><div id=b></div></div><div id=c></div><div id=d></div>",
        r#"
        #a { overflow:scroll; scrollbar-width:thin; scrollbar-color:red blue }
        #b { overflow:auto }
        #c { overflow:auto; scrollbar-width:none }
        #d { overflow:scroll }
        #a::-webkit-scrollbar:vertical { width:40px }
        #c::-webkit-scrollbar-thumb { background:green }
        #d::-webkit-scrollbar { display:none }
    "#,
    );
    let id = |name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let a = styles.scrollbars(id("a")).unwrap();
    assert!(a.axes[0].force);
    assert!(a.axes[1].enabled);
    assert_eq!(
        a.axes[1].thickness,
        nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(8.0))
    );
    assert_eq!(a.axes[1].thumb.0[1].background, Color::rgb(255, 0, 0));
    assert_eq!(a.axes[1].bar.0[0].background, Color::rgb(0, 0, 255));
    assert_eq!(
        styles.get(id("b")).unwrap().scrollbar_width,
        nativeboat_css::types::ScrollbarWidth::Auto
    );
    assert_eq!(
        styles.scrollbars(id("b")).unwrap().axes[1].thumb.0[0].background,
        Color::rgb(255, 0, 0)
    );
    for name in ["c", "d"] {
        assert_eq!(
            styles.scrollbars(id(name)).unwrap().axes[1].thickness,
            nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(0.0))
        );
    }
    let (dom, styles) = styles_for("<p>plain</p>", "::before{content:'x'}");
    assert!(styles.scrollbars(dom.document_element().unwrap()).is_none());
}

fn styles_for(html: &str, css: &str) -> (Dom, Styles) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let (sheet, _) = parse_stylesheet(css);
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    (dom, styles)
}

#[test]
fn every_element_gets_a_style() {
    let (dom, styles) = styles_for(
        "<div id=a style='width: 2rem'><span id=b>x</span></div>",
        "html { font-size: 10px } span { color: red }",
    );
    assert_eq!(styles.len(), 5);
    assert!(!styles.is_empty());
    assert_eq!(styles.environment().root_font_size, 10.0);
    let div = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let span = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    assert_eq!(
        styles.get(div).unwrap().width,
        nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(20.0))
    );
    assert_eq!(
        styles.get(span).unwrap().color,
        nativeboat_css::Color::rgb(255, 0, 0)
    );
    assert!(styles.pseudo(div, &PseudoElement::Before).is_none());
}

#[test]
fn table_attributes_are_presentational_hints() {
    let (dom, styles) = styles_for(
        "<table id=t cellspacing=' 4px' cellpadding=3 align=center bgcolor=red>\
         <tr id=r align=RIGHT valign=top><td id=c nowrap>a</td><th id=h valign=sideways>b</th></tr>\
         <tr><td><table cellpadding=junk><tr><td id=n>c</td></tr></table></td></tr></table>\
         <table id=a cellspacing=x align=left></table><table id=f align=right></table>",
        "#h { padding-left: 9px }",
    );
    let style = |id: &str| {
        styles
            .get(dom.get_element_by_id(Dom::DOCUMENT, id).unwrap())
            .unwrap()
    };
    let table = style("t");
    assert_eq!(
        (
            table.border_spacing.horizontal.0,
            table.border_spacing.vertical.0
        ),
        (4.0, 4.0)
    );
    assert_eq!(
        table.background_color,
        nativeboat_css::Color::rgb(255, 0, 0)
    );
    assert_eq!(
        table.margin.left,
        nativeboat_css::types::LengthPercentageAuto::Auto
    );
    assert_eq!(style("a").float, nativeboat_css::types::Float::Left);
    assert_eq!(
        style("a").border_spacing,
        nativeboat_css::types::Spacing::default()
    );
    assert_eq!(style("f").float, nativeboat_css::types::Float::Right);
    let row = style("r");
    assert_eq!(row.text_align, nativeboat_css::types::TextAlign::Right);
    assert_eq!(
        row.vertical_align,
        nativeboat_css::types::VerticalAlign::Top
    );
    let cell = style("c");
    assert_eq!(
        cell.padding.top,
        nativeboat_css::values::LengthPercentage::Px(3.0)
    );
    assert_eq!(cell.white_space, nativeboat_css::types::WhiteSpace::NoWrap);
    // Author styles win over hints; unknown values hint nothing.
    let header = style("h");
    assert_eq!(
        header.padding.left,
        nativeboat_css::values::LengthPercentage::Px(9.0)
    );
    assert_eq!(
        header.padding.top,
        nativeboat_css::values::LengthPercentage::Px(3.0)
    );
    assert_ne!(
        header.vertical_align,
        nativeboat_css::types::VerticalAlign::Top
    );
    // A cell takes the padding of its own table, not an outer one.
    assert_eq!(
        style("n").padding.top,
        nativeboat_css::values::LengthPercentage::Px(0.0)
    );
}

#[test]
fn presentation_styles_preserve_shared_snapshots() {
    let (dom, mut styles) = styles_for("<p id=p>x</p>", "p { opacity: .8 }");
    let node = dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap();
    let original = styles.clone();
    assert_eq!(styles.iter().count(), styles.len());
    styles.get_mut(node).unwrap().opacity = 0.2;
    assert_eq!(styles.get(node).unwrap().opacity, 0.2);
    assert_eq!(original.get(node).unwrap().opacity, 0.8);
    assert!(styles.get_mut(NodeId(u32::MAX)).is_none());
}

#[test]
fn pseudo_elements_are_computed_when_rules_exist() {
    for css in [
        "p::before { content: '*' } p::after { content: 'x' } input::placeholder { color: red }",
        "@media all { p::before { content: '*' } }",
        "@supports (display: grid) { p::before { content: '*' } }",
        "@layer a { p::before { content: '*' } }",
    ] {
        let (dom, styles) = styles_for("<p id=p>x</p><input id=i>", css);
        let paragraph = dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap();
        assert!(
            styles.pseudo(paragraph, &PseudoElement::Before).is_some(),
            "{css}"
        );
    }
    let (dom, styles) = styles_for(
        "<p id=p>x</p><input id=i>",
        "p::after { content: 'x' } input::placeholder { color: red } @font-face { font-family: x } a::marker {}",
    );
    let paragraph = dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap();
    let input = dom.get_element_by_id(Dom::DOCUMENT, "i").unwrap();
    assert!(styles.pseudo(paragraph, &PseudoElement::After).is_some());
    assert!(styles.pseudo(input, &PseudoElement::Placeholder).is_some());
    let (_, plain) = styles_for("<p>x</p>", "@font-face { font-family: x } a::marker {}");
    assert!(plain.pseudo.is_empty());
}

#[test]
fn documents_without_elements_have_no_styles() {
    let styles = Styles::compute(&Dom::new(), &[], &StyleEnvironment::default());
    assert!(styles.is_empty());
    assert_eq!(styles.environment().root_font_size, 16.0);
}

#[test]
fn hidden_elements_are_styled_on_demand() {
    let html = "<div id=shown>a</div><div id=hidden hidden><p id=inside><b id=deep>b</b></p></div>\
        <svg style='display:none'><symbol id=icon><path id=shape d='M0 0'/></symbol><g><rect id='other'/></g></svg>\
        <svg><use href='#icon'/><use xlink:href='#gone'/><use href='icon'/><use/></svg>\
        <p id=para>x</p><input id=field>";
    let css = "[hidden] { display: none } div { color: red } p { color: blue } \
               b { font-size: 20px } ::placeholder { color: gray }";
    let (mut dom, styles) = styles_for(html, css);
    let id = |dom: &Dom, name: &str| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    // The descendants of hidden elements are left out.
    assert!(styles.get(id(&dom, "hidden")).is_some());
    assert!(styles.get(id(&dom, "inside")).is_none());
    assert!(styles.get(id(&dom, "deep")).is_none());
    // What `<use>` shows is styled even inside a hidden element.
    assert!(styles.get(id(&dom, "shape")).is_some());
    assert!(styles.get(id(&dom, "other")).is_none());
    // Hidden elements get their styles when asked.
    let (author, _) = parse_stylesheet(css);
    let sheets = [SheetRef {
        sheet: &author,
        origin: Origin::Author,
    }];
    let deep = styles.style_of(&dom, &sheets, id(&dom, "deep")).unwrap();
    assert_eq!(deep.color, nativeboat_css::Color::rgb(0, 0, 255));
    assert_eq!(deep.font_size.0, 20.0);
    // Hidden SVG elements are styled with their presentation attributes.
    assert!(styles.style_of(&dom, &sheets, id(&dom, "other")).is_some());
    let shown = id(&dom, "shown");
    assert!(Rc::ptr_eq(
        &styles.style_of(&dom, &sheets, shown).unwrap(),
        styles.get(shown).unwrap()
    ));
    // Elements outside the document have none.
    let loose = dom.create_element("i").unwrap();
    assert!(styles.style_of(&dom, &sheets, loose).is_none());
    // Only text fields get placeholder styles.
    assert!(
        styles
            .pseudo(id(&dom, "para"), &PseudoElement::Placeholder)
            .is_none()
    );
    assert!(
        styles
            .pseudo(id(&dom, "field"), &PseudoElement::Placeholder)
            .is_some()
    );
}

#[test]
fn reused_scroll_containers_keep_scrollbars_until_their_part_rules_change() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document("<div id=a>text</div>"));
    let (sheet, _) = parse_stylesheet(
        "#a { overflow:auto } #a::-webkit-scrollbar:vertical { width:12px } #a.wide::-webkit-scrollbar:vertical { width:20px }",
    );
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let a = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let width = |styles: &Styles| styles.scrollbars(a).unwrap().axes[1].thickness.clone();
    let first = Styles::compute(&dom, &sheets, &environment);
    let px =
        |value| nativeboat_css::types::Size::Length(nativeboat_css::LengthPercentage::Px(value));
    assert_eq!(width(&first), px(12.0));
    // Text changes reuse the container's style and its scrollbars.
    dom.set_data(dom.first_child(a).unwrap(), "typing");
    let typed = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert!(Rc::ptr_eq(first.get(a).unwrap(), typed.get(a).unwrap()));
    assert_eq!(width(&typed), px(12.0));
    // A class only a scrollbar rule names changes the scrollbars alone.
    dom.set_attribute(a, "class", "wide").unwrap();
    let wide = Styles::compute_reusing(&dom, &sheets, &environment, Some(&typed));
    assert_eq!(width(&wide), px(20.0));
}

#[test]
fn elements_in_unchanged_selector_contexts_keep_their_matches() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=d><p id=p>text</p></div><ul id=u><li id=one>1</li><li id=two>2</li></ul><span id=s></span>",
    ));
    let (sheet, diagnostics) = parse_stylesheet(
        "div .on {color:red} .wrap p {opacity:.25} li:nth-child(2) {color:blue} li:last-child {opacity:.5} span {color:green}",
    );
    assert!(diagnostics.is_empty());
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (d, p, u, one, two, s) = (
        id(&dom, "d"),
        id(&dom, "p"),
        id(&dom, "u"),
        id(&dom, "one"),
        id(&dom, "two"),
        id(&dom, "s"),
    );
    let first = Styles::compute(&dom, &sheets, &environment);
    let matched = |styles: &Styles, node| styles.retained.get(&node).unwrap().matched.clone();
    // An element's own class changes what it matches; elements in other
    // contexts keep their matches.
    dom.set_attribute(p, "class", "on").unwrap();
    let on = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert_eq!(on.get(p).unwrap().color, Color::rgb(255, 0, 0));
    assert!(!Rc::ptr_eq(&matched(&first, p), &matched(&on, p)));
    for node in [d, u, one, two, s] {
        assert!(Rc::ptr_eq(&matched(&first, node), &matched(&on, node)));
    }
    // An ancestor's class changes what its descendants match.
    dom.set_attribute(d, "class", "wrap").unwrap();
    let wrapped = Styles::compute_reusing(&dom, &sheets, &environment, Some(&on));
    assert_eq!(wrapped.get(p).unwrap().opacity, 0.25);
    // A new sibling changes what its siblings match.
    let three = dom.create_element("li").unwrap();
    dom.append_child(u, three).unwrap();
    let added = Styles::compute_reusing(&dom, &sheets, &environment, Some(&wrapped));
    assert_eq!(added.get(two).unwrap().opacity, 1.0);
    assert_eq!(added.get(three).unwrap().opacity, 0.5);
    assert_eq!(added.get(two).unwrap().color, Color::rgb(0, 0, 255));
}

#[test]
fn flex_items_floats_and_absolute_boxes_compute_block_displays() {
    let (dom, styles) = styles_for(
        "<div id=flex style='display:flex'><span id=item>x</span><span id=box style='display:inline-flex'></span></div>\
         <span id=float style='float:left'></span><span id=absolute style='position:absolute;display:inline-grid'></span>\
         <span id=plain></span><div id=grid style='display:inline-grid'><i id=cell></i></div>",
        "",
    );
    let display = |id| {
        let node = dom.get_element_by_id(Dom::DOCUMENT, id).unwrap();
        styles.get(node).unwrap().display
    };
    assert_eq!(display("item"), Display::Block);
    assert_eq!(display("box"), Display::Flex);
    assert_eq!(display("float"), Display::Block);
    assert_eq!(display("absolute"), Display::Grid);
    assert_eq!(display("plain"), Display::Inline);
    assert_eq!(display("cell"), Display::Block);
}

#[test]
fn siblings_keep_their_matches_unless_a_sibling_selector_tests_the_change() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div><p id=a>a</p><p id=b>b</p><p id=c>c</p></div>",
    ));
    let (sheet, _) =
        parse_stylesheet(".on {color:red} .lead + p {color:blue} p:first-child {margin:0}");
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (a, b, c) = (id(&dom, "a"), id(&dom, "b"), id(&dom, "c"));
    let matched = |styles: &Styles, node| styles.retained[&node].matched.clone();
    let first = Styles::compute(&dom, &sheets, &environment);
    // No `+` or `~` selector tests `on`: the siblings are not matched again.
    dom.set_attribute(a, "class", "on").unwrap();
    let second = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert_eq!(second.get(a).unwrap().color, Color::rgb(255, 0, 0));
    assert!(Rc::ptr_eq(&matched(&first, b), &matched(&second, b)));
    assert!(Rc::ptr_eq(first.get(b).unwrap(), second.get(b).unwrap()));
    // `lead` is tested through `+`.
    dom.set_attribute(a, "class", "on lead").unwrap();
    let third = Styles::compute_reusing(&dom, &sheets, &environment, Some(&second));
    assert_eq!(third.get(b).unwrap().color, Color::rgb(0, 0, 255));
    assert_eq!(third.get(c).unwrap().color, Color::BLACK);
    dom.set_attribute(a, "class", "on").unwrap();
    let fourth = Styles::compute_reusing(&dom, &sheets, &environment, Some(&third));
    assert_eq!(fourth.get(b).unwrap().color, Color::BLACK);
}

#[test]
fn descendants_keep_their_matches_unless_an_ancestor_selector_tests_the_change() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=d><p id=p><span id=s tabindex=0>a</span></p><p id=q><span id=t>b</span></p></div>",
    ));
    let (sheet, _) = parse_stylesheet(".on {color:red} .dark span {color:blue}");
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (d, s, q, t) = (id(&dom, "d"), id(&dom, "s"), id(&dom, "q"), id(&dom, "t"));
    let matched = |styles: &Styles, node| styles.retained[&node].matched.clone();
    let first = Styles::compute(&dom, &sheets, &environment);
    // No compound left of a combinator tests `on` or the focus: elements
    // off the focused path are not matched again.
    dom.set_attribute(d, "class", "on").unwrap();
    assert!(dom.set_focus(Some(s), false).is_some());
    let second = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert_eq!(second.get(d).unwrap().color, Color::rgb(255, 0, 0));
    assert!(Rc::ptr_eq(&matched(&first, q), &matched(&second, q)));
    assert!(Rc::ptr_eq(&matched(&first, t), &matched(&second, t)));
    assert!(!Rc::ptr_eq(&matched(&first, s), &matched(&second, s)));
    // `dark` is.
    dom.set_attribute(d, "class", "on dark").unwrap();
    let third = Styles::compute_reusing(&dom, &sheets, &environment, Some(&second));
    assert_eq!(third.get(s).unwrap().color, Color::rgb(0, 0, 255));
}

#[test]
fn an_inserted_sibling_rematches_only_the_siblings_whose_place_selectors_test() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<ul id=u><li id=a><b id=ab>a</b></li><li id=b>b</li><li id=c>c</li></ul>\
         <ol id=o><li id=x>x</li><li id=y>y</li></ol><div id=t><i id=i></i><p id=p></p></div>",
    ));
    let (sheet, _) = parse_stylesheet(
        "ul li:last-child {color:red} ol li:nth-child(2) {color:blue} ol li:nth-child(3) {opacity:.5} \
         div p:first-of-type {color:green} li:first-child b {opacity:.25}",
    );
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (u, a, ab, b, c) = (
        id(&dom, "u"),
        id(&dom, "a"),
        id(&dom, "ab"),
        id(&dom, "b"),
        id(&dom, "c"),
    );
    let (o, x, y, t, p) = (
        id(&dom, "o"),
        id(&dom, "x"),
        id(&dom, "y"),
        id(&dom, "t"),
        id(&dom, "p"),
    );
    let matched = |styles: &Styles, node| styles.retained[&node].matched.clone();
    let first = Styles::compute(&dom, &sheets, &environment);
    assert_eq!(first.get(c).unwrap().color, Color::rgb(255, 0, 0));
    assert_eq!(first.get(ab).unwrap().opacity, 0.25);
    // Appended: only the last child's place changes; the first child and
    // its descendants keep their matches.
    let d = dom.create_element("li").unwrap();
    dom.append_child(u, d).unwrap();
    let appended = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert_eq!(appended.get(c).unwrap().color, Color::BLACK);
    assert_eq!(appended.get(d).unwrap().color, Color::rgb(255, 0, 0));
    for node in [a, ab, b] {
        assert!(Rc::ptr_eq(
            &matched(&first, node),
            &matched(&appended, node)
        ));
    }
    // Inserted first: indexes move after it.
    let w = dom.create_element("li").unwrap();
    dom.insert_before(o, w, Some(x)).unwrap();
    let inserted = Styles::compute_reusing(&dom, &sheets, &environment, Some(&appended));
    assert_eq!(inserted.get(x).unwrap().color, Color::rgb(0, 0, 255));
    assert_eq!(inserted.get(y).unwrap().opacity, 0.5);
    assert_eq!(inserted.get(y).unwrap().color, Color::BLACK);
    // Another type before it leaves the first of its type in place.
    let before = dom.create_element("p").unwrap();
    dom.insert_before(t, before, Some(p)).unwrap();
    let typed = Styles::compute_reusing(&dom, &sheets, &environment, Some(&inserted));
    assert_eq!(typed.get(before).unwrap().color, Color::rgb(0, 128, 0));
    assert_eq!(typed.get(p).unwrap().color, Color::BLACK);
}

#[test]
fn only_boxes_that_can_show_scrollbars_style_them() {
    let (dom, styles) = styles_for(
        "<div id=hidden style='overflow:hidden'></div><div id=auto style='overflow:auto'></div><div id=clip style='overflow-x:clip;overflow-y:scroll'></div>",
        "::-webkit-scrollbar { width: 6px } ::-webkit-scrollbar-thumb { background: red }",
    );
    let id = |name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    assert!(styles.scrollbars(id("hidden")).is_none());
    assert!(styles.scrollbars(id("auto")).is_some());
    assert!(styles.scrollbars(id("clip")).is_some());
}

#[test]
fn ancestor_states_matter_only_on_ancestors_that_can_match_the_rest_of_their_compound() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=menu class=menu><span id=item class=item tabindex=0>a</span></div>\
         <div id=other><span id=plain tabindex=0>b</span></div>",
    ));
    let (sheet, _) = parse_stylesheet(".menu:focus-within .item {color:red}");
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (item, plain) = (id(&dom, "item"), id(&dom, "plain"));
    let matched = |styles: &Styles, node| styles.retained[&node].matched.clone();
    let first = Styles::compute(&dom, &sheets, &environment);
    // Focus within `#other`, and within the document's root and body, which
    // no `.menu` compound matches: the menu's item keeps its matches.
    assert!(dom.set_focus(Some(plain), false).is_some());
    let second = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert!(Rc::ptr_eq(&matched(&first, item), &matched(&second, item)));
    // Focus within the menu restyles its item.
    assert!(dom.set_focus(Some(item), false).is_some());
    let third = Styles::compute_reusing(&dom, &sheets, &environment, Some(&second));
    assert_eq!(third.get(item).unwrap().color, Color::rgb(255, 0, 0));
}

#[test]
fn children_keep_their_styles_when_their_parent_changes_nothing_they_take_from_it() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=p><span id=c>a</span><span id=i class=inherit>b</span></div>",
    ));
    let (sheet, _) = parse_stylesheet(
        ".boxed {background-color:red} .tinted {color:blue} .inherit {background-color:inherit}",
    );
    let sheets = [SheetRef {
        sheet: &sheet,
        origin: Origin::Author,
    }];
    let environment = StyleEnvironment::default();
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let (p, c, i) = (id(&dom, "p"), id(&dom, "c"), id(&dom, "i"));
    let first = Styles::compute(&dom, &sheets, &environment);
    // A background is not inherited: the child keeps its style, the one
    // that inherits it explicitly takes the new one.
    dom.set_attribute(p, "class", "boxed").unwrap();
    let boxed = Styles::compute_reusing(&dom, &sheets, &environment, Some(&first));
    assert!(Rc::ptr_eq(first.get(c).unwrap(), boxed.get(c).unwrap()));
    assert_eq!(
        boxed.get(i).unwrap().background_color,
        Color::rgb(255, 0, 0)
    );
    // A color is inherited.
    dom.set_attribute(p, "class", "boxed tinted").unwrap();
    let tinted = Styles::compute_reusing(&dom, &sheets, &environment, Some(&boxed));
    assert_eq!(tinted.get(c).unwrap().color, Color::rgb(0, 0, 255));
}

#[test]
fn shadow_trees_match_their_own_rules_their_hosts_and_slotted_elements() {
    use nativeboat_dom::ShadowRootMode;
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=host class=dark><b id=light slot=title>l</b><i id=plain>p</i></div><p id=outside class=inner>o</p>",
    ));
    let host = dom.get_element_by_id(Dom::DOCUMENT, "host").unwrap();
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    dom.set_inner_html(
        root,
        "<p class=inner id=inner>i</p><slot name=title></slot><span class=wrap id=wrap><em id=em>e</em></span>",
    );
    let (document_sheet, _) = parse_stylesheet(
        ".inner { color: red } #host { opacity: .5 } span em { color: purple } #light { opacity: .5 }",
    );
    let (mut shadow_sheet, diagnostics) = parse_stylesheet(
        ":host { opacity: .2; color: blue } :host(.dark) { background-color: black } \
         :host-context(body) .wrap { opacity: .25 } .inner { color: green } \
         ::slotted(b) { opacity: .75 !important } #host { background-color: red }",
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    shadow_sheet.scope = root.0;
    let sheets = [
        SheetRef {
            sheet: &document_sheet,
            origin: Origin::Author,
        },
        SheetRef {
            sheet: &shadow_sheet,
            origin: Origin::Author,
        },
    ];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let style = |scope: NodeId, name: &str| {
        styles
            .get(dom.get_element_by_id(scope, name).unwrap())
            .unwrap()
            .clone()
    };
    // Each tree's rules apply in it alone.
    assert_eq!(style(root, "inner").color, Color::rgb(0, 128, 0));
    assert_eq!(style(Dom::DOCUMENT, "outside").color, Color::rgb(255, 0, 0));
    assert_ne!(style(root, "em").color, Color::rgb(128, 0, 128));
    // The host takes `:host` rules, but the document's own win; rules of
    // the shadow tree that are not `:host` rules never match it.
    let host_style = styles.get(host).unwrap();
    assert_eq!(host_style.opacity, 0.5);
    assert_eq!(host_style.color, Color::rgb(0, 0, 255));
    assert_eq!(host_style.background_color, Color::rgb(0, 0, 0));
    // Shadow trees inherit from their hosts and match `:host-context()`.
    assert_eq!(style(root, "inner").opacity, 1.0);
    assert_eq!(style(root, "wrap").opacity, 0.25);
    assert_eq!(style(root, "em").color, Color::rgb(0, 0, 255));
    // A slotted element takes `::slotted()` rules; important ones of the
    // inner tree win.
    assert_eq!(style(Dom::DOCUMENT, "light").opacity, 0.75);
    // A host child no slot shows is not rendered.
    assert!(
        styles
            .get(dom.get_element_by_id(Dom::DOCUMENT, "plain").unwrap())
            .is_none()
    );
}
