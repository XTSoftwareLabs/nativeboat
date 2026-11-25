use super::*;
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet};
use nativeboat_dom::Dom;

fn styles(css: &str) -> (NodeId, Styles) {
    let dom = Dom::from_document(&nativeboat_html::parse_document("<div id=a></div>"));
    let node = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let sheet = parse_stylesheet(css).0;
    (
        node,
        Styles::compute(
            &dom,
            &[SheetRef {
                sheet: &sheet,
                origin: Origin::Author,
            }],
            &StyleEnvironment::default(),
        ),
    )
}

fn opacity(value: f32, timing: &str) -> (NodeId, Styles) {
    styles(&format!("#a {{opacity:{value}; transition:{timing}}}"))
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}
fn kinds(motion: &mut Motion) -> Vec<&'static str> {
    core::mem::take(&mut motion.events)
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

#[test]
fn timing_lists_repeat_and_last_shorthand_match_wins() {
    let (node, styles) = styles(
        "#a {transition-property: all, opacity, border-color; transition-duration: 1s, 2s; transition-delay: -100ms; transition-timing-function: linear}",
    );
    let style = styles.get(node).unwrap();
    let result = timing(style, PropertyId::Opacity).unwrap();
    close(result.0 as f32, 2.0);
    close(result.1 as f32, -0.1);
    assert_eq!(result.2, Easing::LINEAR);
    close(
        timing(style, PropertyId::BorderLeftColor).unwrap().0 as f32,
        1.0,
    );
    assert_eq!(
        timing(
            &ComputedStyle {
                transition_property: nativeboat_css::types::List(vec![]),
                ..ComputedStyle::default()
            },
            PropertyId::Opacity
        ),
        None
    );
}

#[test]
fn delayed_negative_and_zero_duration_transitions_emit_their_events() {
    for (source, midpoint, end, expected_middle, expected_elapsed) in [
        ("opacity 1s linear .5s", 1.0, 1.5, 0.5, 0.0),
        ("opacity 1s linear -.5s", 0.0, 0.5, 0.5, 0.5),
        ("opacity 0s linear .5s", 0.0, 0.5, 0.0, 0.0),
    ] {
        let (node, before) = opacity(0.0, source);
        let mut motion = Motion::default();
        motion.update(before, 0.0);
        assert!(!motion.active());
        motion.update(opacity(1.0, source).1, 0.0);
        assert!(motion.has_work());
        assert_eq!(motion.events[0].kind, "transitionrun");
        assert_eq!(motion.events[0].elapsed, expected_elapsed);
        close(
            motion.sample(midpoint).get(node).unwrap().opacity,
            expected_middle,
        );
        close(motion.sample(end).get(node).unwrap().opacity, 1.0);
        assert!(!motion.active());
        assert_eq!(
            kinds(&mut motion),
            vec!["transitionrun", "transitionstart", "transitionend"]
        );
        assert!(!motion.has_work());
    }
}

#[test]
fn interrupted_transitions_reverse_faster_and_keep_the_presented_value() {
    let source = "opacity 1s linear";
    let (node, before) = opacity(0.0, source);
    let mut motion = Motion::default();
    motion.update(before.clone(), 0.0);
    motion.update(opacity(1.0, source).1, 0.0);
    close(motion.sample(0.25).get(node).unwrap().opacity, 0.25);
    kinds(&mut motion);
    motion.update(before, 0.25);
    assert_eq!(
        kinds(&mut motion),
        vec!["transitioncancel", "transitionrun", "transitionstart"]
    );
    close(motion.sample(0.375).get(node).unwrap().opacity, 0.125);
    motion.update(opacity(1.0, source).1, 0.375);
    close(motion.sample(0.8125).get(node).unwrap().opacity, 0.5625);
    close(motion.sample(1.25).get(node).unwrap().opacity, 1.0);
}

#[test]
fn retargeting_starts_from_current_value_without_restarting_unchanged_targets() {
    let source = "opacity 1s linear";
    let (node, before) = opacity(0.0, source);
    let mut motion = Motion::default();
    motion.update(before, 0.0);
    motion.update(opacity(1.0, source).1, 0.0);
    motion.update(
        styles("#a {opacity:1; transition:opacity 5s ease; color:red}").1,
        0.25,
    );
    close(motion.sample(0.5).get(node).unwrap().opacity, 0.5);
    motion.update(opacity(0.8, source).1, 0.5);
    close(motion.sample(1.0).get(node).unwrap().opacity, 0.65);
    close(motion.sample(1.5).get(node).unwrap().opacity, 0.8);
    kinds(&mut motion);
    motion.update(opacity(0.8, source).1, 2.0);
    assert!(!motion.has_work());
}

