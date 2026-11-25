use super::*;
use crate::tests::page;

fn text(dom: &mut Dom, parent: NodeId, data: &str) -> NodeId {
    let node = dom.create_text(data);
    dom.append_child(parent, node).unwrap();
    node
}

#[test]
fn ranges_are_created_moved_and_freed() {
    let (mut dom, body) = page();
    let first = dom.create_range((body, 0), (body, 0));
    let second = dom.create_range((Dom::DOCUMENT, 0), (body, 0));
    assert_eq!(dom.range_count(), 2);
    dom.set_range(first, (body, 0), (Dom::DOCUMENT, 1));
    assert_eq!(dom.range(first), Some([(body, 0), (Dom::DOCUMENT, 1)]));
    dom.drop_range(first);
    dom.drop_range(first);
    dom.set_range(first, (body, 0), (body, 0));
    dom.set_range(99, (body, 0), (body, 0));
    dom.drop_range(99);
    assert_eq!((dom.range(first), dom.range(99)), (None, None));
    assert_eq!(dom.range_count(), 1);
    assert_eq!(dom.create_range((body, 0), (body, 0)), first);
    assert_eq!(dom.range(second), Some([(Dom::DOCUMENT, 0), (body, 0)]));
}

#[test]
fn insertion_and_removal_move_boundaries() {
    let (mut dom, body) = page();
    let a = text(&mut dom, body, "a");
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    let inner = text(&mut dom, div, "inner");
    let range = dom.create_range((body, 1), (body, 2));
    let inside = dom.create_range((inner, 2), (inner, 3));
    let b = dom.create_text("b");
    dom.insert_before(body, b, Some(a)).unwrap();
    assert_eq!(dom.range(range), Some([(body, 2), (body, 3)]));
    let span = dom.create_element("span").unwrap();
    dom.append_child(div, span).unwrap();
    dom.remove(span);
    let fragment = dom.create_fragment();
    let c = text(&mut dom, fragment, "c");
    let d = text(&mut dom, fragment, "d");
    let held = dom.create_range((fragment, 1), (d, 1));
    dom.append_child(body, fragment).unwrap();
    assert_eq!(dom.range(held), Some([(fragment, 0), (fragment, 0)]));
    assert_eq!(dom.range(range), Some([(body, 2), (body, 3)]));
    dom.insert_before(body, c, Some(b)).unwrap();
    assert_eq!(dom.range(range), Some([(body, 3), (body, 4)]));
    dom.remove(b);
    dom.remove(d);
    assert_eq!(dom.range(range), Some([(body, 2), (body, 3)]));
    assert_eq!(dom.range(inside), Some([(inner, 2), (inner, 3)]));
    dom.remove(div);
    assert_eq!(dom.range(range), Some([(body, 2), (body, 2)]));
    assert_eq!(dom.range(inside), Some([(body, 2), (body, 2)]));
    dom.set_inner_html(body, "<p>x</p>");
    assert_eq!(dom.range(range), Some([(body, 0), (body, 0)]));
}

#[test]
fn replacing_data_moves_boundaries_in_the_text() {
    let (mut dom, body) = page();
    let node = text(&mut dom, body, "a😀bcdef");
    let range = dom.create_range((node, 1), (node, 6));
    let inside = dom.create_range((node, 4), (node, 4));
    dom.replace_data(node, 3, 2, "XYZ").unwrap();
    assert_eq!(dom.data(node), Some("a😀XYZdef"));
    assert_eq!(dom.range(range), Some([(node, 1), (node, 7)]));
    assert_eq!(dom.range(inside), Some([(node, 3), (node, 3)]));
    dom.replace_data(node, 6, 99, "").unwrap();
    assert_eq!(dom.data(node), Some("a😀XYZ"));
    assert_eq!(dom.range(range), Some([(node, 1), (node, 6)]));
    assert_eq!(
        dom.replace_data(node, 9, 0, "").unwrap_err().name(),
        "IndexSizeError"
    );
    dom.set_data(node, "short");
    assert_eq!(dom.range(range), Some([(node, 0), (node, 0)]));
    assert_eq!(dom.replace_data(body, 0, 0, "ignored"), Ok(()));
    dom.set_data(body, "ignored");
    assert_eq!(dom.text_content(body).as_deref(), Some("short"));
}

