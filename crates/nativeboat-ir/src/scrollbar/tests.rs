use super::*;
use crate::native::{ScrollbarAxis, ScrollbarPart};
use nativeboat_css::LengthPercentage;
use nativeboat_css::types::{BorderStyle, Radius};

fn paint(color: Color) -> ScrollbarPaint {
    ScrollbarPaint {
        visible: true,
        background: color,
        widths: [0.0; 4],
        colors: [Color::BLACK; 4],
        borders: [BorderStyle::None; 4],
        radii: core::array::from_fn(|_| Radius::default()),
        margins: core::array::from_fn(|_| LengthPercentageAuto::zero()),
    }
}

fn theme() -> Scrollbars {
    let background = ScrollbarPart(core::array::from_fn(|_| paint(Color::rgb(230, 230, 230))));
    Scrollbars {
        axes: core::array::from_fn(|_| ScrollbarAxis {
            enabled: true,
            force: false,
            thickness: Size::Length(LengthPercentage::Px(12.0)),
            bar: background.clone(),
            track: ScrollbarPart(core::array::from_fn(|_| paint(Color::TRANSPARENT))),
            thumb: ScrollbarPart([
                paint(Color::rgb(255, 0, 0)),
                paint(Color::rgb(0, 128, 0)),
                paint(Color::rgb(0, 0, 255)),
            ]),
        }),
        corner: background,
        platform: None,
    }
}

#[test]
fn geometry_follows_overflow_axes_widths_scroll_offsets_and_corner() {
    let mut style = theme();
    let geo = Geometry::new(
        &style,
        (300.0, 200.0),
        (900.0, 1000.0),
        (300.0, 800.0),
        17.0,
    );
    let horizontal = geo.bars[0].unwrap();
    let vertical = geo.bars[1].unwrap();
    assert_eq!(horizontal.track, Rect::new(0.0, 188.0, 288.0, 12.0));
    assert_eq!(vertical.track, Rect::new(288.0, 0.0, 12.0, 188.0));
    assert_eq!(horizontal.thumb, Rect::new(96.0, 188.0, 96.0, 12.0));
    assert!((vertical.thumb.y + vertical.thumb.height - 188.0).abs() < 0.001);
    assert_eq!(geo.hit(295.0, 195.0), Some(Hit::Corner));
    assert_eq!(geo.hit(105.0, 195.0), Some(Hit::Thumb(0)));
    assert_eq!(geo.hit(10.0, 195.0), Some(Hit::Track(0)));
    assert_eq!(geo.hit(295.0, 5.0), Some(Hit::Track(1)));
    assert_eq!(geo.hit(0.0, 0.0), None);
    assert_eq!(geo.hit(300.0, 200.0), None);
    assert!(
        Geometry::new(&style, (300.0, 200.0), (300.0, 200.0), (0.0, 0.0), 17.0)
            .bars
            .iter()
            .all(Option::is_none)
    );
    style.axes[0].enabled = false;
    style.axes[1].thickness = Size::Length(LengthPercentage::Percent(0.1));
    let geo = Geometry::new(&style, (300.0, 200.0), (900.0, 1000.0), (0.0, 0.0), 17.0);
    assert!(geo.bars[0].is_none());
    assert_eq!(geo.bars[1].unwrap().track.width, 30.0);
    assert!(geo.corner.is_none());
    style.axes[1].thickness = Size::Auto;
    style.axes[1].force = true;
    let geo = Geometry::new(&style, (5.0, 10.0), (0.0, 0.0), (0.0, 0.0), 17.0);
    // A forced bar with nothing to scroll has no thumb, as in Chromium.
    assert_eq!(geo.bars[1].unwrap().thumb, Rect::new(0.0, 0.0, 5.0, 0.0));
    assert!(!geo.bars[1].unwrap().has_thumb());
    assert_eq!(geo.bars[1].unwrap().travel, 0.0);
    assert_eq!(geo.hit(2.0, 5.0), Some(Hit::Track(1)));
    // Nor does one whose content overflows by less than half a pixel, which
    // Chromium's pixel snapping rounds away; half a pixel more scrolls.
    let thumb = |content: f32| {
        let geo = Geometry::new(&style, (5.0, 10.0), (0.0, content), (0.0, 0.0), 17.0);
        (
            geo.bars[1].unwrap().has_thumb(),
            geo.bars[1].unwrap().maximum,
        )
    };
    assert_eq!(thumb(10.4), (false, 0.0));
    assert_eq!(thumb(11.0), (true, 1.0));
    style.axes[1].thickness = Size::Length(LengthPercentage::Px(-1.0));
    assert!(
        Geometry::new(&style, (5.0, 10.0), (900.0, 1000.0), (0.0, 0.0), 17.0).bars[1].is_none()
    );
}

