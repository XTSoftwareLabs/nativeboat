use super::*;

use crate::tests::page;
use nativeboat_css::Element;

#[test]
fn selector_state_is_lazy_and_shared_only_during_an_immutable_view() {
    use nativeboat_css::selector::Element;
    let mut dom = Dom::from_document(&nativeboat_html::parse_document("<p id=p>text</p>"));
    let p = dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap();
    let view = ElementRef::new(&dom, p);
    assert!(
        SelectorList::parse("p:only-child:not(:empty)")
            .unwrap()
            .matches(&view)
    );
    assert!(view.state.get().is_none());
    assert!(!SelectorList::parse("p:hover").unwrap().matches(&view));
    let flags = view.state.get().unwrap();
    assert_eq!(Element::state(&view), flags);
    assert_eq!(Element::state(&view.clone()), flags);
    dom.set_hovered(Some(p));
    let fresh = ElementRef::new(&dom, p);
    assert!(SelectorList::parse("p:hover").unwrap().matches(&fresh));
    assert_ne!(Element::state(&fresh), flags);
}

fn by_id(dom: &Dom, id: &str) -> NodeId {
    dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()
}

#[test]
fn element_trait_view() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<div id=a class='x y'><span id=b></span>t<p id=c></p></div><i id=e><!--c--></i><u id=f>x</u>",
    );
    let empty_text = dom.create_text("");
    let e = by_id(&dom, "e");
    dom.append_child(e, empty_text).unwrap();
    let a = ElementRef::new(&dom, by_id(&dom, "a"));
    let b = ElementRef::new(&dom, by_id(&dom, "b"));
    let c = ElementRef::new(&dom, by_id(&dom, "c"));
    assert_eq!(a.local_name(), "div");
    assert_eq!(Element::id(&a), Some("a"));
    assert!(a.has_class("y"));
    assert_eq!(a.attribute("class"), Some("x y"));
    assert_eq!(b.parent_element().unwrap().id, a.id);
    assert_eq!(c.previous_element_sibling().unwrap().id, b.id);
    assert_eq!(b.next_element_sibling().unwrap().id, c.id);
    let root = ElementRef::new(&dom, dom.document_element().unwrap());
    assert!(root.is_root());
    assert!(!a.is_root());
    assert!(b.is_empty());
    assert!(!a.is_empty());
    assert!(ElementRef::new(&dom, e).is_empty());
    assert!(!ElementRef::new(&dom, by_id(&dom, "f")).is_empty());
    let text = dom.create_text("t");
    assert_eq!(ElementRef::new(&dom, text).local_name(), "");
    assert_eq!(dom.element_state(text), 0);
}

