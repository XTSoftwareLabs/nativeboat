use super::super::Motion;
use super::*;
use nativeboat_css::{Origin, SheetRef, StyleEnvironment, parse_stylesheet};
use nativeboat_dom::{Dom, NodeId};

fn styles(css: &str) -> (NodeId, Styles, Rc<Keyframes>) {
    let dom = Dom::from_document(&nativeboat_html::parse_document("<div id=a></div>"));
    let node = dom.get_element_by_id(Dom::DOCUMENT, "a").unwrap();
    let sheet = parse_stylesheet(css).0;
    let environment = StyleEnvironment::default();
    let keyframes = Rc::new(keyframes([&sheet], &environment.media));
    (
        node,
        Styles::compute(
            &dom,
            &[SheetRef {
                sheet: &sheet,
                origin: Origin::Author,
            }],
            &environment,
        ),
        keyframes,
    )
}

fn started(css: &str, now: f64) -> (NodeId, Motion) {
    let (node, styles, keyframes) = styles(css);
    let mut motion = Motion::default();
    motion.set_keyframes(keyframes);
    motion.update(styles, now);
    (node, motion)
}

fn opacity(motion: &mut Motion, node: NodeId, now: f64) -> f32 {
    motion.sample(now).get(node).unwrap().opacity
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

fn kinds(motion: &mut Motion) -> Vec<(&'static str, String, f64)> {
    core::mem::take(&mut motion.events)
        .into_iter()
        .map(|event| (event.kind, event.name, event.elapsed))
        .collect()
}

#[test]
fn keyframes_interpolate_between_stops_and_iterations_report_events() {
    let (node, mut motion) = started(
        "@keyframes fade { from { opacity: 0 } 50% { opacity: 1 } to { opacity: 0.5 } }\n\
         #a { animation: fade 2s linear 2 }",
        0.0,
    );
    assert!(motion.active());
    close(opacity(&mut motion, node, 0.5), 0.5);
    close(opacity(&mut motion, node, 1.5), 0.75);
    close(opacity(&mut motion, node, 2.5), 0.5);
    assert_eq!(
        kinds(&mut motion),
        vec![
            ("animationstart", "fade".into(), 0.0),
            ("animationiteration", "fade".into(), 2.0)
        ]
    );
    // Without a fill mode the element shows its own value afterwards.
    close(opacity(&mut motion, node, 5.0), 1.0);
    assert_eq!(
        kinds(&mut motion),
        vec![("animationend", "fade".into(), 4.0)]
    );
    assert!(!motion.active());
}

#[test]
fn missing_end_keyframes_take_the_elements_value() {
    let (node, mut motion) = started(
        "@keyframes pulse { 50% { opacity: 0 } }\n#a { opacity: 0.8; animation: pulse 1s linear }",
        0.0,
    );
    close(opacity(&mut motion, node, 0.25), 0.4);
    close(opacity(&mut motion, node, 0.75), 0.4);
}

#[test]
fn directions_fill_modes_and_delays() {
    let css = |rule: &str| {
        format!(
            "@keyframes grow {{ from {{ opacity: 0.2 }} to {{ opacity: 1 }} }}\n#a {{ {rule} }}"
        )
    };
    let (node, mut motion) = started(&css("animation: grow 1s linear 2 alternate"), 0.0);
    close(opacity(&mut motion, node, 0.5), 0.6);
    close(opacity(&mut motion, node, 1.25), 0.8);
    let (node, mut motion) = started(&css("animation: grow 1s linear reverse"), 0.0);
    close(opacity(&mut motion, node, 0.25), 0.8);
    let (node, mut motion) = started(&css("animation: grow 1s linear 1s backwards"), 0.0);
    close(opacity(&mut motion, node, 0.5), 0.2);
    close(opacity(&mut motion, node, 1.5), 0.6);
    let (node, mut motion) = started(&css("animation: grow 1s linear forwards"), 0.0);
    close(opacity(&mut motion, node, 3.0), 1.0);
    assert!(motion.affects() && !motion.active());
    let (node, mut motion) = started(&css("animation: grow 1s linear 1s"), 0.0);
    close(opacity(&mut motion, node, 0.5), 1.0);
}

#[test]
fn infinite_animations_keep_running_and_pausing_holds_them() {
    let css = |state: &str| {
        format!(
            "@keyframes spin {{ from {{ opacity: 0 }} to {{ opacity: 1 }} }}\n\
             #a {{ animation: spin 1s linear infinite {state} }}"
        )
    };
    let (node, styles_running, keyframes) = styles(&css("running"));
    let mut motion = Motion::default();
    motion.set_keyframes(keyframes);
    motion.update(styles_running, 0.0);
    close(opacity(&mut motion, node, 10.25), 0.25);
    assert!(motion.active());
    motion.update(styles(&css("paused")).1, 10.25);
    close(opacity(&mut motion, node, 20.0), 0.25);
    assert!(!motion.active() && motion.affects());
    motion.update(styles(&css("running")).1, 20.0);
    close(opacity(&mut motion, node, 20.5), 0.75);
}

#[test]
fn keyframe_timing_functions_and_pseudo_elements() {
    let (node, mut motion) = started(
        "@keyframes step { from { opacity: 0; animation-timing-function: steps(2, end) } to { opacity: 1 } }\n\
         #a::before { content: ''; animation: step 1s linear }",
        0.0,
    );
    let presented = motion.sample(0.75);
    close(presented.target((node, 1)).unwrap().opacity, 0.5);
    assert_eq!(motion.events[0].pseudo, "::before");
}

#[test]
fn changing_the_name_restarts_and_removing_it_cancels() {
    let css = |name: &str| {
        format!(
            "@keyframes a {{ to {{ opacity: 0 }} }} @keyframes b {{ to {{ opacity: 0 }} }}\n\
             #a {{ animation: {name} 1s linear }}"
        )
    };
    let (node, styles_a, keyframes) = styles(&css("a"));
    let mut motion = Motion::default();
    motion.set_keyframes(keyframes);
    motion.update(styles_a, 0.0);
    kinds(&mut motion);
    motion.update(styles(&css("b")).1, 0.5);
    assert_eq!(
        kinds(&mut motion),
        vec![
            ("animationcancel", "a".into(), 0.5),
            ("animationstart", "b".into(), 0.0)
        ]
    );
    close(opacity(&mut motion, node, 0.75), 0.75);
    motion.update(styles(&css("none")).1, 0.8);
    let events = kinds(&mut motion);
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].0, events[0].1.as_str()),
        ("animationcancel", "b")
    );
    close(events[0].2 as f32, 0.3);
    assert!(!motion.affects());
}

