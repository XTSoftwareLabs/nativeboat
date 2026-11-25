use super::*;

static LAST_MENU_COMMAND: AtomicUsize = AtomicUsize::new(0);
static LAST_MENU_WINDOW: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_LAYOUT_CHANGES: AtomicUsize = AtomicUsize::new(0);

static LAST_MENU_ACCELERATOR: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn record_menu_command(
    _: *mut c_void,
    window: usize,
    command: u16,
    accelerator: bool,
) {
    LAST_MENU_WINDOW.store(window, Ordering::Release);
    LAST_MENU_ACCELERATOR.store(usize::from(accelerator), Ordering::Release);
    LAST_MENU_COMMAND.store(usize::from(command), Ordering::Release);
}

unsafe extern "system" fn record_keyboard_layout_change(_: *mut c_void) {
    KEYBOARD_LAYOUT_CHANGES.fetch_add(1, Ordering::AcqRel);
}

#[test]
fn electron_edit_roles_map_to_native_focused_control_messages() {
    let _host_lock = lock_host_tests();
    assert_eq!(edit_action_message("undo"), Some((0x0304, 0, 0)));
    assert_eq!(edit_action_message("cut"), Some((0x0300, 0, 0)));
    assert_eq!(edit_action_message("copy"), Some((0x0301, 0, 0)));
    assert_eq!(edit_action_message("paste"), Some((0x0302, 0, 0)));
    assert_eq!(edit_action_message("delete"), Some((0x0303, 0, 0)));
    assert_eq!(edit_action_message("selectAll"), Some((0x00b1, 0, -1)));
    assert_eq!(edit_action_message("clearrecentdocuments"), None);
}

#[test]
fn native_window_can_be_created_destroyed_and_quit_the_message_loop() {
    let _host_lock = lock_host_tests();
    let title = "native host test".encode_utf16().collect::<Vec<_>>();
    let window = create_window(&title, 320, 200).unwrap();
    assert_ne!(window.raw(), 0);
    assert_eq!(window_count(), 1);
    assert!(is_window(window));
    set_application_menu(&[
        NativeMenuItem {
            id: 0,
            label: "File".encode_utf16().chain([0]).collect(),
            separator: false,
            submenu: true,
            enabled: true,
            checked: false,
            radio: false,
            accelerator: None,
            shortcut: None,
            children: vec![NativeMenuItem {
                id: 7,
                label: "Open".encode_utf16().chain([0]).collect(),
                separator: false,
                submenu: false,
                enabled: true,
                checked: false,
                radio: false,
                accelerator: Some("Ctrl+O".encode_utf16().collect()),
                shortcut: None,
                children: Vec::new(),
            }],
        },
        NativeMenuItem {
            id: 0,
            label: vec![0],
            separator: true,
            submenu: false,
            enabled: true,
            checked: false,
            radio: false,
            accelerator: None,
            shortcut: None,
            children: Vec::new(),
        },
    ])
    .unwrap();
    set_window_menu(window, &[]).unwrap();
    clear_window_menu(window).unwrap();
    set_application_menu(&[]).unwrap();
    set_menu_command_handler(core::ptr::null_mut(), Some(record_menu_command));
    // SAFETY: the HWND is a live window created above; WM_COMMAND with a zero
    // lParam represents a menu command and the installed handler has no context.
    unsafe { window_procedure(window.raw() as Hwnd, WM_COMMAND, 37, 0) };
    assert_eq!(LAST_MENU_COMMAND.load(Ordering::Acquire), 37);
    assert_eq!(LAST_MENU_ACCELERATOR.load(Ordering::Acquire), 0);
    set_keyboard_layout_change_handler(core::ptr::null_mut(), Some(record_keyboard_layout_change));
    // SAFETY: this live host HWND receives the documented input-language
    // change message, and the installed callback has no context preconditions.
    unsafe { window_procedure(window.raw() as Hwnd, WM_INPUTLANGCHANGE, 0, 0) };
    assert_eq!(KEYBOARD_LAYOUT_CHANGES.load(Ordering::Acquire), 1);
    set_keyboard_layout_change_handler(core::ptr::null_mut(), None);
    clear_application_menu().unwrap();
    destroy_window(window).unwrap();
    assert_eq!(LAST_MENU_COMMAND.load(Ordering::Acquire), 0);
    assert_eq!(LAST_MENU_WINDOW.load(Ordering::Acquire), window.raw());
    set_menu_command_handler(core::ptr::null_mut(), None);
    assert_eq!(window_count(), 0);
    assert!(!is_window(window));
    assert_eq!(run_message_loop().unwrap(), 0);
}