#[test]
fn state_flags() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<form id=form><input id=text placeholder=p required><input id=box type=checkbox checked>\
         <input id=ro readonly><textarea id=area disabled></textarea>\
         <select><option id=opt selected>a</option></select><button id=btn>b</button></form>\
         <a id=link href=x>l</a><div id=edit contenteditable><p id=inner>x</p>\
         <p id=fixed contenteditable=false>y</p></div><div id=plain></div>",
    );
    let flags = |dom: &Dom, id: &str| dom.element_state(by_id(dom, id));
    let text = by_id(&dom, "text");
    dom.set_focus(Some(text), false);
    dom.set_hovered(Some(text));
    dom.set_active(Some(text));
    let input = flags(&dom, "text");
    for flag in [
        state::FOCUS,
        state::FOCUS_VISIBLE,
        state::FOCUS_WITHIN,
        state::HOVER,
        state::ACTIVE,
        state::REQUIRED,
        state::EDITABLE,
        state::PLACEHOLDER_SHOWN,
        state::FORM_CONTROL,
    ] {
        assert_ne!(input & flag, 0, "{flag}");
    }
    let form = flags(&dom, "form");
    assert_ne!(form & state::FOCUS_WITHIN, 0);
    assert_ne!(form & state::HOVER, 0);
    assert_eq!(form & state::FOCUS, 0);
    dom.set_value(text, "typed");
    assert_eq!(flags(&dom, "text") & state::PLACEHOLDER_SHOWN, 0);
    let button = by_id(&dom, "btn");
    dom.set_focus(Some(button), false);
    assert_eq!(flags(&dom, "btn") & state::FOCUS_VISIBLE, 0);
    dom.set_focus(Some(text), true);
    dom.set_focus(Some(button), true);
    assert_ne!(flags(&dom, "btn") & state::FOCUS_VISIBLE, 0);
    assert_ne!(flags(&dom, "box") & state::CHECKED, 0);
    dom.set_indeterminate(by_id(&dom, "box"), true);
    assert_ne!(flags(&dom, "box") & state::INDETERMINATE, 0);
    assert_eq!(flags(&dom, "box") & state::EDITABLE, 0);
    assert_eq!(flags(&dom, "ro") & state::EDITABLE, 0);
    assert_ne!(flags(&dom, "area") & state::DISABLED, 0);
    assert_eq!(flags(&dom, "area") & state::EDITABLE, 0);
    assert_ne!(flags(&dom, "opt") & state::CHECKED, 0);
    assert_ne!(flags(&dom, "link") & state::LINK, 0);
    assert_ne!(flags(&dom, "edit") & state::EDITABLE, 0);
    assert_ne!(flags(&dom, "inner") & state::EDITABLE, 0);
    assert_eq!(flags(&dom, "fixed") & state::EDITABLE, 0);
    assert_eq!(flags(&dom, "plain"), 0);
}

#[test]
fn queries() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<ul id=list class=menu><li class='item first'>a</li><li class=item>b<span>s</span></li></ul><p class=item>c</p>",
    );
    let list = by_id(&dom, "list");
    let items = dom.query_selector_all(Dom::DOCUMENT, ".item").unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(dom.query_selector_all(list, "li").unwrap().len(), 2);
    assert_eq!(
        dom.query_selector(Dom::DOCUMENT, "li + li").unwrap(),
        Some(items[1])
    );
    assert_eq!(dom.query_selector(list, "p").unwrap(), None);
    assert_eq!(
        dom.query_selector(Dom::DOCUMENT, "li:has(span)").unwrap(),
        Some(items[1])
    );
    assert!(matches!(
        dom.query_selector(list, "!!"),
        Err(DomError::Syntax(_))
    ));
    assert!(dom.query_selector_all(list, "").is_err());
    assert!(dom.matches(items[0], "li.first").unwrap());
    assert!(!dom.matches(items[1], ".first").unwrap());
    assert!(dom.matches(items[0], "[").is_err());
    let text = dom.create_text("x");
    assert!(!dom.matches(text, "*").unwrap());
    let span = dom.query_selector(Dom::DOCUMENT, "span").unwrap().unwrap();
    assert_eq!(dom.closest(span, "ul").unwrap(), Some(list));
    assert_eq!(dom.closest(span, "span").unwrap(), Some(span));
    assert_eq!(dom.closest(span, "table").unwrap(), None);
    assert!(dom.closest(span, ")").is_err());
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "nope"), None);
    assert_eq!(dom.get_elements_by_tag_name(Dom::DOCUMENT, "LI").len(), 2);
    assert_eq!(dom.get_elements_by_tag_name(list, "*").len(), 3);
    assert_eq!(
        dom.get_elements_by_class_name(Dom::DOCUMENT, "item first")
            .len(),
        1
    );
    assert_eq!(
        dom.get_elements_by_class_name(Dom::DOCUMENT, "item").len(),
        3
    );
    assert!(
        dom.get_elements_by_class_name(Dom::DOCUMENT, "  ")
            .is_empty()
    );
}

#[test]
fn the_trait_reports_state() {
    let (mut dom, body) = page();
    dom.set_hovered(Some(body));
    assert_ne!(ElementRef::new(&dom, body).state() & state::HOVER, 0);
}

