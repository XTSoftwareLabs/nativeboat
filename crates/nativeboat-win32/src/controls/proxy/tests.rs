use super::*;
use crate::tests::support::{Canvas, send, window};
use crate::window::{apply, destroy, focus_control, render_surface};
use nativeboat_ir::{Change, ControlState, Rect};
use shimforge::{Session, mock};
use windows_sys::Win32::Graphics::GdiPlus::GdipDisposeImage;

/// A box turned a quarter round, its corner at (100, 20) in the window:
/// its own point (u, v) shows at (100 - v, 20 + u).
const TURNED: [f32; 6] = [0.0, 1.0, -1.0, 0.0, 100.0, 20.0];

fn field(window: WindowId, id: u64, kind: ControlKind, transform: Option<[f32; 6]>) -> HWND {
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: Control {
                id,
                kind,
                rect: Rect::new(10.0, 10.0, 80.0, 24.0),
                state: ControlState {
                    text: "native".to_owned(),
                    visible: true,
                    transform,
                    ..ControlState::default()
                },
            },
        }],
    );
    hwnd(window, id).unwrap()
}

/// Where a point of the window's client area, in CSS pixels, falls in a
/// window's client area, in device pixels.
fn client_point(window: WindowId, target: HWND, (x, y): (f32, f32)) -> LPARAM {
    let scale = crate::window::scale(window);
    let mut at = POINT {
        x: px(x, scale),
        y: px(y, scale),
    };
    // SAFETY: both windows belong to this test.
    unsafe {
        ClientToScreen(crate::window::hwnd(window).unwrap(), &mut at);
        ScreenToClient(target, &mut at);
    }
    point(at.x, at.y)
}

fn coordinates(lparam: LPARAM) -> (i32, i32) {
    (
        i32::from(lparam as u16 as i16),
        i32::from((lparam >> 16) as u16 as i16),
    )
}

fn dispose(snapshots: Vec<(Snapshot, nativeboat_ir::Rect)>) {
    for (snapshot, _) in snapshots {
        // SAFETY: the snapshot's bitmap is owned here.
        unsafe { GdipDisposeImage(snapshot.image) };
    }
}

#[test]
fn turned_controls_are_kept_out_of_sight_and_paint_as_their_images() {
    let window = window();
    let scale = crate::window::scale(window);
    let edit = field(window, 1, ControlKind::TextBox, Some(TURNED));
    field(window, 2, ControlKind::TextBox, None);
    // Native frames print only for windows that show: the window shows
    // far off screen, without being activated.
    // SAFETY: moves and shows the test's own window.
    unsafe {
        let root = crate::window::hwnd(window).unwrap();
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
    let mut bounds = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: reads test-owned windows.
    unsafe {
        GetWindowRect(edit, &mut bounds);
        ClientToScreen(crate::window::hwnd(window).unwrap(), &mut origin);
    }
    assert_eq!(
        (bounds.left - origin.x, bounds.top - origin.y),
        (AWAY, AWAY)
    );
    // Only the turned control is printed, at its size, for its box.
    let printed = snapshots(window, 0);
    assert_eq!(printed.len(), 1);
    let (snapshot, rect) = &printed[0];
    assert_eq!(snapshot.id, 1);
    assert_eq!(
        snapshot.size,
        (px(80.0, scale) as u32, px(24.0, scale) as u32)
    );
    assert_eq!(*rect, Rect::new(10.0, 10.0, 80.0, 24.0));
    dispose(printed);
    assert!(snapshots(u64::MAX, 0).is_empty());
    assert!(snapshots(window, 99).is_empty());
    // The image paints where the control's marker is: its frame shows.
    apply(
        window,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::Child(NativeChild::Control(1)),
                DisplayItem::Child(NativeChild::Control(2)),
            ],
        }],
    );
    let (width, height) = (px(120.0, scale), px(60.0, scale));
    let canvas = Canvas::new(width, height);
    let area = RECT {
        right: width,
        bottom: height,
        ..Default::default()
    };
    render_surface(window, 0, canvas.dc, area);
    assert_ne!(
        canvas.pixel(px(10.0, scale), px(10.0, scale)),
        (255, 255, 255)
    );
    // Focused, the image shows the caret.
    focus_control(window, 1);
    send(edit, EM_SETSEL, 0, 0);
    let printed = snapshots(window, 0);
    assert_eq!(printed.len(), 1);
    dispose(printed);
    // Nothing is printed without room, or without a bitmap.
    let mut session = Session::new();
    let bitmap = mock!(
        session,
        CreateCompatibleBitmap,
        unsafe extern "system" fn(HDC, i32, i32) -> HBITMAP
    );
    bitmap.expect().returning(|_, _, _| null_mut());
    assert!(snapshots(window, 0).is_empty());
    session.restore();
    drop(session);
    apply(
        window,
        &[Change::UpdateControl {
            control: Control {
                id: 1,
                kind: ControlKind::TextBox,
                rect: Rect::new(10.0, 10.0, 0.0, 24.0),
                state: ControlState {
                    visible: true,
                    transform: Some(TURNED),
                    ..ControlState::default()
                },
            },
        }],
    );
    assert!(snapshots(window, 0).is_empty());
    destroy(window);
}

