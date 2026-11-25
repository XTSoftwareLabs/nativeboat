use super::*;
use crate::tests::support::{Canvas, pack, record, send, window};
use nativeboat_css::types::{BorderStyle, LengthPercentageAuto, Radius, Size};
use nativeboat_css::{Color, LengthPercentage};
use nativeboat_ir::{Corner, ScrollbarAxis, ScrollbarPaint, ScrollbarPart};
use shimforge::{Session, mock};

fn paint(color: Color) -> ScrollbarPaint {
    ScrollbarPaint {
        visible: true,
        background: color,
        widths: [0.0; 4],
        colors: [Color::BLACK; 4],
        borders: [BorderStyle::None; 4],
        radii: std::array::from_fn(|_| Radius::default()),
        margins: std::array::from_fn(|_| LengthPercentageAuto::zero()),
    }
}

fn theme() -> Scrollbars {
    let background = ScrollbarPart(std::array::from_fn(|_| paint(Color::rgb(230, 230, 230))));
    Scrollbars {
        axes: std::array::from_fn(|_| ScrollbarAxis {
            enabled: true,
            force: false,
            thickness: Size::Length(LengthPercentage::Px(12.0)),
            bar: background.clone(),
            track: ScrollbarPart(std::array::from_fn(|_| paint(Color::TRANSPARENT))),
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

fn fixture(content: (f32, f32)) -> WindowId {
    let id = window();
    apply(
        id,
        &[
            Change::UpdateSurface {
                id: 0,
                rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                content_width: content.0,
                content_height: content.1,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            Change::Scrollbars {
                surface: 0,
                style: Some(Box::new(theme())),
            },
        ],
    );
    id
}

fn snapshot(id: WindowId) -> Geometry {
    STATE.with_borrow(|state| {
        let data = &state.windows[&id];
        geometry(&data.surfaces[&0], data.scale, data.zoom)
    })
}

fn mouse(id: WindowId, message: u32, flags: usize, x: f32, y: f32) {
    send(
        hwnd(id).unwrap(),
        message,
        flags,
        pack(px(x, scale(id)), px(y, scale(id))),
    );
}

fn draw(id: WindowId) -> Canvas {
    let canvas = Canvas::new(px(300.0, scale(id)), px(200.0, scale(id)));
    render_surface(id, 0, canvas.dc, client(hwnd(id).unwrap()));
    canvas
}

#[test]
fn thumbs_paint_hover_and_active_and_drag_without_editor_pointer_events() {
    let id = fixture((900.0, 1000.0));
    let root = hwnd(id).unwrap();
    // SAFETY: inspects the test's own live window.
    let window_style = unsafe { GetWindowLongW(root, GWL_STYLE) } as u32;
    assert_eq!(window_style & (WS_HSCROLL | WS_VSCROLL), 0);
    let events = record();
    let pixel = |canvas: &Canvas| canvas.pixel(px(295.0, scale(id)), px(10.0, scale(id)));
    assert_eq!(pixel(&draw(id)), (255, 0, 0));
    mouse(id, WM_MOUSEMOVE, 0, 295.0, 10.0);
    assert!(events.take().is_empty());
    assert!(cursor(id, 0));
    assert_eq!(pixel(&draw(id)), (0, 128, 0));
    send(root, WM_SETCURSOR, 0, HTCLIENT as isize);
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, 10.0);
    assert_eq!(pixel(&draw(id)), (0, 0, 255));
    send(root, WM_MOUSELEAVE, 0, 0);
    events.take();
    assert!(cursor(id, 0));
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 295.0, 100.0);
    let scroll = events.take();
    assert!(matches!(scroll.as_slice(),[Event::Scroll { surface:0,x:0.0,y }] if *y>400.0));
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 295.0, 500.0);
    assert!(matches!(
        events.take().as_slice(),
        [Event::Scroll { y: 800.0, .. }]
    ));
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 295.0, -100.0);
    assert!(matches!(
        events.take().as_slice(),
        [Event::Scroll { y: 0.0, .. }]
    ));
    mouse(id, WM_LBUTTONUP, 0, 295.0, 10.0);
    assert!(events.take().is_empty());
    mouse(id, WM_LBUTTONDOWN, 0, 20.0, 195.0);
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 110.0, 195.0);
    assert!(matches!(events.take().as_slice(),[Event::Scroll { x,y:0.0,.. }] if *x>0.0));
    mouse(id, WM_LBUTTONUP, 0, 110.0, 195.0);
    mouse(id, WM_MOUSEMOVE, 0, 20.0, 20.0);
    assert!(matches!(
        events.take().as_slice(),
        [Event::Pointer {
            kind: PointerKind::Move,
            ..
        }]
    ));
    assert!(!cursor(id, 0));
    send(root, WM_MOUSELEAVE, 0, 0);
    send(root, WM_MOUSELEAVE, 0, 0);
    destroy(id);
}