#[test]
fn scope_matches_the_query_root_or_the_document_element() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=r><ul id=u><li id=l></li></ul></div>",
    ));
    let r = by_id(&dom, "r");
    let found = dom.query_selector_all(r, ":scope > ul").unwrap();
    assert_eq!(found, [by_id(&dom, "u")]);
    assert!(dom.query_selector_all(r, ":scope > li").unwrap().is_empty());
    assert_eq!(
        dom.query_selector_all(Dom::DOCUMENT, ":scope").unwrap(),
        [dom.document_element().unwrap()]
    );
    let l = by_id(&dom, "l");
    assert!(dom.matches(l, ":scope").unwrap());
    // closest() scopes to the element it starts from.
    assert_eq!(
        dom.closest(l, "ul:has(> :scope)").unwrap(),
        Some(by_id(&dom, "u"))
    );
    let html = ElementRef::new(&dom, dom.document_element().unwrap());
    assert!(SelectorList::parse(":scope").unwrap().matches(&html));
}

#[test]
fn ids_find_the_first_contained_element_as_they_change() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<div id=outer><p id=x>one</p></div><section id=s><p id=x>two</p></section>",
    );
    let outer = by_id(&dom, "outer");
    let section = by_id(&dom, "s");
    let first = dom.child_elements(outer)[0];
    let second = dom.child_elements(section)[0];
    // The first in tree order, within the root, never the root itself.
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "x"), Some(first));
    assert_eq!(dom.get_element_by_id(section, "x"), Some(second));
    assert_eq!(dom.get_element_by_id(section, "s"), None);
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "missing"), None);
    // Moving the first after the second reorders them.
    dom.append_child(section, first).unwrap();
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "x"), Some(second));
    // Renamed and removed ids are found by their new names only.
    dom.set_attribute(second, "ID", "y").unwrap();
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "x"), Some(first));
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "y"), Some(second));
    dom.remove_attribute(first, "id");
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "x"), None);
    // Detached elements and their copies are found under their own roots.
    dom.remove_child(section, second).unwrap();
    assert_eq!(dom.get_element_by_id(Dom::DOCUMENT, "y"), None);
    let holder = dom.create_element("div").unwrap();
    let copy = dom.clone_node(second, true);
    dom.append_child(holder, copy).unwrap();
    assert_eq!(dom.get_element_by_id(holder, "y"), Some(copy));
    // Elements named in other namespaces keep their attribute case.
    let svg = dom.create_element_ns(crate::Namespace::Svg, "g").unwrap();
    dom.set_attribute(svg, "ID", "z").unwrap();
    dom.append_child(holder, svg).unwrap();
    assert_eq!(dom.get_element_by_id(holder, "z"), None);
}

#[test]
fn descendant_selectors_use_the_ancestor_filter() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<section id=s class='Outer box'><div class=mid><b id=t>x</b></div></section>",
    );
    let target = ElementRef::new(&dom, by_id(&dom, "t"));
    let matches = |selector: &str| SelectorList::parse(selector).unwrap().matches(&target);
    // Ancestors named by tag (any case), id and class, near or far.
    assert!(matches("SECTION b"));
    assert!(matches("#s .mid b"));
    assert!(matches(".Outer.box b"));
    assert!(matches("body section > div b"));
    // A class no ancestor has, or with another case, fails at once.
    assert!(!matches(".outer b"));
    assert!(!matches("article b"));
    assert!(!matches("#t b"));
    let filter = target.ancestor_filter().unwrap();
    assert_eq!(target.ancestor_filter(), Some(filter));
}

