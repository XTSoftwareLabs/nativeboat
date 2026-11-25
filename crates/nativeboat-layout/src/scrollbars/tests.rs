use super::*;
use crate::paint::scene;
use crate::styles::Styles;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, TextMeasurer};
use nativeboat_css::{
    Color, Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet,
};
use nativeboat_ir::{DisplayItem, Font};

struct Mono;

impl TextMeasurer for Mono {
    fn width(&self, text: &str, font: &Font) -> f32 {
        text.chars().count() as f32 * font.size * 0.5
    }

    fn metrics(&self, font: &Font) -> FontMetrics {
        FontMetrics {
            ascent: font.size * 0.8,
            descent: font.size * 0.2,
        }
    }
}

fn render(dom: &Dom, css: &str) -> Scene {
    render_with(dom, css, &Mono)
}

fn render_with(dom: &Dom, css: &str, measurer: &dyn TextMeasurer) -> Scene {
    let (author, _) = parse_stylesheet(css);
    let sheets = [
        SheetRef {
            sheet: user_agent_stylesheet(),
            origin: Origin::UserAgent,
        },
        SheetRef {
            sheet: &author,
            origin: Origin::Author,
        },
    ];
    let styles = Styles::compute(dom, &sheets, &StyleEnvironment::default());
    let options = LayoutOptions {
        viewport: (400.0, 300.0),
        measurer,
        image_size: &|_| Some((40.0, 20.0)),
    };
    let layout = layout(dom, &styles, &options);
    scene(dom, &styles, &layout, (400.0, 300.0))
}

const HTML: &str = "<div id=pop><div id=list><p id=first>a</p><p>b</p><p>c</p>\
    <p>d</p><p>e</p><p>f</p></div></div>";
/// A list scaled to half its size, whose content overflows both ways.
const CSS: &str = "body{margin:0} #pop{transform:scale(0.5);transform-origin:0 0} \
    #list{width:200px;height:100px;overflow:auto} p{margin:0;height:40px;width:400px}";

fn page() -> (Dom, NodeId) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(HTML));
    let list = dom.get_element_by_id(Dom::DOCUMENT, "list").unwrap();
    (dom, list)
}

/// A point of the list's padding box in window coordinates.
fn at(x: f32, y: f32) -> (f32, f32) {
    (x * 0.5, y * 0.5)
}

fn fills(items: &[DisplayItem]) -> Vec<Color> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { color, .. } => Some(*color),
            _ => None,
        })
        .collect()
}

#[test]
fn parts_keep_their_codes() {
    for part in [
        Part::Track(0),
        Part::Track(1),
        Part::Thumb(0),
        Part::Thumb(1),
        Part::Button(0, false),
        Part::Button(0, true),
        Part::Button(1, false),
        Part::Button(1, true),
        Part::Corner,
    ] {
        assert_eq!(super::part(code(part)), Some(part));
    }
    assert_eq!(super::part(9), None);
}

#[test]
fn scaled_scroll_containers_paint_scrollbars_that_follow_the_transform() {
    let (mut dom, list) = page();
    let scene = render(&dom, CSS);
    let scroller = *scene.in_place(list).unwrap();
    assert_eq!(scroller.view, (183.0, 83.0));
    assert_eq!(scroller.maximum, (217.0, 157.0));
    assert!(!scene.has_surface(list));
    let root = &scene.tree.root;
    assert!(root.children.is_empty());
    // Chromium's default look: tracks, arrows, thumbs and the corner.
    let colors = fills(&root.items);
    assert!(colors.contains(&Color::rgb(241, 241, 241)));
    assert!(colors.contains(&Color::rgb(193, 193, 193)));
    assert!(colors.contains(&Color::rgb(220, 220, 220)));
    let (x, y) = at(190.0, 5.0);
    assert_eq!(
        scene
            .scrollbar_at(x, y)
            .map(|(found, part)| (found.node, part)),
        Some((list, Part::Button(1, false)))
    );
    // Hovered and pressed parts paint darker.
    let (x, y) = at(190.0, 20.0);
    assert!(hover(&scene, &mut dom, x, y));
    assert_eq!(
        dom.scrollbar_parts(),
        (Some((list, code(Part::Thumb(1)))), None)
    );
    let scene = render(&dom, CSS);
    assert!(fills(&scene.tree.root.items).contains(&Color::rgb(168, 168, 168)));
    assert!(!hover(&scene, &mut dom, 10.0, 10.0));
    assert_eq!(dom.scrollbar_parts(), (None, None));
    // Styled scrollbars paint as their styles say, without arrows.
    let styled = format!(
        "{CSS} #list::-webkit-scrollbar{{width:10px;height:10px}} \
        #list::-webkit-scrollbar-thumb{{background:#0000ff}}"
    );
    let scene = render(&dom, &styled);
    let scroller = scene.in_place(list).unwrap();
    assert_eq!(scroller.bars.bars[1].unwrap().buttons, None);
    assert!(fills(&scene.tree.root.items).contains(&Color::rgb(0, 0, 255)));
    // Untransformed, the list scrolls on a native surface of its own.
    let scene = render(
        &dom,
        "#list{width:200px;height:100px;overflow:auto} p{height:40px}",
    );
    assert!(scene.has_surface(list));
    assert!(scene.in_place(list).is_none());
    let first = dom.get_element_by_id(Dom::DOCUMENT, "first").unwrap();
    assert!(!wheel(&scene, &mut dom, first, (0.0, 40.0)));
}