#[test]
fn boxes_apply_visibility_margins_radii_and_borders() {
    let mut style = paint(Color::BLACK);
    let mut list = Vec::new();
    style.visible = false;
    box_items(&style, Rect::new(0.0, 0.0, 12.0, 100.0), &mut list);
    assert!(list.is_empty());
    style.visible = true;
    style.widths = [1.0; 4];
    style.borders = [BorderStyle::Solid; 4];
    style.margins = [
        LengthPercentageAuto::Length(LengthPercentage::Px(2.0)),
        LengthPercentageAuto::Auto,
        LengthPercentageAuto::Length(LengthPercentage::Px(2.0)),
        LengthPercentageAuto::Length(LengthPercentage::Px(1.0)),
    ];
    style.radii = core::array::from_fn(|_| Radius {
        horizontal: LengthPercentage::Percent(0.5),
        vertical: LengthPercentage::Px(8.0),
    });
    box_items(&style, Rect::new(10.0, 20.0, 12.0, 100.0), &mut list);
    assert_eq!(list.len(), 2);
    assert!(
        matches!(&list[0],DisplayItem::Fill{rect,radii,..} if *rect==Rect::new(11.0,22.0,11.0,96.0) && *radii==[Corner{x:5.5,y:8.0};4])
    );
    assert!(matches!(&list[1],DisplayItem::Border{widths,..} if *widths==[1.0;4]));
}

fn colors(items: &[DisplayItem]) -> Vec<Color> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { color, .. } => Some(*color),
            DisplayItem::Path { fill, .. } => *fill,
            _ => None,
        })
        .collect()
}

#[test]
fn styled_scrollbars_paint_their_parts_as_the_pointer_hovers_and_presses_them() {
    let style = theme();
    let geometry = Geometry::new(&style, (100.0, 80.0), (300.0, 300.0), (0.0, 0.0), 17.0);
    let mut items = Vec::new();
    styled_items(&style, &geometry, None, None, &mut items);
    // Two bars of a background, a track and a thumb, and the corner.
    assert_eq!(items.len(), 7);
    assert_eq!(colors(&items)[2], Color::rgb(255, 0, 0));
    let mut hovered = Vec::new();
    styled_items(&style, &geometry, Some(Hit::Thumb(0)), None, &mut hovered);
    assert_eq!(colors(&hovered)[2], Color::rgb(0, 128, 0));
    let mut pressed = Vec::new();
    styled_items(
        &style,
        &geometry,
        Some(Hit::Track(1)),
        Some(Hit::Thumb(1)),
        &mut pressed,
    );
    assert_eq!(colors(&pressed)[5], Color::rgb(0, 0, 255));
    assert_eq!(Hit::Corner.axis(), None);
    assert_eq!(Hit::Button(1, true).axis(), Some(1));
}

