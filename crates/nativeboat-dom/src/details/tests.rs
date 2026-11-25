use super::*;
use crate::tests::page;

fn details(dom: &mut Dom, parent: NodeId, name: &str) -> NodeId {
    let node = dom.create_element("details").unwrap();
    dom.set_attribute(node, "name", name).unwrap();
    dom.append_child(parent, node).unwrap();
    node
}

#[test]
fn notifications_follow_presence_and_html_namespace() {
    let (mut dom, body) = page();
    let a = details(&mut dom, body, "");
    dom.set_attribute(a, "OPEN", "false").unwrap();
    dom.set_attribute(a, "open", "different value").unwrap();
    dom.set_attribute(a, "title", "unchanged state").unwrap();
    dom.toggle_attribute(a, "open", Some(true)).unwrap();
    assert_eq!(dom.take_details_toggles(), [(a, false, true)]);
    dom.remove_attribute(a, "open");
    dom.remove_attribute(a, "open");
    assert_eq!(dom.take_details_toggles(), [(a, true, false)]);
    assert!(dom.take_details_toggles().is_empty());
    for namespace in [Namespace::Svg, Namespace::MathMl] {
        let foreign = dom.create_element_ns(namespace, "details").unwrap();
        dom.set_attribute(foreign, "name", "g").unwrap();
        dom.set_attribute(foreign, "open", "").unwrap();
        dom.append_child(body, foreign).unwrap();
    }
    let dialog = dom.create_element("dialog").unwrap();
    dom.set_attribute(dialog, "open", "").unwrap();
    assert!(dom.take_details_toggles().is_empty());
    assert!(dom.named_open_details.is_empty());
}

#[test]
fn opening_closes_a_peer_but_joining_closes_the_new_member() {
    let (mut dom, body) = page();
    let a = details(&mut dom, body, "Group");
    let b = details(&mut dom, body, "Group");
    let other = details(&mut dom, body, "group");
    dom.set_attribute(a, "open", "").unwrap();
    dom.set_attribute(other, "open", "").unwrap();
    dom.take_details_toggles();
    dom.set_attribute(b, "open", "").unwrap();
    assert!(!dom.has_attribute(a, "open"));
    assert!(dom.has_attribute(b, "open") && dom.has_attribute(other, "open"));
    assert_eq!(
        dom.take_details_toggles(),
        [(b, false, true), (a, true, false)]
    );
    dom.set_attribute(other, "name", "Group").unwrap();
    assert!(!dom.has_attribute(other, "open"));
    assert_eq!(dom.take_details_toggles(), [(other, true, false)]);
    dom.set_attribute(other, "name", "").unwrap();
    dom.set_attribute(other, "open", "").unwrap();
    dom.remove_attribute(other, "name");
    assert!(dom.has_attribute(b, "open") && dom.has_attribute(other, "open"));
    dom.remove_attribute(b, "open");
    dom.set_attribute(a, "open", "").unwrap();
    assert!(dom.has_attribute(a, "open"));
    assert_eq!(dom.named_open_details.len(), 1);
}

#[test]
fn insertion_enforces_groups_in_detached_trees_and_cloned_subtrees() {
    let (mut dom, body) = page();
    let a = details(&mut dom, body, "g");
    dom.set_attribute(a, "open", "").unwrap();
    let wrapper = dom.create_element("section").unwrap();
    let b = details(&mut dom, wrapper, "g");
    dom.set_attribute(b, "open", "").unwrap();
    assert!(dom.has_attribute(a, "open") && dom.has_attribute(b, "open"));
    dom.take_details_toggles();
    let clone = dom.clone_node(wrapper, true);
    let cloned_details = dom.child_elements(clone)[0];
    assert!(dom.has_attribute(cloned_details, "open"));
    assert_eq!(dom.take_details_toggles(), [(cloned_details, false, true)]);
    dom.append_child(body, clone).unwrap();
    assert!(!dom.has_attribute(cloned_details, "open"));
    assert!(dom.has_attribute(a, "open") && dom.has_attribute(b, "open"));
    dom.append_child(body, wrapper).unwrap();
    assert!(!dom.has_attribute(b, "open"));
    dom.remove(wrapper);
    dom.set_attribute(b, "open", "").unwrap();
    let c = details(&mut dom, wrapper, "g");
    dom.set_attribute(c, "open", "").unwrap();
    assert!(!dom.has_attribute(b, "open") && dom.has_attribute(c, "open"));
    assert!(dom.has_attribute(a, "open"));
}

#[test]
fn parser_keeps_first_open_member_and_reports_both_attribute_changes() {
    let mut dom = Dom::from_document(&nativeboat_html::parse_document(
        "<details id=a name=g open></details><section><details id=b open name=g></details></section>",
    ));
    let a = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let b = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    assert!(dom.has_attribute(a, "open"));
    assert!(!dom.has_attribute(b, "open"));
    assert_eq!(
        dom.take_details_toggles(),
        [(a, false, true), (b, false, true), (b, true, false)]
    );
}
