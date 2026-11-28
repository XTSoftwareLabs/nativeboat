use super::*;
use crate::tests::support::Canvas;
use nativeboat_ir::Font;
use shimforge::{Session, mock};

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);
const NONE: Radii = [Corner::SQUARE; 4];
const WHITE: (u8, u8, u8) = (255, 255, 255);

fn fill(rect: Rect, color: Color) -> DisplayItem {
    DisplayItem::Fill {
        rect,
        color,
        radii: NONE,
    }
}

/// Paints a whole display list without the caret.
fn paint(dc: HDC, items: &[DisplayItem], scale: f32, scroll: (f32, f32)) {
    paint_area(dc, items, scale, scroll, None, false);
}

#[test]
fn carets_paint_sharp_in_their_blink_phase() {
    let caret = DisplayItem::Caret {
        rect: Rect::new(4.3, 2.0, 1.0, 10.0),
        color: RED,
    };
    let shown = Canvas::new(20, 20);
    paint_area(
        shown.dc,
        std::slice::from_ref(&caret),
        1.25,
        (0.0, 0.0),
        None,
        true,
    );
    // 4.3 * 1.25 rounds to pixel 5, one pixel wide.
    assert_eq!(shown.pixel(4, 6), WHITE);
    assert_eq!(shown.pixel(5, 6), (255, 0, 0));
    assert_eq!(shown.pixel(6, 6), WHITE);
    let hidden = Canvas::new(20, 20);
    paint_area(hidden.dc, &[caret], 1.25, (0.0, 0.0), None, false);
    assert_eq!(hidden.pixel(5, 6), WHITE);
}

#[test]
fn areas_limit_painting() {
    let canvas = Canvas::new(20, 20);
    let area = RECT {
        left: 5,
        top: 5,
        right: 10,
        bottom: 10,
    };
    let items = [
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED),
        DisplayItem::PushClip {
            rect: Rect::new(0.0, 0.0, 8.0, 20.0),
            radii: NONE,
        },
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), BLUE),
    ];
    paint_area(canvas.dc, &items, 1.0, (0.0, 0.0), Some(area), false);
    assert_eq!(canvas.pixel(2, 2), WHITE);
    assert_eq!(canvas.pixel(6, 6), (0, 0, 255));
    assert_eq!(canvas.pixel(9, 9), (255, 0, 0));
    assert_eq!(canvas.pixel(12, 12), WHITE);
    // The clip of the area is lifted afterwards.
    paint(
        canvas.dc,
        &[fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED)],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(2, 2), (255, 0, 0));
}

#[test]
fn clips_beside_the_area_skip_their_content_and_keep_later_state() {
    let clip = |x: f32| DisplayItem::PushClip {
        rect: Rect::new(x, 0.0, 4.0, 20.0),
        radii: NONE,
    };
    let items = [
        // Beside the area: skipped whole, with its nested groups.
        clip(14.0),
        DisplayItem::PushOpacity(0.5),
        clip(14.0),
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), BLUE),
        DisplayItem::PopClip,
        DisplayItem::PopOpacity,
        DisplayItem::PopClip,
        // Over the area: painted, and its pop still lifts it.
        clip(4.0),
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED),
        DisplayItem::PopClip,
        fill(Rect::new(8.0, 8.0, 2.0, 2.0), BLUE),
    ];
    let area = RECT {
        left: 5,
        top: 5,
        right: 10,
        bottom: 10,
    };
    let canvas = Canvas::new(20, 20);
    paint_area(canvas.dc, &items, 1.0, (0.0, 0.0), Some(area), false);
    assert_eq!(canvas.pixel(6, 6), (255, 0, 0));
    assert_eq!(canvas.pixel(9, 9), (0, 0, 255));
    assert_eq!(canvas.pixel(15, 6), WHITE);
    // Painting everything, clips past the target's edge are skipped too.
    let whole = Canvas::new(20, 20);
    let mut beyond = items.to_vec();
    beyond[0] = clip(40.0);
    paint(whole.dc, &beyond, 1.0, (0.0, 0.0));
    assert_eq!(whole.pixel(6, 6), (255, 0, 0));
    assert_eq!(whole.pixel(15, 6), WHITE);
    assert_eq!(whole.pixel(9, 9), (0, 0, 255));
}

/// A 2x2 24-bit BMP: blue and white on top, red and green below.
pub(crate) fn bitmap_file() -> Vec<u8> {
    let rows: [[u8; 8]; 2] = [
        // Bottom row first: red, green.
        [0, 0, 255, 0, 255, 0, 0, 0],
        // Top row: blue, white.
        [255, 0, 0, 255, 255, 255, 0, 0],
    ];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(54_u32 + 16).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for row in rows {
        bytes.extend_from_slice(&row);
    }
    bytes
}

#[test]
fn text_goes_to_the_nearest_row_and_halves_to_the_lower_as_skia_places_glyphs() {
    // As measured in Electron at 125%: text at each 32nd of a device pixel
    // keeps its row until halfway, and from there takes the next one down.
    for j in 0..32 {
        let expected = 40 + i32::from(j >= 16);
        assert_eq!(text_row(40.0 + j as f32 / 32.0), expected, "{j}");
    }
    assert_eq!(text_row(-3.5), -3);
    // 22.4px lines from a box at 2px sit 28 device pixels apart from a half
    // pixel; f32 noise around the half must not split them between rows,
    // and in Electron every one goes down.
    for k in 0..11 {
        let device = (2.0 + k as f32 * 22.4) * 1.25;
        assert_eq!(text_row(device), device.ceil() as i32, "{k}");
    }
    assert_eq!(text_row(219.499_98), 220);
    assert_eq!(text_row(779.500_06), 780);
}

#[test]
fn colors_convert() {
    assert_eq!(argb(Color::rgba(1, 2, 3, 0.5), 1.0), 0x80010203);
    assert_eq!(argb(Color::rgb(1, 2, 3), 0.0), 0x00010203);
    assert_eq!(colorref(Color::rgb(1, 2, 3)), 0x00030201);
}

#[test]
fn images_are_decoded_and_replaced() {
    assert_eq!(add_image("a.bmp", &bitmap_file()), Some((2.0, 2.0)));
    assert_eq!(add_image("a.bmp", &bitmap_file()), Some((2.0, 2.0)));
    assert_eq!(image_size("a.bmp"), Some((2.0, 2.0)));
    assert_eq!(image_size("missing.png"), None);
    assert_eq!(add_image("bad.png", b"not an image"), None);
    let mut session = Session::new();
    let create = mock!(
        session,
        SHCreateMemStream,
        unsafe extern "system" fn(*const u8, u32) -> *mut core::ffi::c_void
    );
    create.expect().returning(|_, _| null_mut());
    assert_eq!(add_image("b.bmp", &bitmap_file()), None);
}

#[test]
fn fills_clip_and_opacity() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            fill(Rect::new(0.0, 0.0, 10.0, 10.0), RED),
            DisplayItem::PushClip {
                rect: Rect::new(10.0, 0.0, 10.0, 10.0),
                radii: NONE,
            },
            fill(Rect::new(0.0, 10.0, 40.0, 10.0), BLUE),
            fill(Rect::new(10.0, 0.0, 30.0, 10.0), BLUE),
            DisplayItem::PopClip,
            DisplayItem::PopClip,
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(0.0, 20.0, 10.0, 10.0), RED),
            DisplayItem::PopOpacity,
            DisplayItem::PopOpacity,
            DisplayItem::Fill {
                rect: Rect::new(20.0, 20.0, 20.0, 20.0),
                color: BLUE,
                radii: [
                    Corner::round(10.0),
                    Corner::SQUARE,
                    Corner::SQUARE,
                    Corner::SQUARE,
                ],
            },
            DisplayItem::PushClip {
                rect: Rect::new(0.0, 30.0, 5.0, 5.0),
                radii: [Corner::round(2.0); 4],
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 5), (255, 0, 0));
    assert_eq!(canvas.pixel(15, 5), (0, 0, 255));
    assert_eq!(canvas.pixel(25, 5), WHITE);
    assert_eq!(canvas.pixel(5, 15), WHITE);
    assert_eq!(canvas.pixel(5, 25), (255, 127, 127));
    assert_eq!(canvas.pixel(21, 21), WHITE);
    assert_eq!(canvas.pixel(38, 38), (0, 0, 255));
}

#[test]
fn group_opacity_does_not_accumulate_where_descendants_overlap() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(0.0, 0.0, 30.0, 30.0), RED),
            fill(Rect::new(10.0, 10.0, 30.0, 30.0), BLUE),
            DisplayItem::PopOpacity,
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 5), (255, 127, 127));
    assert_eq!(canvas.pixel(15, 15), (127, 127, 255));
    assert_eq!(canvas.pixel(35, 35), (127, 127, 255));
    assert_eq!(canvas.pixel(35, 5), WHITE);
    let nested = Canvas::new(40, 40);
    paint(
        nested.dc,
        &[
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(10.0, 10.0, 20.0, 20.0), BLUE),
            DisplayItem::PopOpacity,
            DisplayItem::PopOpacity,
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(nested.pixel(5, 5), (255, 127, 127));
    assert_eq!(nested.pixel(15, 15), (191, 127, 191));
    let unclosed = Canvas::new(40, 40);
    paint(
        unclosed.dc,
        &[
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(unclosed.pixel(5, 5), (255, 127, 127));
}

#[test]
fn group_text_is_composited_over_each_backdrop_pixel_once() {
    let backdrop = DisplayItem::Gradient {
        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
        angle: 90.0,
        stops: vec![(RED, 0.0), (BLUE, 1.0)],
        radii: NONE,
    };
    let contents = vec![
        fill(Rect::new(0.0, 0.0, 40.0, 30.0), Color::WHITE),
        text("MW", Color::BLACK),
    ];
    let before = Canvas::new(40, 40);
    paint(before.dc, std::slice::from_ref(&backdrop), 1.0, (0.0, 0.0));
    let opaque = Canvas::new(40, 40);
    let mut items = vec![backdrop.clone()];
    items.extend(contents.clone());
    paint(opaque.dc, &items, 1.0, (0.0, 0.0));
    let faded = Canvas::new(40, 40);
    let mut items = vec![backdrop, DisplayItem::PushOpacity(0.6)];
    items.extend(contents);
    items.push(DisplayItem::PopOpacity);
    paint(faded.dc, &items, 1.0, (0.0, 0.0));
    for y in 0..40 {
        for x in 0..40 {
            let front = opaque.pixel(x, y);
            let back = before.pixel(x, y);
            let expected = std::array::from_fn::<_, 3, _>(|i| {
                let channels = |color: (u8, u8, u8)| [color.0, color.1, color.2];
                ((u32::from(channels(front)[i]) * 153 + u32::from(channels(back)[i]) * 102 + 127)
                    / 255) as u8
            });
            let actual = faded.pixel(x, y);
            assert_eq!([actual.0, actual.1, actual.2], expected, "pixel {x},{y}");
        }
    }
}

#[test]
fn bounded_opacity_matches_full_group_compositing_across_filters_and_transforms() {
    use nativeboat_css::types::Filter;
    for scale in [1.0, 1.25, 2.0] {
        let backdrop = DisplayItem::Gradient {
            rect: Rect::new(-20.0, -20.0, 220.0, 160.0),
            angle: 90.0,
            stops: vec![(RED, 0.0), (BLUE, 1.0)],
            radii: NONE,
        };
        let content = vec![
            DisplayItem::PushTransform([0.97, 0.2, -0.2, 0.97, 14.0, 2.0]),
            DisplayItem::PushClip {
                rect: Rect::new(0.0, 0.0, 75.0, 55.0),
                radii: NONE,
            },
            DisplayItem::PushFilter(vec![Filter::Brightness(1.0)]),
            fill(Rect::new(5.0, 5.0, 12.0, 12.0), RED),
            DisplayItem::PopFilter,
            // This later shape must remain inside the outer opacity group.
            fill(Rect::new(35.0, 20.0, 20.0, 18.0), BLUE),
            DisplayItem::PopClip,
            DisplayItem::PopTransform,
        ];
        let before = Canvas::new(160, 100);
        paint(
            before.dc,
            std::slice::from_ref(&backdrop),
            scale,
            (3.5, 1.25),
        );
        let opaque = Canvas::new(160, 100);
        let mut items = vec![backdrop.clone()];
        items.extend(content.clone());
        paint(opaque.dc, &items, scale, (3.5, 1.25));
        let faded = Canvas::new(160, 100);
        let mut items = vec![backdrop, DisplayItem::PushOpacity(0.6)];
        items.extend(content);
        items.push(DisplayItem::PopOpacity);
        paint(faded.dc, &items, scale, (3.5, 1.25));
        for y in 0..100 {
            for x in 0..160 {
                let (r, g, b) = opaque.pixel(x, y);
                let (br, bg, bb) = before.pixel(x, y);
                let expected = std::array::from_fn::<_, 3, _>(|i| {
                    ((u32::from([r, g, b][i]) * 153 + u32::from([br, bg, bb][i]) * 102 + 127) / 255)
                        as u8
                });
                let actual = faded.pixel(x, y);
                assert_eq!(
                    [actual.0, actual.1, actual.2],
                    expected,
                    "scale {scale}, pixel {x},{y}"
                );
            }
        }
    }
}

#[test]
fn empty_offscreen_and_nonfinite_opacity_content_preserves_pixels() {
    let canvas = Canvas::new(20, 20);
    for content in [
        Vec::new(),
        vec![fill(Rect::new(50.0, 50.0, 2.0, 2.0), RED)],
        vec![fill(Rect::new(f32::INFINITY, 0.0, 1.0, 1.0), RED)],
    ] {
        let mut items = vec![DisplayItem::PushOpacity(0.5)];
        items.extend(content);
        items.push(DisplayItem::PopOpacity);
        paint(canvas.dc, &items, 1.0, (0.0, 0.0));
        assert_eq!(canvas.pixel(10, 10), WHITE);
    }
}

#[test]
fn transparent_groups_paint_nothing_and_keep_later_state() {
    let canvas = Canvas::new(20, 20);
    let items = vec![
        DisplayItem::PushOpacity(0.0),
        DisplayItem::PushClip {
            rect: Rect::new(0.0, 0.0, 5.0, 5.0),
            radii: NONE,
        },
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED),
        text("W", RED),
        DisplayItem::PopClip,
        DisplayItem::PopOpacity,
        fill(Rect::new(10.0, 10.0, 5.0, 5.0), BLUE),
    ];
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert_eq!(canvas.pixel(2, 2), WHITE);
    assert_eq!(canvas.pixel(12, 12), (0, 0, 255));

    // A transform left in place by the group still moves what follows.
    let canvas = Canvas::new(20, 20);
    let items = vec![
        DisplayItem::PushOpacity(0.0),
        DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        fill(Rect::new(0.0, 0.0, 5.0, 5.0), RED),
        DisplayItem::PopOpacity,
        fill(Rect::new(0.0, 10.0, 5.0, 5.0), BLUE),
        DisplayItem::PopTransform,
    ];
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert_eq!(canvas.pixel(12, 2), WHITE);
    assert_eq!(canvas.pixel(2, 12), WHITE);
    assert_eq!(canvas.pixel(12, 12), (0, 0, 255));
}

#[test]
fn opacity_groups_keep_scroll_dpi_transforms_clipping_and_pixel_alpha() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            DisplayItem::PushClip {
                rect: Rect::new(5.0, 5.0, 10.0, 10.0),
                radii: NONE,
            },
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 2.0, 0.0]),
            DisplayItem::PushOpacity(0.5),
            fill(Rect::new(5.0, 5.0, 10.0, 10.0), RED),
            fill(Rect::new(5.0, 5.0, 10.0, 10.0), Color::rgba(0, 0, 255, 0.5)),
            DisplayItem::PopOpacity,
            DisplayItem::PopTransform,
            DisplayItem::PopClip,
        ],
        2.0,
        (2.0, 2.0),
    );
    assert_eq!(canvas.pixel(12, 10), (191, 127, 191));
    assert_eq!(canvas.pixel(8, 10), WHITE);
    assert_eq!(canvas.pixel(28, 10), WHITE);
}