#[test]
fn invalid_native_window_dimensions_are_rejected_without_calling_win32() {
    let _host_lock = lock_host_tests();
    assert_eq!(create_window(&[], 0, 100), Err(87));
    assert_eq!(create_window(&[], 100, -1), Err(87));
}

#[test]
fn menus_open_at_client_points_scaled_to_the_window_or_at_the_cursor() {
    let _host_lock = lock_host_tests();
    let title = "menu point test".encode_utf16().collect::<Vec<_>>();
    let window = create_window(&title, 320, 200).unwrap();
    let mut origin = frame::Point::default();
    // SAFETY: converts the origin of this live window's client area.
    unsafe { ClientToScreen(window.raw() as Hwnd, &mut origin) };
    let scale = scale(window);
    let point = menu_point(Some(window), Some((23, 20)));
    assert_eq!(point.x, origin.x + (23.0 * scale) as i32);
    assert_eq!(point.y, origin.y + (20.0 * scale) as i32);
    let mut cursor = frame::Point::default();
    // SAFETY: reads the cursor position.
    unsafe { GetCursorPos(&mut cursor) };
    for point in [
        menu_point(Some(window), None),
        menu_point(None, Some((5, 5))),
    ] {
        assert_eq!(point, cursor);
    }
    destroy_window(window).unwrap();
}

#[test]
fn electron_accelerators_become_win32_accelerator_keys() {
    let parse = |text: &str| accelerator(&text.encode_utf16().collect::<Vec<_>>());
    assert_eq!(parse("CmdOrCtrl+Shift+S"), Some((0x01 | 0x08 | 0x04, 0x53)));
    assert_eq!(parse("Ctrl++"), Some((0x09, 0xbb)));
    assert_eq!(parse("CommandOrControl+Plus"), Some((0x09, 0xbb)));
    assert_eq!(parse("Alt+F4"), Some((0x11, 0x73)));
    assert_eq!(parse("Option+F24"), Some((0x11, 0x87)));
    assert_eq!(parse("F11"), Some((0x01, 0x7a)));
    assert_eq!(parse("Control+num5"), Some((0x09, 0x65)));
    assert_eq!(parse("Ctrl+\\"), Some((0x09, 0xdc)));
    assert_eq!(parse("Shift+Esc"), Some((0x05, 0x1b)));
    assert_eq!(parse("ctrl+1"), Some((0x09, 0x31)));
    assert_eq!(parse("Ctrl+PageDown"), Some((0x09, 0x22)));
    assert_eq!(parse("Super+A"), None);
    assert_eq!(parse("Ctrl"), None);
    assert_eq!(parse("F25"), None);
    assert_eq!(parse("num10"), None);
    assert_eq!(accelerator(&[0xd800]), None);
}

static FILTERED_KEYS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn drop_q(
    _: *mut c_void,
    _: usize,
    message: u32,
    wparam: usize,
    _: isize,
) -> bool {
    assert_eq!(message, 0x0100);
    FILTERED_KEYS.fetch_add(1, Ordering::AcqRel);
    wparam == 0x51
}