#[test]
fn pressing_scrollbars_scrolls_by_steps_pages_and_drags() {
    let (mut dom, list) = page();
    let scene = render(&dom, CSS);
    // The arrow at the bottom scrolls a step.
    let (x, y) = at(190.0, 70.0);
    assert_eq!(press(&scene, &mut dom, x, y), Some(None));
    assert_eq!(dom.scroll(list), (0.0, 40.0));
    let pressed = Some((list, code(Part::Button(1, true))));
    assert_eq!(dom.scrollbar_parts(), (pressed, pressed));
    release(&mut dom);
    assert_eq!(dom.scrollbar_parts(), (pressed, None));
    // The track below the thumb pages down, above it up: by seven eighths
    // of the 83 pixels the list shows, as in Chromium.
    let scene = render(&dom, CSS);
    let (x, y) = at(190.0, 60.0);
    assert_eq!(press(&scene, &mut dom, x, y), Some(None));
    assert_eq!(dom.scroll(list), (0.0, 112.0));
    let scene = render(&dom, CSS);
    let (x, y) = at(190.0, 20.0);
    assert_eq!(press(&scene, &mut dom, x, y), Some(None));
    assert_eq!(dom.scroll(list), (0.0, 40.0));
    // The arrow at the left cannot scroll further left.
    let (x, y) = at(5.0, 90.0);
    assert_eq!(press(&scene, &mut dom, x, y), Some(None));
    assert_eq!(dom.scroll(list), (0.0, 40.0));
    // The corner does nothing.
    let (x, y) = at(190.0, 90.0);
    assert_eq!(press(&scene, &mut dom, x, y), Some(None));
    // The thumb drags: the content follows it as far as it goes.
    let scene = render(&dom, CSS);
    let bar = scene.in_place(list).unwrap().bars.bars[1].unwrap();
    let grab = bar.thumb.y + 2.0;
    let (x, y) = at(190.0, grab);
    let dragging = press(&scene, &mut dom, x, y).unwrap().unwrap();
    assert_eq!(dragging.node(), list);
    let (x, y) = at(190.0, grab + 10.0);
    dragging.drag(&scene, &mut dom, x, y);
    let top = dom.scroll(list).1;
    assert!(
        (top - (40.0 + 10.0 / bar.travel * 157.0)).abs() < 0.01,
        "{top}"
    );
    let (x, y) = at(190.0, grab + 500.0);
    dragging.drag(&scene, &mut dom, x, y);
    assert_eq!(dom.scroll(list).1, 157.0);
    // Nothing is pressed off the scrollbars.
    assert_eq!(press(&scene, &mut dom, 10.0, 10.0), None);
}

