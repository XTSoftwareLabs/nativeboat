use super::*;
use crate::tests::support::{Canvas, pack, record, send, window};
use nativeboat_css::Color;
use nativeboat_ir::{Control, ControlKind, ControlState, Corner};
use shimforge::{Session, mock};
use std::sync::{Arc, Mutex};

#[test]
fn moving_to_top_preserves_focus_geometry_and_visibility() {
    let id = window();
    let expected = hwnd(id).unwrap() as usize;
    let mut session = Session::new();
    let position = mock!(
        session,
        SetWindowPos,
        unsafe extern "system" fn(
            HWND,
            HWND,
            i32,
            i32,
            i32,
            i32,
            SET_WINDOW_POS_FLAGS,
        ) -> windows_sys::core::BOOL
    );
    position
        .expect()
        .once()
        .returning(move |window, after, x, y, width, height, flags| {
            assert_eq!(window as usize, expected);
            assert_eq!(after, HWND_TOP);
            assert_eq!((x, y, width, height), (0, 0, 0, 0));
            assert_eq!(flags, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            1
        });
    move_top(id);
    move_top(u64::MAX);
    session.restore();
    assert!(!is_visible(id));
    destroy(id);
}

#[test]
fn shell_drops_deliver_paths_coordinates_and_release_the_handle() {
    let id = window();
    let events = record();
    let mut session = Session::new();
    let query = mock!(
        &mut session,
        windows_sys::Win32::UI::Shell::DragQueryFileW,
        unsafe extern "system" fn(HDROP, u32, windows_sys::core::PWSTR, u32) -> u32
    );
    query.expect().returning(|drop, index, buffer, capacity| {
        if drop as usize == 2 {
            return 0;
        }
        if index == u32::MAX {
            return 2;
        }
        if index == 1 {
            return 0;
        }
        let path: Vec<u16> = "C:\\drop\\a.txt".encode_utf16().chain(Some(0)).collect();
        if capacity != 0 {
            // SAFETY: the production caller allocated the queried size plus NUL.
            unsafe { std::ptr::copy_nonoverlapping(path.as_ptr(), buffer, path.len()) };
        }
        (path.len() - 1) as u32
    });
    let point = mock!(
        &mut session,
        windows_sys::Win32::UI::Shell::DragQueryPoint,
        unsafe extern "system" fn(HDROP, *mut POINT) -> windows_sys::core::BOOL
    );
    point.expect().returning(|_, point| {
        // SAFETY: the caller passes a writable POINT.
        unsafe {
            *point = POINT { x: 20, y: 40 };
        }
        1
    });
    let finished = Arc::new(Mutex::new(0));
    let called = finished.clone();
    let finish = mock!(
        &mut session,
        windows_sys::Win32::UI::Shell::DragFinish,
        unsafe extern "system" fn(HDROP)
    );
    finish.expect().returning(move |_| {
        *called.lock().unwrap() += 1;
    });
    send(handle(id), WM_DROPFILES, 1, 0);
    assert_eq!(
        events.take(),
        [Event::FilesDropped {
            paths: vec!["C:\\drop\\a.txt".into()],
            strings: Vec::new(),
            x: 20.0 / scale(id),
            y: 40.0 / scale(id)
        }]
    );
    send(handle(id), WM_DROPFILES, 2, 0);
    assert!(events.take().is_empty());
    assert_eq!(*finished.lock().unwrap(), 2);
    session.restore();
    destroy(id);
}

fn handle(window: WindowId) -> HWND {
    hwnd(window).unwrap()
}

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= 1.0
}

fn style(hwnd: HWND) -> u32 {
    // SAFETY: reads the style of a live window.
    unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 }
}

fn alive(hwnd: HWND) -> bool {
    // SAFETY: IsWindow accepts any handle.
    unsafe { IsWindow(hwnd) != 0 }
}

fn origin_in_root(window: WindowId, child: HWND) -> (i32, i32) {
    let mut point = POINT::default();
    // SAFETY: both windows are live.
    unsafe { MapWindowPoints(child, handle(window), &mut point, 1) };
    (point.x, point.y)
}

fn scroll_position(hwnd: HWND, bar: SCROLLBAR_CONSTANTS) -> i32 {
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_POS,
        ..SCROLLINFO::default()
    };
    // SAFETY: `info` is writable.
    unsafe { GetScrollInfo(hwnd, bar, &mut info) };
    info.nPos
}

#[test]
fn pages_have_the_client_area_in_whole_device_independent_pixels_rounded_up() {
    let id = window();
    let hwnd = handle(id);
    // A client area a device pixel past whole device-independent pixels at
    // any scale above 1, as 563 rows are 450.4 at 125%.
    let dpi = scale(id);
    let (width, height) = (px(320.0, dpi) + 1, px(240.0, dpi) + 1);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    // SAFETY: reads the styles of a window of this thread and resizes it.
    unsafe {
        AdjustWindowRectEx(&mut outer, style(hwnd), 0, 0);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            outer.right - outer.left,
            outer.bottom - outer.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let area = client(hwnd);
    let whole = |pixels: i32| (pixels as f32 / dpi).ceil();
    assert_eq!(client_size(id), (whole(area.right), whole(area.bottom)));
    // Zoom divides the whole pixels.
    set_zoom(id, 2.0);
    assert_eq!(
        client_size(id),
        (whole(area.right) / 2.0, whole(area.bottom) / 2.0)
    );
    // Surfaces inside the page keep their exact size.
    // SAFETY: the child belongs to this test's window on this thread.
    let surface = unsafe {
        CreateWindowExW(
            0,
            SURFACE_CLASS,
            null(),
            WS_CHILD,
            0,
            0,
            101,
            51,
            hwnd,
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!surface.is_null());
    assert_eq!(view(surface, 1.25, 1.0), (80.8, 40.8));
    destroy(id);
}

#[test]
fn windows_start_hidden_with_their_options() {
    let id = window();
    let hwnd = handle(id);
    assert!(window_ids().contains(&id));
    assert!(!is_visible(id));
    assert_eq!(title(id), "Test");
    set_title(id, "Renamed");
    assert_eq!(title(id), "Renamed");
    let (width, height) = client_size(id);
    assert!(
        near(width, 300.0) && near(height, 200.0),
        "{width} {height}"
    );
    set_client_size(id, 320.0, 240.0);
    let (width, height) = client_size(id);
    assert!(
        near(width, 320.0) && near(height, 240.0),
        "{width} {height}"
    );
    assert!(scale(id) >= 1.0);
    assert_ne!(style(hwnd) & WS_THICKFRAME, 0);
    assert_eq!(scale(0), 1.0);
    assert_eq!(client_size(0), (0.0, 0.0));
    assert_eq!(title(0), "");
    set_client_size(0, 1.0, 1.0);
    set_title(0, "none");
    set_cursor(0, Cursor::Pointer);
    focus_control(id, 77);
    destroy(id);
    assert!(!window_ids().contains(&id));
}

#[test]
fn styles_and_positions() {
    let fixed = create_window(&WindowOptions {
        resizable: false,
        minimizable: false,
        maximizable: false,
        position: Some((10.0, 20.0)),
        ..WindowOptions::default()
    })
    .unwrap();
    let hwnd = handle(fixed);
    let bits = style(hwnd);
    assert_eq!(bits & (WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX), 0);
    let mut rect = RECT::default();
    // SAFETY: `rect` is writable.
    unsafe { GetWindowRect(hwnd, &mut rect) };
    assert_eq!(
        (rect.left, rect.top),
        (px(10.0, scale(fixed)), px(20.0, scale(fixed)))
    );
    let maximizable = create_window(&WindowOptions {
        maximizable: false,
        ..WindowOptions::default()
    })
    .unwrap();
    assert_eq!(style(handle(maximizable)) & WS_MAXIMIZEBOX, 0);
    assert_ne!(style(handle(maximizable)) & WS_MINIMIZEBOX, 0);
    destroy(fixed);
    destroy(maximizable);
}

#[test]
fn show_and_state_commands() {
    let id = window();
    let mut session = Session::new();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let seen = commands.clone();
    let show_window = mock!(
        session,
        ShowWindow,
        unsafe extern "system" fn(HWND, SHOW_WINDOW_CMD) -> windows_sys::core::BOOL
    );
    show_window.expect().returning(move |_, command| {
        seen.lock().unwrap().push(command);
        1
    });
    let set_focus = mock!(session, SetFocus, unsafe extern "system" fn(HWND) -> HWND);
    set_focus.expect().times(2).returning(|_| null_mut());
    let foreground = mock!(
        session,
        SetForegroundWindow,
        unsafe extern "system" fn(HWND) -> windows_sys::core::BOOL
    );
    foreground.expect().once().returns(1);
    show(id, true);
    show(id, false);
    minimize(id);
    maximize(id);
    restore(id);
    focus(id);
    focus_content(id);
    for unknown in [0, 999] {
        show(unknown, true);
        minimize(unknown);
        focus(unknown);
        focus_content(unknown);
        close(unknown);
        destroy(unknown);
    }
    assert_eq!(
        *commands.lock().unwrap(),
        vec![SW_SHOW, SW_HIDE, SW_MINIMIZE, SW_MAXIMIZE, SW_RESTORE]
    );
    session.restore();
    destroy(id);
}

#[test]
fn creation_failures_are_errors() {
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            WINDOW_EX_STYLE,
            PCWSTR,
            PCWSTR,
            WINDOW_STYLE,
            i32,
            i32,
            i32,
            i32,
            HWND,
            HMENU,
            HINSTANCE,
            *const core::ffi::c_void,
        ) -> HWND
    );
    create
        .expect()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| null_mut());
    let error = create_window(&WindowOptions::default()).unwrap_err();
    assert!(
        error.message.starts_with("CreateWindowExW failed"),
        "{error}"
    );
}

