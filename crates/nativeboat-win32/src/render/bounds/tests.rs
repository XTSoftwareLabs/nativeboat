use super::*;
use nativeboat_css::Color;
use nativeboat_css::types::{BorderStyle, FillRule, Filter, LineCap, LineJoin};
use nativeboat_ir::{Corner, Font, Stroke};

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const RED: Color = Color::rgb(255, 0, 0);

fn fill(x: f32, y: f32, width: f32, height: f32) -> DisplayItem {
    DisplayItem::Fill {
        rect: Rect::new(x, y, width, height),
        color: RED,
        radii: [Corner::SQUARE; 4],
    }
}

#[test]
fn nothing_painted_has_no_bounds() {
    assert_eq!(bounds(&[], IDENTITY), None);
    assert_eq!(
        bounds(
            &[
                DisplayItem::PushClip {
                    rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                    radii: [Corner::SQUARE; 4],
                },
                DisplayItem::PushOpacity(0.5),
                DisplayItem::PopOpacity,
                DisplayItem::PopClip,
                DisplayItem::PopFilter,
                DisplayItem::PushFilter(vec![Filter::Blur(2.0)]),
                DisplayItem::PopFilter,
            ],
            IDENTITY
        ),
        None
    );
}

#[test]
fn boxes_text_and_shadows_cover_what_they_paint() {
    let rect = Rect::new(10.0, 20.0, 30.0, 40.0);
    let boxes = [
        fill(10.0, 20.0, 30.0, 40.0),
        DisplayItem::Gradient {
            rect,
            angle: 0.0,
            stops: Vec::new(),
            radii: [Corner::SQUARE; 4],
        },
        DisplayItem::Border {
            rect,
            widths: [1.0; 4],
            colors: [RED; 4],
            styles: [BorderStyle::Solid; 4],
            radii: [Corner::SQUARE; 4],
        },
        DisplayItem::Canvas {
            rect,
            key: 1,
            version: 1,
        },
        DisplayItem::Image {
            rect,
            url: "a.png".into(),
        },
        DisplayItem::Caret { rect, color: RED },
        DisplayItem::Backdrop {
            rect,
            radii: [Corner::SQUARE; 4],
            filters: Vec::new(),
        },
    ];
    for item in boxes {
        assert_eq!(
            bounds(std::slice::from_ref(&item), IDENTITY),
            Some([10.0, 20.0, 40.0, 60.0]),
            "{item:?}"
        );
    }
    let text = DisplayItem::Text {
        rect: Rect::new(0.0, 0.0, 50.0, 10.0),
        text: "x".into(),
        font: Font::default(),
        color: RED,
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
    };
    // Ink may reach an em past a short run: 13.333px here, over half the
    // run's 10px height.
    assert_eq!(
        bounds(std::slice::from_ref(&text), IDENTITY),
        Some([-13.333, -13.333, 63.333, 23.333])
    );
    let tall = DisplayItem::Text {
        rect: Rect::new(0.0, 0.0, 50.0, 40.0),
        text: "x".into(),
        font: Font::default(),
        color: RED,
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
    };
    assert_eq!(bounds(&[tall], IDENTITY), Some([-20.0, -20.0, 70.0, 60.0]));
    let shadow = |inset| DisplayItem::Shadow {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        radii: [Corner::SQUARE; 4],
        color: RED,
        blur: 4.0,
        inset,
        clip: Rect::new(1.0, 1.0, 8.0, 8.0),
        clip_radii: [Corner::SQUARE; 4],
    };
    assert_eq!(
        bounds(&[shadow(false)], IDENTITY),
        Some([-6.0, -6.0, 16.0, 16.0])
    );
    assert_eq!(
        bounds(&[shadow(true)], IDENTITY),
        Some([1.0, 1.0, 9.0, 9.0])
    );
}

#[test]
fn paths_cover_their_points_and_strokes() {
    use PathSegment::*;
    let path = |stroke: Option<Stroke>| DisplayItem::Path {
        segments: vec![
            Move(0.0, 0.0),
            Line(10.0, 0.0),
            Cubic(10.0, 5.0, 20.0, 5.0, 20.0, 10.0),
            Close,
        ],
        transform: [2.0, 0.0, 0.0, 1.0, 5.0, 0.0],
        fill: Some(RED),
        fill_rule: FillRule::NonZero,
        stroke,
    };
    assert_eq!(
        bounds(&[path(None)], IDENTITY),
        Some([5.0, 0.0, 45.0, 10.0])
    );
    let stroke = Stroke {
        color: RED,
        width: 1.0,
        cap: LineCap::Butt,
        join: LineJoin::Miter,
        dashes: Vec::new(),
        dash_offset: 0.0,
    };
    // Five stroke widths, stretched by the path's own scale.
    assert_eq!(
        bounds(&[path(Some(stroke))], IDENTITY),
        Some([-5.0, -5.0, 55.0, 15.0])
    );
}

