use super::*;
use crate::paint::scene;
use crate::tree::{LayoutOptions, layout};
use crate::{FontMetrics, TextMeasurer};
use PathSegment::{Close, Cubic, Line, Move};
use nativeboat_css::types::{FillRule, LineCap, LineJoin};
use nativeboat_css::{
    Color, Origin, SheetRef, StyleEnvironment, parse_stylesheet, user_agent_stylesheet,
};
use nativeboat_ir::{Corner, Font};

fn close(actual: &[PathSegment], expected: &[PathSegment]) {
    let flatten = |segments: &[PathSegment]| -> Vec<f32> {
        segments
            .iter()
            .flat_map(|segment| match *segment {
                Move(x, y) => vec![0.0, x, y],
                Line(x, y) => vec![1.0, x, y],
                Cubic(a, b, c, d, e, f) => vec![2.0, a, b, c, d, e, f],
                Close => vec![3.0],
            })
            .collect()
    };
    let (actual_numbers, expected_numbers) = (flatten(actual), flatten(expected));
    assert_eq!(
        actual_numbers.len(),
        expected_numbers.len(),
        "{actual:?} != {expected:?}"
    );
    for (a, e) in actual_numbers.iter().zip(&expected_numbers) {
        assert!((a - e).abs() < 1e-3, "{actual:?} != {expected:?}");
    }
}

fn matrix_close(actual: Matrix, expected: Matrix) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{actual:?} != {expected:?}");
    }
}

/// The end point of the last segment.
fn end(segments: &[PathSegment]) -> (f32, f32) {
    match segments.last() {
        Some(Line(x, y) | Move(x, y) | Cubic(_, _, _, _, x, y)) => (*x, *y),
        other => panic!("{other:?}"),
    }
}

#[test]
fn numbers_follow_svg_syntax() {
    assert_eq!(
        numbers("1.5.5-1-2,+.5 1e2 1E-1"),
        [1.5, 0.5, -1.0, -2.0, 0.5, 100.0, 0.1]
    );
    assert_eq!(numbers("2e"), [2.0]);
    assert_eq!(numbers("3e+"), [3.0]);
    assert!(numbers(".").is_empty());
    assert!(numbers("-").is_empty());
    assert!(numbers("abc").is_empty());
}

#[test]
fn lines_and_implicit_commands() {
    close(
        &path_data("M10 20 30 40 m1 1 2 2 H5 h1 V6 v1 L1 1 l1 1 z"),
        &[
            Move(10.0, 20.0),
            Line(30.0, 40.0),
            Move(31.0, 41.0),
            Line(33.0, 43.0),
            Line(5.0, 43.0),
            Line(6.0, 43.0),
            Line(6.0, 6.0),
            Line(6.0, 7.0),
            Line(1.0, 1.0),
            Line(2.0, 2.0),
            Close,
        ],
    );
    close(
        &path_data("M0 0 L5 5 Z l1 1"),
        &[Move(0.0, 0.0), Line(5.0, 5.0), Close, Line(1.0, 1.0)],
    );
}

