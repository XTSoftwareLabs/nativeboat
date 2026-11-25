use super::*;
use crate::app_host::{
    CreateWindowExW, Hinstance, NativeMenuItem, WindowEvent, create, destroy_window,
    lock_host_tests, set_window_menu,
};

fn options(title: &[u16]) -> WindowOptions<'_> {
    WindowOptions {
        show: false,
        ..WindowOptions::new(title)
    }
}

fn close(window: Window) {
    destroy_window(window).unwrap();
}

/// The device pixels of a window's client area.
fn device_client(window: Window) -> (i32, i32) {
    let mut rect = WinRect::default();
    // SAFETY: `rect` is writable.
    unsafe { GetClientRect(window.0 as Hwnd, &mut rect) };
    (rect.right, rect.bottom)
}

fn near(actual: i32, expected: i32) -> bool {
    (actual - expected).abs() <= 1
}

#[test]
fn windows_measure_in_device_independent_pixels() {
    let _lock = lock_host_tests();
    let window = create(&WindowOptions {
        position: Some((40.0, 50.0)),
        size: (400.0, 300.0),
        minimum: (500.0, 0.0),
        maximum: (0.0, 250.0),
        ..options(&[])
    })
    .unwrap();
    let scale = scale(window);
    assert!(scale >= 1.0);
    // Clamped to the minimum width and maximum height.
    let (x, y, width, height) = window_bounds(window).unwrap();
    assert!(near(x, 40) && near(y, 50), "{x} {y}");
    assert!(near(width, 500) && near(height, 250), "{width} {height}");
    assert!(has_frame(window));
    let (_, _, content_width, content_height) = content_bounds(window).unwrap();
    assert!(content_width < width && content_height < height);
    let (client_width, client_height) = client_size(window).unwrap();
    assert!((client_width - content_width as f32).abs() <= 1.0);
    assert!((client_height - content_height as f32).abs() <= 1.0);
    assert_eq!(size_limits(window), ((500.0, 0.0), (0.0, 250.0)));
    set_size_limits(window, (f64::NAN, -1.0), (900.0, 700.0));
    assert_eq!(size_limits(window), ((0.0, 0.0), (900.0, 700.0)));

    set_window_bounds(window, 60.0, 70.0, 320.0, 240.0);
    let (x, y, width, height) = window_bounds(window).unwrap();
    assert!(near(x, 60) && near(y, 70) && near(width, 320) && near(height, 240));
    set_content_size(window, 300.0, 200.0);
    let (_, _, width, height) = content_bounds(window).unwrap();
    assert!(near(width, 300) && near(height, 200), "{width} {height}");
    center(window);
    let (x, _, _, _) = window_bounds(window).unwrap();
    assert!(x > 60);

    assert!(is_resizable(window));
    set_resizable(window, false);
    assert!(!is_resizable(window));
    set_resizable(window, true);
    assert!(is_resizable(window));
    close(window);

    // Content-sized windows grow by their frame; centered ones sit on the
    // primary work area.
    let window = create(&WindowOptions {
        size: (300.0, 200.0),
        content_size: true,
        resizable: false,
        ..options(&[])
    })
    .unwrap();
    let (_, _, width, height) = content_bounds(window).unwrap();
    assert!(near(width, 300) && near(height, 200), "{width} {height}");
    assert!(!is_resizable(window));
    close(window);
}