#[test]
fn initialization_failures_are_errors() {
    std::thread::spawn(|| {
        let mut session = Session::new();
        let startup = mock!(
            session,
            GdiplusStartup,
            unsafe extern "system" fn(
                *mut usize,
                *const GdiplusStartupInput,
                *mut windows_sys::Win32::Graphics::GdiPlus::GdiplusStartupOutput,
            ) -> windows_sys::Win32::Graphics::GdiPlus::Status
        );
        startup.expect().returning(|_, _, _| 3);
        let error = init().unwrap_err();
        assert_eq!(error.message, "GDI+ failed to start with status 3");
    })
    .join()
    .unwrap();
    std::thread::spawn(|| {
        let mut session = Session::new();
        let register = mock!(
            session,
            RegisterClassExW,
            unsafe extern "system" fn(*const WNDCLASSEXW) -> u16
        );
        register.expect().returns(0);
        let last = mock!(
            session,
            GetLastError,
            unsafe extern "system" fn() -> WIN32_ERROR
        );
        last.expect().returns(5);
        let error = crate::init().unwrap_err();
        assert_eq!(error.message, "RegisterClassExW failed with error 5");
    })
    .join()
    .unwrap();
}

#[test]
fn surfaces_follow_changes() {
    let id = window();
    let root = handle(id);
    let scale = scale(id);
    let events = record();
    apply(
        id,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::AddSurface { id: 6, parent: 42 },
            Change::UpdateSurface {
                id: 0,
                rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                content_width: 300.0,
                content_height: 600.0,
                scroll_x: 0.0,
                scroll_y: 10.0,
            },
            Change::UpdateSurface {
                id: 5,
                rect: Rect::new(10.0, 40.0, 100.0, 50.0),
                content_width: 100.0,
                content_height: 300.0,
                scroll_x: 0.0,
                scroll_y: 30.0,
            },
            Change::UpdateSurface {
                id: 9,
                rect: Rect::default(),
                content_width: 0.0,
                content_height: 0.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            Change::Paint {
                surface: 5,
                items: vec![DisplayItem::Fill {
                    rect: Rect::new(0.0, 0.0, 100.0, 300.0),
                    color: Color::rgb(255, 0, 0),
                    radii: [Corner::SQUARE; 4],
                }],
            },
            Change::Paint {
                surface: 9,
                items: Vec::new(),
            },
        ],
    );
    let child = surface_hwnd(id, 5).unwrap();
    assert_eq!(surface_hwnd(id, 6), None);
    assert_eq!(
        origin_in_root(id, child),
        (px(10.0, scale), px(30.0, scale))
    );
    assert_eq!(scroll_position(child, SB_VERT), px(30.0, scale));
    assert_eq!(scroll_position(root, SB_VERT), px(10.0, scale));
    let canvas = Canvas::new(10, 10);
    render_surface(id, 5, canvas.dc, rect(10, 10));
    assert_eq!(canvas.pixel(5, 5), (255, 0, 0));
    let printed = Canvas::new(px(100.0, scale), px(50.0, scale));
    assert_eq!(send(child, WM_PRINTCLIENT, printed.dc as usize, 0), 0);
    assert_eq!(printed.pixel(5, 5), (255, 0, 0));
    let offset = Canvas::new(px(100.0, scale) + 20, px(50.0, scale) + 20);
    // SAFETY: this is the test's live memory DC.
    unsafe {
        SetViewportOrgEx(offset.dc, 20, 20, null_mut());
        IntersectClipRect(offset.dc, 0, 0, 10, 10);
    }
    send(child, WM_PRINTCLIENT, offset.dc as usize, 0);
    assert_eq!(offset.pixel(5, 5), (255, 255, 255));
    assert_eq!(offset.pixel(25, 25), (255, 0, 0));
    assert_eq!(offset.pixel(35, 25), (255, 255, 255));
    render_surface(id, 77, canvas.dc, rect(10, 10));
    assert_eq!(canvas.pixel(5, 5), (255, 255, 255));
    assert_eq!(send(child, WM_ERASEBKGND, 0, 0), 1);
    assert_eq!(send(child, WM_PAINT, 0, 0), 0);
    assert!(scroll_events(&events).is_empty());
    apply(
        id,
        &[
            Change::RemoveSurface { id: 5 },
            Change::RemoveSurface { id: 5 },
        ],
    );
    assert!(!alive(child));
    destroy(id);
}

#[test]
fn surface_creation_failures_are_skipped() {
    let id = window();
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            WINDOW_EX_STYLE,
            PCWSTR,
            PCWSTR,
            WINDOW_STYLE,
            i32,
            i32,
            i32,
            i32,
            HWND,
            HMENU,
            HINSTANCE,
            *const core::ffi::c_void,
        ) -> HWND
    );
    create
        .expect()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| null_mut());
    apply(id, &[Change::AddSurface { id: 5, parent: 0 }]);
    session.restore();
    assert_eq!(surface_hwnd(id, 5), None);
    destroy(id);
}

fn scroll_events(events: &crate::tests::support::Events) -> Vec<(u64, f32, f32)> {
    events
        .take()
        .into_iter()
        .filter_map(|event| match event {
            Event::Scroll { surface, x, y } => Some((surface, x, y)),
            _ => None,
        })
        .collect()
}

