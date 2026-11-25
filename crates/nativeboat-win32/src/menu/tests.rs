use super::*;
use crate::Event;
use crate::tests::support::{record, send, window};
use shimforge::{Session, mock};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMenu, GetMenuItemCount, TPMPARAMS, WM_COMMAND, WM_KEYDOWN,
};
use windows_sys::core::BOOL;

pub(crate) fn items() -> Vec<MenuItem> {
    vec![MenuItem {
        label: "&File".to_owned(),
        kind: MenuKind::Submenu,
        enabled: true,
        items: vec![
            MenuItem {
                id: 1,
                label: "&Open".to_owned(),
                enabled: true,
                accelerator: Some(Accelerator {
                    key: 0x4F,
                    ctrl: true,
                    shift: true,
                    alt: true,
                    label: "Ctrl+Shift+Alt+O".to_owned(),
                }),
                ..MenuItem::default()
            },
            MenuItem {
                kind: MenuKind::Separator,
                ..MenuItem::default()
            },
            MenuItem {
                id: 2,
                label: "Reload".to_owned(),
                enabled: true,
                accelerator: Some(Accelerator {
                    key: 0x74,
                    label: "F5".to_owned(),
                    ..Accelerator::default()
                }),
                ..MenuItem::default()
            },
            MenuItem {
                id: 3,
                label: "Wrap".to_owned(),
                kind: MenuKind::Checkbox,
                enabled: true,
                checked: true,
                ..MenuItem::default()
            },
            MenuItem {
                id: 4,
                label: "Big".to_owned(),
                kind: MenuKind::Radio,
                ..MenuItem::default()
            },
        ],
        ..MenuItem::default()
    }]
}

#[test]
fn menu_state_updates_retain_native_handles_and_do_not_resize_the_editor() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, GetSubMenu};
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let handle = hwnd(window).unwrap();
    let mut menu = items();
    set_menu(window, Some(&menu)).unwrap();
    // SAFETY: the live owned window has this menu and submenu.
    let (bar, child) = unsafe {
        let bar = GetMenu(handle);
        (bar, GetSubMenu(bar, 0))
    };
    let key = MSG {
        hwnd: handle,
        message: WM_KEYDOWN,
        wParam: 0x74,
        ..MSG::default()
    };
    let size = crate::client_size(window);
    events.take();
    set_menu(window, Some(&menu)).unwrap();
    menu[0].items[2].enabled = false;
    menu[0].items[3].checked = false;
    set_menu(window, Some(&menu)).unwrap();
    // SAFETY: state updates retain these live owned handles.
    unsafe {
        assert_eq!(GetMenu(handle), bar);
        assert_eq!(GetSubMenu(bar, 0), child);
        assert_ne!(GetMenuState(child, 2, MF_BYPOSITION) & MF_GRAYED, 0);
        assert_eq!(GetMenuState(child, 3, MF_BYPOSITION) & MF_CHECKED, 0);
    }
    assert_eq!(crate::client_size(window), size);
    assert!(!translate(&key));
    assert!(events.take().is_empty());
    menu[0].items[2].enabled = true;
    menu[0].items[3].checked = true;
    set_menu(window, Some(&menu)).unwrap();
    assert!(translate(&key));
    events.take();
    menu[0].items[2].label = "Changed".into();
    set_menu(window, Some(&menu)).unwrap();
    // SAFETY: a structural change installs a fresh menu.
    assert_ne!(unsafe { GetMenu(handle) }, bar);
    crate::destroy(window);
}

