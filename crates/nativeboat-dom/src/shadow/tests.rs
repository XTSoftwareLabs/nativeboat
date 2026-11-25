use super::*;
use crate::tests::page;

fn element(dom: &mut Dom, parent: NodeId, name: &str) -> NodeId {
    let node = dom.create_element(name).unwrap();
    dom.append_child(parent, node).unwrap();
    node
}

#[test]
fn only_some_html_elements_host_one_shadow_root() {
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "div");
    let version = dom.version();
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    assert!(dom.version() > version);
    assert_eq!(dom.shadow_root(host), Some(root));
    assert_eq!(dom.shadow_host(root), Some(host));
    assert!(dom.is_shadow_root(root));
    assert!(dom.has_shadow_roots());
    assert_eq!(dom.shadow_roots(), [root]);
    assert_eq!(
        dom.shadow_root_data(root),
        Some(ShadowRootData {
            host,
            mode: ShadowRootMode::Open,
            delegates_focus: false
        })
    );
    assert_eq!(dom.node_type(root), crate::node_type::FRAGMENT);
    assert_eq!(dom.owner_document(root), dom.owner_document(host));
    assert_eq!(dom.parent(root), None);
    assert_eq!(
        dom.attach_shadow(host, ShadowRootMode::Closed, false)
            .unwrap_err()
            .name(),
        "NotSupportedError"
    );
    let custom = element(&mut dom, body, "x-card");
    assert!(
        dom.attach_shadow(custom, ShadowRootMode::Closed, true)
            .is_ok()
    );
    for name in ["img", "input", "x"] {
        let other = element(&mut dom, body, name);
        let error = dom
            .attach_shadow(other, ShadowRootMode::Open, false)
            .unwrap_err();
        assert_eq!(error.name(), "NotSupportedError", "{name}");
    }
    let svg = dom.create_element_ns(Namespace::Svg, "div").unwrap();
    assert!(dom.attach_shadow(svg, ShadowRootMode::Open, false).is_err());
}

#[test]
fn shadow_trees_are_connected_through_their_hosts_but_not_contained() {
    let (mut dom, body) = page();
    let host = dom.create_element("div").unwrap();
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let inner = element(&mut dom, root, "span");
    let text = dom.create_text("deep");
    dom.append_child(inner, text).unwrap();
    assert!(!dom.is_connected(inner));
    dom.append_child(body, host).unwrap();
    assert!(dom.is_connected(inner));
    assert!(dom.is_connected(root));
    assert!(!dom.contains(Dom::DOCUMENT, inner));
    assert!(dom.shadow_including_contains(Dom::DOCUMENT, inner));
    assert!(dom.shadow_including_contains(host, text));
    assert!(!dom.shadow_including_contains(inner, host));
    assert_eq!(dom.tree_root(text), root);
    assert_eq!(dom.root_node(text, false), root);
    assert_eq!(dom.root_node(text, true), Dom::DOCUMENT);
    assert_eq!(dom.parent_or_host(root), Some(host));
    assert_eq!(dom.text_content(root).as_deref(), Some("deep"));
    assert_eq!(dom.inner_html(root), "<span>deep</span>");
    // The tree order of the shadow-including descendants puts each shadow
    // tree before its host's children.
    let light = element(&mut dom, host, "b");
    assert_eq!(
        dom.shadow_including_descendants(host),
        [root, inner, text, light]
    );
    let all = dom.shadow_including_descendants(Dom::DOCUMENT);
    assert!(all.contains(&inner) && all.contains(&light));
}

#[test]
fn shadow_roots_cannot_be_inserted_and_hosts_cannot_enter_their_shadow_trees() {
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "section");
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let inner = element(&mut dom, root, "p");
    let other = element(&mut dom, body, "div");
    let error = dom.append_child(other, root).unwrap_err();
    assert_eq!(error.name(), "HierarchyRequestError");
    let error = dom.append_child(inner, host).unwrap_err();
    assert_eq!(error.name(), "HierarchyRequestError");
    dom.set_inner_html(root, "<i>one</i><slot></slot>");
    assert_eq!(dom.inner_html(root), "<i>one</i><slot></slot>");
}

#[test]
fn removing_a_host_clears_focus_hover_and_activation_inside_its_shadow_tree() {
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "div");
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let button = element(&mut dom, root, "button");
    dom.set_focus(Some(button), false);
    dom.set_hovered(Some(button));
    dom.set_active(Some(button));
    assert_eq!(dom.focused(), Some(button));
    dom.remove(host);
    assert_eq!(dom.focused(), None);
    assert_eq!(dom.hovered(), None);
    assert_eq!(dom.active(), None);
}

#[test]
fn retargeting_replaces_nodes_by_the_hosts_of_trees_that_do_not_contain_the_target() {
    let (mut dom, body) = page();
    let outer = element(&mut dom, body, "div");
    let outer_root = dom
        .attach_shadow(outer, ShadowRootMode::Open, false)
        .unwrap();
    let inner = element(&mut dom, outer_root, "x-inner");
    let inner_root = dom
        .attach_shadow(inner, ShadowRootMode::Closed, false)
        .unwrap();
    let deep = element(&mut dom, inner_root, "span");
    assert_eq!(dom.retarget(deep, Dom::DOCUMENT), outer);
    assert_eq!(dom.retarget(deep, body), outer);
    assert_eq!(dom.retarget(deep, inner), inner);
    assert_eq!(dom.retarget(deep, outer_root), inner);
    assert_eq!(dom.retarget(deep, deep), deep);
    assert_eq!(dom.retarget(body, deep), body);
}