#[test]
fn track_clicks_page_both_axes_and_capture_cancellation_stops_dragging() {
    let id = fixture((900.0, 1000.0));
    let root = hwnd(id).unwrap();
    let events = record();
    // Seven eighths of the 288 by 188 pixels the bars leave, a page.
    for (x, y, expected) in [
        (295.0, 150.0, (0.0, 164.0)),
        (230.0, 195.0, (252.0, 164.0)),
        (10.0, 195.0, (0.0, 164.0)),
        (295.0, 5.0, (0.0, 0.0)),
    ] {
        mouse(id, WM_MOUSEMOVE, 0, x, y);
        mouse(id, WM_LBUTTONDOWN, 0, x, y);
        draw(id);
        mouse(id, WM_LBUTTONUP, 0, x, y);
        assert!(matches!(events.take().as_slice(),[Event::Scroll { x,y,.. }] if (*x,*y)==expected));
    }
    mouse(id, WM_MOUSEMOVE, 0, 295.0, 195.0);
    draw(id);
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, 195.0);
    draw(id);
    mouse(id, WM_LBUTTONUP, 0, 295.0, 195.0);
    assert!(events.take().is_empty());
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, 10.0);
    send(root, WM_CANCELMODE, 0, 0);
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 295.0, 100.0);
    assert!(
        events
            .take()
            .iter()
            .any(|event| matches!(event, Event::Pointer { .. }))
    );
    mouse(id, WM_LBUTTONUP, 0, 295.0, 100.0);
    events.take();
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, 10.0);
    send(root, WM_CAPTURECHANGED, 0, 0);
    assert!(STATE.with_borrow(|state| state.windows[&id].surfaces[&0].scrollbar.drag.is_none()));
    set(id, 0, None);
    assert!(!cursor(id, 0));
    mouse(id, WM_MOUSEMOVE, 0, 295.0, 100.0);
    assert!(!events.take().is_empty());
    assert!(STATE.with_borrow(|state| {
        items(&state.windows[&id].surfaces[&0], scale(id), zoom(id)).is_empty()
    }));
    destroy(id);
}

#[test]
fn fitting_forced_tracks_hidden_widths_and_unknown_surfaces_are_safe() {
    let id = fixture((300.0, 200.0));
    let mut style = theme();
    style.axes[0].force = true;
    style.axes[1].force = true;
    set(id, 0, Some(Box::new(style.clone())));
    let events = record();
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, 10.0);
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 295.0, 100.0);
    mouse(id, WM_LBUTTONUP, 0, 295.0, 100.0);
    assert!(events.take().is_empty());
    assert!(!pointer(id, 0, WM_RBUTTONDOWN, 0, pack(10, 10)));
    for axis in &mut style.axes {
        axis.thickness = Size::Length(LengthPercentage::Px(0.0));
    }
    set(id, 0, Some(Box::new(style)));
    assert!(snapshot(id).bars.iter().all(Option::is_none));
    draw(id);
    assert!(!pointer(u64::MAX, 0, WM_MOUSEMOVE, 0, 0));
    assert!(!pointer(id, u64::MAX, WM_MOUSEMOVE, 0, 0));
    for unknown in [(u64::MAX, 0), (id, u64::MAX)] {
        set(unknown.0, unknown.1, None);
        cancel(unknown.0, unknown.1, true);
        leave(unknown.0, unknown.1);
        assert!(!cursor(unknown.0, unknown.1));
    }
    destroy(id);
}