#[test]
fn the_pointer_reaches_turned_fields_where_their_images_show() {
    let window = window();
    let scale = crate::window::scale(window);
    let edit = field(window, 1, ControlKind::TextBox, Some(TURNED));
    let plain = field(window, 2, ControlKind::TextBox, None);
    let flat = field(window, 3, ControlKind::TextBox, Some([0.0; 6]));
    let selection = || {
        let (mut start, mut end) = (0u32, 0u32);
        send(
            edit,
            EM_GETSEL,
            &mut start as *mut u32 as usize,
            &mut end as *mut u32 as isize,
        );
        (start, end)
    };
    // A click past the text puts the caret after it; a double click on
    // the word selects it.
    let at = |u: f32, v: f32| (100.0 - v, 20.0 + u);
    assert!(forward(
        window,
        1,
        (PointerKind::Down, 0, 1, 1),
        at(70.0, 12.0)
    ));
    assert!(forward(
        window,
        1,
        (PointerKind::Up, 0, 0, 1),
        at(70.0, 12.0)
    ));
    assert_eq!(selection(), (6, 6));
    assert!(forward(
        window,
        1,
        (PointerKind::Down, 0, 1, 2),
        at(5.0, 12.0)
    ));
    assert!(forward(
        window,
        1,
        (PointerKind::Up, 0, 0, 2),
        at(5.0, 12.0)
    ));
    assert_eq!(selection(), (0, 6));
    // Controls not turned, turned flat, or events they do not take are
    // not forwarded.
    assert!(!forward(
        window,
        2,
        (PointerKind::Down, 0, 1, 1),
        (20.0, 20.0)
    ));
    assert!(!forward(
        window,
        3,
        (PointerKind::Down, 0, 1, 1),
        (20.0, 20.0)
    ));
    assert!(!forward(
        window,
        1,
        (PointerKind::Leave, 0, 0, 0),
        (20.0, 20.0)
    ));
    assert!(!forward(
        window,
        9,
        (PointerKind::Move, 0, 0, 0),
        (20.0, 20.0)
    ));
    for (event, expected) in [
        (
            (PointerKind::Move, 0, 7),
            (WM_MOUSEMOVE, MK_LBUTTON | MK_RBUTTON | MK_MBUTTON),
        ),
        ((PointerKind::Down, 1, 4), (WM_MBUTTONDOWN, MK_MBUTTON)),
        ((PointerKind::Down, 2, 2), (WM_RBUTTONDOWN, MK_RBUTTON)),
        ((PointerKind::Up, 1, 0), (WM_MBUTTONUP, 0)),
        ((PointerKind::Up, 2, 0), (WM_RBUTTONUP, 0)),
    ] {
        let (kind, button, buttons) = event;
        assert_eq!(
            message(kind, button, buttons, 1),
            Some((expected.0, expected.1 as WPARAM))
        );
    }
    assert_eq!(message(PointerKind::Down, 3, 0, 1), None);
    // Mouse input the field captured comes in its out-of-sight client
    // area and maps to its own box.
    let captured = client_point(window, edit, at(30.0, 10.0));
    let (x, y) = coordinates(input(edit, 1, WM_MOUSEMOVE, captured));
    let (left, top) = client_offset(edit, box_height(window, 1));
    assert!(left > 0 && top > 0);
    let expected = (px(30.0, scale) - left, px(10.0, scale) - top);
    assert!(
        (x - expected.0).abs() <= 1 && (y - expected.1).abs() <= 1,
        "{x},{y}"
    );
    assert_eq!(box_height(u64::MAX, 1), 0);
    assert_eq!(box_height(window, 9), 0);
    assert_eq!(input(edit, 1, WM_CHAR, captured), captured);
    assert_eq!(input(plain, 2, WM_MOUSEMOVE, captured), captured);
    assert_eq!(input(flat, 3, WM_MOUSEMOVE, captured), captured);
    assert_eq!(input(null_mut(), 9, WM_MOUSEMOVE, captured), captured);
    FORWARDING.set(true);
    assert_eq!(input(edit, 1, WM_MOUSEMOVE, captured), captured);
    FORWARDING.set(false);
    // Typing repaints the image; other messages and controls do not.
    send(edit, WM_CHAR, 'x' as usize, 1);
    after(edit, 1, WM_NULL);
    after(plain, 2, WM_CHAR);
    after(null_mut(), 9, WM_CHAR);
    destroy(window);
}