#[test]
fn scrolling_and_scaling() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[fill(Rect::new(10.0, 10.0, 5.0, 5.0), RED)],
        2.0,
        (5.0, 5.0),
    );
    assert_eq!(canvas.pixel(9, 9), WHITE);
    assert_eq!(canvas.pixel(11, 11), (255, 0, 0));
    assert_eq!(canvas.pixel(18, 18), (255, 0, 0));
    assert_eq!(canvas.pixel(21, 21), WHITE);
}

#[test]
fn clip_paths_clip_fills_and_text_to_their_outline() {
    use nativeboat_css::types::FillRule;
    use nativeboat_ir::PathSegment::{Close, Line, Move};
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            // The lower left half of the square.
            DisplayItem::PushClipPath {
                segments: vec![Move(0.0, 0.0), Line(0.0, 40.0), Line(40.0, 40.0), Close],
                fill_rule: FillRule::NonZero,
            },
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
            text("MW", Color::BLACK),
            DisplayItem::PopClip,
            fill(Rect::new(30.0, 0.0, 10.0, 5.0), BLUE),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 35), (255, 0, 0));
    assert_eq!(canvas.pixel(35, 15), WHITE);
    // GDI's text keeps out of the clipped half too, and shows in the rest.
    let dark = |x: i32, y: i32| {
        let (r, g, b) = canvas.pixel(x, y);
        r < 100 && g < 100 && b < 100
    };
    assert!((0..40).all(|x| (0..20).all(|y| y + 2 >= x || !dark(x, y))));
    assert!((0..40).any(|x| (0..20).any(|y| y > x && dark(x, y))));
    // The clip ends with its group.
    assert_eq!(canvas.pixel(35, 2), (0, 0, 255));
}

#[test]
fn masks_keep_what_the_mask_alpha_covers() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            // Opaque at the top, transparent at the bottom.
            DisplayItem::PushMask(vec![DisplayItem::Gradient {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                angle: 180.0,
                stops: vec![(Color::BLACK, 0.0), (Color::TRANSPARENT, 1.0)],
                radii: NONE,
            }]),
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), BLUE),
            DisplayItem::PopMask,
        ],
        1.0,
        (0.0, 0.0),
    );
    let (top, middle, bottom) = (
        canvas.pixel(20, 1),
        canvas.pixel(20, 20),
        canvas.pixel(20, 39),
    );
    assert!(top.0 < 20 && top.2 > 230, "{top:?}");
    assert!(
        middle.0 > 90 && middle.0 < 170 && middle.2 > 230,
        "{middle:?}"
    );
    assert!(bottom.0 > 235, "{bottom:?}");
}

#[test]
fn gradients() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            DisplayItem::Gradient {
                rect: Rect::new(0.0, 0.0, 40.0, 20.0),
                angle: 180.0,
                stops: vec![(RED, 0.25), (BLUE, 0.75)],
                radii: NONE,
            },
            DisplayItem::Gradient {
                rect: Rect::new(0.0, 20.0, 40.0, 20.0),
                angle: 90.0,
                stops: Vec::new(),
                radii: NONE,
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(20, 1), (255, 0, 0));
    assert_eq!(canvas.pixel(20, 18), (0, 0, 255));
    assert_eq!(canvas.pixel(20, 30), WHITE);
}

