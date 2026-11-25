use super::*;

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn rows_are_as_wide_as_the_widest_label_and_shortcut() {
    let columns = Columns {
        label: 84,
        shortcut: 32,
        marks: false,
        arrows: false,
    };
    // Chromium's context menu: 20 + label + 10 + shortcut + 20.
    assert_eq!(columns.width(1.0), 166);
    assert_eq!(columns.width(1.25), 25 * 2 + 84 + 13 + 32);
    let marked = Columns {
        marks: true,
        ..columns
    };
    assert_eq!(marked.width(1.0), 166 + 28);
    let arrows = Columns {
        shortcut: 0,
        arrows: true,
        ..columns
    };
    assert_eq!(arrows.width(1.0), 20 + 84 + 32 + 20);
    // Shortcuts and arrows share the right column, which takes the wider:
    // Electron 38's menu with a 313-pixel label, a Ctrl+Shift+L shortcut
    // and submenus is 463 pixels wide at 125%.
    let both = Columns {
        label: 313,
        shortcut: 88,
        marks: false,
        arrows: true,
    };
    assert_eq!(both.width(1.25), 25 * 2 + 313 + 13 + 88);
    let narrow = Columns {
        shortcut: 20,
        ..both
    };
    assert_eq!(narrow.width(1.25), 25 * 2 + 313 + 40);
    // Narrow menus keep Chromium's minimum width.
    assert_eq!(Columns::default().width(1.0), 122);
    assert_eq!(Columns::default().width(1.25), 153);
}

#[test]
fn first_and_last_rows_hold_the_end_padding() {
    assert_eq!(row_height(1.0, false, false, false), 28);
    assert_eq!(row_height(1.0, true, false, false), 17);
    assert_eq!(row_height(1.0, false, true, false), 40);
    assert_eq!(row_height(1.0, false, true, true), 52);
    assert_eq!(row_height(1.25, false, false, true), 35 + 15);
}

#[test]
fn access_keys_follow_single_ampersands() {
    assert_eq!(access_key(&units("&File")), Some(u16::from(b'f')));
    assert_eq!(
        access_key(&units("Save &As\0ignored")),
        Some(u16::from(b'a'))
    );
    assert_eq!(access_key(&units("Fish && &Chips")), Some(u16::from(b'c')));
    assert_eq!(access_key(&units("Fish && Chips")), None);
    assert_eq!(access_key(&units("Trailing&")), None);
    assert_eq!(access_key(&units("主题(&T)")), Some(u16::from(b't')));
}

#[test]
fn typed_keys_pick_access_keys_then_first_letters() {
    let labels = [
        units("&Undo"),
        units("Cu&t"),
        units("&Copy"),
        units("Paste"),
        units("Print"),
    ];
    let rows: Vec<(&[u16], bool)> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| (label.as_slice(), index != 4))
        .collect();
    assert_eq!(rows_for_key(&rows, u16::from(b'T')), [1]);
    assert_eq!(rows_for_key(&rows, u16::from(b'c')), [2]);
    // Without an access key, labels starting with the key; disabled rows
    // are never picked.
    assert_eq!(rows_for_key(&rows, u16::from(b'p')), [3]);
    assert_eq!(rows_for_key(&rows, u16::from(b'u')), [0]);
    assert!(rows_for_key(&rows, u16::from(b'z')).is_empty());
    let empty = [(&[][..], true)];
    assert!(rows_for_key(&empty, u16::from(b'a')).is_empty());
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetTimer(
        window: Hwnd,
        id: usize,
        milliseconds: u32,
        procedure: Option<unsafe extern "system" fn(Hwnd, u32, usize, u32)>,
    ) -> usize;
    fn KillTimer(window: Hwnd, id: usize) -> i32;
    fn EndMenu() -> i32;
    fn FindWindowExW(parent: Hwnd, after: Hwnd, class: *const u16, title: *const u16) -> Hwnd;
    fn GetWindowThreadProcessId(window: Hwnd, process: *mut u32) -> u32;
    fn SetThreadDpiAwarenessContext(context: isize) -> isize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: Hdc) -> Hdc;
    fn CreateCompatibleBitmap(dc: Hdc, width: i32, height: i32) -> GdiObject;
    fn BitBlt(
        target: Hdc,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        source: Hdc,
        source_x: i32,
        source_y: i32,
        operation: u32,
    ) -> i32;
    fn GetDIBits(
        dc: Hdc,
        bitmap: GdiObject,
        start: u32,
        lines: u32,
        bits: *mut c_void,
        info: *mut u8,
        usage: u32,
    ) -> i32;
    fn DeleteDC(dc: Hdc) -> i32;
}

