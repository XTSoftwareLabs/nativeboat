use super::*;
use crate::tests::support::{Canvas, send, window};
use nativeboat_css::Color;
use nativeboat_ir::{Control, ControlKind, ControlState, Corner};

fn fill(rect: Rect, color: Color) -> DisplayItem {
    DisplayItem::Fill {
        rect,
        color,
        radii: [Corner::SQUARE; 4],
    }
}

fn surface(
    window: WindowId,
    id: u64,
    parent: u64,
    rect: Rect,
    scroll: (f32, f32),
    items: Vec<DisplayItem>,
) -> HWND {
    apply(
        window,
        &[
            Change::AddSurface { id, parent },
            Change::UpdateSurface {
                id,
                rect,
                content_width: rect.width + scroll.0,
                content_height: rect.height + scroll.1,
                scroll_x: scroll.0,
                scroll_y: scroll.1,
            },
            Change::Paint { surface: id, items },
        ],
    );
    surface_hwnd(window, id).unwrap()
}

fn control(window: WindowId, id: u64, surface: u64, rect: Rect) -> HWND {
    apply(
        window,
        &[Change::AddControl {
            surface,
            control: Control {
                id,
                kind: ControlKind::TextBox,
                rect,
                state: ControlState {
                    visible: true,
                    bare: true,
                    text: "native".to_owned(),
                    ..ControlState::default()
                },
            },
        }],
    );
    crate::controls::hwnd(window, id).unwrap()
}

fn origin(window: WindowId, target: HWND) -> POINT {
    let mut point = POINT::default();
    // SAFETY: both handles belong to this test's window.
    unsafe { MapWindowPoints(target, hwnd(window).unwrap(), &mut point, 1) };
    point
}

#[test]
fn native_children_share_bottom_to_top_order_without_moving_or_focusing() {
    let window = window();
    let first = surface(
        window,
        10,
        0,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        (0.0, 0.0),
        vec![],
    );
    let edit = control(window, 1, 0, Rect::new(10.0, 10.0, 50.0, 20.0));
    let second = surface(
        window,
        20,
        0,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        (0.0, 0.0),
        vec![],
    );
    let nested = control(window, 2, 10, Rect::new(0.0, 0.0, 50.0, 20.0));
    let before = origin(window, edit);
    // SAFETY: reads test-owned handles.
    let focus = unsafe { GetFocus() };
    apply(
        window,
        &[Change::Stack {
            surface: 0,
            children: vec![
                NativeChild::Surface(20),
                NativeChild::Surface(10),
                NativeChild::Control(1),
                NativeChild::Control(2),
                NativeChild::Control(999),
                NativeChild::Surface(999),
            ],
        }],
    );
    // SAFETY: reads test-owned HWND order and keyboard focus.
    unsafe {
        assert_eq!(GetWindow(hwnd(window).unwrap(), GW_CHILD), edit);
        assert_eq!(GetWindow(edit, GW_HWNDNEXT), first);
        assert_eq!(GetWindow(first, GW_HWNDNEXT), second);
        assert_eq!(GetParent(nested), first);
        assert_eq!(GetFocus(), focus);
    }
    assert_eq!(
        (origin(window, edit).x, origin(window, edit).y),
        (before.x, before.y)
    );
    let mut changed =
        STATE.with_borrow(|state| state.windows[&window].controls[&1].control.clone());
    changed.state.text = "updated".to_owned();
    apply(window, &[Change::UpdateControl { control: changed }]);
    // SAFETY: state changes preserve the owned sibling paint order.
    unsafe {
        assert_eq!(GetWindow(hwnd(window).unwrap(), GW_CHILD), edit);
    }
    order(u64::MAX, 0, &[NativeChild::Control(1)]);
    destroy(window);
}