#[test]
fn keys_meet_the_key_filter_then_menu_accelerators() {
    let _host_lock = lock_host_tests();
    let window = create_window(&[0], 320, 200).unwrap();
    let item = |id: u16, enabled: bool, accelerator: &str| NativeMenuItem {
        id,
        label: "Item".encode_utf16().chain([0]).collect(),
        separator: false,
        submenu: false,
        enabled,
        checked: false,
        radio: false,
        accelerator: Some(accelerator.encode_utf16().collect()),
        shortcut: None,
        children: Vec::new(),
    };
    set_application_menu(&[NativeMenuItem {
        id: 0,
        submenu: true,
        accelerator: None,
        shortcut: None,
        children: vec![
            item(9, true, "F5"),
            item(10, false, "F6"),
            item(11, true, "Super+Q"),
        ],
        ..item(0, true, "")
    }])
    .unwrap();
    set_menu_command_handler(core::ptr::null_mut(), Some(record_menu_command));
    set_key_filter(core::ptr::null_mut(), Some(drop_q));
    LAST_MENU_COMMAND.store(0, Ordering::Release);
    let commands = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = commands.clone();
    let mut step = 0;
    let status = run_host_loop(move || {
        step += 1;
        seen.borrow_mut()
            .push(LAST_MENU_COMMAND.swap(0, Ordering::AcqRel));
        // The page ignores menu shortcuts for the fourth key, then heeds them.
        set_ignore_menu_shortcuts(window, step == 4);
        let key = [0x51, 0x74, 0x75, 0x74, 0x74].get(step - 1).copied();
        match key {
            // SAFETY: posts to the live window created above.
            Some(key) => unsafe { PostMessageW(window.raw() as Hwnd, 0x0100, key, 0) },
            None => return Err(3),
        };
        Ok((Some(0), Vec::new()))
    });
    assert_eq!(status, Ok(3));
    // Q was dropped, F5 chose its item and the disabled F6 chose none;
    // F5 chose nothing while shortcuts were ignored, then its item again.
    assert_eq!(*commands.borrow(), [0, 0, 9, 0, 0, 9]);
    set_ignore_menu_shortcuts(Window::from_raw(1), true);
    assert_eq!(LAST_MENU_ACCELERATOR.load(Ordering::Acquire), 1);
    assert_eq!(FILTERED_KEYS.load(Ordering::Acquire), 5);
    set_key_filter(core::ptr::null_mut(), None);
    set_menu_command_handler(core::ptr::null_mut(), None);
    clear_application_menu().unwrap();
    destroy_window(window).unwrap();
    assert_eq!(run_message_loop().unwrap(), 0);
}

#[test]
fn page_text_needs_a_page_handler() {
    let _lock = lock_host_tests();
    unsafe fn accept(_: *mut c_void, window: usize, method: &str, text: &[u16]) -> bool {
        window == 7 && method == "replaceMisspelling" && text == [0x61]
    }
    assert!(!page_text(Window(7), "replaceMisspelling", &[0x61]));
    set_page_text_handler(core::ptr::null_mut(), Some(accept));
    assert!(page_text(Window(7), "replaceMisspelling", &[0x61]));
    assert!(!page_text(Window(8), "replaceMisspelling", &[0x61]));
    set_page_text_handler(core::ptr::null_mut(), None);
}

#[test]
fn edit_roles_run_as_the_focused_pages_commands() {
    let _lock = lock_host_tests();
    unsafe fn page(_: *mut c_void, window: usize, method: &str, text: &[u16]) -> bool {
        // The page in window 7 runs copy and paste; window 8 is a control.
        window == 7
            && method == "command"
            && (text == "copy".encode_utf16().collect::<Vec<_>>()
                || text == "paste".encode_utf16().collect::<Vec<_>>())
    }
    set_page_text_handler(core::ptr::null_mut(), Some(page));
    assert!(edit_action_in(7 as Hwnd, "copy"));
    assert!(edit_action_in(7 as Hwnd, "pasteAndMatchStyle"));
    // Elsewhere the role becomes the control's edit message, when it has one.
    assert!(edit_action_in(8 as Hwnd, "paste"));
    assert!(!edit_action_in(8 as Hwnd, "redo"));
    set_page_text_handler(core::ptr::null_mut(), None);
}