#[test]
fn held_arrows_and_tracks_scroll_again_while_the_pointer_stays_over_them() {
    let (mut dom, list) = page();
    let scene = render(&dom, CSS);
    let arrow = at(190.0, 70.0);
    assert_eq!(press(&scene, &mut dom, arrow.0, arrow.1), Some(None));
    assert!(repeat(&scene, &mut dom, arrow.0, arrow.1));
    assert_eq!(dom.scroll(list), (0.0, 80.0));
    // Off the arrow it stays held but scrolls no more, until the pointer
    // comes back.
    let away = at(10.0, 10.0);
    hover(&scene, &mut dom, away.0, away.1);
    assert!(repeat(&scene, &mut dom, away.0, away.1));
    assert_eq!(dom.scroll(list), (0.0, 80.0));
    hover(&scene, &mut dom, arrow.0, arrow.1);
    assert!(repeat(&scene, &mut dom, arrow.0, arrow.1));
    assert_eq!(dom.scroll(list), (0.0, 120.0));
    release(&mut dom);
    assert!(!repeat(&scene, &mut dom, arrow.0, arrow.1));
    // The track pages toward the pointer until the thumb reaches it.
    dom.set_scroll(list, 0.0, 0.0);
    let track = at(190.0, 60.0);
    let scene = render(&dom, CSS);
    assert_eq!(press(&scene, &mut dom, track.0, track.1), Some(None));
    assert_eq!(dom.scroll(list), (0.0, 72.0));
    let scene = render(&dom, CSS);
    assert!(repeat(&scene, &mut dom, track.0, track.1));
    assert_eq!(dom.scroll(list), (0.0, 144.0));
    let scene = render(&dom, CSS);
    let bar = scene.in_place(list).unwrap().bars.bars[1].unwrap();
    assert!(bar.thumb.y <= 60.0 && bar.thumb.y + bar.thumb.height > 60.0);
    assert!(repeat(&scene, &mut dom, track.0, track.1));
    assert_eq!(dom.scroll(list), (0.0, 144.0));
    // A held thumb drags rather than repeating.
    release(&mut dom);
    let grab = at(190.0, bar.thumb.y + 2.0);
    assert!(press(&scene, &mut dom, grab.0, grab.1).unwrap().is_some());
    assert!(!repeat(&scene, &mut dom, grab.0, grab.1));
}

#[test]
fn wheels_scroll_the_innermost_container_painted_in_place_by_a_page_at_most() {
    let (mut dom, list) = page();
    let scene = render(&dom, CSS);
    let first = dom.get_element_by_id(Dom::DOCUMENT, "first").unwrap();
    assert!(wheel(&scene, &mut dom, first, (0.0, 1000.0)));
    assert_eq!(dom.scroll(list), (0.0, 83.0));
    assert!(wheel(&scene, &mut dom, first, (30.0, 1000.0)));
    assert_eq!(dom.scroll(list), (30.0, 157.0));
    // At the end, the wheel passes the list by.
    assert!(!wheel(&scene, &mut dom, first, (0.0, 40.0)));
    // A drag of a container that is gone does nothing.
    let dragging = Dragging {
        node: first,
        axis: 1,
        pointer: 0.0,
        scroll: 0.0,
    };
    dragging.drag(&scene, &mut dom, 10.0, 10.0);
    assert_eq!(dom.scroll(first), (0.0, 0.0));
}

#[test]
fn content_past_a_scroller_by_less_than_half_a_pixel_does_not_scroll() {
    /// Measures as [`Mono`], at 125%, so boxes lie at fractions of pixels.
    struct Device;
    impl TextMeasurer for Device {
        fn width(&self, text: &str, font: &Font) -> f32 {
            Mono.width(text, font)
        }
        fn metrics(&self, font: &Font) -> FontMetrics {
            Mono.metrics(font)
        }
        fn device_pixel(&self) -> f32 {
            0.8
        }
    }
    // Chromium snaps both sizes to whole pixels before it measures how far
    // a box scrolls, so a forced bar shows no thumb. The list is scaled to
    // be painted in place.
    let scroller = |height: &str| {
        let dom = Dom::from_document(&nativeboat_html::parse_document(
            "<div id=pop><div id=list><p>a</p></div></div>",
        ));
        let list = dom.get_element_by_id(Dom::DOCUMENT, "list").unwrap();
        let css = format!(
            "body{{margin:0}} #pop{{transform:scale(0.5)}} \
            #list{{width:200px;height:100px;overflow-y:scroll}} p{{margin:0;height:{height}}}"
        );
        let scene = render_with(&dom, &css, &Device);
        let thumb = fills(&scene.tree.root.items).contains(&Color::rgb(193, 193, 193));
        (scene.in_place(list).unwrap().maximum.1, thumb)
    };
    assert_eq!(scroller("100.3px"), (0.0, false));
    assert_eq!(scroller("101px"), (1.0, true));
}