#[test]
fn ignored_menu_shortcuts_are_per_window_and_survive_menu_replacement() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let first = window();
    let second = window();
    set_ignore_menu_shortcuts(first, true);
    set_ignore_menu_shortcuts(u64::MAX, true);
    assert!(!IGNORED.with_borrow(|ignored| ignored.contains(&u64::MAX)));
    for window in [first, second] {
        set_menu(window, Some(&items())).unwrap();
    }
    events.take();
    let key = |window| MSG {
        hwnd: hwnd(window).unwrap(),
        message: WM_KEYDOWN,
        wParam: 0x74,
        ..MSG::default()
    };
    assert!(!translate(&key(first)));
    assert!(translate(&key(second)));
    assert_eq!(
        events.take(),
        [Event::Menu(crate::MenuCommand {
            id: 2,
            accelerator: true,
            modifiers: crate::Modifiers::default()
        })]
    );
    send(hwnd(first).unwrap(), WM_COMMAND, 3, 0);
    assert_eq!(
        events.take(),
        [Event::Menu(3.into())],
        "mouse commands are retained"
    );
    set_menu(first, None).unwrap();
    set_menu(first, Some(&items())).unwrap();
    events.take();
    assert!(!translate(&key(first)));
    set_ignore_menu_shortcuts(first, false);
    assert!(translate(&key(first)));
    assert_eq!(
        events.take(),
        [Event::Menu(crate::MenuCommand {
            id: 2,
            accelerator: true,
            modifiers: crate::Modifiers::default()
        })]
    );
    set_ignore_menu_shortcuts(first, true);
    crate::destroy(first);
    assert!(!IGNORED.with_borrow(|ignored| ignored.contains(&first)));
    crate::destroy(second);
    let fresh = window();
    assert!(!IGNORED.with_borrow(|ignored| ignored.contains(&fresh)));
    crate::destroy(fresh);
}

#[test]
fn menu_bars_and_accelerators() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let handle = hwnd(window).unwrap();
    set_menu(window, Some(&items())).unwrap();
    // SAFETY: reads the menu of a live window.
    let bar = unsafe { GetMenu(handle) };
    // SAFETY: counts the items of a live menu.
    assert_eq!(unsafe { GetMenuItemCount(bar) }, 1);
    let key = MSG {
        hwnd: handle,
        message: WM_KEYDOWN,
        wParam: 0x74,
        ..MSG::default()
    };
    assert!(translate(&key));
    assert!(!translate(&MSG::default()));
    // The menu bar takes room from the client area.
    let received = events.take();
    assert!(matches!(received[0], Event::Resized { height, .. } if height < 200.0));
    assert_eq!(
        received[1..],
        [Event::Menu(crate::MenuCommand {
            id: 2,
            accelerator: true,
            modifiers: crate::Modifiers::default()
        })]
    );
    send(handle, WM_COMMAND, 3, 0);
    assert_eq!(events.take(), [Event::Menu(3.into())]);
    // Client sizes stay exact with a menu bar.
    crate::set_client_size(window, 300.0, 200.0);
    assert_eq!(crate::client_size(window), (300.0, 200.0));
    events.take();
    let plain = MenuItem {
        id: 9,
        label: "Only".to_owned(),
        enabled: true,
        ..MenuItem::default()
    };
    set_menu(window, Some(&[plain])).unwrap();
    assert!(!translate(&key));
    set_menu(window, None).unwrap();
    // SAFETY: as above.
    assert!(unsafe { GetMenu(handle) }.is_null());
    assert!(set_menu(u64::MAX, None).is_err());
    set_menu(window, Some(&items())).unwrap();
    crate::destroy(window);
    assert!(BARS.with_borrow(HashMap::is_empty));
}

#[test]
fn popup_menus() {
    let mut session = Session::new();
    let foreground = mock!(
        session,
        SetForegroundWindow,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    foreground.expect().returning(|_| 1);
    let track = mock!(
        session,
        TrackPopupMenuEx,
        unsafe extern "system" fn(HMENU, u32, i32, i32, HWND, *const TPMPARAMS) -> BOOL
    );
    track.expect().once().returning(|menu, flags, _, _, _, _| {
        // SAFETY: the menu is live during the call.
        assert_eq!(unsafe { GetMenuItemCount(menu) }, 1);
        assert_ne!(flags & TPM_RETURNCMD, 0);
        7
    });
    track.expect().once().returning(|_, _, _, _, _, _| 0);
    let window = window();
    assert_eq!(
        popup_menu(Some(window), ptr::null_mut(), &items(), Some((10.0, 20.0))),
        Some(7)
    );
    assert_eq!(
        popup_menu(None, hwnd(window).unwrap(), &items(), None),
        None
    );
}

#[test]
fn edit_commands_reach_the_focused_control() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, GetWindowTextLengthW, WS_CHILD,
    };
    let window = window();
    let class = wide("EDIT");
    let text = wide("hello");
    // SAFETY: creates an edit control in a live window of this thread.
    let edit = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD,
            0,
            0,
            100,
            20,
            hwnd(window).unwrap(),
            ptr::null_mut(),
            crate::window::instance(),
            ptr::null(),
        )
    };
    let mut session = Session::new();
    let focus = mock!(session, GetFocus, unsafe extern "system" fn() -> HWND);
    focus.expect().once().returning(ptr::null_mut);
    let target = edit as usize;
    focus.expect().returning(move || target as HWND);
    assert!(!edit_command("selectAll"));
    assert!(edit_command("selectAll"));
    assert!(edit_command("delete"));
    assert!(!edit_command("explode"));
    // SAFETY: reads the text length of a live control.
    assert_eq!(unsafe { GetWindowTextLengthW(edit) }, 0);
}