#[test]
fn gradient_angles_preserve_css_projection_on_non_square_boxes() {
    // Independent reference: project the pixel center onto the CSS
    // direction; the projected corner extrema determine the stop range.
    for (width, height) in [(180, 40), (40, 180), (100, 100)] {
        for angle in [
            0.0_f32, 25.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0, -60.0, 420.0,
        ] {
            let canvas = Canvas::new(width, height);
            paint(
                canvas.dc,
                &[DisplayItem::Gradient {
                    rect: Rect::new(0.0, 0.0, width as f32, height as f32),
                    angle,
                    stops: vec![(RED, 0.0), (BLUE, 1.0)],
                    radii: NONE,
                }],
                1.0,
                (0.0, 0.0),
            );
            let direction = (angle.to_radians().sin(), -angle.to_radians().cos());
            let projections: Vec<f32> = [(0, 0), (width, 0), (0, height), (width, height)]
                .into_iter()
                .map(|(x, y)| x as f32 * direction.0 + y as f32 * direction.1)
                .collect();
            let low = projections.iter().copied().fold(f32::INFINITY, f32::min);
            let high = projections
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            for y in (2..height - 2).step_by(7) {
                for x in (2..width - 2).step_by(7) {
                    let t = (((x as f32 + 0.5) * direction.0 + (y as f32 + 0.5) * direction.1
                        - low)
                        / (high - low))
                        .clamp(0.0, 1.0);
                    let expected = (
                        (255.0 * (1.0 - t)).round() as u8,
                        0,
                        (255.0 * t).round() as u8,
                    );
                    let pixel = canvas.pixel(x, y);
                    assert!(
                        pixel.0.abs_diff(expected.0) <= 3
                            && pixel.1 == 0
                            && pixel.2.abs_diff(expected.2) <= 3,
                        "{width}x{height} at {angle}deg ({x}, {y}): {pixel:?}, expected {expected:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn gradients_preserve_stops_rounding_scroll_dpi_and_opacity() {
    let canvas = Canvas::new(160, 80);
    paint(
        canvas.dc,
        &[
            DisplayItem::PushOpacity(0.5),
            DisplayItem::Gradient {
                rect: Rect::new(15.0, 10.0, 60.0, 30.0),
                angle: 90.0,
                stops: vec![(RED, 0.25), (Color::rgb(0, 255, 0), 0.5), (BLUE, 0.75)],
                radii: [Corner::round(8.0); 4],
            },
        ],
        2.0,
        (5.0, 5.0),
    );
    assert_eq!(canvas.pixel(20, 10), WHITE);
    assert_eq!(canvas.pixel(10, 30), WHITE);
    for (point, expected) in [
        ((30, 40), (255, 127, 127)),
        ((80, 40), (127, 255, 127)),
        ((130, 40), (127, 127, 255)),
    ] {
        let pixel = canvas.pixel(point.0, point.1);
        assert!(
            pixel.0.abs_diff(expected.0) <= 5
                && pixel.1.abs_diff(expected.1) <= 5
                && pixel.2.abs_diff(expected.2) <= 5,
            "{point:?}: {pixel:?}, expected {expected:?}"
        );
    }
}

#[test]
fn gradients_with_outside_stops_match_the_css_line_and_color_intervals() {
    let yellow = Color::rgb(254, 220, 42);
    let pink = Color::rgb(221, 87, 137);
    let purple = Color::rgb(122, 44, 158);
    for stops in [
        vec![(yellow, -0.2909), (pink, 0.5177), (purple, 1.2935)],
        vec![(RED, -3.0), (BLUE, -1.0)],
        vec![(RED, 2.0), (BLUE, 4.0)],
        vec![(RED, -0.2), (BLUE, 0.8)],
        vec![(RED, 0.2), (BLUE, 1.8)],
    ] {
        for angle in [90.0_f32, 180.0, 200.96] {
            let (width, height) = (180, 80);
            let canvas = Canvas::new(width, height);
            paint(
                canvas.dc,
                &[DisplayItem::Gradient {
                    rect: Rect::new(0.0, 0.0, width as f32, height as f32),
                    angle,
                    stops: stops.clone(),
                    radii: NONE,
                }],
                1.0,
                (0.0, 0.0),
            );
            let direction = (angle.to_radians().sin(), -angle.to_radians().cos());
            let projection = |x: f32, y: f32| x * direction.0 + y * direction.1;
            let corners = [
                projection(0.0, 0.0),
                projection(width as f32, 0.0),
                projection(0.0, height as f32),
                projection(width as f32, height as f32),
            ];
            let low = corners.iter().copied().fold(f32::INFINITY, f32::min);
            let high = corners.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            for y in (2..height - 2).step_by(11) {
                for x in (2..width - 2).step_by(13) {
                    let t = (projection(x as f32 + 0.5, y as f32 + 0.5) - low) / (high - low);
                    let color = if t <= stops[0].1 {
                        stops[0].0
                    } else if t >= stops.last().unwrap().1 {
                        stops.last().unwrap().0
                    } else {
                        let pair = stops.windows(2).find(|pair| t <= pair[1].1).unwrap();
                        let weight = (t - pair[0].1) / (pair[1].1 - pair[0].1);
                        let channel = |a: u8, b: u8| {
                            (f32::from(a) * (1.0 - weight) + f32::from(b) * weight).round() as u8
                        };
                        Color::rgb(
                            channel(pair[0].0.r, pair[1].0.r),
                            channel(pair[0].0.g, pair[1].0.g),
                            channel(pair[0].0.b, pair[1].0.b),
                        )
                    };
                    let pixel = canvas.pixel(x, y);
                    assert!(
                        pixel.0.abs_diff(color.r) <= 3
                            && pixel.1.abs_diff(color.g) <= 3
                            && pixel.2.abs_diff(color.b) <= 3,
                        "{angle}deg {stops:?} ({x},{y}): {pixel:?} expected {color:?}"
                    );
                }
            }
        }
    }
}

fn border(
    widths: [f32; 4],
    colors: [Color; 4],
    styles: [BorderStyle; 4],
    radii: Radii,
) -> DisplayItem {
    DisplayItem::Border {
        rect: Rect::new(0.0, 0.0, 20.0, 20.0),
        widths,
        colors,
        styles,
        radii,
    }
}

type Samples = ((u8, u8, u8), (u8, u8, u8), (u8, u8, u8), (u8, u8, u8));

fn bordered(item: DisplayItem) -> Samples {
    let canvas = Canvas::new(20, 20);
    paint(canvas.dc, &[item], 1.0, (0.0, 0.0));
    (
        canvas.pixel(0, 10),
        canvas.pixel(10, 0),
        canvas.pixel(19, 10),
        canvas.pixel(10, 10),
    )
}

#[test]
fn borders() {
    let solid = [BorderStyle::Solid; 4];
    let red = (255, 0, 0);
    assert_eq!(
        bordered(border([2.0; 4], [RED; 4], solid, NONE)),
        (red, red, red, WHITE)
    );
    assert_eq!(
        bordered(border(
            [2.0, 0.0, 0.0, 2.0],
            [BLUE, RED, RED, RED],
            solid,
            NONE
        )),
        (red, (0, 0, 255), WHITE, WHITE)
    );
    assert_eq!(
        bordered(border([2.0; 4], [RED; 4], [BorderStyle::None; 4], NONE)),
        (WHITE, WHITE, WHITE, WHITE)
    );
    assert_eq!(
        bordered(border([0.0; 4], [RED; 4], solid, NONE)),
        (WHITE, WHITE, WHITE, WHITE)
    );
    let mixed = [
        BorderStyle::Solid,
        BorderStyle::Hidden,
        BorderStyle::Solid,
        BorderStyle::Solid,
    ];
    assert_eq!(bordered(border([2.0; 4], [RED; 4], mixed, NONE)).2, WHITE);
    for style in [BorderStyle::Dashed, BorderStyle::Dotted] {
        let (left, ..) = bordered(border(
            [2.0; 4],
            [RED; 4],
            [style; 4],
            [Corner::round(4.0); 4],
        ));
        assert_ne!(left, WHITE);
    }
}

#[test]
fn boxes_are_painted_at_whole_device_pixels_as_chromium_snaps_them() {
    let black = (0, 0, 0);
    // At 125%, boxes at fractions of a pixel. A border a device pixel wide
    // fills one row, and the rules of boxes that overlap by a CSS pixel, as
    // tables draw their cells' rules, meet in the same row.
    let ruled = |y: f32| DisplayItem::Border {
        rect: Rect::new(2.3, y, 16.0, 8.8),
        widths: [0.8; 4],
        colors: [Color::BLACK; 4],
        styles: [BorderStyle::Solid; 4],
        radii: NONE,
    };
    let rules = Canvas::new(30, 30);
    paint(rules.dc, &[ruled(1.1), ruled(8.9)], 1.25, (0.0, 0.0));
    let column: Vec<_> = (0..24).map(|y| rules.pixel(10, y) == black).collect();
    let rows: Vec<_> = (0..24).filter(|y| column[*y]).collect();
    assert_eq!(rows, [1, 11, 21]);
    assert_eq!(
        [2, 3, 4, 22, 23].map(|x| rules.pixel(x, 5) == black),
        [false, true, false, true, false]
    );
    // Fills and gradients, scrolled by a fraction of a pixel too, and a
    // sliver of a fill, which stays a pixel high.
    let boxes = Canvas::new(40, 40);
    let shade = DisplayItem::Gradient {
        rect: Rect::new(10.3, 4.3, 4.0, 4.0),
        angle: 90.0,
        stops: vec![(Color::BLACK, 0.0), (Color::BLACK, 1.0)],
        radii: NONE,
    };
    paint(
        boxes.dc,
        &[
            fill(Rect::new(0.3, 4.3, 4.0, 4.0), Color::BLACK),
            shade,
            fill(Rect::new(20.3, 4.3, 4.0, 0.1), Color::BLACK),
            // Nothing is moved where no pixel can be found.
            fill(Rect::new(0.0, 30.0, f32::INFINITY, 1.0), Color::BLACK),
        ],
        1.25,
        (0.0, 0.3),
    );
    assert_eq!(
        [(0, 5), (4, 9), (5, 9), (4, 10), (4, 4)].map(|(x, y)| boxes.pixel(x, y) == black),
        [true, true, false, false, false]
    );
    assert_eq!(
        [(13, 5), (12, 5), (17, 9), (18, 9)].map(|(x, y)| boxes.pixel(x, y) == black),
        [true, false, true, false]
    );
    assert_eq!(
        [(27, 5), (27, 6), (27, 4)].map(|(x, y)| boxes.pixel(x, y) == black),
        [true, false, false]
    );
    // A shadow a device pixel wide keeps its place against its box, and a
    // backdrop filter covers its box's pixels.
    let effects = Canvas::new(40, 40);
    let shadowed = Rect::new(2.3, 20.3, 8.0, 6.0);
    paint(
        effects.dc,
        &[
            DisplayItem::Shadow {
                rect: Rect::new(1.5, 19.5, 9.6, 7.6),
                radii: NONE,
                color: Color::BLACK,
                blur: 0.0,
                inset: false,
                clip: shadowed,
                clip_radii: NONE,
            },
            backdrop(
                Rect::new(20.3, 20.3, 8.0, 8.0),
                NONE,
                vec![Filter::Invert(1.0)],
            ),
        ],
        1.25,
        (0.0, 0.0),
    );
    assert_eq!(
        [(6, 23), (6, 24), (6, 25), (2, 28), (3, 28)].map(|(x, y)| effects.pixel(x, y) == black),
        [false, true, false, true, false]
    );
    assert_eq!(
        [(25, 25), (24, 25), (34, 34), (35, 34)].map(|(x, y)| effects.pixel(x, y) == black),
        [true, false, true, false]
    );
}

fn text(text: &str, color: Color) -> DisplayItem {
    DisplayItem::Text {
        rect: Rect::new(0.0, 0.0, 40.0, 20.0),
        text: text.to_owned(),
        font: Font {
            families: vec!["Arial".to_owned()],
            size: 18.0,
            weight: 700,
            italic: false,
            features: Vec::new(),
        },
        color,
        underline: true,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
        decorations: Vec::new(),
    }
}

fn darkest(canvas: &Canvas) -> (u8, u8, u8) {
    (0..40)
        .flat_map(|x| (0..20).map(move |y| (x, y)))
        .map(|(x, y)| canvas.pixel(x, y))
        .min_by_key(|(r, g, b)| u32::from(*r) + u32::from(*g) + u32::from(*b))
        .unwrap()
}

#[test]
fn text_is_drawn_with_blended_alpha() {
    let canvas = Canvas::new(40, 20);
    paint(canvas.dc, &[text("MW", Color::BLACK)], 1.0, (0.0, 0.0));
    assert_eq!(darkest(&canvas), (0, 0, 0));
    let faint = Canvas::new(40, 20);
    paint(
        faint.dc,
        &[text("MW", Color::rgba(0, 0, 0, 0.5))],
        1.0,
        (0.0, 0.0),
    );
    let (r, _, _) = darkest(&faint);
    assert!((120..=135).contains(&r), "{r}");
    let hidden = Canvas::new(40, 20);
    paint(
        hidden.dc,
        &[DisplayItem::PushOpacity(0.0), text("MW", Color::BLACK)],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(darkest(&hidden), WHITE);
}

#[test]
fn grayscale_text_takes_the_mean_coverage_of_its_channels() {
    let mut gray = text("MW", Color::BLACK);
    if let DisplayItem::Text { grayscale, .. } = &mut gray {
        *grayscale = true;
    }
    let canvas = Canvas::new(40, 20);
    paint(canvas.dc, &[gray], 1.0, (0.0, 0.0));
    let pixels: Vec<_> = (0..40)
        .flat_map(|x| (0..20).map(move |y| (x, y)))
        .map(|(x, y)| canvas.pixel(x, y))
        .collect();
    assert!(pixels.iter().all(|(r, g, b)| r == g && g == b));
    assert!(pixels.iter().any(|(r, ..)| *r < 64));
    // Coverage of all, half and none of a pixel over white averages out.
    let white = 0x00FF_FFFF;
    assert_eq!(gray_pixel(white, 0x0000_80FF, Color::BLACK), 0x0080_8080);
    // Channels where text and background nearly match are left out.
    let red = Color::rgb(255, 0, 0);
    assert_eq!(gray_pixel(white, 0x00FF_0080, red), 0x00FF_4040);
    // Untouched pixels, and text the color of its background, stay.
    assert_eq!(gray_pixel(white, white, Color::BLACK), white);
    let pale = Color::rgb(250, 250, 250);
    assert_eq!(gray_pixel(white, 0x00FE_FEFE, pale), 0x00FE_FEFE);
}

#[test]
fn grayscale_text_is_rasterized_and_corrected_as_skia_does() {
    // Without decorations, DirectWrite rasterizes the glyphs and Skia's
    // correction for the text's luminance weighs light text on a dark
    // background more than dark text on a light one.
    let plain = |color| {
        let mut item = text("MW", color);
        if let DisplayItem::Text {
            grayscale,
            underline,
            ..
        } = &mut item
        {
            (*grayscale, *underline) = (true, false);
        }
        item
    };
    let ink = |canvas: &Canvas, back: u8| {
        let pixels: Vec<_> = (0..40)
            .flat_map(|x| (0..20).map(move |y| (x, y)))
            .map(|(x, y)| canvas.pixel(x, y))
            .collect();
        assert!(pixels.iter().all(|(r, g, b)| r == g && g == b));
        pixels
            .iter()
            .map(|(r, ..)| f32::from(r.abs_diff(back)))
            .sum::<f32>()
    };
    let light = Canvas::new(40, 20);
    paint(light.dc, &[plain(Color::BLACK)], 1.0, (0.0, 0.0));
    let dark = Canvas::new(40, 20);
    let night = DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, 40.0, 20.0),
        color: Color::BLACK,
        radii: [Corner::round(0.0); 4],
    };
    paint(
        dark.dc,
        &[night.clone(), plain(Color::WHITE)],
        1.0,
        (0.0, 0.0),
    );
    let (on_light, on_dark) = (ink(&light, 255), ink(&dark, 0));
    assert!(
        on_light > 0.0 && on_dark > on_light * 1.1,
        "{on_light} {on_dark}"
    );
    // Clipped, it changes nothing outside the clip.
    let clipped = Canvas::new(40, 20);
    paint(
        clipped.dc,
        &[
            night,
            DisplayItem::PushClip {
                rect: Rect::new(0.0, 0.0, 10.0, 20.0),
                radii: [Corner::round(0.0); 4],
            },
            plain(Color::WHITE),
            DisplayItem::PopClip,
        ],
        1.0,
        (0.0, 0.0),
    );
    assert!((10..40).all(|x| (0..20).all(|y| clipped.pixel(x, y) == (0, 0, 0))));
    assert!((0..10).any(|x| (0..20).any(|y| clipped.pixel(x, y) != (0, 0, 0))));
    // Clipped away, it changes nothing.
    let hidden = Canvas::new(40, 20);
    paint(
        hidden.dc,
        &[
            DisplayItem::PushClip {
                rect: Rect::new(30.0, 30.0, 5.0, 5.0),
                radii: [Corner::round(0.0); 4],
            },
            plain(Color::BLACK),
            DisplayItem::PopClip,
        ],
        1.0,
        (0.0, 0.0),
    );
    assert!((0..40).all(|x| (0..20).all(|y| hidden.pixel(x, y) == WHITE)));
}

#[test]
fn grayscale_coverage_is_corrected_for_the_text_luminance_as_skia_does() {
    // Skia's tables for black, a dark and a light gray, and white text.
    for (color, expected) in [
        (Color::BLACK, [0, 58, 119, 185, 255]),
        (Color::rgb(100, 100, 100), [0, 69, 135, 198, 255]),
        (Color::rgb(140, 140, 140), [0, 81, 149, 207, 255]),
        (Color::WHITE, [0, 137, 188, 225, 255]),
    ] {
        let table = mask_gamma(color);
        assert_eq!(
            [0, 64, 128, 192, 255].map(|at| table[at]),
            expected,
            "{color:?}"
        );
    }
}

#[test]
fn images_are_drawn() {
    add_image("draw.bmp", &bitmap_file()).unwrap();
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            DisplayItem::Image {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                url: "draw.bmp".to_owned(),
            },
            DisplayItem::PushOpacity(0.5),
            DisplayItem::Image {
                rect: Rect::new(20.0, 0.0, 20.0, 20.0),
                url: "draw.bmp".to_owned(),
            },
            DisplayItem::Image {
                rect: Rect::new(0.0, 20.0, 20.0, 20.0),
                url: "unknown.png".to_owned(),
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    let (r, g, b) = canvas.pixel(3, 3);
    assert!(r < 128 && g < 128 && b > 200, "{:?}", (r, g, b));
    let (r, _, b) = canvas.pixel(23, 3);
    assert!(r > 100 && b > 200, "{r} {b}");
    assert_eq!(canvas.pixel(5, 25), WHITE);
}

#[test]
fn video_frames_are_sampled_bilinearly_as_chromium_composites_them() {
    // Eight black columns but the fourth, white, two rows tall.
    let mut picture = Vec::new();
    picture.extend_from_slice(b"BM");
    picture.extend_from_slice(&(54_u32 + 48).to_le_bytes());
    picture.extend_from_slice(&[0; 4]);
    picture.extend_from_slice(&54_u32.to_le_bytes());
    picture.extend_from_slice(&40_u32.to_le_bytes());
    picture.extend_from_slice(&8_i32.to_le_bytes());
    picture.extend_from_slice(&2_i32.to_le_bytes());
    picture.extend_from_slice(&1_u16.to_le_bytes());
    picture.extend_from_slice(&24_u16.to_le_bytes());
    picture.extend_from_slice(&[0; 24]);
    for _ in 0..2 {
        for column in 0..8 {
            picture.extend_from_slice(&[if column == 3 { 255 } else { 0 }; 3]);
        }
    }
    let frame = format!("{}column.mp4", nativeboat_layout::VIDEO_FRAME);
    add_image("column.bmp", &picture).unwrap();
    add_image(&frame, &picture).unwrap();
    let canvas = Canvas::new(4, 2);
    paint(
        canvas.dc,
        &[
            DisplayItem::Image {
                rect: Rect::new(0.0, 0.0, 2.0, 2.0),
                url: "column.bmp".to_owned(),
            },
            DisplayItem::Image {
                rect: Rect::new(2.0, 0.0, 2.0, 2.0),
                url: frame,
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    // Shrunk to two pixels, an image is filtered, so its white column greys
    // the first pixel; a frame takes the two columns nearest each pixel's
    // centre, which are black.
    let (grey, ..) = canvas.pixel(0, 0);
    assert!(grey > 20, "{grey}");
    assert_eq!(canvas.pixel(2, 0), (0, 0, 0));
    assert_eq!(canvas.pixel(3, 1), (0, 0, 0));
}

#[test]
fn image_alpha_fallback_draws_and_releases_its_image_attributes() {
    add_image("fallback.bmp", &bitmap_file()).unwrap();
    let canvas = Canvas::new(40, 40);
    let mut graphics = null_mut();
    // SAFETY: graphics wraps the live canvas DC and is released below.
    unsafe {
        assert_eq!(GdipCreateFromHDC(canvas.dc, &mut graphics), Ok);
        GdipSetInterpolationMode(graphics, InterpolationModeNearestNeighbor);
    }
    let mut painter = Painter {
        graphics,
        dc: canvas.dc,
        scale: 1.0,
        scroll: (0.0, 0.0),
        opacity: vec![],
        // SAFETY: an unavailable backdrop causes alpha fallback without
        // accessing the rejected descriptor's pixels.
        groups: unsafe { opacity::Groups::new(BITMAP::default()) },
        // SAFETY: an unavailable bitmap has no pixels to blend.
        corners: RefCell::new(unsafe { corners::Coverage::new(BITMAP::default()) }),
        rounded_clips: vec![],
        target: BITMAP::default(),
        states: vec![],
        transforms: vec![],
        caret: false,
        area: None,
    };
    painter.push_opacity(0.5);
    assert_eq!(painter.opacity(), 0.5);
    painter.image(Rect::new(0.0, 0.0, 20.0, 20.0), "fallback.bmp");
    painter.pop_opacity();
    // SAFETY: graphics was created above and has finished drawing.
    unsafe {
        GdipDeleteGraphics(graphics);
    }
    let (r, g, b) = canvas.pixel(3, 3);
    assert!(
        (120..=135).contains(&r) && (120..=135).contains(&g) && b > 240,
        "{r},{g},{b}"
    );
}

#[test]
fn dashed_strokes_leave_gaps_from_their_offset() {
    use nativeboat_css::types::{FillRule, LineCap, LineJoin};
    use nativeboat_ir::PathSegment::{Line, Move};
    let dashed = |offset: f32| DisplayItem::Path {
        segments: vec![Move(0.0, 5.0), Line(40.0, 5.0)],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        fill: None,
        fill_rule: FillRule::NonZero,
        stroke: Some(Stroke {
            color: Color::BLACK,
            width: 2.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            dashes: vec![6.0, 4.0],
            dash_offset: offset,
        }),
    };
    let canvas = Canvas::new(40, 10);
    paint(canvas.dc, &[dashed(0.0)], 1.0, (0.0, 0.0));
    let dark = |canvas: &Canvas, x| canvas.pixel(x, 5).0 < 128;
    // Six units of dash, four of gap, repeated.
    assert!(dark(&canvas, 2) && dark(&canvas, 13));
    assert!(!dark(&canvas, 8) && !dark(&canvas, 18));
    // The offset starts the line that far into the pattern.
    let shifted = Canvas::new(40, 10);
    paint(shifted.dc, &[dashed(3.0)], 1.0, (0.0, 0.0));
    assert!(dark(&shifted, 1) && !dark(&shifted, 5) && dark(&shifted, 8));
}

#[test]
fn vector_paths_fill_and_stroke() {
    use nativeboat_css::types::{FillRule, LineCap, LineJoin};
    use nativeboat_ir::PathSegment::{Close, Cubic, Line, Move};
    let square = |size: f32, at: f32| {
        vec![
            Move(at, at),
            Line(at + size, at),
            Line(at + size, at + size),
            Line(at, at + size),
            Close,
        ]
    };
    let stroke = |color, cap, join| {
        Some(Stroke {
            color,
            width: 4.0,
            cap,
            join,
            dashes: Vec::new(),
            dash_offset: 0.0,
        })
    };
    let canvas = Canvas::new(80, 80);
    let mut ring = square(30.0, 0.0);
    ring.extend(square(10.0, 10.0));
    paint(
        canvas.dc,
        &[
            DisplayItem::Path {
                segments: square(10.0, 0.0),
                transform: [1.0, 0.0, 0.0, 1.0, 5.0, 5.0],
                fill: Some(RED),
                fill_rule: FillRule::NonZero,
                stroke: None,
            },
            DisplayItem::Path {
                segments: vec![Move(20.0, 5.0), Cubic(25.0, 5.0, 30.0, 5.0, 35.0, 5.0)],
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: None,
                fill_rule: FillRule::NonZero,
                stroke: stroke(BLUE, LineCap::Round, LineJoin::Round),
            },
            DisplayItem::Path {
                segments: ring,
                transform: [1.0, 0.0, 0.0, 1.0, 40.0, 40.0],
                fill: Some(RED),
                fill_rule: FillRule::EvenOdd,
                stroke: stroke(BLUE, LineCap::Square, LineJoin::Bevel),
            },
            DisplayItem::Path {
                segments: vec![Move(0.0, 0.0), Line(5.0, 0.0), Line(5.0, 5.0)],
                transform: [2.0, 0.0, 0.0, 2.0, 5.0, 40.0],
                fill: None,
                fill_rule: FillRule::NonZero,
                stroke: stroke(RED, LineCap::Butt, LineJoin::Miter),
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(10, 10), (255, 0, 0));
    assert_eq!(canvas.pixel(3, 3), WHITE);
    assert_eq!(canvas.pixel(28, 5), (0, 0, 255));
    assert_eq!(canvas.pixel(28, 12), WHITE);
    assert_eq!(canvas.pixel(45, 55), (255, 0, 0));
    assert_eq!(canvas.pixel(40, 55), (0, 0, 255), "the ring's outline");
    assert_eq!(canvas.pixel(55, 55), WHITE, "the even-odd hole");
    assert_eq!(canvas.pixel(10, 40), (255, 0, 0), "a scaled stroke");
    assert_eq!(canvas.pixel(10, 47), WHITE);
}

#[test]
fn vector_images_are_drawn_into_their_box() {
    use nativeboat_css::types::FillRule;
    use nativeboat_ir::PathSegment::{Close, Line, Move};
    let square = DisplayItem::Path {
        segments: vec![
            Move(0.0, 0.0),
            Line(10.0, 0.0),
            Line(10.0, 10.0),
            Line(0.0, 10.0),
            Close,
        ],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        fill: Some(RED),
        fill_rule: FillRule::NonZero,
        stroke: None,
    };
    add_vector_image(
        "vector.svg",
        (10.0, 10.0),
        vec![
            DisplayItem::PushClip {
                rect: Rect::new(0.0, 0.0, 5.0, 10.0),
                radii: NONE,
            },
            square,
            DisplayItem::PopClip,
        ],
    );
    assert_eq!(image_size("vector.svg"), Some((10.0, 10.0)));
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[DisplayItem::Image {
            rect: Rect::new(10.0, 10.0, 20.0, 20.0),
            url: "vector.svg".to_owned(),
        }],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(15, 25), (255, 0, 0));
    assert_eq!(canvas.pixel(25, 15), WHITE, "clipped to the left half");
    assert_eq!(canvas.pixel(5, 5), WHITE);
}

/// SVG files added as images become vector images at their natural size,
/// drawn scaled into the image's box: after a byte order mark, an XML
/// declaration, a comment and a doctype, with the file's own style sheet.
/// Text whose root is not `<svg>` is no SVG file.
#[test]
fn svg_files_added_as_images_draw_as_vectors() {
    let svg = "\u{feff}<?xml version=\"1.0\"?>\n<!-- icon -->\n<!DOCTYPE svg>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"8\" viewBox=\"0 0 2 1\"><style>.a { fill: #ff0000 }</style><rect class=\"a\" width=\"1\" height=\"1\"/></svg>";
    assert_eq!(add_image("icon.svg", svg.as_bytes()), Some((16.0, 8.0)));
    assert_eq!(image_size("icon.svg"), Some((16.0, 8.0)));
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[DisplayItem::Image {
            rect: Rect::new(0.0, 0.0, 40.0, 20.0),
            url: "icon.svg".to_owned(),
        }],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(
        canvas.pixel(10, 10),
        (255, 0, 0),
        "the left half is the rect"
    );
    assert_eq!(canvas.pixel(30, 10), WHITE);
    assert_eq!(canvas.pixel(10, 30), WHITE);
    for text in [
        "<svgx/>",
        "<html><svg/></html>",
        "plain text",
        "<?xml version=\"1.0\"?>",
        "<!-- open",
    ] {
        assert_eq!(svg_source(text.as_bytes()), None, "{text}");
    }
    assert_eq!(svg_source(b"\xff<svg/>"), None);
    assert!(svg_source(b"  <svg/>").is_some());
}

#[test]
fn canvases_are_painted_from_their_bitmaps() {
    use nativeboat_css::types::FillRule;
    use nativeboat_ir::PathSegment::{Close, Line, Move};
    use nativeboat_ir::{CanvasCommand, CanvasPaint};
    crate::canvas::canvas_draw(
        9100,
        2,
        2,
        &[CanvasCommand::Fill {
            path: vec![
                Move(0.0, 0.0),
                Line(2.0, 0.0),
                Line(2.0, 2.0),
                Line(0.0, 2.0),
                Close,
            ],
            paint: CanvasPaint::Color(RED),
            rule: FillRule::NonZero,
            alpha: 1.0,
            copy: false,
        }],
    );
    let canvas = Canvas::new(20, 20);
    paint(
        canvas.dc,
        &[
            DisplayItem::Canvas {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                key: 9100,
                version: 1,
            },
            DisplayItem::Canvas {
                rect: Rect::new(10.0, 10.0, 10.0, 10.0),
                key: 404,
                version: 1,
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 5), (255, 0, 0));
    assert_eq!(canvas.pixel(15, 15), WHITE);
    crate::canvas::canvas_release(9100);
}

#[test]
fn shadows_fall_outside_or_inside_boxes() {
    let canvas = Canvas::new(80, 40);
    let box_rect = Rect::new(10.0, 10.0, 20.0, 20.0);
    let inner = Rect::new(50.0, 10.0, 20.0, 20.0);
    paint(
        canvas.dc,
        &[
            DisplayItem::Shadow {
                rect: Rect::new(14.0, 14.0, 20.0, 20.0),
                radii: NONE,
                color: BLUE,
                blur: 0.0,
                inset: false,
                clip: box_rect,
                clip_radii: NONE,
            },
            DisplayItem::Shadow {
                rect: box_rect,
                radii: [Corner::round(4.0); 4],
                color: RED,
                blur: 6.0,
                inset: false,
                clip: box_rect,
                clip_radii: [Corner::round(4.0); 4],
            },
            DisplayItem::Shadow {
                rect: Rect::new(54.0, 14.0, 12.0, 12.0),
                radii: [Corner::round(2.0); 4],
                color: BLUE,
                blur: 4.0,
                inset: true,
                clip: inner,
                clip_radii: NONE,
            },
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(32, 32), (0, 0, 255), "the offset shadow");
    assert_eq!(canvas.pixel(20, 20), WHITE, "nothing under the box");
    let blurred = canvas.pixel(9, 20);
    assert!(blurred.0 > blurred.2 && blurred != WHITE, "{blurred:?}");
    assert_eq!(canvas.pixel(5, 5), WHITE, "beyond the blur");
    let inset = canvas.pixel(51, 20);
    assert!(
        inset.2 == 255 && inset.0 < 30 && inset.1 == inset.0,
        "the inset shadow: {inset:?}"
    );
    assert_eq!(canvas.pixel(60, 20), WHITE, "the middle of the box");
    assert_eq!(canvas.pixel(45, 20), WHITE, "outside the box");
}

#[test]
#[ignore = "manual shadow painting benchmark"]
fn shadow_paint_benchmark() {
    let canvas = Canvas::new(1000, 400);
    let items: Vec<_> = (0..3)
        .map(|index| DisplayItem::Shadow {
            rect: Rect::new(30.0 + 320.0 * index as f32, 42.0, 300.0, 180.0),
            radii: [Corner::round(12.0); 4],
            color: Color::rgba(15, 23, 42, 0.12),
            blur: 30.0,
            inset: false,
            clip: Rect::new(30.0 + 320.0 * index as f32, 30.0, 300.0, 180.0),
            clip_radii: [Corner::round(12.0); 4],
        })
        .collect();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        paint(canvas.dc, &items, 1.25, (0.0, 0.0));
    }
    eprintln!(
        "three shadows: {:.3} ms per paint",
        start.elapsed().as_secs_f64() * 10.0
    );
}

#[test]
fn blurred_shadows_match_a_gaussian_reference_without_dark_fringes() {
    // Independent discrete Gaussian convolution of a straight box edge.
    // Include both dimensions so the large-blur/downsampling case is checked.
    let mass = |start: i32, end: i32, sample: i32, sigma: f64| {
        let kernel = |distance: f64| (-distance * distance / (2.0 * sigma * sigma)).exp();
        let radius = (4.0 * sigma).ceil() as i32;
        let total: f64 = (-radius..=radius).map(|d| kernel(d as f64)).sum();
        (start..end)
            .map(|p| kernel((p - sample) as f64))
            .sum::<f64>()
            / total
    };
    for blur in [0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 60.0, 120.0, 300.0] {
        let canvas = Canvas::new(1250, 700);
        let rect = Rect::new(250.0, 0.0, 300.0, 700.0);
        paint(
            canvas.dc,
            &[DisplayItem::Shadow {
                rect,
                radii: NONE,
                color: RED,
                blur,
                inset: false,
                clip: rect,
                clip_radii: NONE,
            }],
            1.0,
            (0.0, 0.0),
        );
        for distance in 0..=(2.0 * blur) as i32 {
            let x = 550 + distance;
            let sigma = blur as f64 / 2.0;
            let expected = 255.0 * mass(250, 550, x, sigma) * mass(0, 700, 350, sigma);
            let pixel = canvas.pixel(x, 350);
            let actual = 255 - pixel.1;
            assert_eq!(
                pixel.0, 255,
                "no dark fringe at blur {blur}, distance {distance}: {pixel:?}"
            );
            assert_eq!(pixel.1, pixel.2);
            assert!(
                (actual as f64 - expected).abs() <= 13.0,
                "blur {blur}, distance {distance}: alpha {actual}, Gaussian {expected}"
            );
        }
    }
}

#[test]
fn shadow_bitmaps_preserve_opacity_scroll_dpi_and_transforms() {
    let canvas = Canvas::new(500, 500);
    let shape = Rect::new(50.0, 50.0, 40.0, 40.0);
    let shadow = DisplayItem::Shadow {
        rect: shape,
        radii: NONE,
        color: Color::rgba(40, 140, 220, 0.5),
        blur: 8.0,
        inset: false,
        clip: Rect::new(0.0, 0.0, 10.0, 10.0),
        clip_radii: NONE,
    };
    let items = [
        DisplayItem::PushOpacity(0.5),
        DisplayItem::PushTransform([0.0, 1.0, -1.0, 0.0, 200.0, 0.0]),
        shadow,
        DisplayItem::PopTransform,
        DisplayItem::PopOpacity,
    ];
    paint(canvas.dc, &items, 2.0, (5.0, 5.0));
    let pixel = canvas.pixel(250, 130);
    for (actual, expected) in [pixel.0, pixel.1, pixel.2].into_iter().zip([201, 226, 246]) {
        assert!(
            (actual as i32 - expected).abs() <= 2,
            "opacity and color: {pixel:?}"
        );
    }
    assert_eq!(canvas.pixel(130, 130), WHITE, "the shape was rotated");
    // Painting the same key again exercises cached drawing through transforms.
    paint(canvas.dc, &items, 2.0, (5.0, 5.0));
}

#[test]
fn shadow_cache_is_bounded_evicts_least_recently_used_and_releases_images() {
    let mut session = Session::new();
    let dispose = mock!(
        session,
        GdipDisposeImage,
        unsafe extern "system" fn(*mut GpImage) -> Status
    );
    let released = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = released.clone();
    dispose.expect().returning(move |_| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok
    });
    let image = |bytes| {
        Rc::new(ShadowImage {
            handle: std::ptr::dangling_mut(),
            bytes,
        })
    };
    let mut cache = ShadowCache::new(8);
    cache.insert([1; 17], image(4));
    cache.insert([2; 17], image(4));
    assert!(cache.get(&[1; 17]).is_some());
    cache.insert([3; 17], image(4));
    assert!(cache.get(&[2; 17]).is_none());
    assert!(cache.get(&[1; 17]).is_some());
    cache.insert([4; 17], image(12));
    assert_eq!(cache.bytes, 8);
    assert!(cache.get(&[4; 17]).is_none());
    drop(cache);
    assert_eq!(released.load(std::sync::atomic::Ordering::SeqCst), 4);
    let mut cache = ShadowCache::new(100);
    for key in 0..34 {
        cache.insert([key; 17], image(1));
    }
    assert_eq!(cache.entries.len(), 32);
    assert!(cache.get(&[0; 17]).is_none());
    drop(cache);
    assert_eq!(released.load(std::sync::atomic::Ordering::SeqCst), 38);
}

#[test]
fn shadow_failures_release_allocations_and_restore_clipping() {
    let canvas = Canvas::new(50, 50);
    let painter = Painter {
        graphics: null_mut(),
        dc: canvas.dc,
        target: BITMAP::default(),
        scale: 1.0,
        scroll: (0.0, 0.0),
        opacity: vec![],
        // SAFETY: the empty descriptor is rejected without accessing pixels.
        groups: unsafe { opacity::Groups::new(BITMAP::default()) },
        // SAFETY: an unavailable bitmap has no pixels to blend.
        corners: RefCell::new(unsafe { corners::Coverage::new(BITMAP::default()) }),
        rounded_clips: vec![],
        states: vec![],
        transforms: vec![],
        caret: false,
        area: None,
    };
    let make = || {
        painter.shadow_bitmap(
            RectF {
                X: 5.0,
                Y: 5.0,
                Width: 10.0,
                Height: 10.0,
            },
            NONE,
            RED,
            4.0,
            false,
            (20, 20),
        )
    };
    let mut session = Session::new();
    let create = mock!(
        session,
        GdipCreateBitmapFromScan0,
        unsafe extern "system" fn(i32, i32, i32, i32, *const u8, *mut *mut GpBitmap) -> Status
    );
    create.expect().returning(|_, _, _, _, _, _| GenericError);
    assert!(make().is_none());
    session.restore();
    let failed_effect = mock!(
        session,
        GdipCreateEffect,
        unsafe extern "system" fn(windows_sys::core::GUID, *mut *mut CGpEffect) -> Status
    );
    failed_effect.expect().returning(|_, _| GenericError);
    let rect = Rect::new(10.0, 10.0, 20.0, 20.0);
    paint(
        canvas.dc,
        &[
            DisplayItem::Shadow {
                rect,
                radii: NONE,
                color: RED,
                blur: 4.0,
                inset: false,
                clip: rect,
                clip_radii: NONE,
            },
            fill(Rect::new(0.0, 0.0, 50.0, 50.0), BLUE),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(
        canvas.pixel(20, 20),
        (0, 0, 255),
        "failed shadow restored the clip"
    );
    session.restore();
    let graphics = mock!(
        session,
        GdipGetImageGraphicsContext,
        unsafe extern "system" fn(*mut GpImage, *mut *mut GpGraphics) -> Status
    );
    graphics.expect().returning(|_, _| GenericError);
    assert!(make().is_none());
    session.restore();
    let effect = mock!(
        session,
        GdipCreateEffect,
        unsafe extern "system" fn(windows_sys::core::GUID, *mut *mut CGpEffect) -> Status
    );
    effect.expect().returning(|_, _| GenericError);
    assert!(make().is_none());
    session.restore();
    let parameters = mock!(
        session,
        GdipSetEffectParameters,
        unsafe extern "system" fn(*mut CGpEffect, *const core::ffi::c_void, u32) -> Status
    );
    parameters.expect().returning(|_, _, _| GenericError);
    assert!(make().is_none());
    session.restore();
    let apply = mock!(
        session,
        GdipBitmapApplyEffect,
        unsafe extern "system" fn(
            *mut GpBitmap,
            *mut CGpEffect,
            *mut windows_sys::Win32::Foundation::RECT,
            i32,
            *mut *mut core::ffi::c_void,
            *mut i32,
        ) -> Status
    );
    apply.expect().returning(|_, _, _, _, _, _| GenericError);
    assert!(make().is_none());
    session.restore();
    let lock = mock!(
        session,
        GdipBitmapLockBits,
        unsafe extern "system" fn(
            *mut GpBitmap,
            *const windows_sys::Win32::Graphics::GdiPlus::Rect,
            u32,
            i32,
            *mut BitmapData,
        ) -> Status
    );
    lock.expect().returning(|_, _, _, _, _| GenericError);
    assert!(make().is_none());
}

#[test]
fn transforms_rotate_painting() {
    use nativeboat_css::types::FillRule;
    use nativeboat_ir::PathSegment::{Close, Line, Move};
    let canvas = Canvas::new(40, 40);
    // A quarter turn around (20, 20) turns a horizontal bar vertical.
    paint(
        canvas.dc,
        &[
            DisplayItem::PushTransform([0.0, 1.0, -1.0, 0.0, 40.0, 0.0]),
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            fill(Rect::new(5.0, 18.0, 30.0, 4.0), RED),
            DisplayItem::Path {
                segments: vec![
                    Move(18.0, 2.0),
                    Line(22.0, 2.0),
                    Line(22.0, 4.0),
                    Line(18.0, 4.0),
                    Close,
                ],
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: Some(BLUE),
                fill_rule: FillRule::NonZero,
                stroke: None,
            },
            text("x", RED),
            DisplayItem::PopTransform,
            DisplayItem::PopTransform,
            fill(Rect::new(0.0, 0.0, 2.0, 2.0), BLUE),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(20, 8), (255, 0, 0), "the bar stands up");
    assert_eq!(canvas.pixel(8, 20), WHITE);
    assert_eq!(canvas.pixel(37, 20), (0, 0, 255), "the path turned too");
    assert_eq!(
        canvas.pixel(1, 1),
        (0, 0, 255),
        "drawing after the transform"
    );
}

const CYAN: Color = Color::rgb(0, 255, 255);

/// Items painted through filter functions.
fn filtered(filters: Vec<Filter>, items: Vec<DisplayItem>) -> Vec<DisplayItem> {
    let mut list = vec![DisplayItem::PushFilter(filters)];
    list.extend(items);
    list.push(DisplayItem::PopFilter);
    list
}

/// Whether colors differ by at most 2 in each channel.
fn close(actual: (u8, u8, u8), expected: (u8, u8, u8)) -> bool {
    actual.0.abs_diff(expected.0) <= 2
        && actual.1.abs_diff(expected.1) <= 2
        && actual.2.abs_diff(expected.2) <= 2
}

fn shadow_filter(x: f32, y: f32, color: Color) -> Filter {
    Filter::DropShadow {
        x,
        y,
        blur: 0.0,
        color,
    }
}

#[test]
fn drop_shadows_recolor_clipped_icons() {
    // Muya colors icons by moving the shadow of an icon into a clipped box
    // that hides the icon itself.
    for scale in [1.0, 1.25] {
        let canvas = Canvas::new(50, 30);
        let mut items = vec![DisplayItem::PushClip {
            rect: Rect::new(16.0, 0.0, 16.0, 16.0),
            radii: NONE,
        }];
        items.extend(filtered(
            vec![shadow_filter(16.0, 0.0, BLUE)],
            vec![fill(Rect::new(0.0, 0.0, 16.0, 16.0), Color::BLACK)],
        ));
        items.push(DisplayItem::PopClip);
        paint(canvas.dc, &items, scale, (0.0, 0.0));
        let px = |css: f32| (css * scale) as i32;
        assert_eq!(canvas.pixel(px(8.0), px(8.0)), WHITE, "the icon is hidden");
        assert_eq!(canvas.pixel(px(16.0), px(8.0)), (0, 0, 255));
        assert_eq!(canvas.pixel(px(32.0) - 1, px(15.0)), (0, 0, 255));
        assert_eq!(canvas.pixel(px(32.0), px(8.0)), WHITE);
        assert_eq!(canvas.pixel(px(24.0), px(16.0)), WHITE);
    }
}

#[test]
fn color_filters_change_what_is_painted() {
    let canvas = Canvas::new(40, 10);
    let mut items = filtered(
        vec![Filter::Grayscale(1.0)],
        vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0), RED)],
    );
    items.extend(filtered(
        vec![Filter::Brightness(0.5)],
        vec![fill(Rect::new(10.0, 0.0, 10.0, 10.0), BLUE)],
    ));
    items.extend(filtered(
        vec![Filter::Invert(1.0), Filter::Opacity(0.5)],
        vec![fill(Rect::new(20.0, 0.0, 10.0, 10.0), RED)],
    ));
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert!(
        close(canvas.pixel(5, 5), (54, 54, 54)),
        "{:?}",
        canvas.pixel(5, 5)
    );
    assert!(
        close(canvas.pixel(15, 5), (0, 0, 128)),
        "{:?}",
        canvas.pixel(15, 5)
    );
    assert!(
        close(canvas.pixel(25, 5), (128, 255, 255)),
        "{:?}",
        canvas.pixel(25, 5)
    );
    assert_eq!(canvas.pixel(35, 5), WHITE);
}

#[test]
fn filtered_text_keeps_its_coverage() {
    let plain = Canvas::new(40, 20);
    paint(plain.dc, &[text("MW", Color::BLACK)], 1.0, (0.0, 0.0));
    let layered = Canvas::new(40, 20);
    paint(
        layered.dc,
        &filtered(
            vec![Filter::Brightness(1.0)],
            vec![text("MW", Color::BLACK)],
        ),
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(darkest(&layered), (0, 0, 0));
    // As in Chrome's filter layers, text loses its subpixel antialiasing:
    // each pixel keeps the coverage of its most covered color channel.
    for y in 0..20 {
        for x in 0..40 {
            let (r, g, b) = plain.pixel(x, y);
            let least = r.min(g).min(b);
            let actual = layered.pixel(x, y);
            assert!(
                close(actual, (least, least, least)),
                "pixel {x},{y}: {actual:?}"
            );
        }
    }
}

#[test]
fn blurs_spread_beyond_the_box_evenly() {
    let canvas = Canvas::new(50, 50);
    paint(
        canvas.dc,
        &filtered(
            vec![Filter::Blur(2.0)],
            vec![fill(Rect::new(20.0, 20.0, 10.0, 10.0), RED)],
        ),
        1.0,
        (0.0, 0.0),
    );
    let (r, g, b) = canvas.pixel(25, 25);
    assert!(r == 255 && g < 40 && b < 40, "{r},{g},{b}");
    let (left, right) = (canvas.pixel(18, 25), canvas.pixel(31, 25));
    assert!(left.0 == 255 && (40..250).contains(&left.1), "{left:?}");
    assert!(close(left, right), "{left:?} {right:?}");
    assert!(close(canvas.pixel(25, 18), left));
    assert_eq!(canvas.pixel(13, 25), WHITE);
    assert_eq!(canvas.pixel(25, 36), WHITE);
}

#[test]
fn filters_follow_transforms_scrolling_and_scaling() {
    let canvas = Canvas::new(50, 40);
    let mut items = vec![DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 5.0, 0.0])];
    items.extend(filtered(
        vec![shadow_filter(5.0, 0.0, BLUE)],
        vec![fill(Rect::new(5.0, 5.0, 5.0, 5.0), RED)],
    ));
    items.push(DisplayItem::PopTransform);
    paint(canvas.dc, &items, 2.0, (2.0, 2.0));
    // The box covers (16, 6) to (26, 16), and its shadow is 10 pixels right.
    assert_eq!(canvas.pixel(15, 10), WHITE);
    assert_eq!(canvas.pixel(16, 10), (255, 0, 0));
    assert_eq!(canvas.pixel(25, 15), (255, 0, 0));
    assert_eq!(canvas.pixel(26, 10), (0, 0, 255));
    assert_eq!(canvas.pixel(35, 6), (0, 0, 255));
    assert_eq!(canvas.pixel(36, 10), WHITE);
    assert_eq!(canvas.pixel(30, 16), WHITE);
    // A quarter turn also turns the shadow's offset, downwards.
    let turned = Canvas::new(40, 40);
    let mut items = vec![DisplayItem::PushTransform([0.0, 1.0, -1.0, 0.0, 30.0, 0.0])];
    items.extend(filtered(
        vec![shadow_filter(10.0, 0.0, BLUE)],
        vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0), RED)],
    ));
    items.push(DisplayItem::PopTransform);
    paint(turned.dc, &items, 1.0, (0.0, 0.0));
    assert!(
        close(turned.pixel(25, 5), (255, 0, 0)),
        "{:?}",
        turned.pixel(25, 5)
    );
    assert!(
        close(turned.pixel(25, 15), (0, 0, 255)),
        "{:?}",
        turned.pixel(25, 15)
    );
    assert_eq!(turned.pixel(15, 5), WHITE);
    assert_eq!(turned.pixel(25, 25), WHITE);
}

#[test]
fn filters_nest_and_fade_with_their_group() {
    let canvas = Canvas::new(40, 10);
    let mut items = filtered(
        vec![Filter::Brightness(0.5)],
        filtered(
            vec![Filter::Invert(1.0)],
            vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0), CYAN)],
        ),
    );
    items.push(DisplayItem::PushOpacity(0.5));
    items.extend(filtered(
        vec![Filter::Invert(1.0)],
        vec![fill(Rect::new(10.0, 0.0, 10.0, 10.0), CYAN)],
    ));
    items.push(DisplayItem::PopOpacity);
    // A stray end is ignored, and a group left open ends with the list.
    items.extend([
        DisplayItem::PopFilter,
        DisplayItem::PushFilter(vec![Filter::Invert(1.0)]),
        fill(Rect::new(20.0, 0.0, 10.0, 10.0), CYAN),
    ]);
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert!(
        close(canvas.pixel(5, 5), (128, 0, 0)),
        "{:?}",
        canvas.pixel(5, 5)
    );
    assert!(
        close(canvas.pixel(15, 5), (255, 128, 128)),
        "{:?}",
        canvas.pixel(15, 5)
    );
    assert_eq!(canvas.pixel(25, 5), (255, 0, 0));
    assert_eq!(canvas.pixel(35, 5), WHITE);
}

#[test]
fn filter_layers_cover_only_what_shows() {
    let canvas = Canvas::new(40, 20);
    let mut items = vec![
        DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
        DisplayItem::PushClip {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            radii: NONE,
        },
    ];
    items.extend(filtered(
        vec![Filter::Invert(1.0)],
        vec![fill(Rect::new(-10.0, 0.0, 40.0, 10.0), CYAN)],
    ));
    // Entirely clipped away.
    items.extend(filtered(
        vec![Filter::Invert(1.0)],
        vec![fill(Rect::new(0.0, 15.0, 10.0, 5.0), CYAN)],
    ));
    items.extend([DisplayItem::PopClip, DisplayItem::PopTransform]);
    // Painting nothing.
    items.extend(filtered(vec![Filter::Invert(1.0)], Vec::new()));
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert_eq!(canvas.pixel(9, 5), WHITE);
    assert_eq!(canvas.pixel(10, 5), (255, 0, 0));
    assert_eq!(canvas.pixel(19, 9), (255, 0, 0));
    assert_eq!(canvas.pixel(20, 5), WHITE);
    assert_eq!(canvas.pixel(15, 17), WHITE);
}

#[test]
fn filters_are_skipped_when_layers_cannot_be_made() {
    let canvas = Canvas::new(20, 20);
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateDIBSection,
        unsafe extern "system" fn(
            HDC,
            *const BITMAPINFO,
            u32,
            *mut *mut core::ffi::c_void,
            windows_sys::Win32::Foundation::HANDLE,
            u32,
        ) -> HBITMAP
    );
    create.expect().returning(|_, _, _, _, _, _| null_mut());
    paint(
        canvas.dc,
        &filtered(
            vec![Filter::Invert(1.0)],
            vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0), RED)],
        ),
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 5), (255, 0, 0));
}

