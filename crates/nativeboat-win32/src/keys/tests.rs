use super::*;
use shimforge::{Session, mock};

#[test]
fn named_keys() {
    let expected = [
        (VK_CLEAR, "Clear"),
        (VK_RETURN, "Enter"),
        (VK_TAB, "Tab"),
        (VK_BACK, "Backspace"),
        (VK_ESCAPE, "Escape"),
        (VK_LEFT, "ArrowLeft"),
        (VK_RIGHT, "ArrowRight"),
        (VK_UP, "ArrowUp"),
        (VK_DOWN, "ArrowDown"),
        (VK_SHIFT, "Shift"),
        (VK_CONTROL, "Control"),
        (VK_MENU, "Alt"),
        (VK_LWIN, "Meta"),
        (VK_RWIN, "Meta"),
        (VK_DELETE, "Delete"),
        (VK_INSERT, "Insert"),
        (VK_HOME, "Home"),
        (VK_END, "End"),
        (VK_PRIOR, "PageUp"),
        (VK_NEXT, "PageDown"),
        (VK_CAPITAL, "CapsLock"),
        (VK_NUMLOCK, "NumLock"),
        (VK_SCROLL, "ScrollLock"),
        (VK_PAUSE, "Pause"),
        (VK_SNAPSHOT, "PrintScreen"),
        (VK_APPS, "ContextMenu"),
        (VK_F1, "F1"),
        (VK_F24, "F24"),
    ];
    for (vk, name) in expected {
        assert_eq!(key(vk, 0), name, "{vk}");
    }
}

#[test]
fn typed_keys() {
    let mut session = Session::new();
    let state = mock!(
        session,
        GetKeyboardState,
        unsafe extern "system" fn(*mut u8) -> windows_sys::core::BOOL
    );
    state.expect().returning(|keys| {
        // SAFETY: the caller passes a 256-byte buffer.
        let keys = unsafe { std::slice::from_raw_parts_mut(keys, 256) };
        keys[usize::from(VK_CONTROL)] = 0x80;
        1
    });
    let translate = mock!(
        session,
        ToUnicodeEx,
        unsafe extern "system" fn(u32, u32, *const u8, *mut u16, i32, u32, HKL) -> i32
    );
    translate
        .expect()
        .with(|vk, _, _, _, _, _, _| *vk == 0x41)
        .returning(|_, _, keys, buffer, _, flags, _| {
            // SAFETY: the caller passes the 256-byte state and an 8-unit buffer.
            unsafe {
                *buffer = if *keys.add(usize::from(VK_CONTROL)) & 0x80 != 0 {
                    1
                } else {
                    u16::from(b'a')
                };
            }
            assert_eq!(flags, 4);
            1
        });
    translate
        .expect()
        .with(|vk, _, _, _, _, _, _| *vk == 0xDE)
        .returns(-1);
    translate.expect().returns(0);
    assert_eq!(key(0x41, 0x1E), "a");
    assert_eq!(key(0xDE, 0x28), "Dead");
    assert_eq!(key(0xFF, 0), "Unidentified");
}