fn scrollable(id: WindowId) -> HWND {
    apply(
        id,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::UpdateSurface {
                id: 0,
                rect: Rect::new(0.0, 0.0, 300.0, 200.0),
                content_width: 900.0,
                content_height: 1000.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            Change::UpdateSurface {
                id: 5,
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                content_width: 100.0,
                content_height: 400.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
        ],
    );
    surface_hwnd(id, 5).unwrap()
}

#[test]
fn scroll_bars_scroll_surfaces() {
    let id = window();
    let root = handle(id);
    let child = scrollable(id);
    let events = record();
    let (_, view) = {
        let area = client(child);
        (
            area.right as f32 / scale(id),
            area.bottom as f32 / scale(id),
        )
    };
    let bottom = 400.0 - view;
    for (code, expected) in [
        (SB_LINEDOWN, Some(40.0)),
        (SB_LINEUP, Some(0.0)),
        (SB_LINEUP, None),
        (SB_PAGEDOWN, Some(view)),
        (SB_PAGEUP, Some(0.0)),
        (SB_BOTTOM, Some(bottom)),
        (SB_TOP, Some(0.0)),
        (SB_ENDSCROLL, None),
        (SB_THUMBTRACK, None),
        (SB_THUMBPOSITION, None),
    ] {
        send(child, WM_VSCROLL, code as usize, 0);
        let scrolled = scroll_events(&events);
        match expected {
            Some(y) => {
                assert_eq!(scrolled.len(), 1, "{code}");
                assert!(near(scrolled[0].2, y), "{code}: {scrolled:?} {y}");
            }
            None => assert!(scrolled.is_empty(), "{code}"),
        }
    }
    send(root, WM_HSCROLL, SB_LINEDOWN as usize, 0);
    assert_eq!(scroll_events(&events), vec![(0, 40.0, 0.0)]);
    destroy(id);
}

#[test]
fn wheels_scroll_the_innermost_scrollable_surface() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let id = window();
    let root = handle(id);
    let child = scrollable(id);
    let events = record();
    let mut origin = POINT::default();
    // SAFETY: maps the child's origin to the screen.
    unsafe { ClientToScreen(child, &mut origin) };
    let at = pack(origin.x + 2, origin.y + 2);
    send(child, WM_MOUSEWHEEL, pack(0, -240) as usize, at);
    let received = events.take();
    assert!(matches!(
        received[0],
        Event::Wheel {
            delta_y: 200.0,
            delta_x: 0.0,
            ..
        }
    ));
    // One message scrolls at most the 100-pixel view, as in Chromium.
    assert_eq!(
        received[1],
        Event::Scroll {
            surface: 5,
            x: 0.0,
            y: 100.0
        }
    );
    // Wheels sent to the top-level window scroll what is under the pointer.
    send(root, WM_MOUSEWHEEL, pack(0, 120) as usize, at);
    assert_eq!(scroll_events(&events), vec![(5, 0.0, 0.0)]);
    for _ in 0..3 {
        send(child, WM_MOUSEWHEEL, pack(0, -1200) as usize, at);
    }
    assert_eq!(
        scroll_events(&events),
        vec![(5, 0.0, 100.0), (5, 0.0, 200.0), (5, 0.0, 300.0)]
    );
    send(child, WM_MOUSEWHEEL, pack(0, -120) as usize, at);
    assert_eq!(scroll_events(&events), vec![(0, 0.0, 100.0)]);
    send(root, WM_MOUSEHWHEEL, pack(0, 120) as usize, at);
    assert_eq!(scroll_events(&events), vec![(0, 100.0, 100.0)]);
    events.cancel.set(true);
    send(root, WM_MOUSEWHEEL, pack(0, -120) as usize, at);
    assert_eq!(events.take().len(), 1);
    events.cancel.set(false);
    {
        let mut session = Session::new();
        let state = mock!(session, GetKeyState, unsafe extern "system" fn(i32) -> i16);
        state
            .expect()
            .returning(|key| if key == i32::from(VK_SHIFT) { -128 } else { 0 });
        send(root, WM_MOUSEWHEEL, pack(0, -120) as usize, at);
    }
    assert_eq!(scroll_events(&events), vec![(0, 200.0, 100.0)]);
    destroy(id);
}

#[test]
fn pointer_messages_become_events() {
    let id = window();
    let root = handle(id);
    let scale = scale(id);
    apply(
        id,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::UpdateSurface {
                id: 5,
                rect: Rect::new(10.0, 20.0, 50.0, 50.0),
                content_width: 50.0,
                content_height: 50.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
        ],
    );
    let child = surface_hwnd(id, 5).unwrap();
    let events = record();
    // Focus changes would activate the test window.
    let mut session = Session::new();
    let set_focus = mock!(session, SetFocus, unsafe extern "system" fn(HWND) -> HWND);
    set_focus.expect().returning(|_| null_mut());
    let at = pack(px(10.0, scale), px(20.0, scale));
    let flags = (MK_LBUTTON | MK_SHIFT | MK_XBUTTON2) as usize;
    send(root, WM_MOUSEMOVE, flags, at);
    let expected_move = Event::Pointer {
        kind: PointerKind::Move,
        button: 0,
        buttons: 17,
        count: 0,
        x: 10.0,
        y: 20.0,
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    };
    let received = events.take();
    let Event::Pointer { x, y, .. } = received[0] else {
        panic!("{received:?}");
    };
    assert!(near(x, 10.0) && near(y, 20.0), "{x} {y}");
    assert_eq!(
        received,
        vec![match expected_move {
            Event::Pointer {
                kind,
                button,
                buttons,
                count,
                modifiers,
                ..
            } => Event::Pointer {
                kind,
                button,
                buttons,
                count,
                x,
                y,
                modifiers,
            },
            other => other,
        }]
    );
    // Windows repeats a move when windows change under a still pointer;
    // pages get no event for it, as in Chromium. A key change is new.
    send(root, WM_MOUSEMOVE, flags, at);
    assert!(events.take().is_empty());
    send(root, WM_MOUSEMOVE, flags & !(MK_SHIFT as usize), at);
    assert_eq!(events.take().len(), 1);
    let x1 = (u32::from(XBUTTON1) << 16) as usize;
    let x2 = (u32::from(XBUTTON2) << 16) as usize;
    let cases = [
        (WM_LBUTTONDOWN, 0, PointerKind::Down, 0, 1),
        (WM_LBUTTONDBLCLK, 0, PointerKind::Down, 0, 2),
        (WM_LBUTTONUP, 0, PointerKind::Up, 0, 2),
        (WM_MBUTTONDOWN, 0, PointerKind::Down, 1, 1),
        (WM_MBUTTONDBLCLK, 0, PointerKind::Down, 1, 2),
        (WM_MBUTTONUP, 0, PointerKind::Up, 1, 2),
        (WM_RBUTTONDOWN, 0, PointerKind::Down, 2, 1),
        (WM_RBUTTONDBLCLK, 0, PointerKind::Down, 2, 2),
        (WM_RBUTTONUP, 0, PointerKind::Up, 2, 2),
        // Counts are Chromium's: another button starts again, whatever
        // message brings its press.
        (WM_XBUTTONDOWN, x1, PointerKind::Down, 3, 1),
        (WM_XBUTTONDBLCLK, x2, PointerKind::Down, 4, 1),
        (WM_XBUTTONUP, x2, PointerKind::Up, 4, 1),
    ];
    super::clicks::forget();
    for (message, wparam, kind, button, count) in cases {
        send(
            child,
            message,
            wparam | MK_CONTROL as usize,
            pack(px(1.0, scale), px(1.0, scale)),
        );
        let received = events.take();
        let Event::Pointer {
            kind: got_kind,
            button: got_button,
            count: got_count,
            x,
            y,
            modifiers,
            ..
        } = received[0]
        else {
            panic!("{received:?}");
        };
        assert_eq!(
            (got_kind, got_button, got_count),
            (kind, button, count),
            "{message}"
        );
        assert!(near(x, 11.0) && near(y, 21.0), "{x} {y}");
        assert!(modifiers.ctrl);
    }
    events.cancel.set(true);
    send(root, WM_LBUTTONDOWN, 0, at);
    send(root, WM_LBUTTONUP, 0, at);
    assert_eq!(events.take().len(), 2);
    destroy(id);
}

#[test]
fn leaving_the_window() {
    let id = window();
    let root = handle(id);
    let events = record();
    let mut session = Session::new();
    let under = mock!(
        session,
        WindowFromPoint,
        unsafe extern "system" fn(POINT) -> HWND
    );
    let inside = root as usize;
    under.expect().once().returning(move |_| inside as HWND);
    under.expect().once().returning(|_| null_mut());
    send(root, WM_MOUSELEAVE, 0, 0);
    assert!(events.take().is_empty());
    send(root, WM_MOUSELEAVE, 0, 0);
    assert!(matches!(
        events.take()[..],
        [Event::Pointer {
            kind: PointerKind::Leave,
            ..
        }]
    ));
    session.restore();
    destroy(id);
}