#[test]
fn filters_fade_by_alpha_where_groups_cannot_be_isolated() {
    let canvas = Canvas::new(40, 40);
    let mut graphics = null_mut();
    // SAFETY: graphics wraps the live canvas DC and is released below.
    unsafe {
        assert_eq!(GdipCreateFromHDC(canvas.dc, &mut graphics), Ok);
    }
    let mut painter = Painter {
        graphics,
        dc: canvas.dc,
        // Without the target's pixels, groups and backdrops are unavailable.
        target: BITMAP::default(),
        scale: 1.0,
        scroll: (0.0, 0.0),
        opacity: vec![],
        // SAFETY: the empty descriptor is rejected without accessing pixels.
        groups: unsafe { opacity::Groups::new(BITMAP::default()) },
        // SAFETY: an unavailable bitmap has no pixels to blend.
        corners: RefCell::new(unsafe { corners::Coverage::new(BITMAP::default()) }),
        rounded_clips: vec![],
        states: vec![],
        transforms: vec![],
        caret: false,
        area: None,
    };
    painter.push_opacity(0.5);
    let mut items = filtered(
        vec![Filter::Invert(1.0)],
        vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0), CYAN)],
    );
    items.push(DisplayItem::Backdrop {
        rect: Rect::new(0.0, 20.0, 20.0, 20.0),
        radii: NONE,
        filters: vec![Filter::Invert(1.0)],
    });
    painter.items(&items);
    painter.pop_opacity();
    // SAFETY: graphics was created above and has finished drawing.
    unsafe {
        GdipDeleteGraphics(graphics);
    }
    assert!(
        close(canvas.pixel(5, 5), (255, 128, 128)),
        "{:?}",
        canvas.pixel(5, 5)
    );
    assert_eq!(canvas.pixel(10, 30), WHITE);
}