#[test]
fn printable_ctrl_alt_glyphs_and_fallbacks_preserve_keyboard_state() {
    use std::sync::{Arc, Mutex};
    let mut session = Session::new();
    let state = Arc::new(Mutex::new([0_u8; 256]));
    let query = state.clone();
    mock!(
        session,
        GetKeyboardState,
        unsafe extern "system" fn(*mut u8) -> windows_sys::core::BOOL
    )
    .expect()
    .returning(move |output| {
        // SAFETY: the adapter supplies the 256-byte output array.
        unsafe { std::ptr::copy_nonoverlapping(query.lock().unwrap().as_ptr(), output, 256) };
        1
    });
    mock!(
        session,
        GetKeyboardLayout,
        unsafe extern "system" fn(u32) -> HKL
    )
    .expect()
    .returning(|thread| {
        assert_eq!(thread, 0);
        0x407_usize as HKL
    });
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let seen = attempts.clone();
    mock!(
        session,
        ToUnicodeEx,
        unsafe extern "system" fn(u32, u32, *const u8, *mut u16, i32, u32, HKL) -> i32
    )
    .expect()
    .returning(
        move |vk, scan, keys, output, capacity, flags, current_layout| {
            assert_eq!(
                (scan, capacity, flags, current_layout as usize),
                (0x10, 8, 4, 0x407)
            );
            // SAFETY: the adapter supplies a 256-byte state and 8-unit output.
            let keys = unsafe { std::slice::from_raw_parts(keys, 256) };
            let ctrl = keys[usize::from(VK_CONTROL)] & 0x80 != 0;
            let alt = keys[usize::from(VK_MENU)] & 0x80 != 0;
            let shift = keys[usize::from(VK_SHIFT)] & 0x80 != 0;
            let caps = keys[usize::from(VK_CAPITAL)] & 1 != 0;
            if !ctrl {
                for vk in [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU] {
                    assert_eq!(keys[usize::from(vk)], 0);
                }
            }
            if !shift {
                assert_eq!(keys[usize::from(VK_LSHIFT)], 0);
                assert_eq!(keys[usize::from(VK_RSHIFT)], 0);
            }
            assert_eq!(keys[usize::from(VK_NUMLOCK)], 1);
            seen.lock().unwrap().push((ctrl, alt, shift, caps));
            let text = match (vk, ctrl, alt, shift, caps) {
                (0x51, true, true, false, _) => "@",
                (0x51, true, true, true, _) => "😀",
                (0x41, true, true, _, _) => "\u{1}",
                (0x41, false, false, true, _) => "A",
                (0x41, false, false, false, _) => "a",
                (0x32, false, false, false, false) => "2",
                (0xDE, true, true, _, _) => return -1,
                _ => return 0,
            };
            let units: Vec<_> = text.encode_utf16().collect();
            // SAFETY: the test strings fit in the supplied output array.
            unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), output, units.len()) };
            units.len() as i32
        },
    );
    {
        let mut keys = state.lock().unwrap();
        for vk in [
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_MENU,
            VK_LMENU,
            VK_RMENU,
        ] {
            keys[usize::from(vk)] = 0x80;
        }
        keys[usize::from(VK_NUMLOCK)] = 1;
    }
    let original = *state.lock().unwrap();
    assert_eq!(key(0x51, 0x10), "@");
    assert_eq!(key(0x41, 0x10), "a");
    assert_eq!(*state.lock().unwrap(), original);
    for vk in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT] {
        state.lock().unwrap()[usize::from(vk)] = 0x80;
    }
    assert_eq!(key(0x51, 0x10), "😀");
    assert_eq!(key(0x41, 0x10), "A");
    state.lock().unwrap()[usize::from(VK_CAPITAL)] = 1;
    assert_eq!(key(0x32, 0x10), "2");
    assert_eq!(key(0xDE, 0x10), "Dead");
    assert_eq!(key(0xFF, 0x10), "Unidentified");
    assert_eq!(
        state.lock().unwrap()[usize::from(VK_RMENU)],
        original[usize::from(VK_RMENU)]
    );
    assert!(
        attempts
            .lock()
            .unwrap()
            .contains(&(false, false, false, false))
    );
}

#[test]
fn real_layouts_type_letters() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    assert_eq!(key(0x41, 0x1E).to_lowercase(), "a");
}