#[test]
fn curves_and_reflections() {
    close(
        &path_data("M0 0 C1 2 3 4 5 6 S9 10 11 12 c1 1 2 2 3 3 s1 1 2 2"),
        &[
            Move(0.0, 0.0),
            Cubic(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
            Cubic(7.0, 8.0, 9.0, 10.0, 11.0, 12.0),
            Cubic(12.0, 13.0, 13.0, 14.0, 14.0, 15.0),
            Cubic(15.0, 16.0, 15.0, 16.0, 16.0, 17.0),
        ],
    );
    close(
        &path_data("M0 0 S3 3 6 0"),
        &[Move(0.0, 0.0), Cubic(0.0, 0.0, 3.0, 3.0, 6.0, 0.0)],
    );
    close(
        &path_data("M0 0 Q3 3 6 0 T12 0 q3 -3 6 0 t6 0"),
        &[
            Move(0.0, 0.0),
            Cubic(2.0, 2.0, 4.0, 2.0, 6.0, 0.0),
            Cubic(8.0, -2.0, 10.0, -2.0, 12.0, 0.0),
            Cubic(14.0, -2.0, 16.0, -2.0, 18.0, 0.0),
            Cubic(20.0, 2.0, 22.0, 2.0, 24.0, 0.0),
        ],
    );
    close(
        &path_data("M0 0 T6 0"),
        &[Move(0.0, 0.0), Cubic(0.0, 0.0, 2.0, 0.0, 6.0, 0.0)],
    );
}

#[test]
fn arcs_become_cubic_curves() {
    // A half circle of radius 5 from (0, 0) to (10, 0).
    let half = path_data("M0 0 A5 5 0 0 1 10 0");
    assert_eq!(half.len(), 3);
    assert_eq!(end(&half), (10.0, 0.0));
    let Cubic(_, _, _, _, x, y) = half[1] else {
        panic!("{half:?}");
    };
    assert!((x - 5.0).abs() < 1e-3 && (y + 5.0).abs() < 1e-3, "{half:?}");
    // The other sweep direction bulges the other way.
    let other = path_data("M0 0 A5 5 0 0 0 10 0");
    let Cubic(_, _, _, _, _, y) = other[1] else {
        panic!("{other:?}");
    };
    assert!((y - 5.0).abs() < 1e-3, "{other:?}");
    // Large arcs take the long way round; flags may be written together.
    let large = path_data("M0 0 a5 5 0 1010 0");
    assert_eq!(large.len(), 3);
    let large = path_data("M0 0 A10 10 0 1 1 10 0");
    assert!(large.len() >= 4, "{large:?}");
    assert_eq!(end(&large), (10.0, 0.0));
    let small = path_data("M0 0 A10 10 0 0 0 10 0");
    assert_eq!(small.len(), 2);
    // Radii too small for the end points are scaled up; rotation applies.
    let scaled = path_data("M0 0 A1 1 30 0 1 10 0");
    assert_eq!(end(&scaled), (10.0, 0.0));
    // Zero radii draw a line and a zero-length arc draws nothing.
    close(
        &path_data("M0 0 A0 5 0 0 1 10 0 A5 5 0 0 1 10 0"),
        &[Move(0.0, 0.0), Line(10.0, 0.0)],
    );
}

#[test]
fn path_errors_stop_drawing() {
    for data in [
        "5 5",
        "M10",
        "M0 0 L",
        "M0 0 H",
        "M0 0 V",
        "M0 0 C",
        "M0 0 C1 2",
        "M0 0 C1 2 3 4",
        "M0 0 S1 2",
        "M0 0 Q1",
        "M0 0 Q1 1",
        "M0 0 T",
        "M0 0 A1 1 0 1",
        "M0 0 A1 1 0 2 1 5 5",
        "M0 0 X 1 1",
        "M0 0 Z 5 5",
    ] {
        let segments = path_data(data);
        assert!(
            segments.len() <= 2,
            "{data}: {segments:?} should stop at the error"
        );
    }
    assert!(path_data("").is_empty());
}

#[test]
fn transform_lists() {
    matrix_close(
        transform_list("matrix(1 2 3 4 5 6)"),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
    );
    matrix_close(
        transform_list("translate(3)"),
        [1.0, 0.0, 0.0, 1.0, 3.0, 0.0],
    );
    matrix_close(
        transform_list(" translate(1,2) scale(2) "),
        [2.0, 0.0, 0.0, 2.0, 1.0, 2.0],
    );
    matrix_close(transform_list("scale(2 3)"), [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    matrix_close(
        transform_list("rotate(90)"),
        [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
    );
    matrix_close(
        transform_list("rotate(90 1 1)"),
        [0.0, 1.0, -1.0, 0.0, 2.0, 0.0],
    );
    matrix_close(transform_list("skewX(45)"), [1.0, 0.0, 1.0, 1.0, 0.0, 0.0]);
    matrix_close(transform_list("skewY(45)"), [1.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
    for invalid in [
        "bogus(1)",
        "rotate(1 2)",
        "scale(1",
        "translate",
        "scale(1) x",
    ] {
        assert_eq!(transform_list(invalid), IDENTITY, "{invalid}");
    }
}

#[test]
fn view_boxes_fit_the_viewport() {
    let viewport = Rect::new(10.0, 20.0, 100.0, 50.0);
    assert_eq!(view_box(None, None, viewport), translate(10.0, 20.0));
    assert_eq!(
        view_box(Some("0 0 0 10"), None, viewport),
        translate(10.0, 20.0)
    );
    // Centered and scaled to fit by default.
    matrix_close(
        view_box(Some("0 0 10 10"), None, viewport),
        [5.0, 0.0, 0.0, 5.0, 35.0, 20.0],
    );
    matrix_close(
        view_box(Some("5 5 10 10"), Some("xMinYMax slice"), viewport),
        [10.0, 0.0, 0.0, 10.0, -40.0, -80.0],
    );
    matrix_close(
        view_box(Some("0 0 10 10"), Some("xMaxYMin meet"), viewport),
        [5.0, 0.0, 0.0, 5.0, 60.0, 20.0],
    );
    matrix_close(
        view_box(Some("0,0,10,10"), Some("none"), viewport),
        [10.0, 0.0, 0.0, 5.0, 10.0, 20.0],
    );
}

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

/// The display items an inline SVG paints into a 100×100 box at (0, 0).
fn painted(html: &str, css: &str) -> Vec<DisplayItem> {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
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
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let svg = dom.get_elements_by_tag_name(Dom::DOCUMENT, "svg")[0];
    let mut items = Vec::new();
    paint(
        &dom,
        &styles,
        svg,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        &mut items,
        Some(&Mono),
    );
    items
}

/// Computed styles of a document with author CSS.
fn styled(html: &str, css: &str) -> (Dom, Styles) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
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
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    (dom, styles)
}

/// A painted path: segments, transform, fill and stroke.
type Painted = (Vec<PathSegment>, Matrix, Option<Color>, Option<Stroke>);

/// The paths among display items.
fn paths(items: &[DisplayItem]) -> Vec<Painted> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Path {
                segments,
                transform,
                fill,
                stroke,
                ..
            } => Some((segments.clone(), *transform, *fill, stroke.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn shapes_take_fill_and_stroke_from_styles() {
    let items = painted(
        r#"<div style="color: rgb(0, 0, 255)"><svg viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="bevel">
        <rect width="4" height="2"/><rect x="1" y="1" width="4" height="4" rx="1"/><rect width="0" height="5"/>
        <circle cx="5" cy="5" r="2" fill="red" stroke="none"/><ellipse cx="5" cy="5" rx="2" ry="1"/><circle r="0"/>
        <line x1="0" y1="0" x2="10" y2="10"/><polyline points="0,0 5,5 10,0"/><polygon points="0 0 5 5 10 0 7"/><polyline/>
        <path d="M0 0 L10 10" fill-rule="evenodd" fill="green"/><path d=""/><title>x</title>
        </svg></div>"#,
        "",
    );
    assert_eq!(
        items.first(),
        Some(&DisplayItem::PushClip {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            radii: [Corner::SQUARE; 4]
        })
    );
    assert_eq!(items.last(), Some(&DisplayItem::PopClip));
    let blue = Color::rgb(0, 0, 255);
    let stroke = Stroke {
        color: blue,
        width: 2.0,
        cap: LineCap::Round,
        join: LineJoin::Bevel,
        dashes: Vec::new(),
        dash_offset: 0.0,
    };
    let paths = paths(&items);
    assert_eq!(paths.len(), 8);
    for (_, transform, _, _) in &paths {
        assert_eq!(*transform, [10.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    }
    close(
        &paths[0].0,
        &[
            Move(0.0, 0.0),
            Line(4.0, 0.0),
            Line(4.0, 2.0),
            Line(0.0, 2.0),
            Close,
        ],
    );
    assert_eq!((paths[0].2, paths[0].3.clone()), (None, Some(stroke)));
    assert_eq!(paths[1].0.len(), 10, "rounded rectangle");
    let elliptical = painted(
        r#"<svg viewBox="0 0 10 10"><rect width="4" height="4" rx="1" ry="2"/></svg>"#,
        "",
    );
    let Cubic(_, _, _, _, x, y) = self::paths(&elliptical)[0].0[2] else {
        panic!("{elliptical:?}");
    };
    assert_eq!((x, y), (4.0, 2.0), "corners use both radii");
    assert_eq!(paths[2].2, Some(Color::rgb(255, 0, 0)));
    assert_eq!(paths[2].3, None);
    assert_eq!(paths[2].0.len(), 6, "circle");
    assert_eq!(paths[3].0.len(), 6, "ellipse");
    close(&paths[4].0, &[Move(0.0, 0.0), Line(10.0, 10.0)]);
    close(
        &paths[5].0,
        &[Move(0.0, 0.0), Line(5.0, 5.0), Line(10.0, 0.0)],
    );
    close(
        &paths[6].0,
        &[Move(0.0, 0.0), Line(5.0, 5.0), Line(10.0, 0.0), Close],
    );
    assert_eq!(paths[7].2, Some(Color::rgb(0, 128, 0)));
    assert!(items.iter().any(|item| matches!(
        item,
        DisplayItem::Path {
            fill_rule: FillRule::EvenOdd,
            ..
        }
    )));
}

#[test]
fn groups_transforms_opacity_and_visibility() {
    let items = painted(
        r#"<svg width="50" height="50"><g transform="translate(5 5)" opacity="0.5" fill="blue" fill-opacity="0.5">
        <rect width="1" height="1" transform="scale(2)"/><rect width="1" height="1" visibility="hidden"/>
        <rect width="1" height="1" style="display: none"/><rect width="1" height="1" fill="none"/>
        <rect width="1" height="1" stroke="red" stroke-width="0" fill="none"/>
        <a><rect width="1" height="1" stroke="red" stroke-opacity="0.5"/></a></g><unknown/></svg>"#,
        ".hidden { visibility: hidden }",
    );
    let paths = paths(&items);
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0].1, [2.0, 0.0, 0.0, 2.0, 5.0, 5.0]);
    let fill = paths[0].2.unwrap();
    assert!((fill.a - 0.25).abs() < 1e-6, "{fill:?}");
    let stroke = paths[1].3.clone().unwrap();
    assert!((stroke.color.a - 0.25).abs() < 1e-6, "{stroke:?}");
}

#[test]
fn nested_viewports_uses_and_percentages() {
    let items = painted(
        r##"<svg viewBox="0 0 200 200"><svg x="100" width="50%" height="100" viewBox="0 0 10 10">
        <rect width="100%" height="50%"/></svg><svg><rect x="10%" width="10" height="10"/></svg>
        <symbol id="icon" viewBox="0 0 2 2"><circle cx="1" cy="1" r="50%"/></symbol>
        <rect id="box" width="1" height="1"/><use href="#icon" x="20" y="30" width="20" height="20"/><use href="#icon"/>
        <use xlink:href="#box" x="7"/><use href="#missing"/><use/><use id="loop" href="#loop"/></svg>"##,
        "",
    );
    let paths = paths(&items);
    assert_eq!(paths.len(), 6, "{paths:?}");
    // The nested viewport maps its 10×10 view box into 100×100 user units.
    assert_eq!(paths[0].1, [5.0, 0.0, 0.0, 5.0, 50.0, 0.0]);
    close(
        &paths[0].0,
        &[
            Move(0.0, 0.0),
            Line(10.0, 0.0),
            Line(10.0, 5.0),
            Line(0.0, 5.0),
            Close,
        ],
    );
    // Without a size, a nested viewport fills its parent's.
    assert_eq!(paths[1].1, [0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
    assert_eq!(
        paths[1].0[0],
        Move(20.0, 0.0),
        "10% of the 200 unit wide view box"
    );
    assert_eq!(paths[2].1, [0.5, 0.0, 0.0, 0.5, 0.0, 0.0], "the rect");
    // A symbol is mapped into the <use> size, or the viewport without one.
    assert_eq!(paths[3].1, [5.0, 0.0, 0.0, 5.0, 10.0, 15.0]);
    let Move(x, _) = paths[3].0[0] else {
        panic!("{paths:?}");
    };
    assert!((x - 2.0).abs() < 1e-4, "r = 50% of the view box diagonal");
    assert_eq!(paths[4].1, [50.0, 0.0, 0.0, 50.0, 0.0, 0.0]);
    assert_eq!(paths[5].1, [0.5, 0.0, 0.0, 0.5, 3.5, 0.0]);
}

fn render(html: &str) -> (Dom, crate::LayoutResult, crate::Scene) {
    let dom = Dom::from_document(&nativeboat_html::parse_document(html));
    let sheets = [SheetRef {
        sheet: user_agent_stylesheet(),
        origin: Origin::UserAgent,
    }];
    let styles = Styles::compute(&dom, &sheets, &StyleEnvironment::default());
    let options = LayoutOptions {
        viewport: (400.0, 300.0),
        measurer: &Mono,
        image_size: &|_| None,
    };
    let layout = layout(&dom, &styles, &options);
    let scene = scene(&dom, &styles, &layout, (400.0, 300.0));
    (dom, layout, scene)
}

#[test]
fn inline_svg_is_laid_out_and_painted() {
    let (dom, layout, scene) = render(
        r#"<div><svg id="a" width="24" height="24" viewBox="0 0 24 24"><path d="M0 0 H24"/></svg>
        <svg id="b" viewBox="0 0 30 10"></svg><svg id="c"></svg></div>"#,
    );
    let size = |id: &str| {
        let rect = layout.boxes[&dom.get_element_by_id(Dom::DOCUMENT, id).unwrap()].rect;
        (rect.width, rect.height)
    };
    assert_eq!(size("a"), (24.0, 24.0));
    // Without a size, an SVG with a ratio fills its line, as in browsers.
    assert_eq!(size("b"), (384.0, 128.0));
    assert_eq!(size("c"), (300.0, 150.0));
    let painted = scene
        .tree
        .root
        .items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Path { .. }))
        .count();
    assert_eq!(painted, 1);
}

#[test]
fn svg_files_become_vector_images() {
    let (size, items) = image(
        r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="48" height="24" viewBox="0 0 24 12">
        <style>.a { fill: red }</style><rect class="a" width="24" height="12"/></svg>"#,
    )
    .unwrap();
    assert_eq!(size, (48.0, 24.0));
    let paths = paths(&items);
    assert_eq!(paths[0].2, Some(Color::rgb(255, 0, 0)));
    assert_eq!(paths[0].1, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let size = |source: &str| image(source).unwrap().0;
    assert_eq!(
        size(r#"<svg width="10" viewBox="0 0 20 10"/>"#),
        (10.0, 5.0)
    );
    assert_eq!(
        size(r#"<svg height="5" viewBox="0 0 20 10"/>"#),
        (10.0, 5.0)
    );
    assert_eq!(
        size(r#"<svg width="50%" viewBox="0 0 20 10"/>"#),
        (20.0, 10.0)
    );
    assert_eq!(size(r#"<svg width="0"></svg>"#), (300.0, 150.0));
    assert!(image("<p>no image</p>").is_none());
}

#[test]
fn elements_without_styles_are_skipped() {
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<svg><rect width='1' height='1'/></svg>",
    ));
    let svg = dom.get_elements_by_tag_name(Dom::DOCUMENT, "svg")[0];
    let mut items = Vec::new();
    paint(
        &dom,
        &Styles::default(),
        svg,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        &mut items,
        None,
    );
    assert_eq!(items.len(), 2, "only the clip");
}

/// The text items among display items, with their rectangles.
fn texts(items: &[DisplayItem]) -> Vec<(String, Rect, f32)> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text {
                text, rect, font, ..
            } => Some((text.clone(), *rect, font.size)),
            _ => None,
        })
        .collect()
}

#[test]
fn text_is_drawn_from_its_position_anchored_and_on_its_baseline() {
    // At 10px, Mono glyphs are 5px wide, 8px above and 2px below the
    // baseline.
    let items = painted(
        "<svg><g transform='translate(5 6)'><text x='10' y='20' font-size='10'>ab 
  cd</text>         <text x='50' y='20' style='font-size:10px;text-anchor:middle;dominant-baseline:central'>mid<tspan dx='2' fill='red'>x</tspan></text>         <text x='90' y='40' font-size='10' text-anchor='end'><tspan x='90' y='60'>end</tspan></text>         <text x='0' y='0' fill='none'>hidden</text></g></svg>",
        "",
    );
    let found = texts(&items);
    let text = |name: &str| found.iter().find(|(text, ..)| text == name).unwrap().1;
    // White space collapses; the text's top is its ascent above `y`.
    assert_eq!(text("ab cd"), Rect::new(10.0, 12.0, 25.0, 10.0));
    // "mid" and "x" (2px further) are 22px wide, centred on x = 50; the
    // central baseline puts the em box's middle on y.
    let mid = text("mid");
    assert_eq!((mid.x, mid.width), (39.0, 15.0));
    assert_eq!(mid.y, 20.0 + 3.0 - 8.0);
    assert_eq!(text("x").x, 56.0);
    // A tspan's position starts a chunk that ends at x = 90.
    assert_eq!(text("end"), Rect::new(75.0, 52.0, 15.0, 10.0));
    assert!(!found.iter().any(|(text, ..)| text == "hidden"));
    // Positions may be in ems, as Mermaid places lines.
    let lines = painted(
        "<svg><text font-size='10'><tspan x='0' y='-0.1em' dy='1.5em'>line</tspan></text></svg>",
        "",
    );
    assert_eq!(texts(&lines)[0].1.y, -1.0 + 15.0 - 8.0);
    // Text is drawn in its group's coordinates.
    assert!(items.contains(&DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 5.0, 6.0])));
}

#[test]
fn bounding_boxes_cover_shapes_text_and_groups_in_user_space() {
    let (dom, styles) = styled(
        "<svg viewBox='0 0 200 100'><g id=g transform='translate(100 0)'>         <rect id=r x='10' y='20' width='30' height='40' transform='scale(2)' fill='none'/>         <circle id=c cx='0' cy='0' r='5'/>         <text id=t x='0' y='50' font-size='10'>abcd</text>         <foreignObject id=f x='-20' y='5' width='10' height='10'></foreignObject></g>         <g id=empty></g><rect id=hidden style='display:none' width='5' height='5'/></svg>",
        "",
    );
    let id = |name: &str| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let bbox = |name: &str| bbox(&dom, &styles, id(name), Some(&Mono));
    // An element's own transform does not apply; its children's do.
    assert_eq!(bbox("r"), Rect::new(10.0, 20.0, 30.0, 40.0));
    assert_eq!(bbox("t"), Rect::new(0.0, 42.0, 20.0, 10.0));
    assert_eq!(bbox("f"), Rect::new(-20.0, 5.0, 10.0, 10.0));
    let circle = bbox("c");
    assert!((circle.x + 5.0).abs() < 0.01 && (circle.width - 10.0).abs() < 0.01);
    let group = bbox("g");
    assert!((group.x + 20.0).abs() < 0.01, "{group:?}");
    assert!((group.y + 5.0).abs() < 0.01, "{group:?}");
    assert!((group.x + group.width - 80.0).abs() < 0.01, "{group:?}");
    assert!((group.y + group.height - 120.0).abs() < 0.01, "{group:?}");
    assert_eq!(bbox("empty"), Rect::new(0.0, 0.0, 0.0, 0.0));
    assert_eq!(bbox("hidden"), Rect::new(0.0, 0.0, 0.0, 0.0));
    let svg = dom.get_elements_by_tag_name(Dom::DOCUMENT, "svg")[0];
    let whole = super::bbox(&dom, &styles, svg, None);
    assert!((whole.x - 80.0).abs() < 0.01, "{whole:?}");
}

#[test]
fn markers_are_drawn_at_vertices_along_the_path() {
    // Mermaid's arrowhead: a marker in user units, its reference point on
    // the end of the line, turned along it.
    let items = painted(
        r##"<svg width="100" height="100"><g><marker id="arrow" viewBox="0 0 10 10" refX="5" refY="5" markerUnits="userSpaceOnUse" markerWidth="8" markerHeight="8" orient="auto"><path d="M 0 0 L 10 5 L 0 10 z" fill="red"/></marker></g>
        <line x1="10" y1="10" x2="10" y2="50" stroke="black" marker-end="url(#arrow)" marker-start="url('#arrow')"/>
        <polyline points="0,0 10,0 10,10" stroke="black" stroke-width="2" fill="none" marker-mid="url(#dot)" marker-end="url(#missing)"/>
        <marker id="dot" markerWidth="2" markerHeight="2" orient="auto"><circle cx="1" cy="1" r="1" fill="blue"/></marker>
        <path d="M0,0 C 0,0 0,0 0,0" stroke="black" marker-end="url(#fixed)"/>
        <marker id="fixed" orient="45" markerUnits="userSpaceOnUse"><rect width="1" height="1" fill="green"/></marker>
        <path d="M0,0 L10,0 Z" stroke="black" marker-start="url(#reversed)" marker-mid="url(#reversed)"/>
        <marker id="reversed" orient="auto-start-reverse" markerUnits="userSpaceOnUse"><rect width="1" height="1" fill="gray"/></marker></svg>"##,
        "",
    );
    let painted = paths(&items);
    let markers: Vec<(Matrix, Option<Color>)> = painted
        .iter()
        .filter(|(_, _, fill, _)| fill.is_some_and(|fill| fill != Color::rgb(0, 0, 0)))
        .map(|(_, transform, fill, _)| (*transform, *fill))
        .collect();
    let red = Some(Color::rgb(255, 0, 0));
    // The start, turned down the line (90°), and the end; 10 units of view
    // box scale to 8, with the reference point (4, 4) on the vertex.
    matrix_close(markers[0].0, [0.0, 0.8, -0.8, 0.0, 14.0, 6.0]);
    assert_eq!(markers[0].1, red);
    matrix_close(markers[1].0, [0.0, 0.8, -0.8, 0.0, 14.0, 46.0]);
    // The polyline's middle vertex turns halfway between right and down,
    // scaled by the stroke width.
    let (sin, cos) = (
        core::f32::consts::FRAC_PI_4.sin(),
        core::f32::consts::FRAC_PI_4.cos(),
    );
    matrix_close(
        markers[2].0,
        [cos * 2.0, sin * 2.0, -sin * 2.0, cos * 2.0, 10.0, 0.0],
    );
    assert_eq!(markers[2].1, Some(Color::rgb(0, 0, 255)));
    // A fixed angle, and a degenerate curve at its own point.
    matrix_close(markers[3].0, [cos, sin, -sin, cos, 0.0, 0.0]);
    // Reversed at the start; the closed path's other vertices follow it.
    matrix_close(markers[4].0, [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
    assert_eq!(markers.len(), 6);
    // getBBox leaves markers out.
    let (dom, styles) = styled(
        r##"<svg><marker id="big" markerUnits="userSpaceOnUse" markerWidth="50" markerHeight="50"><rect width="50" height="50"/></marker><line id="l" x2="10" stroke="black" marker-end="url(#big)"/></svg>"##,
        "",
    );
    let line = dom.get_element_by_id(Dom::DOCUMENT, "l").unwrap();
    assert_eq!(
        bbox(&dom, &styles, line, Some(&Mono)),
        Rect::new(0.0, 0.0, 10.0, 0.0)
    );
}

#[test]
fn dashes_and_markers_come_from_the_style_attribute_and_css() {
    // js-sequence-diagrams sets them in `style`, a dashed reply line ending
    // in an arrowhead; presentation attributes and inheritance work too.
    let items = painted(
        r##"<svg width="100" height="100"><marker id="m" markerUnits="userSpaceOnUse" orient="auto"><rect width="1" height="1" fill="red"/></marker>
        <path d="M0,0 L50,0" stroke="black" style="stroke-dasharray: 6, 2; stroke-dashoffset: 1; marker-end: url(&quot;#m&quot;)"/>
        <g stroke-dasharray="3" marker-start="url(#m)"><path d="M0,10 L50,10" stroke="black"/></g>
        <path d="M0,20 L50,20" stroke="black" stroke-dasharray="0 0"/>
        <path class="css" d="M0,30 L50,30" stroke="black"/></svg>"##,
        ".css { stroke-dasharray: 4px 1; marker: url(#m) }",
    );
    let painted = paths(&items);
    let lines: Vec<&Stroke> = painted
        .iter()
        .filter_map(|(_, _, _, stroke)| stroke.as_ref())
        .collect();
    assert_eq!(lines[0].dashes, [6.0, 2.0]);
    assert_eq!(lines[0].dash_offset, 1.0);
    // An odd list repeats; one with no length draws solid.
    assert_eq!(lines[1].dashes, [3.0, 3.0]);
    assert!(lines[2].dashes.is_empty());
    assert_eq!(lines[3].dashes, [4.0, 1.0]);
    let markers = painted
        .iter()
        .filter(|(_, _, fill, _)| *fill == Some(Color::rgb(255, 0, 0)))
        .count();
    // The end of the first, the start of the second, both ends of the last.
    assert_eq!(markers, 4);
}

#[test]
fn elements_without_area_stay_out_of_group_bounding_boxes() {
    // As Chromium measures Mermaid's empty edge labels: a foreignObject or
    // rectangle without width or height adds nothing, a line does.
    let (dom, styles) = styled(
        "<svg><g id=g><g><foreignObject width='0' height='0'></foreignObject></g>\
         <foreignObject x='5' width='5' height='0'></foreignObject>\
         <rect x='1' y='1' width='0' height='9'/><circle r='0'/>\
         <rect x='50' y='50' width='10' height='10'/></g>\
         <g id=line><line x1='20' y1='70' x2='30' y2='70' stroke='black'/><rect x='50' y='50' width='10' height='10'/></g></svg>",
        "",
    );
    let id = |name: &str| dom.get_element_by_id(Dom::DOCUMENT, name).unwrap();
    let bbox = |name: &str| bbox(&dom, &styles, id(name), Some(&Mono));
    assert_eq!(bbox("g"), Rect::new(50.0, 50.0, 10.0, 10.0));
    assert_eq!(bbox("line"), Rect::new(20.0, 50.0, 40.0, 20.0));
}