#[test]
fn cursors() {
    let id = window();
    let root = handle(id);
    let names = [
        (Cursor::Pointer, IDC_HAND),
        (Cursor::Grab, IDC_HAND),
        (Cursor::Text, IDC_IBEAM),
        (Cursor::Move, IDC_SIZEALL),
        (Cursor::NotAllowed, IDC_NO),
        (Cursor::Wait, IDC_WAIT),
        (Cursor::Progress, IDC_APPSTARTING),
        (Cursor::Help, IDC_HELP),
        (Cursor::Crosshair, IDC_CROSS),
        (Cursor::ColResize, IDC_SIZEWE),
        (Cursor::RowResize, IDC_SIZENS),
        (Cursor::NeswResize, IDC_SIZENESW),
        (Cursor::NwseResize, IDC_SIZENWSE),
        (Cursor::Auto, IDC_ARROW),
    ];
    for (cursor, name) in names {
        assert_eq!(cursor_name(cursor), name);
    }
    let mut session = Session::new();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let seen = shown.clone();
    let set = mock!(
        session,
        SetCursor,
        unsafe extern "system" fn(HCURSOR) -> HCURSOR
    );
    set.expect().returning(move |cursor| {
        seen.lock().unwrap().push(cursor as usize);
        null_mut()
    });
    set_cursor(id, Cursor::Pointer);
    assert_eq!(
        send(root, WM_SETCURSOR, root as usize, HTCLIENT as isize),
        1
    );
    set_cursor(id, Cursor::None);
    assert_eq!(
        send(root, WM_SETCURSOR, root as usize, HTCLIENT as isize),
        1
    );
    send(root, WM_SETCURSOR, root as usize, HTCAPTION as isize);
    session.restore();
    // SAFETY: loads a shared system cursor.
    let hand = unsafe { LoadCursorW(null_mut(), IDC_HAND) } as usize;
    assert_eq!(shown.lock().unwrap()[..2], [hand, 0]);
    destroy(id);
}

#[test]
fn keys_and_window_events() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let id = window();
    let root = handle(id);
    let events = record();
    send(root, WM_KEYDOWN, 0x41, 0x1E << 16 | 1 << 30);
    send(
        root,
        WM_SYSKEYUP,
        usize::from(VK_RETURN),
        0x1C << 16 | 1 << 24,
    );
    let received = events.take();
    assert!(matches!(
        &received[0],
        Event::Key { down: true, code, repeat: true, control: None, .. } if code == "KeyA"
    ));
    assert!(matches!(
        &received[1],
        Event::Key { down: false, key, code, repeat: false, .. } if key == "Enter" && code == "NumpadEnter"
    ));
    events.cancel.set(true);
    assert_eq!(send(root, WM_KEYDOWN, usize::from(VK_ESCAPE), 0), 0);
    events.cancel.set(false);
    events.take();
    send(root, WM_SIZE, 0, 0);
    send(root, WM_SIZE, SIZE_MINIMIZED as usize, 0);
    send(root, WM_ACTIVATE, WA_INACTIVE as usize, 0);
    send(root, WM_COMMAND, 1, 0);
    let received = events.take();
    assert!(matches!(received[0], Event::Resized { .. }));
    assert_eq!(
        received[1..],
        [Event::Activated(false), Event::Menu(1.into())]
    );
    events.cancel.set(true);
    close(id);
    assert!(alive(root));
    events.cancel.set(false);
    close(id);
    assert!(!alive(root));
    let received = events.take();
    assert!(received.contains(&Event::Close));
    assert_eq!(received.last(), Some(&Event::Destroyed));
    assert!(!window_ids().contains(&id));
}

#[test]
fn typed_characters_become_text() {
    let id = window();
    let root = handle(id);
    let events = record();
    assert_eq!(send(root, WM_CHAR, 'a' as usize, 0), 0);
    send(root, WM_CHAR, 0xD83D, 0);
    send(root, WM_CHAR, 0xDE00, 0);
    send(root, WM_CHAR, 0xDC00, 0);
    send(root, WM_CHAR, 8, 0);
    events.cancel.set(true);
    send(root, WM_KEYDOWN, 0x42, 0);
    events.cancel.set(false);
    send(root, WM_CHAR, 'b' as usize, 0);
    send(root, WM_KEYUP, 0x42, 0);
    send(root, WM_CHAR, 'c' as usize, 0);
    let texts: Vec<_> = events
        .take()
        .into_iter()
        .filter_map(|event| match event {
            Event::Text(text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["a", "\u{1F600}", "c"]);
    destroy(id);
}

#[test]
fn size_limits() {
    let limited = create_window(&WindowOptions {
        min_size: Some((200.0, 100.0)),
        max_size: Some((900.0, 700.0)),
        ..WindowOptions::default()
    })
    .unwrap();
    let scale = scale(limited);
    let mut info = MINMAXINFO::default();
    send(
        handle(limited),
        WM_GETMINMAXINFO,
        0,
        &mut info as *mut MINMAXINFO as isize,
    );
    assert_eq!(info.ptMinTrackSize.x, px(200.0, scale));
    assert_eq!(info.ptMaxTrackSize.y, px(700.0, scale));
    set_size_limits(limited, None, Some((500.0, 400.0)));
    let mut info = MINMAXINFO::default();
    send(
        handle(limited),
        WM_GETMINMAXINFO,
        0,
        &mut info as *mut MINMAXINFO as isize,
    );
    assert_eq!(
        (info.ptMinTrackSize.x, info.ptMaxTrackSize.x),
        (0, px(500.0, scale))
    );
    set_size_limits(0, None, None);
    assert!(
        !is_minimized(limited) && !is_maximized(limited) && !is_minimized(0) && !is_maximized(0)
    );
    let free = window();
    let mut info = MINMAXINFO::default();
    send(
        handle(free),
        WM_GETMINMAXINFO,
        0,
        &mut info as *mut MINMAXINFO as isize,
    );
    assert_eq!(info.ptMinTrackSize.x, 0);
    destroy(limited);
    destroy(free);
}

#[test]
fn dpi_changes_rescale_everything() {
    let id = window();
    let root = handle(id);
    let control = Control {
        id: 3,
        kind: ControlKind::Button,
        rect: Rect::new(10.0, 10.0, 50.0, 20.0),
        state: ControlState {
            text: "Go".to_owned(),
            visible: true,
            ..ControlState::default()
        },
    };
    apply(
        id,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::AddControl {
                surface: 0,
                control,
            },
        ],
    );
    let suggested = RECT {
        left: 0,
        top: 0,
        right: 800,
        bottom: 600,
    };
    send(
        root,
        WM_DPICHANGED,
        pack(192, 192) as usize,
        &suggested as *const RECT as isize,
    );
    assert_eq!(scale(id), 2.0);
    let button = crate::controls::hwnd(id, 3).unwrap();
    assert_eq!(origin_in_root(id, button), (20, 20));
    destroy(id);
}

#[test]
fn unknown_windows_ignore_changes() {
    apply(
        999,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::RemoveSurface { id: 5 },
            Change::RemoveControl { id: 5 },
            Change::Order {
                surface: 0,
                controls: vec![5],
            },
        ],
    );
    assert_eq!(surface_hwnd(999, 0), None);
    assert_eq!(parent_surface(999, 3), 0);
    update_scrollbars(999, 0);
    reposition_children(999, 0);
}

fn rect(width: i32, height: i32) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    }
}