#[test]
fn surface_overlays_preserve_clips_transforms_scroll_and_dpi_rounded_origins() {
    for scale in [1.0, 1.5, 2.0] {
        let window = window();
        STATE.with_borrow_mut(|state| state.windows.get_mut(&window).unwrap().scale = scale);
        let child = surface(
            window,
            10,
            0,
            Rect::new(20.3, 20.3, 100.0, 80.0),
            (5.0, 7.0),
            vec![fill(
                Rect::new(0.0, 0.0, 200.0, 200.0),
                Color::rgb(0, 255, 0),
            )],
        );
        let items = vec![
            fill(Rect::new(0.0, 0.0, 300.0, 200.0), Color::rgb(255, 0, 0)),
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 5.0, 0.0]),
            DisplayItem::PushClip {
                rect: Rect::new(40.0, 30.0, 20.0, 20.0),
                radii: [Corner::SQUARE; 4],
            },
            DisplayItem::Child(NativeChild::Surface(10)),
            fill(Rect::new(30.0, 25.0, 60.0, 40.0), Color::rgb(0, 0, 255)),
            DisplayItem::PopClip,
            DisplayItem::PopTransform,
        ];
        apply(window, &[Change::Paint { surface: 0, items }]);
        assert!(exists(window, NativeChild::Surface(10)));
        let width = px(100.0, scale);
        let height = px(80.0, scale);
        let canvas = Canvas::new(width, height);
        render_surface(
            window,
            10,
            canvas.dc,
            RECT {
                right: width,
                bottom: height,
                ..Default::default()
            },
        );
        let origin = origin(window, child);
        assert_eq!(
            canvas.pixel(px(50.0, scale) - origin.x, px(40.0, scale) - origin.y),
            (0, 0, 255)
        );
        assert_eq!(
            canvas.pixel(px(30.0, scale) - origin.x, px(40.0, scale) - origin.y),
            (0, 255, 0)
        );
        assert_eq!(
            canvas.pixel(px(75.0, scale) - origin.x, px(40.0, scale) - origin.y),
            (0, 255, 0)
        );
        destroy(window);
    }
}

#[test]
fn nested_surfaces_and_native_edit_clients_receive_later_ancestor_pixels() {
    let window = window();
    let scale = scale(window);
    let outer = surface(
        window,
        10,
        0,
        Rect::new(20.0, 20.0, 120.0, 100.0),
        (3.0, 4.0),
        vec![],
    );
    let inner = surface(
        window,
        11,
        10,
        Rect::new(10.0, 10.0, 80.0, 60.0),
        (0.0, 0.0),
        vec![fill(Rect::new(0.0, 0.0, 80.0, 60.0), Color::rgb(0, 255, 0))],
    );
    let edit = control(window, 1, 11, Rect::new(10.0, 10.0, 40.0, 25.0));
    apply(
        window,
        &[
            Change::Paint {
                surface: 11,
                items: vec![DisplayItem::Child(NativeChild::Control(1))],
            },
            Change::Paint {
                surface: 10,
                items: vec![
                    DisplayItem::Child(NativeChild::Surface(11)),
                    fill(Rect::new(20.0, 20.0, 20.0, 20.0), Color::rgb(0, 0, 255)),
                ],
            },
            Change::Paint {
                surface: 0,
                items: vec![
                    DisplayItem::Child(NativeChild::Surface(10)),
                    fill(Rect::new(40.0, 40.0, 5.0, 5.0), Color::rgb(255, 0, 0)),
                ],
            },
        ],
    );
    assert!(exists(window, NativeChild::Control(1)));
    for (target, width, height) in [(inner, 80.0, 60.0), (edit, 40.0, 25.0)] {
        let canvas = Canvas::new(px(width, scale), px(height, scale));
        if target == inner {
            render_surface(
                window,
                11,
                canvas.dc,
                RECT {
                    right: px(width, scale),
                    bottom: px(height, scale),
                    ..Default::default()
                },
            );
        } else {
            send(
                target,
                WM_PRINTCLIENT,
                canvas.dc as usize,
                PRF_CLIENT as isize,
            );
        }
        let origin = origin(window, target);
        assert_eq!(
            canvas.pixel(px(42.0, scale) - origin.x, px(42.0, scale) - origin.y),
            (255, 0, 0)
        );
        assert_eq!(
            canvas.pixel(px(50.0, scale) - origin.x, px(50.0, scale) - origin.y),
            (0, 0, 255)
        );
    }
    // A parent-only display update invalidates descendant native clients.
    // SAFETY: validate this owned control's region before the parent update.
    unsafe {
        let root = hwnd(window).unwrap();
        SetWindowPos(
            root,
            null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(root, SW_SHOWNOACTIVATE);
        ValidateRect(edit, null());
    }
    apply(
        window,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::Child(NativeChild::Surface(10)),
                fill(Rect::new(0.0, 0.0, 300.0, 200.0), Color::BLACK),
            ],
        }],
    );
    // SAFETY: reads only the owned control's update region and parent.
    unsafe {
        assert_ne!(GetUpdateRect(edit, null_mut(), 0), 0);
        assert_eq!(GetParent(inner), outer);
    }
    destroy(window);
}