#[test]
fn zoom_belongs_to_host_windows() {
    let _lock = lock_host_tests();
    let window = create(&WindowOptions {
        show: false,
        ..WindowOptions::new(&[])
    })
    .unwrap();
    assert_eq!(zoom(window), 1.0);
    set_zoom(window, 1.5);
    assert_eq!(zoom(window), 1.5);
    set_zoom(window, -1.0);
    set_zoom(window, f64::INFINITY);
    assert_eq!(zoom(window), 1.5);
    zoom_changed(window, true);
    destroy_window(window).unwrap();
    assert_eq!(zoom(window), 1.0);
    set_zoom(window, 2.0);
    assert_eq!(zoom(window), 1.0);
}

#[test]
fn presses_wheel_turns_and_characters_are_separate_inputs() {
    // WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL, WM_CHAR and WM_KEYUP end
    // a run of dispatched messages; moves and key presses do not.
    for message in [0x0201, 0x0202, 0x020A, 0x0102, 0x0101, 0x0105, 0x020E] {
        assert!(separate_input(message), "{message:#x}");
    }
    for message in [0x0200, 0x0100, 0x0104, 0x000F, 0x0113] {
        assert!(!separate_input(message), "{message:#x}");
    }
}

#[test]
fn menu_bars_that_hide_until_alt_start_hidden_and_show_on_request() {
    let _host_lock = lock_host_tests();
    let title: Vec<u16> = "auto hide".encode_utf16().collect();
    let window = create(&WindowOptions {
        auto_hide_menu_bar: true,
        ..WindowOptions::new(&title)
    })
    .unwrap();
    let file = NativeMenuItem {
        id: 3,
        label: "File".encode_utf16().chain([0]).collect(),
        separator: false,
        submenu: false,
        enabled: true,
        checked: false,
        radio: false,
        accelerator: None,
        shortcut: None,
        children: Vec::new(),
    };
    set_window_menu(window, &[file]).unwrap();
    // The menu is kept, but its bar takes none of the window.
    assert!(is_menu_bar_auto_hide(window));
    assert!(!is_menu_bar_visible(window));
    let hidden = client_size(window).unwrap();
    set_menu_bar_visibility(window, true);
    assert!(is_menu_bar_visible(window));
    let shown = client_size(window).unwrap();
    assert!(shown.1 < hidden.1, "{shown:?} {hidden:?}");
    // Closing a menu the Alt key did not open leaves the bar as it is.
    // SAFETY: the HWND is the live window created above.
    unsafe { window_procedure(window.raw() as Hwnd, 0x0212, 0, 0) };
    assert!(is_menu_bar_visible(window));
    set_menu_bar_visibility(window, false);
    assert!(!is_menu_bar_visible(window));
    assert_eq!(client_size(window).unwrap(), hidden);
    set_menu_bar_auto_hide(window, false);
    assert!(!is_menu_bar_auto_hide(window));
    destroy_window(window).unwrap();
}

#[test]
fn windows_take_their_own_icons_and_free_them() {
    let _host_lock = lock_host_tests();
    let title: Vec<u16> = "icons".encode_utf16().collect();
    let window = create(&WindowOptions::new(&title)).unwrap();
    const WM_GETICON: u32 = 0x007f;
    let shown = |kind: usize| {
        // SAFETY: the HWND is the live window created above.
        unsafe { SendMessageW(window.raw() as Hwnd, WM_GETICON, kind, 0) as usize }
    };
    let pixels = [255u8, 0, 0, 255].repeat(16 * 16);
    let first = Icon::from_rgba(16, 16, &pixels).unwrap();
    set_window_icon(window, Some(first));
    assert_eq!((shown(0), shown(1)), (first.raw(), first.raw()));
    // A new icon replaces the first, which is freed.
    let second = Icon::from_rgba(16, 16, &pixels).unwrap();
    set_window_icon(window, Some(second));
    assert_eq!(shown(1), second.raw());
    set_window_icon(window, None);
    assert_eq!(shown(1), 0);
    destroy_window(window).unwrap();
    // Icons for windows that are gone are freed at once.
    set_window_icon(window, Icon::from_rgba(16, 16, &pixels));
}
