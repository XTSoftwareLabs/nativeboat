use super::*;
use nativeboat_ir::{Corner, Radii};

fn look() -> Look {
    Look {
        radio: false,
        checked: false,
        indeterminate: false,
        disabled: false,
        hovered: false,
        pressed: false,
        focus_visible: false,
        accent: None,
    }
}

fn near(one: Rect, other: Rect) -> bool {
    [
        one.x - other.x,
        one.y - other.y,
        one.width - other.width,
        one.height - other.height,
    ]
    .iter()
    .all(|difference| difference.abs() < 1e-4)
}

fn painted(rect: Rect, look: &Look) -> Vec<DisplayItem> {
    let mut items = Vec::new();
    paint(&mut items, rect, look);
    items
}

fn fill(item: &DisplayItem) -> (Rect, Color, Radii) {
    match item {
        DisplayItem::Fill { rect, color, radii } => (*rect, *color, *radii),
        other => panic!("not a fill: {other:?}"),
    }
}

fn border_color(item: &DisplayItem) -> Color {
    match item {
        DisplayItem::Border {
            widths,
            colors,
            radii,
            ..
        } => {
            assert_eq!(widths, &[1.0; 4]);
            assert_eq!(radii[0], radii[3]);
            colors[0]
        }
        other => panic!("not a border: {other:?}"),
    }
}

fn check(item: &DisplayItem) -> (Vec<PathSegment>, Stroke) {
    match item {
        DisplayItem::Path {
            segments,
            fill: None,
            stroke: Some(stroke),
            ..
        } => (segments.clone(), stroke.clone()),
        other => panic!("not a check mark: {other:?}"),
    }
}

const BOX: Rect = Rect {
    x: 10.0,
    y: 20.0,
    width: 13.0,
    height: 13.0,
};

#[test]
fn unchecked_checkboxes_are_white_boxes_with_a_gray_border() {
    let items = painted(BOX, &look());
    assert_eq!(items.len(), 2);
    let (rect, color, radii) = fill(&items[0]);
    assert!(near(rect, Rect::new(10.2, 20.2, 12.6, 12.6)), "{rect:?}");
    assert_eq!(
        (color, radii),
        (Color::rgb(255, 255, 255), [Corner::round(2.0); 4])
    );
    assert_eq!(border_color(&items[1]), Color::rgb(0x76, 0x76, 0x76));
    let hovered = painted(
        BOX,
        &Look {
            hovered: true,
            ..look()
        },
    );
    assert_eq!(border_color(&hovered[1]), Color::rgb(0x4F, 0x4F, 0x4F));
    let pressed = painted(
        BOX,
        &Look {
            hovered: true,
            pressed: true,
            ..look()
        },
    );
    assert_eq!(border_color(&pressed[1]), Color::rgb(0x8D, 0x8D, 0x8D));
}

#[test]
fn checked_checkboxes_fill_with_the_accent_and_a_white_check_mark() {
    let items = painted(
        BOX,
        &Look {
            checked: true,
            ..look()
        },
    );
    // The background, the accent fill over the whole box with no border,
    // and the mark.
    assert_eq!(items.len(), 3);
    assert_eq!(
        fill(&items[1]),
        (BOX, Color::rgb(0x00, 0x75, 0xFF), [Corner::round(2.0); 4])
    );
    let (segments, stroke) = check(&items[2]);
    let points: Vec<_> = segments
        .iter()
        .map(|segment| match segment {
            PathSegment::Move(x, y) | PathSegment::Line(x, y) => (*x, *y),
            other => panic!("{other:?}"),
        })
        .collect();
    for ((x, y), (ex, ey)) in points
        .into_iter()
        .zip([(12.6, 26.5), (15.2, 29.1), (20.4, 22.6)])
    {
        assert!(
            (x - ex).abs() < 1e-4 && (y - ey).abs() < 1e-4,
            "{segments:?}"
        );
    }
    assert_eq!(stroke.color, Color::rgb(255, 255, 255));
    assert!((stroke.width - 2.08).abs() < 1e-5);
    let hovered = painted(
        BOX,
        &Look {
            checked: true,
            hovered: true,
            ..look()
        },
    );
    assert_eq!(fill(&hovered[1]).1, Color::rgb(0x00, 0x5C, 0xC8));
}