#[test]
fn absent_markers_closed_scopes_and_clipped_ancestors_do_not_overpaint() {
    let child = NativeChild::Surface(10);
    assert_eq!(following(&[], child), None);
    assert_eq!(
        following(
            &[
                DisplayItem::Child(child),
                DisplayItem::PopClip,
                DisplayItem::Child(NativeChild::Control(1))
            ],
            child
        ),
        None
    );
    assert!(!exists(u64::MAX, child));
    let window = window();
    assert!(!exists(window, NativeChild::Surface(0)));
    assert!(!exists(window, NativeChild::Surface(999)));
    assert!(!exists(window, NativeChild::Control(999)));
    let target = surface(
        window,
        10,
        0,
        Rect::new(400.0, 400.0, 10.0, 10.0),
        (0.0, 0.0),
        vec![],
    );
    let items = vec![
        DisplayItem::Child(child),
        fill(Rect::new(0.0, 0.0, 500.0, 500.0), Color::BLACK),
    ];
    apply(window, &[Change::Paint { surface: 0, items }]);
    let canvas = Canvas::new(10, 10);
    let area = RECT {
        right: 10,
        bottom: 10,
        ..Default::default()
    };
    paint(window, child, target, canvas.dc, area);
    assert_eq!(canvas.pixel(5, 5), (255, 255, 255));
    paint(u64::MAX, child, target, canvas.dc, area);
    paint(window, NativeChild::Surface(999), target, canvas.dc, area);
    STATE.with_borrow_mut(|state| {
        state
            .windows
            .get_mut(&window)
            .unwrap()
            .surfaces
            .get_mut(&10)
            .unwrap()
            .parent = 999
    });
    assert!(!exists(window, child));
    paint(window, child, target, canvas.dc, area);
    destroy(window);
}

#[test]
fn overlay_scope_replay_preserves_closed_groups_filters_and_alpha() {
    use nativeboat_css::types::Filter;
    let window = window();
    STATE.with_borrow_mut(|state| state.windows.get_mut(&window).unwrap().scale = 1.0);
    let child = surface(
        window,
        10,
        0,
        Rect::new(0.0, 0.0, 40.0, 40.0),
        (0.0, 0.0),
        vec![fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::rgb(0, 255, 0))],
    );
    apply(
        window,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 100.0, 100.0]),
                fill(Rect::new(0.0, 0.0, 40.0, 40.0), Color::BLACK),
                DisplayItem::PopTransform,
                DisplayItem::PushFilter(vec![Filter::Brightness(1.0)]),
                DisplayItem::PopFilter,
                DisplayItem::PushOpacity(0.5),
                DisplayItem::PushFilter(vec![Filter::Brightness(1.0)]),
                DisplayItem::Child(NativeChild::Surface(10)),
                fill(Rect::new(10.0, 10.0, 20.0, 20.0), Color::rgb(255, 0, 0)),
                DisplayItem::PopFilter,
                DisplayItem::PopOpacity,
            ],
        }],
    );
    let canvas = Canvas::new(40, 40);
    render_surface(
        window,
        10,
        canvas.dc,
        RECT {
            right: 40,
            bottom: 40,
            ..Default::default()
        },
    );
    assert_eq!(canvas.pixel(5, 5), (0, 255, 0));
    let (r, g, b) = canvas.pixel(20, 20);
    assert!(
        (126..=129).contains(&r) && (126..=129).contains(&g) && b == 0,
        "{r},{g},{b}"
    );
    // All ancestor scope/DC state is restored after overlay rendering.
    // SAFETY: the owned canvas DC is live.
    unsafe {
        SetPixel(canvas.dc, 0, 0, 0);
        GdiFlush();
    }
    assert_eq!(canvas.pixel(0, 0), (0, 0, 0));
    assert_eq!(origin(window, child).x, 0);
    destroy(window);
}

#[test]
fn children_under_ancestor_content_repaint_whole_when_scrolling_moves_them() {
    let window = window();
    let scale = scale(window);
    surface(
        window,
        10,
        0,
        Rect::new(0.0, 0.0, 100.0, 100.0),
        (0.0, 0.0),
        vec![DisplayItem::Child(NativeChild::Surface(11))],
    );
    // Partly shown: only its top 20 pixels are inside the scroller.
    let inner = surface(
        window,
        11,
        10,
        Rect::new(0.0, 80.0, 50.0, 40.0),
        (0.0, 0.0),
        vec![fill(Rect::new(0.0, 0.0, 50.0, 40.0), Color::rgb(0, 255, 0))],
    );
    // Content of the window lies over the scroller, as a search bar over
    // an editor does.
    apply(
        window,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::Child(NativeChild::Surface(10)),
                fill(Rect::new(0.0, 60.0, 100.0, 10.0), Color::rgb(255, 0, 0)),
            ],
        }],
    );
    // SAFETY: shows the owned window off screen and validates the child.
    unsafe {
        let root = hwnd(window).unwrap();
        SetWindowPos(
            root,
            null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(root, SW_SHOWNOACTIVATE);
        UpdateWindow(root);
        ValidateRect(inner, null());
    }
    apply(
        window,
        &[Change::UpdateSurface {
            id: 10,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            content_width: 100.0,
            content_height: 200.0,
            scroll_x: 0.0,
            scroll_y: 30.0,
        }],
    );
    let mut area = RECT::default();
    // SAFETY: reads the owned child's update region.
    unsafe { GetUpdateRect(inner, &mut area, 0) };
    assert_eq!(
        (area.top, area.bottom),
        (0, px(40.0, scale)),
        "the moved child repaints whole, with what lies over it"
    );
    destroy(window);
}