#[test]
fn scrollbar_metrics_and_capture_use_mockable_platform_calls() {
    let id = fixture((900.0, 1000.0));
    let mut style = theme();
    style.axes[1].thickness = Size::Auto;
    set(id, 0, Some(Box::new(style)));
    let mut session = Session::new();
    let metric = mock!(
        session,
        GetSystemMetricsForDpi,
        unsafe extern "system" fn(SYSTEM_METRICS_INDEX, u32) -> i32
    );
    let expected = px(20.0, scale(id));
    let dpi = px(96.0, scale(id)) as u32;
    metric.expect().returning(move |index, actual_dpi| {
        assert_eq!(index, SM_CXVSCROLL);
        assert_eq!(actual_dpi, dpi);
        expected
    });
    assert_eq!(snapshot(id).bars[1].unwrap().track.width, 20.0);
    session.restore();
    destroy(id);
}

#[test]
fn hidden_scrollbars_keep_wheel_scrolling_and_hidden_thumbs_do_not_capture_drags() {
    let _keys = crate::tests::support::KeyboardState::neutral();
    let id = fixture((900.0, 1000.0));
    let events = record();
    let mut style = theme();
    for axis in &mut style.axes {
        axis.thickness = Size::Length(LengthPercentage::Px(0.0));
    }
    set(id, 0, Some(Box::new(style)));
    send(
        hwnd(id).unwrap(),
        WM_MOUSEWHEEL,
        usize::from(-120_i16 as u16) << 16,
        0,
    );
    assert!(
        events
            .take()
            .iter()
            .any(|event| matches!(event, Event::Scroll { y: 100.0, .. }))
    );
    let mut style = theme();
    for paint in &mut style.axes[1].thumb.0 {
        paint.visible = false;
    }
    set(id, 0, Some(Box::new(style)));
    let bar = snapshot(id).bars[1].unwrap();
    mouse(id, WM_LBUTTONDOWN, 0, 295.0, bar.thumb.y + 5.0);
    assert!(STATE.with_borrow(|state| state.windows[&id].surfaces[&0].scrollbar.drag.is_none()));
    mouse(id, WM_LBUTTONUP, 0, 295.0, bar.thumb.y + 5.0);
    destroy(id);
}

#[test]
fn replacing_a_hovered_style_preserves_the_pages_pointer_capture() {
    let id = fixture((900.0, 1000.0));
    let root = hwnd(id).unwrap();
    // SAFETY: captures only this test's own window on its thread.
    unsafe {
        SetCapture(root);
    }
    assert!(!pointer(
        id,
        0,
        WM_MOUSEMOVE,
        MK_LBUTTON as usize,
        pack(px(295.0, scale(id)), px(10.0, scale(id)))
    ));
    set(id, 0, None);
    // SAFETY: reads this thread's capture, then releases the test's capture.
    unsafe {
        assert_eq!(GetCapture(), root);
        ReleaseCapture();
    }
    destroy(id);
}

#[test]
fn overlays_show_the_scrollbars_beneath_them() {
    // A translucent overlay over a scrolling surface dims its thumb with
    // the rest of what lies beneath it.
    let id = fixture((300.0, 800.0));
    apply(
        id,
        &[
            Change::AddSurface { id: 2, parent: 0 },
            Change::UpdateSurface {
                id: 2,
                rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                content_width: 300.0,
                content_height: 200.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            Change::Paint {
                surface: 0,
                items: vec![DisplayItem::Child(nativeboat_ir::NativeChild::Surface(2))],
            },
            Change::Paint {
                surface: 2,
                items: vec![
                    DisplayItem::Beneath,
                    DisplayItem::Fill {
                        rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                        color: Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0.5,
                        },
                        radii: [Corner::SQUARE; 4],
                    },
                ],
            },
        ],
    );
    let scale = scale(id);
    let canvas = Canvas::new(px(300.0, scale), px(200.0, scale));
    render_surface(id, 2, canvas.dc, client(hwnd(id).unwrap()));
    let (r, g, b) = canvas.pixel(px(294.0, scale), px(10.0, scale));
    assert!(
        (110..150).contains(&r) && g < 20 && b < 20,
        "{:?}",
        (r, g, b)
    );
    destroy(id);
}