#[test]
fn windows_take_the_pixels_electron_gives_them_at_fractional_scales() {
    // As measured in Electron 38 at 125%, on a 1920×1030 work area.
    let scale = 1.25;
    let work = WinRect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1030,
    };
    let rect = |left, top, width, height| WinRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    // 800×450 is 1000×562.5 device pixels, rounded up and centered.
    let size = (ceiled(800.0, scale), ceiled(450.0, scale));
    assert_eq!(size, (1000, 563));
    let plain = placed(size, work, None, false, scale);
    assert_eq!(plain, rect(460, 233, 1000, 563));
    assert_eq!(logical(plain, scale), (368, 186, 800, 451));
    // `center` reads the size back as 451 and makes it 564.
    let centered_again = placed(size, work, None, true, scale);
    assert_eq!(centered_again, rect(460, 233, 1000, 564));
    assert_eq!(logical(centered_again, scale), (368, 186, 800, 452));
    let odd = (ceiled(801.0, scale), ceiled(451.0, scale));
    let odd = placed(odd, work, None, true, scale);
    assert_eq!(odd, rect(458, 232, 1003, 565));
    assert_eq!(logical(odd, scale), (366, 185, 803, 453));
    // A position moves it twice, each time at its size read back.
    let moved = placed(size, work, Some((100, 100)), false, scale);
    assert_eq!(moved, rect(125, 125, 1000, 565));
    assert_eq!(logical(moved, scale), (100, 100, 800, 452));
    let moved = placed(size, work, Some((101, 101)), false, scale);
    assert_eq!(moved, rect(126, 126, 1003, 566));
    assert_eq!(logical(moved, scale), (100, 100, 804, 454));
    let small = (ceiled(640.0, scale), ceiled(361.0, scale));
    let small = placed(small, work, None, false, scale);
    assert_eq!(small, rect(560, 289, 800, 452));
    assert_eq!(logical(small, scale), (448, 231, 640, 362));
    // Windows larger than the work area fill it.
    assert_eq!(centered(work, (3000, 2000)), work);
    // An empty span covers no pixels.
    assert_eq!(device_rect((10, 10, 0, 0), scale), rect(12, 12, 0, 0));
}

#[test]
fn transparent_topmost_and_taskbar_less_windows() {
    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetPropW(window: Hwnd, name: *const u16) -> *mut c_void;
    }
    let marked = |window: Window| {
        let name: Vec<u16> = "NativeboatTransparent".encode_utf16().chain([0]).collect();
        // SAFETY: the name is NUL-terminated.
        !unsafe { GetPropW(window.0 as Hwnd, name.as_ptr()) }.is_null()
    };
    let _lock = lock_host_tests();
    // Out of sight, so that showing it covers nothing.
    let window = create(&WindowOptions {
        position: Some((-3000.0, -3000.0)),
        size: (64.0, 64.0),
        frame: false,
        transparent: true,
        always_on_top: true,
        skip_taskbar: true,
        background: [1, 2, 3],
        ..options(&[])
    })
    .unwrap();
    let hwnd = window.0 as Hwnd;
    // SAFETY: the window is live.
    let extended = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    assert_ne!(extended & 0x8, 0, "topmost");
    assert!(marked(window));
    let frame = frame(hwnd).unwrap();
    // It erases to black, which its extended frame shows through.
    assert_eq!(frame.background.load(Ordering::Acquire), 0);
    assert!(frame.skip_taskbar.load(Ordering::Acquire));
    // Showing it takes away the button the taskbar gives it; the size and
    // move messages still follow.
    let placement = WindowPos {
        window: hwnd,
        after: core::ptr::null_mut(),
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        flags: SWP_SHOWWINDOW,
    };
    assert_eq!(
        message(
            hwnd,
            WM_WINDOWPOSCHANGED,
            0,
            &placement as *const _ as isize
        ),
        None
    );
    // While it shows, the button comes and goes at once.
    const SW_SHOWNOACTIVATE: i32 = 4;
    // SAFETY: the window is live.
    unsafe { super::super::ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    super::super::set_skip_taskbar(window, false);
    assert!(!frame.skip_taskbar.load(Ordering::Acquire));
    super::super::hide_window(window).unwrap();
    super::super::set_skip_taskbar(window, true);
    assert!(frame.skip_taskbar.load(Ordering::Acquire));
    super::super::set_skip_taskbar(Window(0x7fff_fff0), true);
    close(window);
    // A framed window is never transparent.
    let framed = create(&WindowOptions {
        transparent: true,
        ..options(&[])
    })
    .unwrap();
    assert!(!marked(framed));
    // SAFETY: the window is live.
    let extended = unsafe { GetWindowLongPtrW(framed.0 as Hwnd, GWL_EXSTYLE) } as u32;
    assert_eq!(extended & 0x8, 0);
    close(framed);
}

#[test]
fn stale_windows_answer_defaults() {
    let _lock = lock_host_tests();
    let stale = Window(0x7fff_fff0);
    assert_eq!(scale(stale), 1.0);
    assert!(has_frame(stale));
    assert_eq!(window_bounds(stale), None);
    assert_eq!(content_bounds(stale), None);
    assert_eq!(client_size(stale), None);
    assert_eq!(size_limits(stale), ((0.0, 0.0), (0.0, 0.0)));
    set_size_limits(stale, (1.0, 1.0), (2.0, 2.0));
    center(stale);
    assert!(!is_resizable(stale));
    assert_eq!(message(stale.0 as Hwnd, WM_NCHITTEST, 0, 0), None);
}