fn backdrop(rect: Rect, radii: Radii, filters: Vec<Filter>) -> DisplayItem {
    DisplayItem::Backdrop {
        rect,
        radii,
        filters,
    }
}

#[test]
fn backdrops_filter_what_is_behind_a_rounded_box() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
            backdrop(
                Rect::new(5.0, 5.0, 30.0, 30.0),
                [Corner::round(10.0); 4],
                vec![Filter::Invert(1.0)],
            ),
            fill(Rect::new(18.0, 18.0, 4.0, 4.0), BLUE),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(2, 2), (255, 0, 0));
    assert_eq!(
        canvas.pixel(6, 6),
        (255, 0, 0),
        "outside the rounded corner"
    );
    assert_eq!(canvas.pixel(10, 20), (0, 255, 255));
    assert_eq!(canvas.pixel(34, 20), (0, 255, 255));
    assert_eq!(canvas.pixel(36, 20), (255, 0, 0));
    assert_eq!(canvas.pixel(20, 20), (0, 0, 255), "content paints over it");
    // Transforms, scrolling and scaling place the box.
    let moved = Canvas::new(60, 30);
    paint(
        moved.dc,
        &[
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 10.0, 0.0]),
            backdrop(
                Rect::new(1.0, 1.0, 10.0, 10.0),
                NONE,
                vec![Filter::Invert(1.0)],
            ),
            DisplayItem::PopTransform,
        ],
        2.0,
        (1.0, 1.0),
    );
    assert_eq!(moved.pixel(19, 5), (255, 0, 0));
    assert_eq!(moved.pixel(20, 0), (0, 255, 255));
    assert_eq!(moved.pixel(39, 19), (0, 255, 255));
    assert_eq!(moved.pixel(40, 5), (255, 0, 0));
    assert_eq!(moved.pixel(30, 20), (255, 0, 0));
}