#[test]
fn splitting_text_carries_the_boundaries_after_the_split() {
    let (mut dom, body) = page();
    let node = text(&mut dom, body, "hello");
    let after = dom.create_range((node, 4), (body, 1));
    let before = dom.create_range((node, 1), (node, 2));
    let tail = dom.split_text(node, 2).unwrap();
    assert_eq!((dom.data(node), dom.data(tail)), (Some("he"), Some("llo")));
    assert_eq!(dom.children(body), vec![node, tail]);
    assert_eq!(dom.range(after), Some([(tail, 2), (body, 2)]));
    assert_eq!(dom.range(before), Some([(node, 1), (node, 2)]));
    assert_eq!(
        dom.split_text(node, 3).unwrap_err().name(),
        "IndexSizeError"
    );
    let detached = dom.create_text("abc");
    let alone = dom.create_range((detached, 3), (detached, 3));
    let rest = dom.split_text(detached, 1).unwrap();
    assert_eq!(
        (dom.data(detached), dom.data(rest)),
        (Some("a"), Some("bc"))
    );
    assert_eq!(dom.parent(rest), None);
    assert_eq!(dom.range(alone), Some([(detached, 1), (detached, 1)]));
}

#[test]
fn normalizing_merges_text_and_keeps_boundaries() {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    let a = text(&mut dom, div, "ab");
    let empty = text(&mut dom, div, "");
    let c = text(&mut dom, div, "c");
    let d = text(&mut dom, div, "de");
    let span = dom.create_element("span").unwrap();
    dom.append_child(div, span).unwrap();
    let lone = text(&mut dom, span, "");
    let range = dom.create_range((c, 1), (d, 1));
    let between = dom.create_range((div, 3), (div, 4));
    dom.normalize(body);
    assert_eq!(dom.children(div), vec![a, span]);
    assert_eq!(dom.data(a), Some("abcde"));
    assert_eq!(dom.parent(lone), None);
    assert_eq!(dom.parent(empty), None);
    assert_eq!(dom.range(range), Some([(a, 3), (a, 4)]));
    assert_eq!(dom.range(between), Some([(a, 3), (div, 1)]));
    let comment = dom.create_comment("x");
    dom.append_child(div, comment).unwrap();
    dom.normalize(div);
    assert_eq!(dom.children(div), vec![a, span, comment]);
}

#[test]
fn the_selection_is_a_live_range() {
    let (mut dom, body) = page();
    let a = text(&mut dom, body, "abc");
    let range = dom.create_range((a, 1), (a, 2));
    assert_eq!(dom.selection(), None);
    dom.set_selection(Some(99));
    assert_eq!(dom.selection(), None);
    dom.set_selection(Some(range));
    assert_eq!(dom.selection(), Some([(a, 1), (a, 2)]));
    dom.replace_data(a, 0, 1, "").unwrap();
    assert_eq!(dom.selection(), Some([(a, 0), (a, 1)]));
    let other = dom.create_range((a, 0), (a, 0));
    dom.drop_range(other);
    assert!(dom.selection().is_some());
    dom.drop_range(range);
    assert_eq!(dom.selection(), None);
    let again = dom.create_range((a, 0), (a, 0));
    assert_eq!(dom.selection(), None);
    dom.set_selection(Some(again));
    dom.set_selection(None);
    assert_eq!(dom.selection(), None);
}

#[test]
fn ranges_cover_the_text_between_their_boundaries() {
    let (mut dom, body) = page();
    dom.set_inner_html(body, "<p>one<b>two</b></p><!--c--><p>three<i></i></p>x");
    let [first, _, second, x] = dom.children(body)[..] else {
        panic!("four children");
    };
    let one = dom.first_child(first).unwrap();
    let bold = dom.last_child(first).unwrap();
    let two = dom.first_child(bold).unwrap();
    let three = dom.first_child(second).unwrap();
    let italic = dom.last_child(second).unwrap();
    assert_eq!(dom.range_texts([(one, 1), (one, 3)]), vec![(one, 1, 3)]);
    assert!(dom.range_texts([(one, 1), (one, 1)]).is_empty());
    assert_eq!(
        dom.range_texts([(one, 1), (three, 2)]),
        vec![(one, 1, 3), (two, 0, 3), (three, 0, 2)]
    );
    assert_eq!(dom.range_texts([(one, 3), (three, 0)]), vec![(two, 0, 3)]);
    assert_eq!(
        dom.range_texts([(first, 1), (second, 1)]),
        vec![(two, 0, 3), (three, 0, 5)]
    );
    assert_eq!(
        dom.range_texts([(bold, 1), (body, 4)]),
        vec![(three, 0, 5), (x, 0, 1)]
    );
    assert!(dom.range_texts([(italic, 0), (italic, 0)]).is_empty());
    assert_eq!(dom.range_texts([(body, 0), (italic, 0)]).len(), 3);
    assert!(dom.range_texts([(x, 1), (body, 0)]).is_empty());
}