#[test]
fn the_shorthand_and_keyframes_in_media_rules() {
    let (node, styles, keyframes) = styles(
        "@media (min-width: 1px) { @keyframes inside { to { opacity: 0 } } }\n\
         @media (max-width: 1px) { @keyframes outside { to { opacity: 0 } } }\n\
         #a { animation: 2s ease-in 1s 3 alternate both paused inside }",
    );
    assert!(keyframes.contains_key("inside") && !keyframes.contains_key("outside"));
    let style = styles.get(node).unwrap();
    assert_eq!(style.animation_name.0[0].0, "inside");
    assert_eq!(style.animation_duration.0[0].0, 2.0);
    assert_eq!(style.animation_delay.0[0].0, 1.0);
    assert_eq!(style.animation_iteration_count.0[0], Iterations::Count(3.0));
    assert_eq!(
        style.animation_direction.0[0],
        AnimationDirection::Alternate
    );
    assert_eq!(style.animation_fill_mode.0[0], FillMode::Both);
    assert_eq!(style.animation_play_state.0[0], PlayState::Paused);
    assert_eq!(style.animation_timing_function.0[0], Easing::EASE_IN);
}

#[test]
fn z_index_interpolates_between_integers_and_flips_from_auto_halfway() {
    let z = |motion: &mut Motion, node, now| motion.sample(now).get(node).unwrap().z_index.0;
    let (node, mut motion) = started(
        "@keyframes rise { from { z-index: 1 } to { z-index: 4 } }\n\
         #a { position: relative; animation: rise 1s linear }",
        0.0,
    );
    assert_eq!(z(&mut motion, node, 0.4), Some(2));
    assert_eq!(z(&mut motion, node, 0.9), Some(4));
    // A missing start keyframe takes the element's `auto`.
    let (node, mut motion) = started(
        "@keyframes over { 50% { transform: none } to { z-index: 200 } }\n\
         #a { position: absolute; animation: over 1.2s infinite linear }",
        0.0,
    );
    assert_eq!(z(&mut motion, node, 0.3), None);
    assert_eq!(z(&mut motion, node, 0.9), Some(200));
    assert_eq!(z(&mut motion, node, 1.5), None);
}