#[test]
fn hidden_menu_entries_keep_shortcuts_and_unregistered_entries_keep_labels() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuStringW, GetSubMenu, MF_BYPOSITION};
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let entry = |id, key, hidden, enabled, ignore_accelerator| MenuItem {
        id,
        label: format!("Item {id}"),
        hidden,
        enabled,
        ignore_accelerator,
        accelerator: Some(Accelerator {
            key,
            label: format!("F{}", key - 0x6f),
            ..Accelerator::default()
        }),
        ..MenuItem::default()
    };
    let menu = vec![
        MenuItem {
            label: "Visible".into(),
            kind: MenuKind::Submenu,
            enabled: true,
            items: vec![
                entry(10, 0x74, true, true, false),
                entry(11, 0x75, false, true, false),
                entry(12, 0x76, true, false, false),
                entry(13, 0x77, false, true, true),
            ],
            ..MenuItem::default()
        },
        MenuItem {
            label: "Hidden parent".into(),
            kind: MenuKind::Submenu,
            enabled: true,
            hidden: true,
            items: vec![entry(14, 0x78, false, true, false)],
            ..MenuItem::default()
        },
    ];
    set_menu(window, Some(&menu)).unwrap();
    let handle = hwnd(window).unwrap();
    // SAFETY: the window owns this menu and its child.
    unsafe {
        let bar = GetMenu(handle);
        assert_eq!(GetMenuItemCount(bar), 1);
        let child = GetSubMenu(bar, 0);
        assert_eq!(GetMenuItemCount(child), 2);
        let mut label = [0; 128];
        let length = GetMenuStringW(
            child,
            1,
            label.as_mut_ptr(),
            label.len() as i32,
            MF_BYPOSITION,
        );
        assert_eq!(
            String::from_utf16_lossy(&label[..length as usize]),
            "Item 13\tF8"
        );
    }
    events.take();
    for (key, id) in [(0x74, 10), (0x75, 11), (0x78, 14)] {
        assert!(translate(&MSG {
            hwnd: handle,
            message: WM_KEYDOWN,
            wParam: key,
            ..MSG::default()
        }));
        assert_eq!(
            events.take(),
            [Event::Menu(crate::MenuCommand {
                id,
                accelerator: true,
                modifiers: crate::Modifiers::default()
            })]
        );
    }
    for key in [0x76, 0x77] {
        assert!(!translate(&MSG {
            hwnd: handle,
            message: WM_KEYDOWN,
            wParam: key,
            ..MSG::default()
        }));
        assert!(events.take().is_empty());
    }
    crate::destroy(window);
}

#[test]
fn menu_command_input_captures_accelerator_and_mouse_modifier_state() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
    };
    let events = record();
    let window = window();
    let handle = hwnd(window).unwrap();
    events.take();
    let mut session = Session::new();
    let state = mock!(session, GetKeyState, unsafe extern "system" fn(i32) -> i16);
    state.expect().returning(|key| {
        if [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN].contains(&(key as u16)) {
            i16::MIN
        } else {
            0
        }
    });
    let held = crate::Modifiers {
        shift: true,
        ctrl: true,
        alt: true,
        meta: true,
    };
    assert_eq!(menu_modifiers(), held);
    send(handle, WM_COMMAND, 42 | (1 << 16), 0);
    send(handle, WM_COMMAND, 43, 0);
    assert_eq!(
        events.take(),
        [
            Event::Menu(crate::MenuCommand {
                id: 42,
                accelerator: true,
                modifiers: held
            }),
            Event::Menu(crate::MenuCommand {
                id: 43,
                accelerator: false,
                modifiers: held
            })
        ]
    );
    crate::destroy(window);
}