#[test]
fn labels_hover_and_press_their_controls() {
    let (mut dom, body) = page();
    dom.set_inner_html(
        body,
        "<label id=wrap>Wrap <span id=inner>x</span><input id=box type=checkbox>\
         <input id=second type=checkbox></label>\
         <label id=named for=target>Named</label><input id=target>\
         <label id=missing for=nothing>M</label><label id=hidden for=h>H</label>\
         <input id=h type=hidden><label id=div for=d>D</label><div id=d></div>\
         <label id=empty>E</label>",
    );
    let flags = |dom: &Dom, id: &str| dom.element_state(by_id(dom, id));
    // Inside a label, its first labelable descendant is hovered and pressed.
    dom.set_hovered(Some(by_id(&dom, "inner")));
    dom.set_active(Some(by_id(&dom, "inner")));
    assert_ne!(flags(&dom, "box") & state::HOVER, 0);
    assert_ne!(flags(&dom, "box") & state::ACTIVE, 0);
    assert_eq!(flags(&dom, "second") & (state::HOVER | state::ACTIVE), 0);
    assert_ne!(flags(&dom, "wrap") & state::HOVER, 0);
    // A label's `for` names its control.
    dom.set_hovered(Some(by_id(&dom, "named")));
    assert_ne!(flags(&dom, "target") & state::HOVER, 0);
    assert_eq!(flags(&dom, "box") & state::HOVER, 0);
    // Names of nothing, hidden inputs and other elements label nothing.
    for (label, control) in [("missing", "target"), ("hidden", "h"), ("div", "d")] {
        dom.set_hovered(Some(by_id(&dom, label)));
        assert_eq!(flags(&dom, control) & state::HOVER, 0, "{label}");
    }
    assert_eq!(
        dom.label_control(by_id(&dom, "wrap")),
        Some(by_id(&dom, "box"))
    );
    assert_eq!(
        dom.label_control(by_id(&dom, "named")),
        Some(by_id(&dom, "target"))
    );
    for label in ["missing", "hidden", "div", "empty"] {
        assert_eq!(dom.label_control(by_id(&dom, label)), None, "{label}");
    }
    dom.set_hovered(None);
    assert_eq!(flags(&dom, "box") & state::HOVER, 0);
}

#[test]
fn lang_matches_the_nearest_language_by_range_and_dir_the_nearest_direction() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<html lang=zh-Hans-CN><body><p id=chinese>x</p><div lang=EN-us><span id=english>y</span></div>\
         <div dir=rtl><span id=right>z</span><b dir=ltr id=left>w</b></div><p dir=auto id=auto>\u{5d0}b</p><bdi id=isolated>a\u{5d0}</bdi></body></html>",
    ));
    let id = |dom: &Dom, name| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let matches = |dom: &Dom, node, selector| dom.matches(node, selector).unwrap();
    let chinese = id(&dom, "chinese");
    let english = id(&dom, "english");
    assert!(matches(&dom, chinese, ":lang(zh-Hans)"));
    assert!(matches(&dom, chinese, ":lang(zh)"));
    assert!(matches(&dom, chinese, ":lang(en, \"zh-hans-cn\")"));
    assert!(!matches(&dom, chinese, ":lang(zh-Hant)"));
    assert!(!matches(&dom, chinese, ":lang(z)"));
    assert!(matches(&dom, english, ":lang(en)"));
    assert!(matches(&dom, english, ":lang(*)"));
    assert!(!matches(&dom, english, ":lang(zh)"));
    assert!(dom.matches(chinese, ":lang()").is_err());
    assert!(matches(&dom, id(&dom, "right"), ":dir(rtl)"));
    assert!(matches(&dom, id(&dom, "left"), ":dir(ltr)"));
    assert!(matches(&dom, chinese, ":dir(ltr)"));
    assert!(!matches(&dom, chinese, ":dir(sideways)"));
    assert!(matches(&dom, id(&dom, "auto"), ":dir(rtl)"));
    assert!(matches(&dom, id(&dom, "isolated"), ":dir(ltr)"));
    // Shadow trees take their host's language.
    let host = id(&dom, "english");
    let root = dom
        .attach_shadow(host, crate::ShadowRootMode::Open, false)
        .unwrap();
    let inner = dom.create_element("i").unwrap();
    dom.append_child(root, inner).unwrap();
    assert_eq!(dom.language(inner), Some("EN-us"));
}