#[test]
fn frameless_windows_are_all_client_area_with_their_own_borders_and_drag_regions() {
    let _lock = lock_host_tests();
    let window = create(&WindowOptions {
        position: Some((100.0, 100.0)),
        size: (400.0, 300.0),
        content_size: true,
        frame: false,
        background: [1, 2, 3],
        ..options(&[])
    })
    .unwrap();
    let hwnd = window.0 as Hwnd;
    assert!(!has_frame(window));
    let (x, y, width, height) = window_bounds(window).unwrap();
    assert_eq!(content_bounds(window), Some((x, y, width, height)));
    assert!(near(width, 400) && near(height, 300));
    // No menu bar.
    set_window_menu(
        window,
        &[NativeMenuItem {
            id: 1,
            label: "File".encode_utf16().chain([0]).collect(),
            separator: false,
            submenu: false,
            enabled: true,
            checked: false,
            radio: false,
            accelerator: None,
            shortcut: None,
            children: Vec::new(),
        }],
    )
    .unwrap();
    // SAFETY: GetMenu accepts any HWND.
    assert!(unsafe { GetMenu(hwnd) }.is_null());
    set_content_size(window, 420.0, 320.0);
    let (_, _, width, _) = window_bounds(window).unwrap();
    assert!(near(width, 420));

    let (right, bottom) = device_client(window);
    // A point of the client area, in screen coordinates as Windows sends it.
    let hit = |x: i32, y: i32| {
        let mut origin = Point::default();
        // SAFETY: `origin` is writable.
        unsafe { ClientToScreen(hwnd, &mut origin) };
        let (x, y) = (origin.x + x, origin.y + y);
        message(
            hwnd,
            WM_NCHITTEST,
            0,
            ((y as u16 as isize) << 16) | x as u16 as isize,
        )
    };
    let middle = (right / 2, bottom / 2);
    assert_eq!(hit(0, 0), Some(HTTOPLEFT));
    assert_eq!(hit(middle.0, 0), Some(HTTOP));
    assert_eq!(hit(right - 1, 0), Some(HTTOPRIGHT));
    assert_eq!(hit(0, middle.1), Some(HTLEFT));
    assert_eq!(hit(right - 1, middle.1), Some(HTRIGHT));
    assert_eq!(hit(0, bottom - 1), Some(HTBOTTOMLEFT));
    assert_eq!(hit(middle.0, bottom - 1), Some(HTBOTTOM));
    assert_eq!(hit(right - 1, bottom - 1), Some(HTBOTTOMRIGHT));
    assert_eq!(hit(middle.0, middle.1), Some(HTCLIENT));

    unsafe extern "system" fn top_half(_: *mut c_void, _: usize, _: f32, y: f32) -> bool {
        y < 100.0
    }
    set_drag_handler(core::ptr::null_mut(), Some(top_half));
    let drag_y = (50.0 * scale(window)) as i32;
    assert_eq!(hit(middle.0, drag_y), Some(HTCAPTION));
    assert_eq!(hit(middle.0, middle.1 + drag_y), Some(HTCLIENT));
    set_drag_handler(core::ptr::null_mut(), None);
    assert_eq!(hit(middle.0, drag_y), Some(HTCLIENT));

    // The whole window is client area; framed windows keep the default.
    let mut rects = [WinRect::default(), WinRect::default(), WinRect::default()];
    assert_eq!(
        message(hwnd, WM_NCCALCSIZE, 1, rects.as_mut_ptr() as isize),
        Some(0)
    );
    assert_eq!(
        message(hwnd, WM_NCCALCSIZE, 0, rects.as_mut_ptr() as isize),
        Some(0)
    );

    let mut info = MinMaxInfo::default();
    set_size_limits(window, (200.0, 150.0), (1000.0, 800.0));
    assert_eq!(
        message(hwnd, WM_GETMINMAXINFO, 0, &mut info as *mut _ as isize),
        Some(0)
    );
    let px = |value: f64| round(value * scale(window));
    assert_eq!((info.min_track.x, info.min_track.y), (px(200.0), px(150.0)));
    assert_eq!(
        (info.max_track.x, info.max_track.y),
        (px(1000.0), px(800.0))
    );
    assert!(info.max_size.x > 0 && info.max_size.y > 0);

    // Erasing paints the background color.
    // SAFETY: the window is live; its DC is released below.
    let dc = unsafe { super::super::GetDC(hwnd) };
    assert_eq!(message(hwnd, WM_ERASEBKGND, dc as usize, 0), Some(1));
    // SAFETY: the DC was obtained above for this window.
    unsafe { super::super::ReleaseDC(hwnd, dc) };

    // Windows follow the DPI Windows suggests.
    let suggested = WinRect {
        left: 10,
        top: 20,
        right: 510,
        bottom: 420,
    };
    assert_eq!(
        message(hwnd, WM_DPICHANGED, 0, &suggested as *const _ as isize),
        Some(0)
    );
    let mut rect = WinRect::default();
    // SAFETY: `rect` is writable.
    unsafe { GetWindowRect(hwnd, &mut rect) };
    assert_eq!(
        (rect.left, rect.top, rect.right, rect.bottom),
        (10, 20, 510, 420)
    );

    // Maximized windows have no resize borders.
    crate::app_host::set_window_state(window, crate::app_host::WindowState::Maximize);
    assert_eq!(hit(0, 0), Some(HTCLIENT));
    close(window);

    // Other messages, and framed windows' hit tests, are left to Windows.
    let framed = create(&options(&[])).unwrap();
    let framed_hwnd = framed.0 as Hwnd;
    assert_eq!(message(framed_hwnd, WM_NCHITTEST, 0, 0), None);
    assert_eq!(message(framed_hwnd, WM_NCCALCSIZE, 1, 0), None);
    let mut info = MinMaxInfo::default();
    assert_eq!(
        message(
            framed_hwnd,
            WM_GETMINMAXINFO,
            0,
            &mut info as *mut _ as isize
        ),
        Some(0)
    );
    assert_eq!(info.max_size, Point::default());
    assert_eq!(message(framed_hwnd, 0x0400, 0, 0), None);
    close(framed);
}