#[test]
fn scrollbars_with_nothing_to_scroll_paint_no_thumb() {
    // Bars forced on a surface whose content fits, in each look: their
    // tracks and buttons, but no thumbs.
    let style = theme();
    let fits = |thickness: f32, buttons: f32| {
        let axes = [Axis {
            enabled: true,
            force: true,
            thickness,
        }; 2];
        Geometry::with(
            axes,
            ((200.0, 100.0), (200.0, 100.0)),
            (200.0, 100.0),
            (0.0, 0.0),
            (buttons, 20.0),
        )
    };
    let mut items = Vec::new();
    styled_items(&style, &fits(12.0, 0.0), None, None, &mut items);
    // Two bars of a background and a track, and the corner.
    assert_eq!(items.len(), 5);
    assert!(!colors(&items).contains(&Color::rgb(255, 0, 0)));
    for look in [Look::Classic, Look::Fluent] {
        let geometry = fits(look.thickness(1.0), look.button_length(1.0));
        let mut items = Vec::new();
        platform_items(look, &geometry, (0.0, 0.0), (None, None), 1.0, &mut items);
        let thumb = if look == Look::Classic {
            Color::rgb(193, 193, 193)
        } else {
            Color::rgb(139, 139, 139)
        };
        let fills = items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Fill { color, .. } if *color == thumb))
            .count();
        assert_eq!(fills, 0, "{look:?}");
        assert!(!arrows(&items).is_empty());
    }
}

#[test]
fn default_scrollbars_have_buttons_at_the_ends_of_their_tracks() {
    let axes = [Axis {
        enabled: true,
        force: false,
        thickness: THICKNESS,
    }; 2];
    let geometry = Geometry::with(
        axes,
        ((200.0, 100.0), (200.0, 100.0)),
        (400.0, 300.0),
        (0.0, 50.0),
        (THICKNESS, 20.0),
    );
    let vertical = geometry.bars[1].unwrap();
    assert_eq!(vertical.whole, Rect::new(183.0, 0.0, 17.0, 83.0));
    assert_eq!(
        vertical.buttons,
        Some([
            Rect::new(183.0, 0.0, 17.0, 17.0),
            Rect::new(183.0, 66.0, 17.0, 17.0)
        ])
    );
    assert_eq!(vertical.track, Rect::new(183.0, 17.0, 17.0, 49.0));
    assert_eq!(geometry.hit(190.0, 5.0), Some(Hit::Button(1, false)));
    assert_eq!(geometry.hit(190.0, 70.0), Some(Hit::Button(1, true)));
    assert_eq!(geometry.hit(5.0, 90.0), Some(Hit::Button(0, false)));
    assert_eq!(
        geometry.hit(190.0, vertical.thumb.y + 1.0),
        Some(Hit::Thumb(1))
    );
    // Buttons share a bar too short for both.
    let short = Geometry::with(
        axes,
        ((200.0, 20.0), (200.0, 20.0)),
        (400.0, 300.0),
        (0.0, 0.0),
        (THICKNESS, 20.0),
    );
    assert_eq!(short.bars[1].unwrap().buttons.unwrap()[0].height, 1.5);
}

#[test]
fn presses_scroll_a_step_or_seven_eighths_of_a_page_toward_the_pointer() {
    let axes = [Axis {
        enabled: true,
        force: false,
        thickness: THICKNESS,
    }; 2];
    let geometry = Geometry::with(
        axes,
        ((200.0, 100.0), (200.0, 100.0)),
        (400.0, 300.0),
        (0.0, 50.0),
        (THICKNESS, 20.0),
    );
    let thumb = geometry.bars[1].unwrap().thumb;
    let amount = |part: Hit, pointer: f32| geometry.press_amount(part, pointer, 100.0);
    assert_eq!(amount(Hit::Button(1, false), 5.0), Some(-STEP));
    assert_eq!(amount(Hit::Button(1, true), 70.0), Some(STEP));
    // The track pages toward the pointer, and not once the thumb is there.
    assert_eq!(amount(Hit::Track(1), thumb.y - 1.0), Some(-87.0));
    assert_eq!(amount(Hit::Track(1), thumb.y + thumb.height), Some(87.0));
    assert_eq!(amount(Hit::Track(1), thumb.y + 1.0), None);
    assert_eq!(amount(Hit::Thumb(1), thumb.y + 1.0), None);
    assert_eq!(amount(Hit::Corner, 0.0), None);
    assert_eq!((page_step(200.0), page_step(0.5)), (175.0, 1.0));
}