#[test]
fn surfaces_parked_outside_the_window_paint_nothing() {
    // A hidden popup a page moved far outside the window renders neither
    // itself nor what lies beneath it until it moves in.
    let id = fixture((300.0, 200.0));
    let fill = DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, 50.0, 20.0),
        color: Color::rgb(255, 0, 0),
        radii: [Corner::SQUARE; 4],
    };
    let place = |x: f32, y: f32| Change::UpdateSurface {
        id: 2,
        rect: Rect::new(x, y, 50.0, 20.0),
        content_width: 50.0,
        content_height: 20.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
    apply(
        id,
        &[
            Change::AddSurface { id: 2, parent: 0 },
            place(10000.0, -9999.0),
            Change::Paint {
                surface: 0,
                items: vec![DisplayItem::Child(nativeboat_ir::NativeChild::Surface(2))],
            },
            Change::Paint {
                surface: 2,
                items: vec![DisplayItem::Beneath, fill],
            },
        ],
    );
    let scale = scale(id);
    let area = RECT {
        left: 0,
        top: 0,
        right: px(50.0, scale),
        bottom: px(20.0, scale),
    };
    let canvas = Canvas::new(area.right, area.bottom);
    render_surface(id, 2, canvas.dc, area);
    assert_ne!(canvas.pixel(2, 2), (255, 0, 0));
    apply(id, &[place(10.0, 10.0)]);
    let shown = Canvas::new(area.right, area.bottom);
    render_surface(id, 2, shown.dc, area);
    assert_eq!(shown.pixel(2, 2), (255, 0, 0));
    destroy(id);
}

#[test]
fn unstyled_scrollbars_are_the_browsers_with_arrow_buttons() {
    use nativeboat_ir::scrollbar::Look;
    let id = fixture((900.0, 1000.0));
    let style = Scrollbars::platform(Look::Fluent, [(true, false); 2]);
    set(id, 0, Some(Box::new(style)));
    let events = record();
    let pixel = |canvas: &Canvas, x: f32, y: f32| canvas.pixel(px(x, scale(id)), px(y, scale(id)));
    // A light track, and a gray thumb 9 wide in its middle, below the up
    // arrow's button.
    let canvas = draw(id);
    assert_eq!(pixel(&canvas, 299.0, 100.0), (252, 252, 252));
    assert_eq!(pixel(&canvas, 292.5, 30.0), (139, 139, 139));
    mouse(id, WM_MOUSEMOVE, 0, 292.5, 30.0);
    assert_eq!(pixel(&draw(id), 292.5, 30.0), (99, 99, 99));
    // The arrow buttons scroll a step.
    let bottom = snapshot(id).bars[1].unwrap().whole.height;
    for (y, expected) in [(bottom - 9.0, 40.0), (bottom - 9.0, 80.0), (9.0, 40.0)] {
        mouse(id, WM_LBUTTONDOWN, 0, 292.5, y);
        mouse(id, WM_LBUTTONUP, 0, 292.5, y);
        assert!(
            matches!(events.take().as_slice(), [Event::Scroll { y, .. }] if *y == expected),
            "{expected}"
        );
    }
    // The horizontal bar's ends: the left arrow cannot scroll further.
    let right = snapshot(id).bars[0].unwrap().whole.width - 9.0;
    for (x, expected) in [(9.0, None), (right, Some(40.0))] {
        mouse(id, WM_LBUTTONDOWN, 0, x, 195.0);
        mouse(id, WM_LBUTTONUP, 0, x, 195.0);
        let scrolled = match events.take().as_slice() {
            [Event::Scroll { x, .. }] => Some(*x),
            _ => None,
        };
        assert_eq!(scrolled, expected);
    }
    destroy(id);
}