/// Saves this thread's open menu windows from the screen as 32-bit BMPs.
unsafe extern "system" fn capture_menus(window: Hwnd, _: u32, id: usize, _: u32) {
    // SAFETY: test-only screen capture of this thread's own menu windows.
    unsafe {
        KillTimer(window, id);
        let path = std::env::var("NATIVEBOAT_MENU_SHOT").unwrap_or_default();
        let class: Vec<u16> = "#32768".encode_utf16().chain([0]).collect();
        let mut after = core::ptr::null_mut();
        let mut index = 0;
        loop {
            after = FindWindowExW(
                core::ptr::null_mut(),
                after,
                class.as_ptr(),
                core::ptr::null(),
            );
            if after.is_null() {
                break;
            }
            if GetWindowThreadProcessId(after, core::ptr::null_mut()) != GetCurrentThreadId()
                || super::super::IsWindowVisible(after) == 0
            {
                continue;
            }
            let mut rect = WinRect::default();
            super::super::GetWindowRect(after, &mut rect);
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            let screen = super::super::GetDC(core::ptr::null_mut());
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let previous = super::super::SelectObject(memory, bitmap);
            BitBlt(
                memory,
                0,
                0,
                width,
                height,
                screen,
                rect.left,
                rect.top,
                0x00CC_0020,
            );
            super::super::SelectObject(memory, previous);
            let mut info = [0u8; 40];
            info[..4].copy_from_slice(&40u32.to_le_bytes());
            info[4..8].copy_from_slice(&width.to_le_bytes());
            info[8..12].copy_from_slice(&height.to_le_bytes());
            info[12..14].copy_from_slice(&1u16.to_le_bytes());
            info[14..16].copy_from_slice(&32u16.to_le_bytes());
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                info.as_mut_ptr(),
                0,
            );
            super::super::DeleteObject(bitmap);
            DeleteDC(memory);
            super::super::ReleaseDC(core::ptr::null_mut(), screen);
            let mut file = Vec::new();
            file.extend_from_slice(b"BM");
            file.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
            file.extend_from_slice(&[0; 4]);
            file.extend_from_slice(&54u32.to_le_bytes());
            file.extend_from_slice(&info);
            file.extend_from_slice(&pixels);
            std::fs::write(format!("{path}-{index}.bmp"), file).unwrap();
            index += 1;
        }
        EndMenu();
    }
}

/// Presses Down three times in the open menu, as a keyboard user does.
unsafe extern "system" fn press_down(window: Hwnd, _: u32, id: usize, _: u32) {
    // SAFETY: test-only keys posted to this thread's hidden owner window.
    unsafe {
        KillTimer(window, id);
        for _ in 0..3 {
            super::super::PostMessageW(window, 0x0100, 0x28, 0);
        }
    }
}

fn item(label: &str, id: u16) -> NativeMenuItem {
    NativeMenuItem {
        id,
        label: label.encode_utf16().chain([0]).collect(),
        separator: false,
        submenu: false,
        enabled: true,
        checked: false,
        radio: false,
        accelerator: None,
        shortcut: None,
        children: Vec::new(),
    }
}

#[test]
#[ignore = "shows a popup menu briefly; NATIVEBOAT_MENU_SHOT=prefix saves its pixels"]
fn drawn_popup_menu_shot() {
    let _host_lock = super::super::lock_host_tests();
    // Per-monitor aware, as applications are, so pixels are not scaled.
    // SAFETY: changes only this test thread's DPI awareness.
    let awareness = unsafe { SetThreadDpiAwarenessContext(-4) };
    let title: Vec<u16> = units("menu shot");
    let window = super::super::create(&super::super::WindowOptions {
        show: false,
        ..super::super::WindowOptions::new(&title)
    })
    .unwrap();
    let owner = window.raw() as Hwnd;
    let separator = NativeMenuItem {
        separator: true,
        ..item("", 0)
    };
    let shortcut = |label: &str, id: u16, keys: &str, enabled: bool| NativeMenuItem {
        shortcut: Some(keys.encode_utf16().collect()),
        enabled,
        ..item(label, id)
    };
    let items = vec![
        item("Insert Paragraph Before", 1),
        item("Insert Paragraph After", 2),
        separator.clone(),
        shortcut("Cut", 3, "Ctrl+X", false),
        shortcut("Copy", 4, "Ctrl+C", true),
        shortcut("Paste", 5, "Ctrl+V", true),
        separator,
        NativeMenuItem {
            checked: true,
            ..item("Check", 6)
        },
        NativeMenuItem {
            checked: true,
            radio: true,
            ..item("Radio", 7)
        },
        NativeMenuItem {
            radio: true,
            ..item("Other radio", 8)
        },
        NativeMenuItem {
            submenu: true,
            children: vec![item("Child", 9)],
            ..item("&Sub", 0)
        },
    ];
    // SAFETY: test-only popup on this thread's hidden window.
    unsafe {
        let menu = super::super::CreatePopupMenu();
        let mut popup = Popup::new(owner);
        popup.append(owner, menu, &items).unwrap();
        popup.activate();
        SetTimer(owner, 2, 600, Some(press_down));
        SetTimer(owner, 1, 1500, Some(capture_menus));
        super::super::TrackPopupMenuEx(
            menu,
            super::super::TPM_RETURNCMD | super::super::TPM_RIGHTBUTTON,
            100,
            100,
            owner,
            core::ptr::null(),
        );
        super::super::DestroyMenu(menu);
        drop(popup);
    }
    super::super::destroy_window(window).unwrap();
    // SAFETY: restores this thread's previous DPI awareness.
    unsafe { SetThreadDpiAwarenessContext(awareness) };
}
