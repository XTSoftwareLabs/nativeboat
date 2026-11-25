use super::*;
use crate::tests::page;

#[test]
fn listeners_are_registered_once() {
    let mut dom = Dom::new();
    let div = dom.create_element("div").unwrap();
    let capture = ListenerOptions {
        capture: true,
        ..ListenerOptions::default()
    };
    assert!(dom.add_event_listener(div, "click", 1, ListenerOptions::default()));
    assert!(!dom.add_event_listener(div, "click", 1, ListenerOptions::default()));
    assert!(dom.add_event_listener(div, "click", 1, capture));
    assert!(dom.add_event_listener(div, "input", 1, ListenerOptions::default()));
    assert_eq!(dom.listeners(div).len(), 3);
    assert!(dom.has_listeners("input"));
    assert!(!dom.has_listeners("keydown"));
    assert_eq!(dom.remove_event_listener(div, "click", 1, true), Some(1));
    assert_eq!(dom.remove_event_listener(div, "click", 1, true), None);
    assert_eq!(dom.listeners(div).len(), 2);
}

#[test]
fn event_paths() {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    let html = dom.document_element().unwrap();
    assert_eq!(
        dom.event_path(div),
        vec![div, body, html, Dom::DOCUMENT, Dom::WINDOW]
    );
    let detached = dom.create_element("p").unwrap();
    assert_eq!(dom.event_path(detached), vec![detached]);
    assert_eq!(dom.event_path(Dom::WINDOW), vec![Dom::WINDOW]);
    let target = dom.create_event_target();
    assert_eq!(dom.event_path(target), vec![target]);
}

fn setup() -> (RefCell<Dom>, NodeId, NodeId) {
    let (mut dom, body) = page();
    let div = dom.create_element("div").unwrap();
    dom.append_child(body, div).unwrap();
    (RefCell::new(dom), body, div)
}

fn listen(dom: &RefCell<Dom>, node: NodeId, callback: u64, capture: bool) {
    dom.borrow_mut().add_event_listener(
        node,
        "click",
        callback,
        ListenerOptions {
            capture,
            ..ListenerOptions::default()
        },
    );
}

#[test]
fn phases_run_in_order() {
    let (dom, body, div) = setup();
    listen(&dom, Dom::WINDOW, 1, true);
    listen(&dom, body, 2, true);
    listen(&dom, div, 3, false);
    listen(&dom, div, 4, true);
    listen(&dom, body, 5, false);
    listen(&dom, Dom::WINDOW, 6, false);
    let event = RefCell::new(Event::new("click", true, true));
    let mut calls = Vec::new();
    let not_canceled = dispatch(&dom, div, &event, &mut |listener, current| {
        calls.push((listener.callback, current, event.borrow().phase));
    });
    assert!(not_canceled);
    assert_eq!(
        calls,
        vec![
            (1, Dom::WINDOW, Phase::Capturing),
            (2, body, Phase::Capturing),
            (4, div, Phase::AtTarget),
            (3, div, Phase::AtTarget),
            (5, body, Phase::Bubbling),
            (6, Dom::WINDOW, Phase::Bubbling),
        ]
    );
    let state = event.borrow();
    assert_eq!(state.phase, Phase::None);
    assert_eq!(state.current_target, None);
    assert_eq!(state.target, Some(div));
    assert!(!state.dispatching);
}

#[test]
fn propagation_can_stop() {
    let (dom, body, div) = setup();
    listen(&dom, body, 1, true);
    listen(&dom, div, 2, false);
    let event = RefCell::new(Event::new("click", true, true));
    let mut calls = Vec::new();
    dispatch(&dom, div, &event, &mut |listener, _| {
        calls.push(listener.callback);
        event.borrow_mut().stop_propagation();
    });
    assert_eq!(calls, vec![1]);

    listen(&dom, div, 3, false);
    listen(&dom, body, 4, false);
    let event = RefCell::new(Event::new("click", true, true));
    let mut calls = Vec::new();
    dispatch(&dom, div, &event, &mut |listener, _| {
        calls.push(listener.callback);
        if listener.callback == 2 {
            event.borrow_mut().stop_immediate_propagation();
        }
    });
    assert_eq!(calls, vec![1, 2]);

    let event = RefCell::new(Event::new("click", true, true));
    let mut calls = Vec::new();
    dispatch(&dom, div, &event, &mut |listener, _| {
        calls.push(listener.callback);
        if listener.callback == 3 {
            event.borrow_mut().stop_propagation();
        }
    });
    assert_eq!(calls, vec![1, 2, 3]);
}

#[test]
fn non_bubbling_events_stay_at_the_target() {
    let (dom, body, div) = setup();
    listen(&dom, body, 1, false);
    listen(&dom, div, 2, false);
    let event = RefCell::new(Event::new("click", false, false));
    let mut calls = Vec::new();
    dispatch(&dom, div, &event, &mut |listener, _| {
        calls.push(listener.callback)
    });
    assert_eq!(calls, vec![2]);
}

#[test]
fn cancellation_respects_passive_listeners() {
    let (dom, _, div) = setup();
    dom.borrow_mut().add_event_listener(
        div,
        "click",
        1,
        ListenerOptions {
            passive: true,
            ..ListenerOptions::default()
        },
    );
    let event = RefCell::new(Event::new("click", true, true));
    assert!(dispatch(&dom, div, &event, &mut |_, _| event
        .borrow_mut()
        .prevent_default()));
    listen(&dom, div, 2, false);
    let event = RefCell::new(Event::new("click", true, true));
    assert!(!dispatch(&dom, div, &event, &mut |listener, _| {
        assert_eq!(event.borrow().in_passive_listener, listener.callback == 1);
        event.borrow_mut().prevent_default();
    }));
    let mut plain = Event::new("x", true, false);
    plain.prevent_default();
    assert!(!plain.canceled);
}

#[test]
fn once_and_removed_listeners() {
    let (dom, _, div) = setup();
    dom.borrow_mut().add_event_listener(
        div,
        "click",
        1,
        ListenerOptions {
            once: true,
            ..ListenerOptions::default()
        },
    );
    listen(&dom, div, 2, false);
    listen(&dom, div, 3, false);
    let mut calls = Vec::new();
    for _ in 0..2 {
        let event = RefCell::new(Event::new("click", true, true));
        dispatch(&dom, div, &event, &mut |listener, current| {
            calls.push(listener.callback);
            if listener.callback == 2 {
                dom.borrow_mut()
                    .remove_event_listener(current, "click", 3, false);
            }
        });
    }
    assert_eq!(calls, vec![1, 2, 2]);
}

#[test]
fn phase_values() {
    assert_eq!(Phase::None.value(), 0);
    assert_eq!(Phase::Capturing.value(), 1);
    assert_eq!(Phase::AtTarget.value(), 2);
    assert_eq!(Phase::Bubbling.value(), 3);
}

#[test]
fn stopping_in_an_outer_capture_skips_inner_ones() {
    let (dom, body, div) = setup();
    listen(&dom, Dom::WINDOW, 1, true);
    listen(&dom, body, 2, true);
    let event = RefCell::new(Event::new("click", true, true));
    let mut calls = Vec::new();
    dispatch(&dom, div, &event, &mut |listener, _| {
        calls.push(listener.callback);
        event.borrow_mut().stop_propagation();
    });
    assert_eq!(calls, vec![1]);
}