#[test]
fn the_windows_scrollbars_take_room_from_the_viewport() {
    let id = fixture((300.0, 200.0));
    let events = record();
    assert_eq!(scrollbar_room(id), (0.0, 0.0));
    // The room after each update that tells the page of it, as a resize of
    // the unchanged client area.
    let update = |content: (f32, f32)| {
        apply(
            id,
            &[Change::UpdateSurface {
                id: 0,
                rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                content_width: content.0,
                content_height: content.1,
                scroll_x: 0.0,
                scroll_y: 0.0,
            }],
        );
        events
            .take()
            .into_iter()
            .filter_map(|event| match event {
                Event::Resized { width, height } => {
                    assert_eq!((width, height), client_size(id));
                    Some(scrollbar_room(id))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // Taller content: the vertical bar's room goes, and the page hears.
    assert_eq!(update((288.0, 1000.0)), [(0.0, 12.0)]);
    assert_eq!(scrollbar_room(id), (0.0, 12.0));
    // Content wider than what the vertical bar leaves takes both; the same
    // room again tells the page nothing.
    assert_eq!(update((900.0, 1000.0)), [(12.0, 12.0)]);
    assert!(update((900.0, 195.0)).is_empty());
    // Shorter content gives the vertical bar's room back, and the content
    // scrolls in what the bars leave.
    assert_eq!(update((900.0, 150.0)), [(12.0, 0.0)]);
    let bar = snapshot(id).bars[0].unwrap();
    assert_eq!(bar.maximum, 600.0);
    assert!(snapshot(id).bars[1].is_none());
    assert!(update((900.0, 150.0)).is_empty());
    assert_eq!(update((300.0, 180.0)), [(0.0, 0.0)]);
    assert!(update((300.0, 180.0)).is_empty());
    // Each bar is judged against what the bars there leave: wider content
    // takes the horizontal bar's room, and then, laid out in what that
    // leaves, the vertical bar's too.
    assert_eq!(update((900.0, 195.0)), [(12.0, 0.0)]);
    assert_eq!(update((900.0, 195.0)), [(12.0, 12.0)]);
    // Content that fits only without a bar keeps it through the layout the
    // bar caused, as in Chromium, rather than toggling it; a later layout
    // where it fits gives the room back.
    assert!(update((288.0, 180.0)).is_empty());
    assert_eq!(update((288.0, 180.0)), [(0.0, 0.0)]);
    assert_eq!(update((300.0, 250.0)), [(0.0, 12.0)]);
    assert!(update((288.0, 190.0)).is_empty());
    assert_eq!(scrollbar_room(id), (0.0, 12.0));
    assert_eq!(update((288.0, 190.0)), [(0.0, 0.0)]);
    set(id, 0, None);
    assert_eq!(scrollbar_room(id), (0.0, 0.0));
    destroy(id);
}

#[test]
fn held_arrow_buttons_and_tracks_scroll_again_on_their_timer() {
    use nativeboat_ir::scrollbar::{Look, page_step};
    let id = fixture((900.0, 1000.0));
    let style = Scrollbars::platform(Look::Fluent, [(true, false); 2]);
    set(id, 0, Some(Box::new(style)));
    let root = hwnd(id).unwrap();
    let events = record();
    let scrolls = || {
        events
            .take()
            .into_iter()
            .filter_map(|event| match event {
                Event::Scroll { y, .. } => Some(y),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let tick = || send(root, WM_TIMER, REPEAT_TIMER, 0);
    let bottom = snapshot(id).bars[1].unwrap().whole.height - 9.0;
    mouse(id, WM_LBUTTONDOWN, 0, 292.5, bottom);
    assert_eq!(scrolls(), [40.0]);
    tick();
    assert_eq!(scrolls(), [80.0]);
    // Off the button it scrolls no more, until the pointer comes back.
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 100.0, 100.0);
    tick();
    assert!(scrolls().is_empty());
    mouse(id, WM_MOUSEMOVE, MK_LBUTTON as usize, 292.5, bottom);
    tick();
    assert_eq!(scrolls(), [120.0]);
    mouse(id, WM_LBUTTONUP, 0, 292.5, bottom);
    tick();
    assert!(scrolls().is_empty());
    // The track pages toward the pointer until the thumb reaches it.
    let page = page_step(200.0 - scrollbar_room(id).0);
    mouse(id, WM_MOUSEMOVE, 0, 292.5, 150.0);
    mouse(id, WM_LBUTTONDOWN, 0, 292.5, 150.0);
    assert_eq!(scrolls(), [120.0 + page]);
    let mut ticks = 0;
    while {
        tick();
        !scrolls().is_empty()
    } {
        ticks += 1;
        assert!(ticks < 10);
    }
    let thumb = snapshot(id).bars[1].unwrap().thumb;
    assert!(thumb.y <= 150.0 && 150.0 < thumb.y + thumb.height);
    mouse(id, WM_LBUTTONUP, 0, 292.5, 150.0);
    destroy(id);
}