#[test]
fn transforms_and_nested_filters_apply() {
    let items = [
        DisplayItem::PushTransform([2.0, 0.0, 0.0, 2.0, 100.0, 0.0]),
        fill(0.0, 0.0, 10.0, 10.0),
        DisplayItem::PopTransform,
        fill(0.0, 0.0, 1.0, 1.0),
    ];
    assert_eq!(bounds(&items, IDENTITY), Some([0.0, 0.0, 120.0, 20.0]));
    // A quarter turn swaps the extents.
    let turned = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
    assert_eq!(
        bounds(&[fill(0.0, 0.0, 10.0, 5.0)], turned),
        Some([-5.0, 0.0, 0.0, 10.0])
    );
    // Extra pops fall back to the outer transform.
    assert_eq!(
        bounds(
            &[DisplayItem::PopTransform, fill(0.0, 0.0, 1.0, 1.0)],
            [1.0, 0.0, 0.0, 1.0, 5.0, 5.0]
        ),
        Some([5.0, 5.0, 6.0, 6.0])
    );
    // A nested group grows by its reach, scaled by the transform.
    let nested = [
        DisplayItem::PushFilter(vec![Filter::DropShadow {
            x: 4.0,
            y: 0.0,
            blur: 0.0,
            color: RED,
        }]),
        fill(0.0, 0.0, 10.0, 10.0),
        DisplayItem::PushFilter(Vec::new()),
        DisplayItem::PopFilter,
        DisplayItem::PopFilter,
        fill(0.0, 0.0, 1.0, 1.0),
    ];
    assert_eq!(
        bounds(&nested, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
        Some([-8.0, 0.0, 28.0, 20.0])
    );
}

#[test]
fn groups_end_at_their_matching_pop() {
    let items = [
        DisplayItem::PushFilter(Vec::new()),
        DisplayItem::PushFilter(Vec::new()),
        DisplayItem::PopFilter,
        fill(0.0, 0.0, 1.0, 1.0),
        DisplayItem::PopFilter,
        DisplayItem::PushFilter(Vec::new()),
    ];
    assert_eq!(group_end(&items, 0), 4);
    assert_eq!(group_end(&items, 1), 2);
    assert_eq!(group_end(&items, 5), 6);
}

#[test]
fn opacity_groups_ignore_filter_pops_and_keep_conservative_extents() {
    let items = [
        DisplayItem::PushOpacity(0.5),
        DisplayItem::PushFilter(Vec::new()),
        DisplayItem::PushOpacity(0.5),
        fill(1.0, 2.0, 3.0, 4.0),
        DisplayItem::PopOpacity,
        DisplayItem::PopFilter,
        fill(20.0, 20.0, 1.0, 1.0),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(0.5),
    ];
    assert_eq!(group_end(&items, 0), 7);
    assert_eq!(group_end(&items, 1), 5);
    assert_eq!(group_end(&items, 2), 4);
    assert_eq!(group_end(&items, 8), 9);
    assert_eq!(
        opacity_extent(&items[1..7], IDENTITY),
        Extent::Known([1.0, 2.0, 21.0, 21.0])
    );
    assert_eq!(opacity_extent(&[], IDENTITY), Extent::Empty);
    let image = DisplayItem::Image {
        rect: Rect::new(1.0, 2.0, 3.0, 4.0),
        url: "overflow.svg".into(),
    };
    assert_eq!(opacity_extent(&[image], IDENTITY), Extent::Unknown);
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
    // Text and shadows reach past their rectangles by bounded amounts.
    let ink = DisplayItem::Text {
        rect,
        text: "ink".into(),
        font: Font::default(),
        color: RED,
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
    };
    assert_eq!(
        opacity_extent(&[ink], IDENTITY),
        Extent::Known([-13.333, -13.333, 23.333, 23.333])
    );
    let shadow = DisplayItem::Shadow {
        rect,
        radii: [Corner::SQUARE; 4],
        color: RED,
        blur: 4.0,
        inset: false,
        clip: rect,
        clip_radii: [Corner::SQUARE; 4],
    };
    assert_eq!(
        opacity_extent(&[shadow], IDENTITY),
        Extent::Known([-6.0, -6.0, 16.0, 16.0])
    );
    for item in [
        DisplayItem::Canvas {
            rect,
            key: 1,
            version: 1,
        },
        DisplayItem::Backdrop {
            rect,
            radii: [Corner::SQUARE; 4],
            filters: Vec::new(),
        },
    ] {
        assert_eq!(opacity_extent(&[item], IDENTITY), Extent::Unknown);
    }
}