#[test]
fn codes() {
    let expected = [
        (0x41, false, 0, "KeyA"),
        (0x5A, false, 0, "KeyZ"),
        (0x30, false, 0, "Digit0"),
        (VK_NUMPAD7, false, 0, "Numpad7"),
        (VK_F5, false, 0, "F5"),
        (VK_RETURN, false, 0, "Enter"),
        (VK_RETURN, true, 0, "NumpadEnter"),
        (VK_CONTROL, false, 0, "ControlLeft"),
        (VK_CONTROL, true, 0, "ControlRight"),
        (VK_MENU, false, 0, "AltLeft"),
        (VK_MENU, true, 0, "AltRight"),
        (VK_SHIFT, false, 0x2A, "ShiftLeft"),
        (VK_SHIFT, false, 0x36, "ShiftRight"),
        (VK_SPACE, false, 0, "Space"),
        (VK_TAB, false, 0, "Tab"),
        (VK_BACK, false, 0, "Backspace"),
        (VK_ESCAPE, false, 0, "Escape"),
        (VK_LEFT, true, 0, "ArrowLeft"),
        (VK_RIGHT, true, 0, "ArrowRight"),
        (VK_UP, true, 0, "ArrowUp"),
        (VK_DOWN, true, 0, "ArrowDown"),
        (VK_LWIN, true, 0, "MetaLeft"),
        (VK_RWIN, true, 0, "MetaRight"),
        (VK_DELETE, true, 0, "Delete"),
        (VK_INSERT, true, 0, "Insert"),
        (VK_HOME, true, 0, "Home"),
        (VK_END, true, 0, "End"),
        (VK_PRIOR, true, 0, "PageUp"),
        (VK_NEXT, true, 0, "PageDown"),
        (VK_CAPITAL, false, 0, "CapsLock"),
        (VK_NUMLOCK, false, 0, "NumLock"),
        (VK_SCROLL, false, 0, "ScrollLock"),
        (VK_PAUSE, false, 0, "Pause"),
        (VK_SNAPSHOT, false, 0, "PrintScreen"),
        (VK_APPS, false, 0, "ContextMenu"),
        (VK_OEM_1, false, 0, "Semicolon"),
        (VK_OEM_PLUS, false, 0, "Equal"),
        (VK_OEM_COMMA, false, 0, "Comma"),
        (VK_OEM_MINUS, false, 0, "Minus"),
        (VK_OEM_PERIOD, false, 0, "Period"),
        (VK_OEM_2, false, 0, "Slash"),
        (VK_OEM_3, false, 0, "Backquote"),
        (VK_OEM_4, false, 0, "BracketLeft"),
        (VK_OEM_5, false, 0, "Backslash"),
        (VK_OEM_6, false, 0, "BracketRight"),
        (VK_OEM_7, false, 0, "Quote"),
        (VK_MULTIPLY, false, 0, "NumpadMultiply"),
        (VK_ADD, false, 0, "NumpadAdd"),
        (VK_SUBTRACT, false, 0, "NumpadSubtract"),
        (VK_DECIMAL, false, 0, "NumpadDecimal"),
        (VK_DIVIDE, false, 0, "NumpadDivide"),
        (0xFF, false, 0, "Unidentified"),
    ];
    for (vk, extended, scan, name) in expected {
        assert_eq!(code(vk, scan, extended), name, "{vk}");
    }
}

#[test]
fn held_modifiers() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    assert_eq!(modifiers(), Modifiers::default());
    let mut session = Session::new();
    let state = mock!(session, GetKeyState, unsafe extern "system" fn(i32) -> i16);
    state.expect().returning(|key| {
        if key == i32::from(VK_RWIN) || key == i32::from(VK_SHIFT) {
            -128
        } else {
            0
        }
    });
    assert_eq!(
        modifiers(),
        Modifiers {
            shift: true,
            ctrl: false,
            alt: false,
            meta: true
        }
    );
}

#[test]
fn keypad_navigation_retains_physical_codes_without_num_lock() {
    for (vk, scan, expected) in [
        (VK_HOME, 0x47, "Numpad7"),
        (VK_UP, 0x48, "Numpad8"),
        (VK_PRIOR, 0x49, "Numpad9"),
        (VK_LEFT, 0x4B, "Numpad4"),
        (VK_CLEAR, 0x4C, "Numpad5"),
        (VK_RIGHT, 0x4D, "Numpad6"),
        (VK_END, 0x4F, "Numpad1"),
        (VK_DOWN, 0x50, "Numpad2"),
        (VK_NEXT, 0x51, "Numpad3"),
        (VK_INSERT, 0x52, "Numpad0"),
        (VK_DELETE, 0x53, "NumpadDecimal"),
    ] {
        assert_eq!(code(vk, scan, false), expected);
        assert!(!code(vk, scan, true).starts_with("Numpad"));
    }
    assert_eq!(code(VK_LEFT, 0, false), "ArrowLeft");
}