#[test]
fn backdrop_blurs_mirror_the_box_edges() {
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            fill(Rect::new(0.0, 0.0, 20.0, 40.0), BLUE),
            fill(Rect::new(20.0, 0.0, 20.0, 40.0), RED),
            // What lies outside the box does not bleed into it.
            backdrop(
                Rect::new(20.0, 0.0, 20.0, 20.0),
                NONE,
                vec![Filter::Blur(3.0)],
            ),
            backdrop(
                Rect::new(0.0, 20.0, 40.0, 20.0),
                NONE,
                vec![Filter::Blur(3.0)],
            ),
        ],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(20, 10), (255, 0, 0));
    assert_eq!(canvas.pixel(19, 10), (0, 0, 255));
    let (left, right) = (canvas.pixel(19, 30), canvas.pixel(20, 30));
    assert!(
        left.0 > 40 && left.2 > 40 && right.0 > 40 && right.2 > 40,
        "{left:?} {right:?}"
    );
    assert!(close(canvas.pixel(0, 39), (0, 0, 255)));
    assert!(close(canvas.pixel(39, 20), (255, 0, 0)));
}

#[test]
fn backdrops_beyond_the_target_or_the_area_change_nothing() {
    let canvas = Canvas::new(40, 40);
    paint_area(
        canvas.dc,
        &[
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED),
            backdrop(
                Rect::new(100.0, 100.0, 10.0, 10.0),
                NONE,
                vec![Filter::Invert(1.0)],
            ),
            backdrop(
                Rect::new(20.0, 20.0, 10.0, 10.0),
                NONE,
                vec![Filter::Blur(1.0)],
            ),
        ],
        1.0,
        (0.0, 0.0),
        Some(RECT {
            left: 0,
            top: 0,
            right: 5,
            bottom: 5,
        }),
        false,
    );
    assert_eq!(canvas.pixel(2, 2), (255, 0, 0));
    assert_eq!(canvas.pixel(25, 25), WHITE);
}