#[test]
fn default_scrollbars_paint_as_chromium_does() {
    let axes = [Axis {
        enabled: true,
        force: false,
        thickness: THICKNESS,
    }; 2];
    let geometry = Geometry::with(
        axes,
        ((200.0, 100.0), (200.0, 100.0)),
        (400.0, 300.0),
        (0.0, 50.0),
        (THICKNESS, 20.0),
    );
    let mut items = Vec::new();
    default_items(&geometry, (0.0, 50.0), None, None, &mut items);
    let found = colors(&items);
    let track = Color::rgb(241, 241, 241);
    let (dim, arrow) = (Color::rgb(163, 163, 163), Color::rgb(80, 80, 80));
    // The horizontal bar cannot scroll left: its first arrow dims.
    assert_eq!(
        found[..5],
        [track, dim, arrow, Color::rgb(193, 193, 193), track]
    );
    assert_eq!(found[5..8], [arrow, arrow, Color::rgb(193, 193, 193)]);
    assert_eq!(found[8], Color::rgb(220, 220, 220));
    let thumb = items.iter().find_map(|item| match item {
        DisplayItem::Fill { rect, color, .. } if *color == Color::rgb(193, 193, 193) => Some(*rect),
        _ => None,
    });
    let bar = geometry.bars[0].unwrap();
    assert_eq!(
        thumb,
        Some(Rect::new(
            bar.thumb.x,
            bar.thumb.y + 2.0,
            bar.thumb.width,
            13.0
        ))
    );
    let mut items = Vec::new();
    default_items(
        &geometry,
        (0.0, 50.0),
        Some(Hit::Button(1, false)),
        Some(Hit::Button(1, true)),
        &mut items,
    );
    let found = colors(&items);
    assert!(found.contains(&Color::rgb(210, 210, 210)));
    assert!(found.contains(&Color::rgb(120, 120, 120)));
    assert!(found.contains(&Color::WHITE));
    let mut items = Vec::new();
    default_items(
        &geometry,
        (0.0, 50.0),
        Some(Hit::Thumb(1)),
        None,
        &mut items,
    );
    assert!(colors(&items).contains(&Color::rgb(168, 168, 168)));
    let mut items = Vec::new();
    default_items(
        &geometry,
        (0.0, 50.0),
        None,
        Some(Hit::Thumb(0)),
        &mut items,
    );
    assert!(colors(&items).contains(&Color::rgb(120, 120, 120)));
    let mut items = Vec::new();
    let none = Geometry::with(
        axes,
        ((200.0, 100.0), (200.0, 100.0)),
        (100.0, 50.0),
        (0.0, 0.0),
        (THICKNESS, 20.0),
    );
    default_items(&none, (0.0, 0.0), None, None, &mut items);
    assert!(items.is_empty());
}

#[test]
fn looks_follow_chromium_releases_and_fit_device_pixels() {
    assert_eq!(Look::of_chromium(134), Look::Classic);
    assert_eq!(Look::of_chromium(135), Look::Fluent);
    assert_eq!(Look::default(), Look::Classic);
    assert_eq!(Look::Classic.thickness(0.8), THICKNESS);
    assert_eq!(Look::Classic.minimum_thumb(0.8), MINIMUM_THUMB);
    assert_eq!(
        (
            Look::Fluent.thickness(0.0),
            Look::Fluent.button_length(0.0),
            Look::Fluent.minimum_thumb(0.0)
        ),
        (15.0, 18.0, 17.0)
    );
    // At 125%, 18.75 device pixels round to 19, and 22.5 to 23.
    assert_eq!(Look::Fluent.thickness(0.8), 19.0 * 0.8);
    assert_eq!(Look::Fluent.button_length(0.8), 23.0 * 0.8);
}