#[test]
fn carets_blink_while_the_page_has_the_focus() {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    let id = window();
    let root = hwnd(id).unwrap();
    let scale = scale(id);
    let caret = |x: f32| DisplayItem::Caret {
        rect: Rect::new(x, 2.0, 1.0, 10.0),
        color: Color::rgb(255, 0, 0),
    };
    let mut session = Session::new();
    let time = Arc::new(AtomicU32::new(500));
    let blink_time = mock!(
        session,
        GetCaretBlinkTime,
        unsafe extern "system" fn() -> u32
    );
    let current = time.clone();
    blink_time
        .expect()
        .returning(move || current.load(Ordering::SeqCst));
    let timers = Arc::new(Mutex::new(Vec::new()));
    let set = timers.clone();
    let set_timer = mock!(
        session,
        SetTimer,
        unsafe extern "system" fn(HWND, usize, u32, TIMERPROC) -> usize
    );
    set_timer.expect().returning(move |hwnd, timer, time, _| {
        set.lock().unwrap().push((hwnd as usize, timer, Some(time)));
        timer
    });
    let killed = timers.clone();
    let kill_timer = mock!(
        session,
        KillTimer,
        unsafe extern "system" fn(HWND, usize) -> windows_sys::core::BOOL
    );
    kill_timer.expect().returning(move |hwnd, timer| {
        killed.lock().unwrap().push((hwnd as usize, timer, None));
        1
    });
    let focused = Arc::new(AtomicUsize::new(root as usize));
    let get_focus = mock!(session, GetFocus, unsafe extern "system" fn() -> HWND);
    let focus = focused.clone();
    get_focus
        .expect()
        .returning(move || focus.load(Ordering::SeqCst) as HWND);
    let invalidated = Arc::new(Mutex::new(Vec::new()));
    let seen = invalidated.clone();
    let invalidate = mock!(
        session,
        InvalidateRect,
        unsafe extern "system" fn(
            HWND,
            *const RECT,
            windows_sys::core::BOOL,
        ) -> windows_sys::core::BOOL
    );
    invalidate.expect().returning(move |_, area, _| {
        // SAFETY: the caller passes no rectangle or a readable one.
        let area = unsafe { area.as_ref() };
        seen.lock()
            .unwrap()
            .push(area.map(|area| (area.left, area.top, area.right, area.bottom)));
        1
    });
    let painted = |shown: bool| {
        let canvas = Canvas::new(40, 40);
        render_surface(id, 0, canvas.dc, rect(40, 40));
        let pixel = canvas.pixel(px(4.0, scale), px(6.0, scale));
        assert_eq!(pixel == (255, 0, 0), shown);
    };
    let root_timer = |restart: bool| (root as usize, CARET_TIMER, restart.then_some(500));
    fn take<T>(list: &Mutex<Vec<T>>) -> Vec<T> {
        std::mem::take(&mut *list.lock().unwrap())
    }

    // A caret that appears shows and starts blinking.
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![caret(4.0)],
        }],
    );
    assert_eq!(take(&timers), vec![root_timer(true)]);
    painted(true);
    // Each tick flips it, repainting only around it.
    take(&invalidated);
    assert_eq!(send(root, WM_TIMER, CARET_TIMER, 0), 0);
    painted(false);
    assert_eq!(
        take(&invalidated),
        vec![Some((
            px(4.0, scale) - 2,
            px(2.0, scale) - 2,
            px(5.0, scale) + 2,
            px(12.0, scale) + 2,
        ))]
    );
    send(root, WM_TIMER, CARET_TIMER, 0);
    painted(true);
    send(root, WM_TIMER, CARET_TIMER, 0);
    // Painting the same caret keeps its phase, and repaints nothing.
    take(&invalidated);
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![caret(4.0)],
        }],
    );
    assert!(take(&invalidated).is_empty());
    painted(false);
    assert!(take(&timers).is_empty());
    // Gaining the focus shows it afresh; losing it stops the blink.
    send(root, WM_SETFOCUS, 0, 0);
    painted(true);
    assert_eq!(take(&timers), vec![root_timer(true)]);
    send(root, WM_KILLFOCUS, 0, 0);
    assert_eq!(take(&timers), vec![root_timer(false)]);
    // Without the focus it never shows.
    focused.store(0, Ordering::SeqCst);
    painted(false);
    focused.store(root as usize, Ordering::SeqCst);
    // A caret that moves restarts visible; a transformed one repaints all.
    send(root, WM_TIMER, CARET_TIMER, 0);
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                caret(4.0),
                DisplayItem::PopTransform,
            ],
        }],
    );
    painted(true);
    assert_eq!(take(&timers), vec![root_timer(true)]);
    take(&invalidated);
    send(root, WM_TIMER, CARET_TIMER, 0);
    assert_eq!(take(&invalidated), vec![None]);
    // Without blinking the caret stays.
    time.store(
        windows_sys::Win32::System::Threading::INFINITE,
        Ordering::SeqCst,
    );
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![caret(8.0)],
        }],
    );
    assert_eq!(take(&timers), vec![root_timer(false)]);
    // An added item repaints only around itself.
    take(&invalidated);
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                caret(8.0),
                DisplayItem::Fill {
                    rect: nativeboat_ir::Rect::new(20.0, 20.0, 5.0, 5.0),
                    color: nativeboat_css::Color::BLACK,
                    radii: Default::default(),
                },
            ],
        }],
    );
    assert_eq!(
        take(&invalidated),
        vec![Some((
            px(19.0, scale) - 1,
            px(19.0, scale) - 1,
            px(26.0, scale) + 1,
            px(26.0, scale) + 1,
        ))]
    );
    // Without a caret the blink stops, also when a tick comes late.
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![DisplayItem::PopTransform],
        }],
    );
    assert_eq!(take(&timers), vec![root_timer(false)]);
    send(root, WM_TIMER, CARET_TIMER, 0);
    assert_eq!(take(&timers), vec![root_timer(false)]);
    caret_phase(u64::MAX, None);
    session.restore();
    // Painting skips an empty update area.
    // SAFETY: the handle belongs to the test's live window.
    unsafe { ValidateRect(root, null()) };
    assert_eq!(send(root, WM_PAINT, 0, 0), 0);
    destroy(id);
}

#[test]
fn backdrops_repaint_the_same_in_part_as_in_whole() {
    let id = window();
    let scale = scale(id);
    let mut items: Vec<_> = (0..8)
        .map(|index| DisplayItem::Fill {
            rect: Rect::new(index as f32 * 5.0, 0.0, 5.0, 40.0),
            color: if index % 2 == 0 {
                Color::rgb(255, 0, 0)
            } else {
                Color::rgb(0, 0, 255)
            },
            radii: [Corner::SQUARE; 4],
        })
        .collect();
    items.push(DisplayItem::Backdrop {
        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
        radii: [Corner::SQUARE; 4],
        filters: vec![nativeboat_css::types::Filter::Blur(2.0)],
    });
    apply(id, &[Change::Paint { surface: 0, items }]);
    let size = px(40.0, scale);
    let whole = Canvas::new(size, size);
    render_surface(id, 0, whole.dc, rect(size, size));
    let (r, _, b) = whole.pixel(px(5.0, scale), px(20.0, scale));
    assert!(r > 40 && b > 40, "the stripes blur together: {r},{b}");
    // The blur reads pixels around a repainted area, which are painted too.
    let part = Canvas::new(size, size);
    let area = RECT {
        left: 10,
        top: 10,
        right: 20,
        bottom: 20,
    };
    render_surface(id, 0, part.dc, area);
    for y in 10..20 {
        for x in 10..20 {
            assert_eq!(part.pixel(x, y), whole.pixel(x, y), "pixel {x},{y}");
        }
    }
    destroy(id);
}