#[test]
fn turned_date_pickers_drop_their_calendars_below_their_images() {
    let window = window();
    let scale = crate::window::scale(window);
    let picker = field(window, 1, ControlKind::Date, Some(TURNED));
    let edit = field(window, 2, ControlKind::TextBox, None);
    // Without a calendar, or for controls that are not turned pickers,
    // nothing moves.
    dropped(window, picker);
    dropped(window, edit);
    dropped(window, null_mut());
    // SAFETY: creates hidden test windows, destroyed below.
    let (popup, calendar, inner) = unsafe {
        let popup = CreateWindowExW(
            0,
            w!("STATIC"),
            null(),
            WS_POPUP,
            0,
            0,
            50,
            40,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        );
        let child = |parent: HWND| {
            CreateWindowExW(
                0,
                w!("STATIC"),
                null(),
                WS_CHILD,
                0,
                0,
                10,
                10,
                parent,
                null_mut(),
                instance(),
                null(),
            )
        };
        (popup, child(popup), child(picker))
    };
    let mut session = Session::new();
    let sent = mock!(
        session,
        SendMessageW,
        unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT
    );
    let (calendar_id, inner_id) = (calendar as isize, inner as isize);
    sent.expect().once().returning(move |_, message, _, _| {
        assert_eq!(message, DTM_GETMONTHCAL);
        calendar_id
    });
    dropped(window, picker);
    session.restore();
    drop(session);
    let mut client = RECT::default();
    let mut shown = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: reads test-owned windows.
    unsafe {
        GetClientRect(picker, &mut client);
        GetWindowRect(popup, &mut shown);
        ClientToScreen(crate::window::hwnd(window).unwrap(), &mut origin);
    }
    let (width, height) = (client.right as f32 / scale, client.bottom as f32 / scale);
    assert_eq!(
        (shown.left - origin.x, shown.top - origin.y),
        (px(100.0 - height, scale), px(20.0 + width, scale))
    );
    // A calendar inside the picker itself stays.
    let mut session = Session::new();
    let sent = mock!(
        session,
        SendMessageW,
        unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT
    );
    sent.expect().once().returning(move |_, _, _, _| inner_id);
    dropped(window, picker);
    session.restore();
    // The drop-down notification places it.
    let header = NMHDR {
        hwndFrom: picker,
        idFrom: 0,
        code: DTN_DROPDOWN,
    };
    notify(window, &header);
    // SAFETY: destroys the test's own windows.
    unsafe {
        DestroyWindow(popup);
    }
    destroy(window);
}
