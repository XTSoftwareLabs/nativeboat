use super::*;
use crate::tests::page;

fn kinds(records: &[MutationRecord]) -> Vec<MutationKind> {
    records.iter().map(|record| record.kind).collect()
}

#[test]
fn identical_writes_notify_observers_without_invalidating_rendering() {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    let text = dom.create_text("hello");
    dom.append_child(body, div).unwrap();
    dom.append_child(div, text).unwrap();
    dom.set_attribute(div, "class", "same").unwrap();
    dom.set_attribute(div, "style", "color:red").unwrap();
    let observer = dom.create_observer();
    dom.observe(
        observer,
        body,
        ObserveOptions {
            attributes: true,
            character_data: true,
            subtree: true,
            attribute_old_value: true,
            character_data_old_value: true,
            ..ObserveOptions::default()
        },
    );
    let range = dom.create_range((text, 2), (text, 4));
    dom.set_selection(Some(range));
    let version = dom.version();
    dom.set_attribute(div, "CLASS", "same").unwrap();
    dom.set_attribute(div, "style", "color:red").unwrap();
    dom.set_data(text, "hello");
    assert_eq!(dom.version(), version);
    assert_eq!(dom.range(range), Some([(text, 0), (text, 0)]));
    assert_eq!(dom.selection(), Some([(text, 0), (text, 0)]));
    assert_eq!(dom.inline_style(div).get("color"), "red");
    let records = dom.take_records(observer);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].old_value.as_deref(), Some("same"));
    assert_eq!(records[1].old_value.as_deref(), Some("color:red"));
    assert_eq!(records[2].old_value.as_deref(), Some("hello"));
    dom.set_attribute(div, "class", "changed").unwrap();
    assert!(dom.version() > version);
    let version = dom.version();
    dom.set_data(text, "changed");
    assert!(dom.version() > version);
    let canvas = dom.create_element("canvas").unwrap();
    dom.set_attribute(canvas, "width", "100").unwrap();
    let version = dom.version();
    dom.set_attribute(canvas, "width", "100").unwrap();
    assert!(dom.version() > version);
}

#[test]
fn child_list_records() {
    let (mut dom, body) = page();
    let observer = dom.create_observer();
    dom.observe(
        observer,
        body,
        ObserveOptions {
            child_list: true,
            ..ObserveOptions::default()
        },
    );
    let div = dom.create_element("div").unwrap();
    let span = dom.create_element("span").unwrap();
    dom.append_child(body, div).unwrap();
    dom.append_child(div, span).unwrap();
    assert_eq!(dom.pending_observers(), vec![observer]);
    let records = dom.take_records(observer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].added, vec![div]);
    assert_eq!(records[0].target, body);
    dom.remove(div);
    let records = dom.take_records(observer);
    assert_eq!(records[0].removed, vec![div]);
    assert!(dom.pending_observers().is_empty());
}

#[test]
fn subtree_attribute_and_character_data_records() {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    let text = dom.create_text("a");
    dom.append_child(body, div).unwrap();
    dom.append_child(div, text).unwrap();
    let observer = dom.create_observer();
    dom.observe(
        observer,
        body,
        ObserveOptions {
            attributes: true,
            character_data: true,
            subtree: true,
            attribute_old_value: true,
            attribute_filter: Some(vec!["class".to_owned()]),
            ..ObserveOptions::default()
        },
    );
    dom.set_attribute(div, "class", "x").unwrap();
    dom.set_attribute(div, "id", "ignored").unwrap();
    dom.set_attribute(div, "class", "y").unwrap();
    dom.set_data(text, "b");
    let records = dom.take_records(observer);
    assert_eq!(
        kinds(&records),
        vec![
            MutationKind::Attributes,
            MutationKind::Attributes,
            MutationKind::CharacterData
        ]
    );
    assert_eq!(records[0].old_value, None);
    assert_eq!(records[1].old_value.as_deref(), Some("x"));
    assert_eq!(records[1].attribute_name.as_deref(), Some("class"));
    assert_eq!(records[2].old_value, None);
    dom.observe(
        observer,
        body,
        ObserveOptions {
            character_data: true,
            character_data_old_value: true,
            subtree: true,
            ..ObserveOptions::default()
        },
    );
    dom.set_data(text, "c");
    dom.set_attribute(div, "class", "z").unwrap();
    let records = dom.take_records(observer);
    assert_eq!(kinds(&records), vec![MutationKind::CharacterData]);
    assert_eq!(records[0].old_value.as_deref(), Some("b"));
}

#[test]
fn observers_without_subtree_ignore_descendants() {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    let observer = dom.create_observer();
    dom.observe(
        observer,
        body,
        ObserveOptions {
            attributes: true,
            ..ObserveOptions::default()
        },
    );
    dom.set_attribute(div, "a", "1").unwrap();
    dom.set_attribute(body, "a", "1").unwrap();
    assert_eq!(dom.take_records(observer).len(), 1);
    dom.disconnect(observer);
    dom.set_attribute(body, "a", "2").unwrap();
    assert!(dom.take_records(observer).is_empty());
}

#[test]
fn unknown_observers_are_ignored() {
    let mut dom = Dom::new();
    dom.observe(42, Dom::DOCUMENT, ObserveOptions::default());
    dom.disconnect(42);
    assert!(dom.take_records(42).is_empty());
    let div = dom.create_element("div").unwrap();
    dom.set_attribute(div, "a", "b").unwrap();
    assert!(dom.pending_observers().is_empty());
}
