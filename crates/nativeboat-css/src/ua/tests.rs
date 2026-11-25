use super::*;
use crate::cascade::{Origin, SheetRef, compute_style};
use crate::computed::StyleEnvironment;
use crate::tests::dom::Tree;
use crate::types::{
    Display, FontWeight, LengthPercentageAuto, ListStylePosition, ListStyleType, Px,
};
use crate::values::LengthPercentage;
use nativeboat_html::parse_document;

#[test]
fn the_sheet_parses_cleanly() {
    let (_, diagnostics) = parse_stylesheet(USER_AGENT_CSS);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(std::ptr::eq(
        user_agent_stylesheet(),
        user_agent_stylesheet()
    ));
}

#[test]
fn defaults_match_browsers() {
    let document = parse_document(
        "<body id=b><h1 id=h>t</h1><a id=a href=x>l</a><span id=s hidden>x</span><input id=i><ul id=u></ul><details><summary id=closed>C</summary></details><details open><summary id=open>O</summary><summary id=second>S</summary></details><summary id=alone>A</summary></body>",
    );
    let tree = Tree::new(&document);
    let sheets = [SheetRef {
        sheet: user_agent_stylesheet(),
        origin: Origin::UserAgent,
    }];
    let environment = StyleEnvironment::default();
    let compute = |id: &str| {
        let mut chain = Vec::new();
        let mut node = Some(tree.by_id(id));
        while let Some(current) = node {
            node = crate::selector::Element::parent_element(&current);
            chain.push(current);
        }
        let mut parent = None;
        for node in chain.into_iter().rev() {
            parent = Some(compute_style(
                &node,
                &sheets,
                &[],
                &[],
                parent.as_ref(),
                &environment,
            ));
        }
        parent.unwrap()
    };
    let body = compute("b");
    assert_eq!(body.display, Display::Block);
    assert_eq!(
        body.margin.top,
        LengthPercentageAuto::Length(LengthPercentage::Px(8.0))
    );
    let heading = compute("h");
    assert_eq!(heading.font_size, Px(32.0));
    assert_eq!(heading.font_weight, FontWeight(700));
    assert_eq!(compute("s").display, Display::None);
    assert_eq!(compute("i").display, Display::InlineBlock);
    // Chromium's controls on Windows show 13.333px Arial.
    assert_eq!(compute("i").font_family.0, ["Arial"]);
    assert_eq!(compute("u").padding.left, LengthPercentage::Px(40.0));
    let link = compute("a");
    assert_eq!(link.color, crate::color::SystemColors::LIGHT.link_text);
    assert!(link.text_decoration_line.underline);
    assert_eq!(compute("closed").display, Display::ListItem);
    assert_eq!(
        compute("closed").list_style_position,
        ListStylePosition::Inside
    );
    assert_eq!(
        compute("closed").list_style_type,
        ListStyleType::DisclosureClosed
    );
    assert_eq!(
        compute("open").list_style_type,
        ListStyleType::DisclosureOpen
    );
    assert_eq!(compute("second").display, Display::Block);
    assert_eq!(compute("alone").display, Display::Block);
}