#[test]
fn backdrop_reach_follows_transforms_and_scale() {
    let blurred = |radius| {
        backdrop(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            NONE,
            vec![Filter::Blur(radius)],
        )
    };
    assert_eq!(
        backdrop_reach(&[fill(Rect::new(0.0, 0.0, 1.0, 1.0), RED)], 1.0),
        0
    );
    let items = [
        DisplayItem::PushTransform([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
        blurred(2.0),
        DisplayItem::PopTransform,
        blurred(1.0),
    ];
    assert_eq!(backdrop_reach(&items, 1.5), 18);
    assert_eq!(backdrop_reach(&items[2..], 1.5), 5);
}
#[test]
fn individual_dashed_and_dotted_borders_have_native_pixel_gaps_on_every_side() {
    for style in [BorderStyle::Dashed, BorderStyle::Dotted] {
        for edge in 0..4 {
            for scale in [1.0, 1.5, 2.0] {
                let canvas = Canvas::new(160, 160);
                let mut widths = [0.0; 4];
                let mut styles = [BorderStyle::None; 4];
                widths[edge] = 4.0;
                styles[edge] = style;
                let item = DisplayItem::Border {
                    rect: Rect::new(4.0, 4.0, 64.0, 64.0),
                    widths,
                    colors: [RED; 4],
                    styles,
                    radii: NONE,
                };
                paint(canvas.dc, &[item], scale, (0.0, 0.0));
                let mut colored = 0;
                let mut gaps = 0;
                for position in 6..66 {
                    let (x, y) = match edge {
                        0 => (position, 6),
                        1 => (66, position),
                        2 => (position, 66),
                        _ => (6, position),
                    };
                    let pixel = canvas.pixel((x as f32 * scale) as i32, (y as f32 * scale) as i32);
                    colored += usize::from(pixel.1 < 200);
                    gaps += usize::from(pixel == WHITE);
                }
                assert!(colored > 8, "missing {style:?} edge {edge} at {scale}");
                assert!(gaps > 8, "solid {style:?} edge {edge} at {scale}");
                assert_eq!(
                    canvas.pixel((35.0 * scale) as i32, (35.0 * scale) as i32),
                    WHITE
                );
            }
        }
    }
}

#[test]
fn individual_patterned_borders_keep_clipping_and_opacity() {
    let canvas = Canvas::new(80, 30);
    let items = [
        DisplayItem::PushClip {
            rect: Rect::new(16.0, 0.0, 48.0, 30.0),
            radii: NONE,
        },
        DisplayItem::PushOpacity(0.5),
        DisplayItem::Border {
            rect: Rect::new(4.0, 4.0, 64.0, 20.0),
            widths: [4.0, 0.0, 0.0, 0.0],
            colors: [RED; 4],
            styles: [
                BorderStyle::Dashed,
                BorderStyle::None,
                BorderStyle::None,
                BorderStyle::None,
            ],
            radii: NONE,
        },
        DisplayItem::PopOpacity,
        DisplayItem::PopClip,
    ];
    paint(canvas.dc, &items, 1.0, (0.0, 0.0));
    assert!((4..16).all(|x| canvas.pixel(x, 6) == WHITE));
    let pixels: Vec<_> = (16..64).map(|x| canvas.pixel(x, 6)).collect();
    assert!(
        pixels
            .iter()
            .any(|p| p.0 == 255 && (120..=135).contains(&p.1))
    );
    assert!(pixels.contains(&WHITE));
    assert_eq!(canvas.pixel(70, 6), WHITE);
}

#[test]
fn individual_dotted_borders_are_round_and_clip_the_last_dot() {
    for length in [2.0, 39.0] {
        let canvas = Canvas::new(60, 30);
        paint(
            canvas.dc,
            &[
                DisplayItem::Border {
                    rect: Rect::new(4.0, 4.0, length, 20.0),
                    widths: [8.0, 0.0, 0.0, 0.0],
                    colors: [RED; 4],
                    styles: [
                        BorderStyle::Dotted,
                        BorderStyle::None,
                        BorderStyle::None,
                        BorderStyle::None,
                    ],
                    radii: NONE,
                },
                fill(Rect::new(50.0, 0.0, 10.0, 10.0), BLUE),
            ],
            1.0,
            (0.0, 0.0),
        );
        assert_eq!(canvas.pixel(4, 4), WHITE, "square dot corner");
        assert!(canvas.pixel(5, 8).1 < 200, "missing circular dot");
        assert_eq!(
            canvas.pixel((4.0 + length) as i32, 8),
            WHITE,
            "dot escapes border side"
        );
        assert_eq!(canvas.pixel(5, 13), WHITE);
        if length > 8.0 {
            assert_eq!(canvas.pixel(8, 8), (255, 0, 0));
            assert_eq!(canvas.pixel(16, 8), WHITE);
        }
        // The temporary side clip must not affect later display items.
        assert_eq!(canvas.pixel(55, 5), (0, 0, 255));
    }
}

#[test]
fn individual_dotted_border_work_is_bounded_by_the_visible_clip() {
    use std::sync::{Arc, Mutex};
    let canvas = Canvas::new(80, 80);
    let offsets = Arc::new(Mutex::new(Vec::new()));
    let values = offsets.clone();
    let mut session = Session::new();
    let ellipse = mock!(
        session,
        GdipFillEllipse,
        unsafe extern "system" fn(*mut GpGraphics, *mut GpBrush, f32, f32, f32, f32) -> Status
    );
    ellipse.expect().returning(move |_, _, x, y, _, _| {
        values.lock().unwrap().push((x, y));
        Ok
    });
    for edge in [0, 3] {
        let mut widths = [0.0; 4];
        let mut styles = [BorderStyle::None; 4];
        widths[edge] = 4.0;
        styles[edge] = BorderStyle::Dotted;
        paint(
            canvas.dc,
            &[
                DisplayItem::PushClip {
                    rect: Rect::new(20.0, 20.0, 40.0, 40.0),
                    radii: NONE,
                },
                DisplayItem::Border {
                    rect: Rect::new(20.0, 20.0, 1_000_000.0, 1_000_000.0),
                    widths,
                    colors: [RED; 4],
                    styles,
                    radii: NONE,
                },
                DisplayItem::PopClip,
            ],
            1.0,
            (0.0, 0.0),
        );
        let calls: Vec<_> = offsets.lock().unwrap().drain(..).collect();
        // GDI+ includes the antialiasing edge in visible clip bounds.
        assert!(
            (5..=6).contains(&calls.len()),
            "generated offscreen dots: {}",
            calls.len()
        );
        for (index, (x, y)) in calls.into_iter().enumerate() {
            let expected = 20.0 + index as f32 * 8.0;
            assert_eq!(
                (x, y),
                if edge == 0 {
                    (expected, 20.0)
                } else {
                    (20.0, expected)
                }
            );
        }
    }
}

#[test]
fn individual_dotted_borders_preserve_scroll_phase_and_clip_query_failure_fallback() {
    let canvas = Canvas::new(80, 30);
    let item = DisplayItem::Border {
        rect: Rect::new(-4.0, 4.0, 60.0, 20.0),
        widths: [8.0, 0.0, 0.0, 0.0],
        colors: [RED; 4],
        styles: [
            BorderStyle::Dotted,
            BorderStyle::None,
            BorderStyle::None,
            BorderStyle::None,
        ],
        radii: NONE,
    };
    paint(canvas.dc, std::slice::from_ref(&item), 1.0, (8.0, 0.0));
    assert_eq!(canvas.pixel(8, 8), (255, 0, 0));
    assert_eq!(canvas.pixel(16, 8), WHITE);
    let mut session = Session::new();
    let bounds = mock!(
        session,
        GdipGetVisibleClipBounds,
        unsafe extern "system" fn(*mut GpGraphics, *mut RectF) -> Status
    );
    bounds.expect().returning(|_, _| GenericError);
    let fallback = Canvas::new(80, 30);
    paint(fallback.dc, &[item], 1.0, (8.0, 0.0));
    for x in 0..80 {
        assert_eq!(fallback.pixel(x, 8), canvas.pixel(x, 8));
    }
}

#[test]
fn individual_patterned_borders_keep_rotated_geometry() {
    for style in [BorderStyle::Dashed, BorderStyle::Dotted] {
        let canvas = Canvas::new(80, 80);
        paint(
            canvas.dc,
            &[
                DisplayItem::PushTransform([0.0, 1.0, -1.0, 0.0, 72.0, 0.0]),
                DisplayItem::Border {
                    rect: Rect::new(4.0, 4.0, 64.0, 64.0),
                    widths: [4.0, 0.0, 0.0, 0.0],
                    colors: [RED; 4],
                    styles: [
                        style,
                        BorderStyle::None,
                        BorderStyle::None,
                        BorderStyle::None,
                    ],
                    radii: NONE,
                },
                DisplayItem::PopTransform,
            ],
            1.0,
            (0.0, 0.0),
        );
        let pixels: Vec<_> = (6..66).map(|y| canvas.pixel(66, y)).collect();
        assert!(
            pixels.iter().filter(|p| p.1 < 200).count() > 8,
            "missing rotated {style:?}"
        );
        assert!(
            pixels.iter().filter(|p| **p == WHITE).count() > 8,
            "solid rotated {style:?}"
        );
        assert_eq!(canvas.pixel(50, 30), WHITE);
    }
}

#[test]
fn drop_shadows_of_clipped_away_images_show_inside_the_clip() {
    add_image("icon.bmp", &bitmap_file()).unwrap();
    let canvas = Canvas::new(40, 40);
    paint(
        canvas.dc,
        &[
            DisplayItem::PushClip {
                rect: Rect::new(0.0, 0.0, 16.0, 16.0),
                radii: [Corner::round(0.0); 4],
            },
            DisplayItem::PushFilter(vec![nativeboat_css::types::Filter::DropShadow {
                x: 16.0,
                y: 0.0,
                blur: 0.0,
                color: Color::rgb(255, 0, 0),
            }]),
            DisplayItem::Image {
                rect: Rect::new(-16.0, 0.0, 16.0, 16.0),
                url: "icon.bmp".to_owned(),
            },
            DisplayItem::PopFilter,
            DisplayItem::PopClip,
        ],
        1.0,
        (0.0, 0.0),
    );
    // The image itself is clipped away; its shadow, moved into the clip, shows.
    assert_eq!(canvas.pixel(8, 8), (255, 0, 0));
    assert_eq!(canvas.pixel(20, 8), WHITE);
}

#[test]
fn translucent_groups_in_an_area_blend_only_its_pixels() {
    let area = RECT {
        left: 5,
        top: 5,
        right: 10,
        bottom: 10,
    };
    let items = [
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), RED),
        DisplayItem::PushOpacity(0.5),
        fill(Rect::new(0.0, 0.0, 20.0, 20.0), BLUE),
        DisplayItem::PopOpacity,
    ];
    let whole = Canvas::new(20, 20);
    paint(whole.dc, &items, 1.0, (0.0, 0.0));
    let part = Canvas::new(20, 20);
    paint_area(part.dc, &items, 1.0, (0.0, 0.0), Some(area), false);
    for y in 0..20 {
        for x in 0..20 {
            let inside = (5..10).contains(&x) && (5..10).contains(&y);
            let expected = if inside { whole.pixel(x, y) } else { WHITE };
            assert_eq!(part.pixel(x, y), expected, "pixel {x},{y}");
        }
    }
}