#[test]
fn backdrops_above_child_surfaces_repaint_them_the_same_in_part() {
    let id = window();
    let scale = scale(id);
    let stripes: Vec<_> = (0..8)
        .map(|index| DisplayItem::Fill {
            rect: Rect::new(index as f32 * 5.0, 0.0, 5.0, 40.0),
            color: if index % 2 == 0 {
                Color::rgb(255, 0, 0)
            } else {
                Color::rgb(0, 0, 255)
            },
            radii: [Corner::SQUARE; 4],
        })
        .collect();
    let blur = |radius| DisplayItem::Backdrop {
        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
        radii: [Corner::SQUARE; 4],
        filters: vec![nativeboat_css::types::Filter::Blur(radius)],
    };
    apply(
        id,
        &[
            Change::AddSurface { id: 1, parent: 0 },
            Change::UpdateSurface {
                id: 1,
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                content_width: 40.0,
                content_height: 40.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
            // A backdrop below the child filters only the window; the one
            // painted above it, as a modal's over a scrolling page, filters
            // the child's stripes.
            Change::Paint {
                surface: 0,
                items: vec![
                    blur(9.0),
                    DisplayItem::PushClip {
                        rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                        radii: [Corner::SQUARE; 4],
                    },
                    DisplayItem::Child(NativeChild::Surface(1)),
                    blur(2.0),
                    DisplayItem::PopClip,
                ],
            },
            Change::Paint {
                surface: 1,
                items: stripes,
            },
        ],
    );
    let reach = STATE.with_borrow(|state| {
        let data = &state.windows[&id];
        stack::backdrop_reach(data, NativeChild::Surface(1))
    });
    assert_eq!(reach, render::backdrop_reach(&[blur(2.0)], scale));
    assert_eq!(backdrop_reach(id, 1), reach);
    let size = px(40.0, scale);
    let whole = Canvas::new(size, size);
    render_surface(id, 1, whole.dc, rect(size, size));
    let (r, _, b) = whole.pixel(px(5.0, scale), px(20.0, scale));
    assert!(r > 40 && b > 40, "the stripes blur together: {r},{b}");
    // The child paints as far as the blur above it reads around an area.
    let part = Canvas::new(size, size);
    let area = RECT {
        left: 10,
        top: 10,
        right: 20,
        bottom: 20,
    };
    render_surface(id, 1, part.dc, area);
    for y in 10..20 {
        for x in 10..20 {
            assert_eq!(part.pixel(x, y), whole.pixel(x, y), "pixel {x},{y}");
        }
    }
    destroy(id);
}

#[test]
fn before_input_captures_locations_locks_composition_and_owning_window() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let root = hwnd(window).unwrap();
    events.take();
    assert!(!before_input(&MSG::default()));
    assert!(!before_input(&MSG {
        message: WM_KEYDOWN,
        ..MSG::default()
    }));
    let foreign_class = wide("STATIC");
    // SAFETY: this test owns the temporary foreign window; it has no app data.
    let foreign = unsafe {
        CreateWindowExW(
            0,
            foreign_class.as_ptr(),
            null(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!foreign.is_null());
    assert!(!before_input(&MSG {
        hwnd: foreign,
        message: WM_KEYDOWN,
        wParam: usize::from(VK_F5),
        ..MSG::default()
    }));
    // SAFETY: closes only this test's temporary foreign window.
    unsafe { DestroyWindow(foreign) };
    // SAFETY: this child and its parent are owned by this test thread.
    let child = unsafe {
        CreateWindowExW(
            0,
            foreign_class.as_ptr(),
            null(),
            WS_CHILD,
            0,
            0,
            0,
            0,
            root,
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!child.is_null());
    assert!(!before_input(&MSG {
        hwnd: child,
        message: WM_KEYDOWN,
        wParam: usize::from(VK_F5),
        ..MSG::default()
    }));
    assert!(matches!(events.take().as_slice(),[Event::BeforeInput(input)] if input.code=="F5"));
    // Hosts with their own message loops read keys of children as their
    // window's without dispatching them.
    let (owner, input) =
        keyboard_input(child as usize, WM_KEYUP, usize::from(VK_F5), 0x3F << 16).unwrap();
    assert_eq!(
        (owner, input.code.as_str(), input.down),
        (window, "F5", false)
    );
    assert!(events.take().is_empty());
    assert!(keyboard_input(child as usize, WM_CHAR, usize::from(b'a'), 0).is_none());
    // SAFETY: this scope owns the child.
    unsafe { DestroyWindow(child) };
    for (vk, scan, extended, expected_code, location) in [
        (VK_RETURN, 0x1C, false, "Enter", 0),
        (VK_CONTROL, 0x1D, false, "ControlLeft", 1),
        (VK_CONTROL, 0x1D, true, "ControlRight", 2),
        (VK_RETURN, 0x1C, true, "NumpadEnter", 3),
        (VK_LEFT, 0x4B, false, "Numpad4", 3),
        (VK_LEFT, 0x4B, true, "ArrowLeft", 0),
    ] {
        assert!(!before_input(&MSG {
            hwnd: root,
            message: WM_KEYDOWN,
            wParam: usize::from(vk),
            lParam: (scan << 16) | if extended { 1 << 24 } else { 0 },
            ..MSG::default()
        }));
        let received = events.take();
        let Event::BeforeInput(input) = &received[0] else {
            panic!("{received:?}")
        };
        assert_eq!(
            (input.code.as_str(), input.location),
            (expected_code, location)
        );
        assert!(input.down);
        assert!(!input.repeat && !input.composing && !input.caps_lock && !input.num_lock);
    }
    let mut session = Session::new();
    let state = mock!(session, GetKeyState, unsafe extern "system" fn(i32) -> i16);
    state.expect().returning(|key| {
        if [VK_CAPITAL, VK_NUMLOCK].contains(&(key as u16)) {
            1
        } else if [VK_SHIFT, VK_CONTROL, VK_MENU, VK_RWIN].contains(&(key as u16)) {
            i16::MIN
        } else {
            0
        }
    });
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_SYSKEYDOWN,
        wParam: usize::from(VK_F5),
        lParam: 1 << 30,
        ..MSG::default()
    }));
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_SYSKEYUP,
        wParam: usize::from(VK_F5),
        lParam: 1 << 30,
        ..MSG::default()
    }));
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_KEYUP,
        wParam: usize::from(VK_PROCESSKEY),
        ..MSG::default()
    }));
    let received = events.take();
    for (index, event) in received.iter().enumerate() {
        let Event::BeforeInput(input) = event else {
            panic!("{event:?}")
        };
        assert_eq!(input.down, index == 0);
        assert_eq!(input.repeat, index == 0);
        assert!(!input.composing);
        assert!(input.caps_lock && input.num_lock);
        assert_eq!(
            input.modifiers,
            Modifiers {
                shift: true,
                ctrl: true,
                alt: true,
                meta: true
            }
        );
    }
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    events.take();
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_KEYDOWN,
        wParam: usize::from(VK_F5),
        ..MSG::default()
    }));
    assert!(matches!(events.take().as_slice(),[Event::BeforeInput(input)] if input.composing));
    send(root, WM_IME_ENDCOMPOSITION, 0, 0);
    events.take();
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_KEYDOWN,
        wParam: usize::from(VK_F5),
        ..MSG::default()
    }));
    assert!(matches!(events.take().as_slice(),[Event::BeforeInput(input)] if !input.composing));
    destroy(window);
}

#[test]
fn ctrl_alt_glyphs_reach_both_before_input_and_page_keys() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let root = hwnd(window).unwrap();
    events.take();
    let mut session = Session::new();
    mock!(
        session,
        GetKeyboardState,
        unsafe extern "system" fn(*mut u8) -> windows_sys::core::BOOL
    )
    .expect()
    .returning(|keys| {
        // SAFETY: the adapter supplies a 256-byte output buffer.
        unsafe {
            *keys.add(usize::from(VK_CONTROL)) = 0x80;
            *keys.add(usize::from(VK_MENU)) = 0x80;
        }
        1
    });
    mock!(session, GetKeyState, unsafe extern "system" fn(i32) -> i16)
        .expect()
        .returning(|key| {
            if key == i32::from(VK_CONTROL) || key == i32::from(VK_MENU) {
                i16::MIN
            } else {
                0
            }
        });
    mock!(
        session,
        ToUnicodeEx,
        unsafe extern "system" fn(u32, u32, *const u8, *mut u16, i32, u32, HKL) -> i32
    )
    .expect()
    .times(2)
    .returning(|vk, scan, keys, output, _, flags, _| {
        assert_eq!((vk, scan, flags), (0x51, 0x10, 4));
        // SAFETY: the adapter supplies a 256-byte state and 8-unit output.
        unsafe {
            assert_eq!(*keys.add(usize::from(VK_CONTROL)), 0x80);
            assert_eq!(*keys.add(usize::from(VK_MENU)), 0x80);
            *output = u16::from(b'@');
        }
        1
    });
    assert!(!before_input(&MSG {
        hwnd: root,
        message: WM_SYSKEYDOWN,
        wParam: 0x51,
        lParam: 0x10 << 16,
        ..MSG::default()
    }));
    assert!(!key(window, None, WM_SYSKEYDOWN, 0x51, 0x10 << 16));
    let received = events.take();
    assert!(
        matches!(&received[0],Event::BeforeInput(input) if input.key=="@"&&input.code=="KeyQ"&&input.modifiers.ctrl&&input.modifiers.alt)
    );
    assert!(
        matches!(&received[1],Event::Key {key,code,modifiers,..} if key=="@"&&code=="KeyQ"&&modifiers.ctrl&&modifiers.alt)
    );
    destroy(window);
}