#[test]
fn host_children_are_assigned_to_the_first_slot_with_their_name() {
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "div");
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let default = element(&mut dom, root, "slot");
    let named = element(&mut dom, root, "slot");
    dom.set_attribute(named, "name", "title").unwrap();
    let duplicate = element(&mut dom, root, "slot");
    dom.set_attribute(duplicate, "name", "title").unwrap();
    let empty = element(&mut dom, root, "slot");
    dom.set_attribute(empty, "name", "none").unwrap();
    let fallback = dom.create_text("fallback");
    dom.append_child(empty, fallback).unwrap();
    let text = dom.create_text("text");
    dom.append_child(host, text).unwrap();
    let title = element(&mut dom, host, "h1");
    dom.set_attribute(title, "slot", "title").unwrap();
    let plain = element(&mut dom, host, "p");
    let comment = dom.create_comment("c");
    dom.append_child(host, comment).unwrap();
    let lost = element(&mut dom, host, "p");
    dom.set_attribute(lost, "slot", "missing").unwrap();
    assert_eq!(dom.assigned_slot(text), Some(default));
    assert_eq!(dom.assigned_slot(title), Some(named));
    assert_eq!(dom.assigned_slot(plain), Some(default));
    assert_eq!(dom.assigned_slot(comment), None);
    assert_eq!(dom.assigned_slot(lost), None);
    assert_eq!(dom.assigned_slot(default), None);
    assert_eq!(dom.assigned_nodes(default, false), [text, plain]);
    assert_eq!(dom.assigned_nodes(named, false), [title]);
    assert!(dom.assigned_nodes(duplicate, false).is_empty());
    assert!(dom.assigned_nodes(empty, false).is_empty());
    assert_eq!(dom.assigned_nodes(empty, true), [fallback]);
}

#[test]
fn flattened_slots_show_what_nested_slots_are_assigned() {
    let (mut dom, body) = page();
    // <x-outer><b>light</b></x-outer>, whose shadow tree passes its slot on
    // to an inner host's slot.
    let outer = element(&mut dom, body, "x-outer");
    let light = element(&mut dom, outer, "b");
    let outer_root = dom
        .attach_shadow(outer, ShadowRootMode::Open, false)
        .unwrap();
    let inner = element(&mut dom, outer_root, "x-inner");
    let passed = element(&mut dom, inner, "slot");
    let inner_root = dom
        .attach_shadow(inner, ShadowRootMode::Open, false)
        .unwrap();
    let inner_slot = element(&mut dom, inner_root, "slot");
    assert_eq!(dom.assigned_nodes(inner_slot, false), [passed]);
    assert_eq!(dom.assigned_nodes(inner_slot, true), [light]);
    // A slot outside any shadow tree has nothing assigned but shows its
    // children when flattened.
    let detached = dom.create_element("slot").unwrap();
    let child = element(&mut dom, detached, "i");
    assert!(dom.assigned_nodes(detached, false).is_empty());
    assert_eq!(dom.assigned_nodes(detached, true), [child]);
}

#[test]
fn adopting_a_host_adopts_its_shadow_tree() {
    let (mut dom, _) = page();
    let host = dom.create_element("div").unwrap();
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let inner = element(&mut dom, root, "span");
    let other = dom.parse_html_document("<!doctype html><body></body>");
    dom.set_owner_document(host, other).unwrap();
    assert_eq!(dom.owner_document(root), Some(other));
    assert_eq!(dom.owner_document(inner), Some(other));
}

#[test]
fn custom_element_candidates_include_shadow_trees() {
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "x-host");
    let root = dom
        .attach_shadow(host, ShadowRootMode::Closed, false)
        .unwrap();
    let inner = element(&mut dom, root, "x-inner");
    let light = element(&mut dom, host, "x-light");
    assert_eq!(dom.custom_element_candidates(body), [host, inner, light]);
}

#[test]
fn hosts_share_the_hover_activation_and_focus_of_their_shadow_trees() {
    use nativeboat_css::selector::state;
    let (mut dom, body) = page();
    let host = element(&mut dom, body, "div");
    let root = dom
        .attach_shadow(host, ShadowRootMode::Open, false)
        .unwrap();
    let button = element(&mut dom, root, "button");
    dom.set_hovered(Some(button));
    dom.set_active(Some(button));
    dom.set_focus(Some(button), false);
    let flags = dom.element_state(host);
    assert_ne!(flags & state::HOVER, 0);
    assert_ne!(flags & state::ACTIVE, 0);
    assert_ne!(flags & state::FOCUS_WITHIN, 0);
    assert_ne!(flags & state::FOCUS, 0);
    assert_ne!(dom.element_state(body) & state::FOCUS_WITHIN, 0);
    assert_eq!(dom.element_state(body) & state::FOCUS, 0);
}