#[test]
fn window_slots_bind_while_their_windows_are_created() {
    let _lock = lock_host_tests();
    assert_eq!(slot(0x7fff_fff0usize as Hwnd), None);
    // A window whose creation fails releases its slot.
    let mut session = shimforge::Session::new();
    let fail = shimforge::mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            u32,
            *const u16,
            *const u16,
            u32,
            i32,
            i32,
            i32,
            i32,
            Hwnd,
            *mut c_void,
            Hinstance,
            *mut c_void,
        ) -> Hwnd
    );
    fail.expect()
        .once()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| core::ptr::null_mut());
    assert!(create(&options(&[])).is_err());
    drop(session);
    assert!(
        HOST_WINDOWS
            .iter()
            .all(|slot| slot.load(Ordering::Acquire) != RESERVED)
    );
    // Every slot taken.
    for slot in &HOST_WINDOWS {
        slot.store(RESERVED + 1, Ordering::Release);
    }
    assert_eq!(create(&options(&[])), Err(8));
    for slot in &HOST_WINDOWS {
        slot.store(0, Ordering::Release);
    }
}

#[test]
fn frames_clamp_sizes_and_grow_content_to_outer_sizes() {
    assert_eq!(
        clamped((100.0, 100.0), (0.0, 0.0), (0.0, 0.0)),
        (100.0, 100.0)
    );
    assert_eq!(
        clamped((100.0, 100.0), (150.0, 0.0), (0.0, 50.0)),
        (150.0, 50.0)
    );
    let caption = super::super::WS_OVERLAPPEDWINDOW;
    assert_eq!(outer((100, 100), WS_POPUP, false, 96), (100, 100));
    let (width, height) = outer((100, 100), caption, false, 96);
    assert!(width > 100 && height > 100);
    let (_, with_menu) = outer((100, 100), caption, true, 96);
    assert!(with_menu > height);
    assert_eq!(dpi_scale(0), 1.0);
    assert_eq!(dpi_scale(144), 1.5);
    assert_eq!(round(1.5), 2);
}

static EVENTS: std::sync::Mutex<Vec<(usize, u8)>> = std::sync::Mutex::new(Vec::new());

unsafe extern "system" fn record_event(_: *mut c_void, window: usize, event: WindowEvent) -> bool {
    EVENTS.lock().unwrap().push((window, event as u8));
    true
}