#[test]
fn frames_and_scroll_bars_paint_with_the_content_above_them() {
    let window = window();
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: Control {
                id: 1,
                kind: ControlKind::TextBox,
                rect: Rect::new(10.0, 10.0, 80.0, 24.0),
                state: ControlState {
                    visible: true,
                    text: "native".to_owned(),
                    ..ControlState::default()
                },
            },
        }],
    );
    let edit = crate::controls::hwnd(window, 1).unwrap();
    let scroller = surface(
        window,
        10,
        0,
        Rect::new(100.0, 10.0, 80.0, 80.0),
        (0.0, 100.0),
        vec![],
    );
    // Native frames print only for windows that show: the window shows
    // far off screen, without being activated.
    // SAFETY: moves and shows the test's own window.
    unsafe {
        let root = hwnd(window).unwrap();
        SetWindowPos(
            root,
            null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(root, SW_SHOWNOACTIVATE);
    }
    let (field, area) = (NativeChild::Control(1), NativeChild::Surface(10));
    // With nothing above them, the system paints their frames.
    assert!(frame(window, field, edit).is_none());
    assert!(!paint_frame(window, area, scroller, |_, _| {}));
    let items = vec![
        DisplayItem::Child(field),
        DisplayItem::Child(area),
        fill(
            Rect::new(0.0, 0.0, 300.0, 200.0),
            Color::rgba(255, 0, 0, 0.5),
        ),
    ];
    apply(window, &[Change::Paint { surface: 0, items }]);
    let found = frame(window, field, edit).unwrap();
    assert!(found.inset.x > 0 && found.inset.y > 0);
    let canvas = Canvas::new(found.size.0, found.size.1);
    let mut printed = None;
    render_frame(window, field, edit, canvas.dc, &found, |_, inset| {
        printed = Some((inset.x, inset.y));
    });
    assert_eq!(printed, Some((found.inset.x, found.inset.y)));
    // The native frame shows through the translucent red above it.
    let (red, green, blue) = canvas.pixel(0, 0);
    assert!(
        red > 200 && green < 110 && blue < 110,
        "{red},{green},{blue}"
    );
    // A scroll container's scroll bar too.
    let bar = frame(window, area, scroller).unwrap();
    assert!(bar.client.right < bar.size.0);
    let canvas = Canvas::new(bar.size.0, bar.size.1);
    render_frame(window, area, scroller, canvas.dc, &bar, |_, _| {});
    let (red, green, _) = canvas.pixel(bar.size.0 - 2, bar.size.1 / 2);
    assert!(red > 200 && green < 128, "{red},{green}");
    assert!(paint_frame(window, field, edit, |_, _| {}));
    // Without a device context or a bitmap, the system paints the frame.
    let mut session = shimforge::Session::new();
    let dc = shimforge::mock!(session, GetWindowDC, unsafe extern "system" fn(HWND) -> HDC);
    dc.expect().returning(|_| null_mut());
    assert!(!paint_frame(window, field, edit, |_, _| {}));
    session.restore();
    drop(session);
    let mut session = shimforge::Session::new();
    let create = shimforge::mock!(
        session,
        CreateDIBSection,
        unsafe extern "system" fn(
            HDC,
            *const BITMAPINFO,
            u32,
            *mut *mut core::ffi::c_void,
            HANDLE,
            u32,
        ) -> HBITMAP
    );
    create.expect().returning(|_, _, _, _, _, _| null_mut());
    assert!(!paint_frame(window, field, edit, |_, _| {}));
    session.restore();
    drop(session);
    assert_eq!(send(edit, WM_NCPAINT, 1, 0), 0);
    assert_eq!(send(scroller, WM_NCPAINT, 1, 0), 0);
    let root = hwnd(window).unwrap();
    invalidate(root, None);
    invalidate(
        root,
        Some(&RECT {
            left: 250,
            top: 150,
            right: 260,
            bottom: 160,
        }),
    );
    destroy(window);
}