std::thread_local! {
    /// The messages the host procedure of an adopted window received.
    static HOST_MESSAGES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    HOST_MESSAGES.with_borrow_mut(|messages| messages.push(message));
    // The host decides about closing, as Electron hosts do.
    if message == WM_CLOSE {
        return 0;
    }
    // Hosts may still message a window they destroy.
    if message == WM_DESTROY {
        send(hwnd, WM_USER + 1, 0, 0);
    }
    // SAFETY: forwards the message unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[test]
fn adopted_windows_route_content_to_the_page_and_the_frame_to_their_host() {
    init().unwrap();
    let class = WNDCLASSW {
        lpfnWndProc: Some(host_proc),
        hInstance: instance(),
        lpszClassName: w!("NativeBoatAdoptHost"),
        ..WNDCLASSW::default()
    };
    // SAFETY: the class structure and its static name outlive the call.
    unsafe { RegisterClassW(&class) };
    // SAFETY: the class is registered above.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            w!("NativeBoatAdoptHost"),
            w!("host"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            300,
            200,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!hwnd.is_null());
    let events = record();
    // SAFETY: the window was just created on this thread.
    let window = unsafe { adopt(hwnd) }.unwrap();
    assert_eq!(self::hwnd(window), Some(hwnd));
    // Its own painting leaves its child windows alone.
    // SAFETY: the window is live.
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    assert_ne!(style & WS_CLIPCHILDREN, 0);
    let host = |expected: u32| {
        HOST_MESSAGES.with_borrow_mut(|messages| {
            let found = messages.contains(&expected);
            messages.clear();
            found
        })
    };
    HOST_MESSAGES.with_borrow_mut(Vec::clear);

    // Pointer input is the page's.
    send(hwnd, WM_LBUTTONDOWN, MK_LBUTTON as usize, pack(10, 20));
    send(hwnd, WM_LBUTTONUP, 0, pack(10, 20));
    assert!(!host(WM_LBUTTONDOWN));
    assert!(events.take().iter().any(|event| matches!(
        event,
        Event::Pointer {
            kind: PointerKind::Down,
            ..
        }
    )));
    // Closing and menu commands are the host's.
    send(hwnd, WM_CLOSE, 0, 0);
    assert!(host(WM_CLOSE));
    assert!(alive(hwnd));
    send(hwnd, WM_COMMAND, 7, 0);
    assert!(host(WM_COMMAND));
    assert!(events.take().is_empty());
    // Sizes reach both.
    send(hwnd, WM_SIZE, 0, pack(300, 200));
    assert!(host(WM_SIZE));
    assert!(
        events
            .take()
            .iter()
            .any(|event| matches!(event, Event::Resized { .. }))
    );
    // What the page leaves alone goes to the host.
    send(hwnd, WM_USER, 0, 0);
    assert!(host(WM_USER));
    // Destroying the window forgets it, and its host still finishes.
    // SAFETY: the window belongs to this thread.
    unsafe { DestroyWindow(hwnd) };
    assert!(HOST_MESSAGES.with_borrow(|messages| messages.contains(&(WM_USER + 1))));
    assert!(host(WM_NCDESTROY));
    assert!(events.take().contains(&Event::Destroyed));
    assert_eq!(self::hwnd(window), None);
    assert!(HOSTS.with_borrow(|hosts| !hosts.contains_key(&(hwnd as usize))));
}

#[test]
fn transparent_windows_paint_what_covers_the_desktop_premultiplied() {
    // What the page paints over black and over white gives each pixel's
    // alpha, and its color over black is its premultiplied color.
    assert_eq!(see_through(0x0000_0000, 0x00ff_ffff), 0);
    assert_eq!(see_through(0x00ff_0000, 0x00ff_0000), 0xffff_0000);
    assert_eq!(see_through(0x0000_0080, 0x007f_7fff), 0x8000_0080);
    // A channel brighter than the average alpha allows is held to it.
    assert_eq!(see_through(0x0000_00c0, 0x007f_7fff), 0x9500_0095);

    init().unwrap();
    let class = WNDCLASSW {
        lpfnWndProc: Some(host_proc),
        hInstance: instance(),
        lpszClassName: w!("NativeBoatTransparentHost"),
        ..WNDCLASSW::default()
    };
    // SAFETY: the class structure and its static name outlive the call.
    unsafe { RegisterClassW(&class) };
    // SAFETY: the class is registered above.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            w!("NativeBoatTransparentHost"),
            w!("host"),
            WS_POPUP,
            0,
            0,
            100,
            50,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!hwnd.is_null());
    // SAFETY: the window is live and the name static.
    unsafe { SetPropW(hwnd, w!("NativeboatTransparent"), 1 as _) };
    // SAFETY: the window was just created on this thread.
    let id = unsafe { adopt(hwnd) }.unwrap();
    assert!(is_transparent(id));
    let opaque = window();
    assert!(!is_transparent(opaque));
    destroy(opaque);

    let scale = scale(id);
    let fill = |x: f32, color: Color| DisplayItem::Fill {
        rect: Rect::new(x, 0.0, 10.0, 10.0),
        color,
        radii: [Corner::SQUARE; 4],
    };
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                fill(0.0, Color::rgb(255, 0, 0)),
                fill(10.0, Color::rgba(0, 0, 255, 0.5)),
            ],
        }],
    );
    let (width, height) = (px(40.0, scale), px(10.0, scale));
    let canvas = Canvas::new(width, height);
    paint_into(id, 0, canvas.dc, rect(width, height));
    let at = |x: f32| canvas.bits(px(x, scale), px(5.0, scale));
    assert_eq!(at(5.0), 0xffff_0000);
    let half = at(15.0);
    assert!(
        (126..=129).contains(&(half >> 24)) && half & 0xff == half >> 24,
        "half covering blue: {half:#x}"
    );
    assert_eq!(at(30.0), 0);
    // SAFETY: the window belongs to this thread.
    unsafe { DestroyWindow(hwnd) };
}