#[test]
fn full_screen_covers_the_monitor_and_state_changes_become_events() {
    use crate::app_host::{
        WindowState, set_window_event_handler, set_window_full_screen, set_window_state,
    };
    let _lock = lock_host_tests();
    let window = create(&WindowOptions {
        position: Some((60.0, 60.0)),
        size: (400.0, 300.0),
        ..options(&[])
    })
    .unwrap();
    let hwnd = window.0 as Hwnd;
    EVENTS.lock().unwrap().clear();
    set_window_event_handler(core::ptr::null_mut(), Some(record_event));
    let events = || {
        let mut events = EVENTS.lock().unwrap();
        let mine: Vec<u8> = events
            .iter()
            .filter(|(owner, event)| *owner == window.0 && *event >= WindowEvent::Maximize as u8)
            .map(|(_, event)| *event)
            .collect();
        events.clear();
        mine
    };
    let before = window_bounds(window).unwrap();
    set_window_state(window, WindowState::Maximize);
    assert_eq!(events(), [WindowEvent::Maximize as u8]);
    set_window_full_screen(window, true);
    set_window_full_screen(window, true);
    assert!(is_full_screen(window));
    assert_eq!(events(), [WindowEvent::EnterFullScreen as u8]);
    // SAFETY: these queries accept any HWND; `rect` is writable.
    let (style, area) = unsafe {
        let (_, area, _) = monitor(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
        (GetWindowLongPtrW(hwnd, GWL_STYLE) as u32, area)
    };
    assert_eq!(style & (WS_CAPTION | WS_THICKFRAME), 0);
    assert_eq!(
        window_bounds(window),
        Some((
            area.left,
            area.top,
            area.right - area.left,
            area.bottom - area.top
        ))
    );
    assert_eq!(hit_test(hwnd, 0), HTCLIENT);
    // Leaving restores the maximized window, whose bounds were saved.
    set_window_full_screen(window, false);
    assert!(!is_full_screen(window));
    assert_eq!(events(), [WindowEvent::LeaveFullScreen as u8]);
    // SAFETY: IsZoomed accepts any HWND.
    assert_ne!(unsafe { IsZoomed(hwnd) }, 0);
    set_window_state(window, WindowState::Restore);
    assert_eq!(window_bounds(window), Some(before));
    set_window_state(window, WindowState::Minimize);
    set_window_state(window, WindowState::Restore);
    assert_eq!(
        events(),
        [
            WindowEvent::Unmaximize as u8,
            WindowEvent::Minimize as u8,
            WindowEvent::Restore as u8
        ]
    );
    set_window_event_handler(core::ptr::null_mut(), None);
    close(window);
    set_window_full_screen(window, true);
    assert!(!is_full_screen(window));
}

#[test]
fn windows_blur_when_deactivated_not_when_their_controls_take_the_focus() {
    use crate::app_host::{SendMessageW, set_window_event_handler};
    let _lock = lock_host_tests();
    let window = create(&options(&[])).unwrap();
    let hwnd = window.0 as Hwnd;
    EVENTS.lock().unwrap().clear();
    set_window_event_handler(core::ptr::null_mut(), Some(record_event));
    let events = || {
        let mut events = EVENTS.lock().unwrap();
        let mine: Vec<u8> = events
            .iter()
            .filter(|(owner, event)| {
                *owner == window.0
                    && (*event == WindowEvent::Focus as u8 || *event == WindowEvent::Blur as u8)
            })
            .map(|(_, event)| *event)
            .collect();
        events.clear();
        mine
    };
    // SAFETY: the messages go to the test's own window.
    unsafe {
        // The keyboard focus moving to one of the page's controls.
        SendMessageW(hwnd, 0x0008, 0, 0);
        SendMessageW(hwnd, 0x0007, 0, 0);
    }
    assert_eq!(events(), Vec::<u8>::new());
    // Activation and deactivation, each reported once.
    // SAFETY: as above.
    unsafe {
        SendMessageW(hwnd, 0x0006, 1, 0);
        SendMessageW(hwnd, 0x0006, 1, 0);
        SendMessageW(hwnd, 0x0006, 0, 0);
        SendMessageW(hwnd, 0x0006, 0, 0);
    }
    assert_eq!(
        events(),
        [WindowEvent::Focus as u8, WindowEvent::Blur as u8]
    );
    set_window_event_handler(core::ptr::null_mut(), None);
    close(window);
}