#[test]
fn light_accents_paint_as_dark_pages_do_so_the_mark_shows() {
    // ytDownloader's accent barely shows on white: the mark is dark.
    let accent = Color::rgb(0x54, 0xAB, 0xDE);
    let items = painted(
        BOX,
        &Look {
            checked: true,
            accent: Some(accent),
            ..look()
        },
    );
    assert_eq!(fill(&items[0]).1, Color::rgb(0x3B, 0x3B, 0x3B));
    assert_eq!(fill(&items[1]).1, accent);
    assert_eq!(check(&items[2]).1.color, Color::rgb(0x3B, 0x3B, 0x3B));
    // Hovered, the accent lightens as in dark pages.
    let hovered = painted(
        BOX,
        &Look {
            checked: true,
            hovered: true,
            accent: Some(accent),
            ..look()
        },
    );
    let lighter = fill(&hovered[1]).1;
    assert!(lighter.r > accent.r && lighter.g > accent.g, "{lighter:?}");
    // A dark accent keeps the white mark, and unchecked boxes ignore it.
    let navy = Color::rgb(0, 0, 0x80);
    let dark = painted(
        BOX,
        &Look {
            checked: true,
            accent: Some(navy),
            ..look()
        },
    );
    assert_eq!(fill(&dark[1]).1, navy);
    assert_eq!(check(&dark[2]).1.color, Color::rgb(255, 255, 255));
    let unchecked = painted(
        BOX,
        &Look {
            accent: Some(accent),
            ..look()
        },
    );
    assert_eq!(fill(&unchecked[0]).1, Color::rgb(255, 255, 255));
}

#[test]
fn accents_shade_by_hsl_lightness() {
    let gray = Color::rgb(128, 128, 128);
    assert_eq!(lighten(gray, 0.11), Color::rgb(156, 156, 156));
    assert_eq!(lighten(gray, -0.11), Color::rgb(100, 100, 100));
    let blue = Color::rgb(0x00, 0x75, 0xFF);
    assert_eq!(lighten(blue, 0.0), blue);
    let darker = lighten(blue, -0.11);
    assert_eq!((darker.r, darker.b), (0, 199));
    assert_eq!(lighten(Color::rgba(250, 250, 250, 0.5), 0.11).a, 0.5);
    assert_eq!(
        lighten(Color::rgb(250, 250, 250), 0.11),
        Color::rgb(255, 255, 255)
    );
}

#[test]
fn radio_buttons_ring_and_dot_with_the_accent_when_checked() {
    let items = painted(
        BOX,
        &Look {
            radio: true,
            ..look()
        },
    );
    assert_eq!(items.len(), 2);
    assert_eq!(fill(&items[0]).2, [Corner::round(6.5); 4]);
    assert_eq!(border_color(&items[1]), Color::rgb(0x76, 0x76, 0x76));
    let checked = painted(
        BOX,
        &Look {
            radio: true,
            checked: true,
            ..look()
        },
    );
    assert_eq!(checked.len(), 3);
    assert_eq!(border_color(&checked[1]), Color::rgb(0x00, 0x75, 0xFF));
    let (rect, color, _) = fill(&checked[2]);
    assert_eq!(color, Color::rgb(0x00, 0x75, 0xFF));
    assert!((rect.x - 12.6).abs() < 1e-5 && (rect.width - 7.8).abs() < 1e-5);
}

#[test]
fn indeterminate_disabled_tiny_and_oblong_checkboxes() {
    let items = painted(
        BOX,
        &Look {
            indeterminate: true,
            checked: true,
            ..look()
        },
    );
    assert_eq!(items.len(), 3);
    let (dash, color, _) = fill(&items[2]);
    assert_eq!(color, Color::rgb(0x76, 0x76, 0x76));
    assert!((dash.width - 8.0).abs() < 1e-5 && (dash.height - 2.0).abs() < 1e-5);
    let disabled = painted(
        BOX,
        &Look {
            disabled: true,
            checked: true,
            accent: Some(Color::rgb(0x54, 0xAB, 0xDE)),
            ..look()
        },
    );
    assert_eq!(
        fill(&disabled[1]).1,
        Color::rgba(0x76, 0x76, 0x76, 0x4D as f32 / 255.0)
    );
    // The largest square, centered.
    let oblong = painted(Rect::new(0.0, 0.0, 40.0, 20.0), &look());
    assert!(near(fill(&oblong[0]).0, Rect::new(10.2, 0.2, 19.6, 19.6)));
    let tiny = painted(Rect::new(0.0, 0.0, 2.0, 2.0), &look());
    assert_eq!(tiny.len(), 1);
    assert!(painted(Rect::new(0.0, 0.0, 0.0, 5.0), &look()).is_empty());
}

#[test]
fn keyboard_focus_rings_the_box() {
    let items = painted(
        BOX,
        &Look {
            focus_visible: true,
            ..look()
        },
    );
    assert_eq!(items.len(), 4);
    match &items[2] {
        DisplayItem::Border { rect, colors, .. } => {
            assert_eq!(*rect, Rect::new(8.0, 18.0, 17.0, 17.0));
            assert_eq!(colors[0], Color::rgb(0x10, 0x10, 0x10));
        }
        other => panic!("{other:?}"),
    }
}