#[test]
fn rounded_corners_that_snapping_leaves_too_round_shrink_to_fit() {
    use nativeboat_css::types::BorderStyle;
    // An 18 pixel circle at 15 is 18.75 to 41.25 device pixels at 125%,
    // which snapping makes 22 wide: too narrow for two 11.25 pixel arcs.
    let gray = Color::rgb(128, 128, 128);
    let rect = Rect::new(15.0, 14.0, 18.0, 18.0);
    let items = [
        DisplayItem::Fill {
            rect,
            color: BLUE,
            radii: [Corner::round(9.0); 4],
        },
        DisplayItem::Border {
            rect,
            widths: [2.0; 4],
            colors: [gray; 4],
            styles: [BorderStyle::Solid; 4],
            radii: [Corner::round(9.0); 4],
        },
    ];
    let canvas = Canvas::new(50, 50);
    paint(canvas.dc, &items, 1.25, (0.0, 0.0));
    // The corners of the box stay clear, where overlapping arcs were joined
    // by lines across it.
    for (x, y) in [(20, 19), (39, 19), (20, 38), (39, 38)] {
        assert_eq!(canvas.pixel(x, y), WHITE, "({x}, {y})");
    }
    // The circle still shows its fill in the middle and its ring at the
    // sides.
    assert_eq!(canvas.pixel(30, 29), (0, 0, 255));
    assert_ne!(canvas.pixel(30, 18), WHITE);
}

#[test]
fn rounded_borders_whose_sides_differ_keep_their_corners() {
    use nativeboat_css::types::BorderStyle;
    let rect = Rect::new(5.0, 5.0, 40.0, 30.0);
    let border = |widths: [f32; 4], colors: [Color; 4]| DisplayItem::Border {
        rect,
        widths,
        colors,
        styles: [BorderStyle::Solid; 4],
        radii: [Corner::round(10.0); 4],
    };
    // A bottom border alone curves up into its corners and tapers away,
    // where it was a straight bar across them.
    let canvas = Canvas::new(50, 40);
    paint(
        canvas.dc,
        &[border([0.0, 0.0, 2.0, 0.0], [BLUE; 4])],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 34), WHITE);
    assert_eq!(canvas.pixel(44, 34), WHITE);
    assert_eq!(canvas.pixel(25, 34), (0, 0, 255));
    assert_ne!(canvas.pixel(8, 31), WHITE);
    assert_eq!(canvas.pixel(25, 6), WHITE);
    // Sides of different colors meet on the line from each outer corner
    // toward its inner corner.
    let canvas = Canvas::new(50, 40);
    paint(
        canvas.dc,
        &[border([3.0; 4], [RED, BLUE, BLUE, BLUE])],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(5, 5), WHITE);
    assert_eq!(canvas.pixel(12, 6), (255, 0, 0));
    assert_eq!(canvas.pixel(6, 12), (0, 0, 255));
    assert_eq!(canvas.pixel(25, 6), (255, 0, 0));
    assert_eq!(canvas.pixel(6, 20), (0, 0, 255));
    assert_eq!(canvas.pixel(25, 20), WHITE);
    // Sides as wide as the corners leave square inner corners, which the
    // split lines reach.
    let canvas = Canvas::new(50, 40);
    paint(
        canvas.dc,
        &[border([12.0, 12.0, 12.0, 12.0], [RED, BLUE, BLUE, BLUE])],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(25, 8), (255, 0, 0));
    assert_eq!(canvas.pixel(8, 20), (0, 0, 255));
    assert_eq!(canvas.pixel(16, 15), (255, 0, 0));
    assert_eq!(canvas.pixel(15, 16), (0, 0, 255));
    // Sides without width draw nothing.
    let canvas = Canvas::new(50, 40);
    paint(
        canvas.dc,
        &[border([0.0; 4], [RED, BLUE, BLUE, BLUE])],
        1.0,
        (0.0, 0.0),
    );
    assert_eq!(canvas.pixel(25, 5), WHITE);
}

#[test]
fn underlines_at_an_offset_sit_that_far_below_the_baseline() {
    // The lowest dark row: the underline's bottom, below the glyphs.
    let lowest = |offset: Option<f32>| {
        let canvas = Canvas::new(40, 40);
        let item = match text("mn", Color::BLACK) {
            DisplayItem::Text {
                rect,
                text,
                font,
                color,
                underline,
                line_through,
                letter_spacing,
                rtl,
                grayscale,
                ..
            } => DisplayItem::Text {
                rect,
                text,
                font,
                color,
                underline,
                line_through,
                letter_spacing,
                rtl,
                grayscale,
                underline_offset: offset,
                decorations: Vec::new(),
            },
            other => other,
        };
        paint(canvas.dc, &[item], 1.0, (0.0, 0.0));
        (0..40)
            .filter(|y| (0..40).any(|x| canvas.pixel(x, *y) == (0, 0, 0)))
            .max()
            .unwrap()
    };
    let near = lowest(Some(2.0));
    let far = lowest(Some(8.0));
    assert_eq!(far - near, 6);
    // Without an offset GDI underlines at the font's position.
    let font = lowest(None);
    assert!(font < far, "{font} {far}");
}

#[test]
fn decorations_apart_from_the_font_take_their_own_color_style_and_place() {
    use nativeboat_css::types::TextDecorationStyle as Style;
    use nativeboat_ir::{DecorationLine as Line, TextDecoration};
    let red = |(r, g, b): (u8, u8, u8)| r > 150 && g < 110 && b < 110;
    // The rows with red, with the red pixels of the first, and the rows
    // with glyph ink.
    let drawn = |line: Line, style: Style, thickness: Option<f32>, under: bool| {
        let canvas = Canvas::new(40, 40);
        let item = match text("mn", Color::BLACK) {
            DisplayItem::Text {
                rect,
                text,
                font,
                color,
                letter_spacing,
                rtl,
                grayscale,
                ..
            } => DisplayItem::Text {
                // Room above for the overline.
                rect: Rect::new(rect.x, 10.0, rect.width, rect.height),
                text,
                font,
                color,
                underline: false,
                line_through: false,
                letter_spacing,
                rtl,
                grayscale,
                underline_offset: None,
                decorations: vec![TextDecoration {
                    line,
                    color: RED,
                    style,
                    thickness,
                    offset: None,
                    under,
                }],
            },
            other => other,
        };
        paint(canvas.dc, &[item], 1.0, (0.0, 0.0));
        let rows: Vec<i32> = (0..40)
            .filter(|y| (0..40).any(|x| red(canvas.pixel(x, *y))))
            .collect();
        let first: Vec<i32> = rows.first().map_or_else(Vec::new, |row| {
            (0..40).filter(|x| red(canvas.pixel(*x, *row))).collect()
        });
        let ink: Vec<i32> = (0..40)
            .filter(|y| (0..40).any(|x| canvas.pixel(x, *y) == (0, 0, 0)))
            .collect();
        (rows, first, ink)
    };
    let (rows, first, ink) = drawn(Line::Underline, Style::Solid, Some(2.0), false);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[1], rows[0] + 1);
    assert_eq!(first.len(), 40);
    // Below the descent, under the font's own place.
    let (under, _, _) = drawn(Line::Underline, Style::Solid, Some(2.0), true);
    assert!(under[0] > rows[0], "{under:?} {rows:?}");
    // An overline sits above the glyphs and a line-through across them.
    let (over, _, _) = drawn(Line::Overline, Style::Solid, None, false);
    assert!(over[0] < ink[0], "{over:?} {ink:?}");
    let (through, _, ink) = drawn(Line::LineThrough, Style::Solid, None, false);
    assert!(through[0] > ink[0] && through[0] < *ink.last().unwrap());
    // Dots leave gaps as long as themselves; a double line two rows.
    let (_, dots, _) = drawn(Line::Underline, Style::Dotted, Some(2.0), false);
    assert_eq!(&dots[..4], &[0, 1, 4, 5]);
    let (double, _, _) = drawn(Line::Underline, Style::Double, Some(1.0), false);
    assert_eq!(double.len(), 2);
    assert_eq!(double[1], double[0] + 2);
    // Antialiased waves reach a thickness and a pixel either side of
    // their middle.
    let (wave, _, _) = drawn(Line::Underline, Style::Wavy, Some(1.0), true);
    assert!(wave.len() >= 4, "{wave:?}");
}

#[test]
fn thin_strokes_keep_their_weight() {
    use nativeboat_css::types::{FillRule, LineCap, LineJoin};
    use nativeboat_ir::PathSegment::{Line, Move};
    // A 14 pixel icon's 2 unit stroke at 125% is 1.46 device pixels wide,
    // which GDI+ pens drew 1 pixel wide.
    let k = 14.0 / 24.0;
    let path = |segments| DisplayItem::Path {
        segments,
        transform: [k, 0.0, 0.0, k, 40.3, 8.3],
        fill: None,
        fill_rule: FillRule::EvenOdd,
        stroke: Some(Stroke {
            color: Color::BLACK,
            width: 2.0,
            cap: LineCap::Round,
            join: LineJoin::Round,
            dashes: vec![],
            dash_offset: 0.0,
        }),
    };
    let canvas = Canvas::new(125, 50);
    paint(
        canvas.dc,
        &[path(vec![
            Move(2.0, 12.0),
            Line(22.0, 12.0),
            Move(12.0, 2.0),
            Line(12.0, 22.0),
        ])],
        1.25,
        (0.0, 0.0),
    );
    let ink = |x: i32| -> f32 {
        (0..50)
            .map(|y| f32::from(255 - canvas.pixel(x, y).0) / 255.0)
            .sum()
    };
    let width = ink(53);
    assert!((1.3..1.65).contains(&width), "{width}");
    // Crossing strokes stay solid where they overlap, even under the
    // even-odd rule, which applies to fills.
    let crossing = canvas.pixel(59, 19).0;
    assert!(crossing < 64, "{crossing}");
    // A path with nothing to stroke draws nothing.
    let canvas = Canvas::new(125, 50);
    paint(canvas.dc, &[path(vec![Move(2.0, 12.0)])], 1.25, (0.0, 0.0));
    assert_eq!(canvas.pixel(53, 18), WHITE);
}