#[test]
fn translucent_surfaces_show_what_lies_beneath_them() {
    let id = window();
    let fill = |rect: Rect, color: Color| DisplayItem::Fill {
        rect,
        color,
        radii: [Corner::SQUARE; 4],
    };
    let update = |surface: u64, rect: Rect| Change::UpdateSurface {
        id: surface,
        rect,
        content_width: rect.width,
        content_height: rect.height,
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
    apply(
        id,
        &[
            Change::AddSurface { id: 1, parent: 0 },
            Change::AddSurface { id: 2, parent: 0 },
            update(0, Rect::new(0.0, 0.0, 100.0, 100.0)),
            // An editor at the left, then a full-window overlay over it.
            update(1, Rect::new(0.0, 0.0, 50.0, 100.0)),
            update(2, Rect::new(0.0, 0.0, 100.0, 100.0)),
            // The window paints the overlay's dimming background after the
            // editor and before the overlay's own surface.
            Change::Paint {
                surface: 0,
                items: vec![
                    fill(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(255, 0, 0)),
                    DisplayItem::Child(nativeboat_ir::NativeChild::Surface(1)),
                    fill(
                        Rect::new(0.0, 0.0, 100.0, 100.0),
                        Color {
                            r: 0,
                            g: 0,
                            b: 0,
                            a: 0.5,
                        },
                    ),
                    DisplayItem::Child(nativeboat_ir::NativeChild::Surface(2)),
                ],
            },
            Change::Paint {
                surface: 1,
                items: vec![fill(
                    Rect::new(0.0, 0.0, 50.0, 100.0),
                    Color::rgb(0, 0, 255),
                )],
            },
            Change::Paint {
                surface: 2,
                items: vec![DisplayItem::Beneath],
            },
        ],
    );
    let scale = scale(id);
    let canvas = Canvas::new(px(100.0, scale), px(100.0, scale));
    render_surface(id, 2, canvas.dc, rect(px(100.0, scale), px(100.0, scale)));
    // Over the editor, the overlay darkens its blue once; beside it, the red.
    let (r, g, b) = canvas.pixel(px(20.0, scale), px(50.0, scale));
    assert!(
        r < 20 && g < 20 && (110..150).contains(&b),
        "{:?}",
        (r, g, b)
    );
    let (r, g, b) = canvas.pixel(px(80.0, scale), px(50.0, scale));
    assert!(
        (110..150).contains(&r) && g < 20 && b < 20,
        "{:?}",
        (r, g, b)
    );
    destroy(id);
}

#[test]
fn zoom_multiplies_the_dpi_scale_and_reports_the_new_size() {
    let id = window();
    let events = record();
    let dpi = scale(id);
    assert_eq!(zoom(id), 1.0);
    set_zoom(id, 2.0);
    assert_eq!((zoom(id), scale(id)), (2.0, dpi * 2.0));
    assert!(
        events
            .take()
            .iter()
            .any(|event| matches!(event, Event::Resized { .. }))
    );
    // Unchanged or invalid zooms change nothing.
    set_zoom(id, 2.0);
    set_zoom(id, 0.0);
    set_zoom(id, f32::NAN);
    assert!(events.take().is_empty());
    set_zoom(id, 1.0);
    assert_eq!(scale(id), dpi);
    set_zoom(999, 2.0);
    assert_eq!(zoom(999), 1.0);
    destroy(id);
}

#[test]
fn surface_views_become_window_regions_and_alpha() {
    let id = window();
    let scale = scale(id);
    apply(
        id,
        &[
            Change::AddSurface { id: 5, parent: 0 },
            Change::UpdateSurface {
                id: 5,
                rect: Rect::new(10.0, 10.0, 100.0, 80.0),
                content_width: 100.0,
                content_height: 300.0,
                scroll_x: 0.0,
                scroll_y: 0.0,
            },
        ],
    );
    let surface = surface_hwnd(id, 5).unwrap();
    // Child windows take an alpha only where the executable's manifest
    // names Windows 8 or later, as applications' do and tests' do not: the
    // request is what is checked.
    let alphas = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::new();
    let layered = mock!(
        session,
        SetLayeredWindowAttributes,
        unsafe extern "system" fn(
            HWND,
            windows_sys::Win32::Foundation::COLORREF,
            u8,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS,
        ) -> windows_sys::core::BOOL
    );
    let seen = alphas.clone();
    let expected = surface as usize;
    layered.expect().returning(move |window, _, alpha, flags| {
        assert_eq!(window as usize, expected);
        assert_eq!(flags, LWA_ALPHA);
        seen.lock().unwrap().push(alpha);
        1
    });
    apply(
        id,
        &[Change::SurfaceView {
            surface: 5,
            clip: Some(Rect::new(0.0, 0.0, 100.0, 30.0)),
            opacity: Some(0.5),
        }],
    );
    session.restore();
    assert_eq!(*alphas.lock().unwrap(), [128]);
    let mut area = RECT::default();
    // SAFETY: reads the region of the test's own surface window.
    assert_ne!(unsafe { GetWindowRgnBox(surface, &mut area) }, 0);
    assert_eq!(
        (area.right, area.bottom),
        (px(100.0, scale), px(30.0, scale))
    );
    apply(
        id,
        &[Change::SurfaceView {
            surface: 5,
            clip: None,
            opacity: None,
        }],
    );
    // SAFETY: as above.
    unsafe {
        assert_eq!(GetWindowRgnBox(surface, &mut area), 0);
        assert_eq!(
            GetWindowLongW(surface, GWL_EXSTYLE) as u32 & WS_EX_LAYERED,
            0
        );
    }
    destroy(id);
}

#[test]
fn changes_repaint_only_the_children_under_them() {
    // Typing in an editor repaints its line, with the parts of child
    // surfaces under it, not every child surface in it.
    let mut session = Session::new();
    let redrawn = Arc::new(Mutex::new(Vec::new()));
    let seen = redrawn.clone();
    let redraw = mock!(
        session,
        RedrawWindow,
        unsafe extern "system" fn(HWND, *const RECT, HRGN, u32) -> windows_sys::core::BOOL
    );
    redraw.expect().returning(move |_, area, _, flags| {
        // SAFETY: the caller passes no rectangle or a readable one.
        let area = unsafe { area.as_ref() };
        seen.lock().unwrap().push((
            area.map(|area| (area.left, area.top, area.right, area.bottom)),
            flags,
        ));
        1
    });
    let id = window();
    let scale = scale(id);
    let fill = |width: f32| DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, width, 10.0),
        color: Color::rgb(255, 0, 0),
        radii: [Corner::SQUARE; 4],
    };
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![fill(20.0)],
        }],
    );
    redrawn.lock().unwrap().clear();
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![fill(30.0)],
        }],
    );
    let calls = core::mem::take(&mut *redrawn.lock().unwrap());
    assert_eq!(
        calls,
        vec![(
            Some((
                px(-1.0, scale) - 1,
                px(-1.0, scale) - 1,
                px(31.0, scale) + 1,
                px(11.0, scale) + 1
            )),
            RDW_INVALIDATE | RDW_ALLCHILDREN
        )]
    );
    // A change of unknown extent repaints all.
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                fill(30.0),
            ],
        }],
    );
    assert_eq!(
        core::mem::take(&mut *redrawn.lock().unwrap()),
        vec![(None, RDW_INVALIDATE | RDW_ALLCHILDREN)]
    );
    destroy(id);
    session.restore();
}

#[test]
#[ignore = "manual timing: prints what painting a window of text costs"]
fn full_window_text_paint_timing() {
    let id = window();
    set_client_size(id, 1500.0, 1000.0);
    let hwnd = handle(id);
    // Shown without taking the focus from whatever has it.
    // SAFETY: shows this test's own window.
    unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    let font = nativeboat_ir::Font {
        families: vec!["Segoe UI".into()],
        size: 16.0,
        weight: 400,
        italic: false,
        features: Vec::new(),
    };
    let line = |y: f32| DisplayItem::Text {
        rect: Rect::new(10.0, y, 1400.0, 20.0),
        text: "The quick brown fox jumps over the lazy dog. ".repeat(4),
        font: font.clone(),
        color: Color::rgb(0, 0, 0),
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: false,
        underline_offset: None,
    };
    let fill = DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, 1500.0, 1000.0),
        color: Color::rgb(255, 255, 255),
        radii: [Corner::SQUARE; 4],
    };
    for (name, items) in [
        ("fill", vec![fill.clone()]),
        (
            "text",
            core::iter::once(fill)
                .chain((0..45).map(|row| line(row as f32 * 22.0)))
                .collect(),
        ),
    ] {
        apply(id, &[Change::Paint { surface: 0, items }]);
        for round in 0..4 {
            let started = std::time::Instant::now();
            // SAFETY: repaints this test's own window now.
            unsafe {
                RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_UPDATENOW);
            }
            eprintln!("{name} {round}: {:?}", started.elapsed());
        }
    }
    destroy(id);
}

#[test]
fn surfaces_showing_an_image_repaint_when_it_arrives() {
    let id = window();
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![DisplayItem::Image {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                url: "https://images.example/arriving.bmp".into(),
            }],
        }],
    );
    let surface = STATE.with_borrow(|state| state.windows[&id].surfaces[&0].hwnd);
    let mut session = Session::new();
    let invalidated = Arc::new(Mutex::new(Vec::new()));
    let seen = invalidated.clone();
    let invalidate = mock!(
        session,
        InvalidateRect,
        unsafe extern "system" fn(
            HWND,
            *const RECT,
            windows_sys::core::BOOL,
        ) -> windows_sys::core::BOOL
    );
    invalidate.expect().returning(move |hwnd, area, _| {
        seen.lock().unwrap().push((hwnd as usize, area.is_null()));
        1
    });
    let bitmap = crate::render::tests::bitmap_file();
    crate::add_image("https://images.example/elsewhere.bmp", &bitmap);
    assert_eq!(
        *invalidated.lock().unwrap(),
        [],
        "an image no surface shows repaints nothing"
    );
    crate::add_image("https://images.example/arriving.bmp", &bitmap);
    assert_eq!(
        *invalidated.lock().unwrap(),
        [(surface as usize, true)],
        "the surface showing the image repaints whole"
    );
    session.restore();
    destroy(id);
}
