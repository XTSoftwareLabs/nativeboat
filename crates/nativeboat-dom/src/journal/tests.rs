use super::*;

fn dom_with(html: &str) -> (Dom, NodeId) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let host = dom.get_element_by_id(Dom::DOCUMENT, "host").unwrap();
    (dom, host)
}

#[test]
fn journals_undo_and_redo_edits() {
    let (mut dom, host) =
        dom_with("<div id=host><p id=a>one</p><p id=b class=x>two</p></div><p id=out>z</p>");
    let original = dom.inner_html(host);
    let id = dom.begin_journal(host);
    let a = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let b = dom.get_element_by_id(Dom::DOCUMENT, "b").unwrap();
    let text = dom.first_child(a).unwrap();
    dom.set_data(text, "one!");
    dom.set_attribute(b, "class", "y").unwrap();
    dom.remove_attribute(a, "id");
    let fresh = dom.create_element("p").unwrap();
    let words = dom.create_text("three");
    dom.append_child(fresh, words).unwrap();
    dom.insert_before(host, fresh, Some(b)).unwrap();
    dom.remove_child(host, a).unwrap();
    // Changes outside the host are not the edit's.
    let out = dom.get_element_by_id(Dom::DOCUMENT, "out").unwrap();
    dom.set_attribute(out, "title", "t").unwrap();
    // Writing what is there changes nothing.
    dom.set_attribute(b, "class", "y").unwrap();
    let journal = dom.end_journal(id).unwrap();
    assert!(!journal.is_empty());
    // The new paragraph's text went in while it was outside the document.
    assert_eq!(journal.changes.len(), 6);
    let edited = dom.inner_html(host);
    assert_eq!(edited, "<p>three</p><p id=\"b\" class=\"y\">two</p>");
    // Undo records the changes that redo them.
    let id = dom.begin_journal(host);
    dom.revert_journal(&journal);
    let redo = dom.end_journal(id).unwrap();
    assert_eq!(dom.inner_html(host), original);
    assert_eq!(dom.first_child(host), Some(a));
    let id = dom.begin_journal(host);
    dom.revert_journal(&redo);
    let undo = dom.end_journal(id).unwrap();
    assert_eq!(dom.inner_html(host), edited);
    dom.revert_journal(&undo);
    assert_eq!(dom.inner_html(host), original);
    assert_eq!(dom.end_journal(id), None);
}

#[test]
fn hosts_count_their_changes_once_recorded() {
    let (mut dom, host) = dom_with("<div id=host><p>a</p></div><p id=out>b</p>");
    let paragraph = dom.first_child(host).unwrap();
    let text = dom.first_child(paragraph).unwrap();
    dom.set_data(text, "x");
    assert_eq!(dom.host_changes(host), 0);
    let id = dom.begin_journal(host);
    let first = dom.end_journal(id).unwrap();
    assert!(first.is_empty());
    dom.set_data(text, "y");
    assert_eq!(dom.host_changes(host), 1);
    let out = dom.get_element_by_id(Dom::DOCUMENT, "out").unwrap();
    dom.set_attribute(out, "title", "t").unwrap();
    dom.set_data(text, "y");
    assert_eq!(dom.host_changes(host), 1);
    // Journals chain into one edit.
    let id = dom.begin_journal(host);
    dom.set_data(text, "z");
    let one = dom.end_journal(id).unwrap();
    let id = dom.begin_journal(host);
    dom.set_data(text, "w");
    let two = dom.end_journal(id).unwrap();
    dom.revert_journal(&one.then(&two));
    assert_eq!(dom.data(text), Some("y"));
}

#[test]
fn nodes_moved_through_detached_trees_come_back() {
    // Splitting a paragraph moves its text into a new one before that goes
    // in; the application's own nodes elsewhere are left alone.
    let (mut dom, host) = dom_with("<div id=host><p id=a>ab<i>cd</i></p></div><p id=status>1</p>");
    let a = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let status = dom.get_element_by_id(Dom::DOCUMENT, "status").unwrap();
    let count = dom.first_child(status).unwrap();
    let italic = dom.last_child(a).unwrap();
    let original = dom.inner_html(host);
    let id = dom.begin_journal(host);
    let fragment = dom.create_fragment();
    dom.append_child(fragment, italic).unwrap();
    let fresh = dom.create_element("p").unwrap();
    dom.append_child(fresh, fragment).unwrap();
    dom.append_child(host, fresh).unwrap();
    // The application shows a new count, from a node it made.
    let shown = dom.create_text("2");
    dom.set_data(shown, "2!");
    dom.remove_child(status, count).unwrap();
    dom.append_child(status, shown).unwrap();
    let journal = dom.end_journal(id).unwrap();
    let split = dom.inner_html(host);
    assert_eq!(split, "<p id=\"a\">ab</p><p><i>cd</i></p>");
    let id = dom.begin_journal(host);
    dom.revert_journal(&journal);
    let redo = dom.end_journal(id).unwrap();
    assert_eq!(dom.inner_html(host), original);
    assert_eq!(dom.inner_html(status), "2!");
    dom.revert_journal(&redo);
    assert_eq!(dom.inner_html(host), split);
    assert_eq!(dom.parent(italic), Some(fresh));
    assert_eq!(dom.inner_html(status), "2!");
}