#[test]
fn removed_hidden_unmatched_and_non_interpolatable_styles_cancel() {
    for after in [
        Styles::default(),
        styles("#a {display:none}").1,
        opacity(1.0, "none").1,
        opacity(0.2, "opacity 0s").1,
        opacity(0.25, "opacity 1s linear").1,
    ] {
        let mut motion = Motion::default();
        motion.update(opacity(0.0, "opacity 1s linear").1, 0.0);
        motion.update(opacity(1.0, "opacity 1s linear").1, 0.0);
        kinds(&mut motion);
        motion.update(after, 0.25);
        assert!(!motion.active());
        assert_eq!(kinds(&mut motion), vec!["transitioncancel"]);
    }
    let mut motion = Motion::default();
    motion.update(styles("#a {width:20px;transition:width 1s}").1, 0.0);
    motion.update(styles("#a {width:40px;transition:width 1s}").1, 0.0);
    kinds(&mut motion);
    motion.update(styles("#a {width:auto;transition:width 1s}").1, 0.25);
    assert_eq!(kinds(&mut motion), vec!["transitioncancel"]);
    assert!(Motion::default().sample(0.0).is_empty());
}

#[test]
fn negative_reverse_delays_scale_with_the_shortened_duration() {
    let source = "opacity 1s linear -.2s";
    let (node, before) = opacity(0.0, source);
    let mut motion = Motion::default();
    motion.update(before.clone(), 0.0);
    motion.update(opacity(1.0, source).1, 0.0);
    close(motion.sample(0.3).get(node).unwrap().opacity, 0.5);
    motion.update(before, 0.3);
    close(motion.sample(0.3).get(node).unwrap().opacity, 0.4);
    close(motion.sample(0.7).get(node).unwrap().opacity, 0.0);
}

#[test]
fn pseudo_elements_transition_and_name_themselves_in_events() {
    let rule = |opacity: f32| {
        styles(&format!(
            "#a::before {{content:''; opacity:{opacity}; transition:opacity 1s linear}}"
        ))
    };
    let (node, before) = rule(0.0);
    let mut motion = Motion::default();
    motion.update(before, 0.0);
    motion.update(rule(1.0).1, 0.0);
    assert!(motion.active());
    let presented = motion.sample(0.5);
    close(presented.target((node, 1)).unwrap().opacity, 0.5);
    assert_eq!(presented.get(node).unwrap().opacity, 1.0);
    let events = core::mem::take(&mut motion.events);
    assert_eq!(
        (events[0].kind, events[0].pseudo, events[0].property),
        ("transitionrun", "::before", "opacity")
    );
    close(motion.sample(1.0).target((node, 1)).unwrap().opacity, 1.0);
    assert!(!motion.active());
}

#[test]
fn descendants_that_inherit_a_transitioning_value_present_it() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<div id=a><p id=b><span id=c></span></p><em id=d></em></div>",
    ));
    let id = |name: &str| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let styled = |color: &str| {
        let sheet = parse_stylesheet(&format!(
            "#a {{color:{color}; transition:color 1s linear}} #a::before {{content:''}} #d {{color:blue}}"
        ))
        .0;
        Styles::compute(
            &dom,
            &[SheetRef {
                sheet: &sheet,
                origin: Origin::Author,
            }],
            &StyleEnvironment::default(),
        )
    };
    let mut motion = Motion::default();
    motion.update(styled("rgb(0, 0, 0)"), 0.0);
    motion.update(styled("rgb(200, 100, 0)"), 0.0);
    let middle = nativeboat_css::Color::rgb(100, 50, 0);
    let presented = motion.sample_inherited(0.5, &dom);
    for name in ["a", "b", "c"] {
        assert_eq!(presented.get(id(name)).unwrap().color, middle, "{name}");
    }
    assert_eq!(presented.target((id("a"), 1)).unwrap().color, middle);
    // An element with a value of its own keeps it.
    assert_eq!(
        presented.get(id("d")).unwrap().color,
        nativeboat_css::Color::rgb(0, 0, 255)
    );
    // Without the document, only the element transitions.
    let plain = motion.sample(0.5);
    assert_eq!(plain.get(id("a")).unwrap().color, middle);
    assert_eq!(
        plain.get(id("c")).unwrap().color,
        nativeboat_css::Color::rgb(200, 100, 0)
    );
}