/// The corners of the triangles among display items, in order.
fn arrows(items: &[DisplayItem]) -> Vec<[(f32, f32); 3]> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Path { segments, .. } => match segments.as_slice() {
                [
                    PathSegment::Move(ax, ay),
                    PathSegment::Line(bx, by),
                    PathSegment::Line(cx, cy),
                    PathSegment::Close,
                ] => Some([(*ax, *ay), (*bx, *by), (*cx, *cy)]),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn fluent_geometry(pixel: f32) -> Geometry {
    let look = Look::Fluent;
    let axes = [Axis {
        enabled: true,
        force: false,
        thickness: look.thickness(pixel),
    }; 2];
    Geometry::with(
        axes,
        ((200.0, 100.0), (200.0, 100.0)),
        (400.0, 300.0),
        (0.0, 50.0),
        (look.button_length(pixel), look.minimum_thumb(pixel)),
    )
}

#[test]
fn fluent_scrollbars_paint_as_chromium_does_since_release_135() {
    let geometry = fluent_geometry(1.0);
    let vertical = geometry.bars[1].unwrap();
    assert_eq!(vertical.whole, Rect::new(185.0, 0.0, 15.0, 85.0));
    assert_eq!(
        vertical.buttons.unwrap()[0],
        Rect::new(185.0, 0.0, 15.0, 18.0)
    );
    let mut items = Vec::new();
    let none = (None, None);
    platform_items(Look::Fluent, &geometry, (0.0, 50.0), none, 1.0, &mut items);
    let (track, thumb) = (Color::rgb(252, 252, 252), Color::rgb(139, 139, 139));
    // Arrows keep their color where the content cannot scroll further.
    assert_eq!(
        colors(&items),
        [
            track, thumb, thumb, thumb, track, thumb, thumb, thumb, track
        ]
    );
    // The thumb is 9 wide in the middle of the track, and round.
    let thumbs: Vec<(Rect, crate::native::Radii)> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Fill { rect, color, radii } if *color == thumb => Some((*rect, *radii)),
            _ => None,
        })
        .collect();
    assert_eq!(thumbs[1].0.x, 188.0);
    assert_eq!(thumbs[1].0.width, 9.0);
    assert_eq!(thumbs[1].1, [Corner::round(4.5); 4]);
    assert_eq!(thumbs[0].0.y, 85.0 + 3.0);
    // Arrows are 9 wide and 5 deep, a pixel off center away from the thumb.
    let found = arrows(&items);
    assert_eq!(found[2], [(188.0, 11.0), (197.0, 11.0), (192.5, 6.0)], "up");
    assert_eq!(
        found[3],
        [(188.0, 73.0), (197.0, 73.0), (192.5, 78.0)],
        "down"
    );
    assert_eq!(found[0], [(10.0, 88.0), (10.0, 97.0), (5.0, 92.5)], "left");
    assert_eq!(
        found[1],
        [(174.0, 88.0), (174.0, 97.0), (179.0, 92.5)],
        "right"
    );
    // The pointer darkens the thumb and the arrows it hovers or presses; a
    // pressed arrow shrinks to 7.
    let dark = Color::rgb(99, 99, 99);
    let mut items = Vec::new();
    let pointer = (Some(Hit::Thumb(1)), Some(Hit::Button(1, false)));
    platform_items(
        Look::Fluent,
        &geometry,
        (0.0, 50.0),
        pointer,
        1.0,
        &mut items,
    );
    assert_eq!(colors(&items)[5..8], [dark, dark, thumb]);
    let up = arrows(&items)[2];
    assert_eq!(up[1].0 - up[0].0, 7.0);
}

#[test]
fn fluent_arrows_fit_device_pixels_at_fractional_scales() {
    // At 125% a button 19 by 23 device pixels holds an arrow 13 wide and 7
    // deep: Chromium's, measured in Electron.
    let geometry = fluent_geometry(0.8);
    let mut items = Vec::new();
    platform_items(
        Look::Fluent,
        &geometry,
        (0.0, 50.0),
        (None, None),
        0.8,
        &mut items,
    );
    let up = arrows(&items)[2];
    let width = (up[1].0 - up[0].0) / 0.8;
    let depth = (up[0].1 - up[2].1) / 0.8;
    assert!(
        (width - 13.0).abs() < 1e-3 && (depth - 7.0).abs() < 1e-3,
        "{width} {depth}"
    );
}
