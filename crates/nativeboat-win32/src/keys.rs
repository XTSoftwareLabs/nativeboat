//! Virtual-key codes to DOM `key` and `code` values.

use crate::Modifiers;
#[allow(unused_imports)]
use crate::prelude::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// The modifier keys held now.
pub(crate) fn modifiers() -> Modifiers {
    // SAFETY: GetKeyState only reads the thread's input state.
    let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key)) } < 0;
    Modifiers {
        shift: held(VK_SHIFT),
        ctrl: held(VK_CONTROL),
        alt: held(VK_MENU),
        meta: held(VK_LWIN) || held(VK_RWIN),
    }
}

fn named(vk: VIRTUAL_KEY) -> Option<&'static str> {
    Some(match vk {
        VK_CLEAR => "Clear",
        VK_RETURN => "Enter",
        VK_TAB => "Tab",
        VK_BACK => "Backspace",
        VK_ESCAPE => "Escape",
        VK_LEFT => "ArrowLeft",
        VK_RIGHT => "ArrowRight",
        VK_UP => "ArrowUp",
        VK_DOWN => "ArrowDown",
        VK_SHIFT => "Shift",
        VK_CONTROL => "Control",
        VK_MENU => "Alt",
        VK_LWIN | VK_RWIN => "Meta",
        VK_DELETE => "Delete",
        VK_INSERT => "Insert",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_CAPITAL => "CapsLock",
        VK_NUMLOCK => "NumLock",
        VK_SCROLL => "ScrollLock",
        VK_PAUSE => "Pause",
        VK_SNAPSHOT => "PrintScreen",
        VK_APPS => "ContextMenu",
        VK_F1..=VK_F24 => FUNCTION_KEYS[usize::from(vk - VK_F1)],
        _ => return None,
    })
}

const FUNCTION_KEYS: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// The DOM `key` of a virtual key: its name, or the text it types with the
/// current keyboard layout and printable modifier combination.
pub(crate) fn key(vk: VIRTUAL_KEY, scan: u32) -> String {
    if let Some(name) = named(vk) {
        return name.to_owned();
    }
    let mut state = [0_u8; 256];
    // SAFETY: the buffer has the 256 entries GetKeyboardState fills.
    unsafe { GetKeyboardState(state.as_mut_ptr()) };
    // Keep printable Ctrl+Alt/AltGr mappings. Control characters from ordinary
    // shortcuts fall back to the same key with only Shift and lock state.
    // Query flag 4 leaves pending dead keys unchanged on every attempt.
    // SAFETY: reads this thread's keyboard layout without changing it.
    let layout = unsafe { GetKeyboardLayout(0) };
    if let Some(key) = translated(vk, scan, &state, layout) {
        return key;
    }
    let original = state;
    for held in [
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
    ] {
        state[usize::from(held)] = 0;
    }
    if state != original
        && let Some(key) = translated(vk, scan, &state, layout)
    {
        return key;
    }
    // A layout may have no glyph for the current Shift/Caps Lock level.
    // The final attempt uses its unmodified glyph, retaining Num Lock.
    let shifted = state;
    for held in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_CAPITAL] {
        state[usize::from(held)] = 0;
    }
    if state != shifted
        && let Some(key) = translated(vk, scan, &state, layout)
    {
        return key;
    }
    "Unidentified".to_owned()
}

fn translated(vk: VIRTUAL_KEY, scan: u32, state: &[u8; 256], layout: HKL) -> Option<String> {
    let mut buffer = [0_u16; 8];
    // SAFETY: the state and output buffers outlive the call and their sizes
    // are passed; flag 4 leaves the keyboard state (dead keys) unchanged.
    let count = unsafe {
        ToUnicodeEx(
            u32::from(vk),
            scan,
            state.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            4,
            layout,
        )
    };
    match count {
        1.. => {
            let key = String::from_utf16_lossy(&buffer[..(count as usize).min(buffer.len())]);
            (!key.chars().any(char::is_control)).then_some(key)
        }
        0 => None,
        _ => Some("Dead".to_owned()),
    }
}

/// The DOM `code` of a virtual key; `extended` and `scan` tell the left and
/// right variants apart.
pub(crate) fn code(vk: VIRTUAL_KEY, scan: u32, extended: bool) -> String {
    // Navigation keys on the numeric keypad retain their physical code when
    // Num Lock is off. Dedicated navigation keys have the extended bit set.
    if !extended
        && matches!(
            vk,
            VK_CLEAR
                | VK_END
                | VK_DOWN
                | VK_NEXT
                | VK_LEFT
                | VK_RIGHT
                | VK_HOME
                | VK_UP
                | VK_PRIOR
                | VK_INSERT
                | VK_DELETE
                | VK_DECIMAL
                | VK_NUMPAD0..=VK_NUMPAD9
        )
    {
        let keypad = match scan {
            0x47 => Some("Numpad7"),
            0x48 => Some("Numpad8"),
            0x49 => Some("Numpad9"),
            0x4B => Some("Numpad4"),
            0x4C => Some("Numpad5"),
            0x4D => Some("Numpad6"),
            0x4F => Some("Numpad1"),
            0x50 => Some("Numpad2"),
            0x51 => Some("Numpad3"),
            0x52 => Some("Numpad0"),
            0x53 => Some("NumpadDecimal"),
            _ => None,
        };
        if let Some(code) = keypad {
            return code.to_owned();
        }
    }

    let side = |left: &str, right: &str| if extended { right } else { left }.to_owned();
    match vk {
        0x41..=0x5A => format!("Key{}", char::from(vk as u8)),
        0x30..=0x39 => format!("Digit{}", char::from(vk as u8)),
        VK_NUMPAD0..=VK_NUMPAD9 => format!("Numpad{}", vk - VK_NUMPAD0),
        VK_F1..=VK_F24 => FUNCTION_KEYS[usize::from(vk - VK_F1)].to_owned(),
        VK_RETURN => side("Enter", "NumpadEnter"),
        VK_CONTROL => side("ControlLeft", "ControlRight"),
        VK_MENU => side("AltLeft", "AltRight"),
        VK_SHIFT => if scan == 0x36 {
            "ShiftRight"
        } else {
            "ShiftLeft"
        }
        .to_owned(),
        _ => match vk {
            VK_SPACE => "Space",
            VK_TAB => "Tab",
            VK_BACK => "Backspace",
            VK_ESCAPE => "Escape",
            VK_LEFT => "ArrowLeft",
            VK_RIGHT => "ArrowRight",
            VK_UP => "ArrowUp",
            VK_DOWN => "ArrowDown",
            VK_LWIN => "MetaLeft",
            VK_RWIN => "MetaRight",
            VK_DELETE => "Delete",
            VK_INSERT => "Insert",
            VK_HOME => "Home",
            VK_END => "End",
            VK_PRIOR => "PageUp",
            VK_NEXT => "PageDown",
            VK_CAPITAL => "CapsLock",
            VK_NUMLOCK => "NumLock",
            VK_SCROLL => "ScrollLock",
            VK_PAUSE => "Pause",
            VK_SNAPSHOT => "PrintScreen",
            VK_APPS => "ContextMenu",
            VK_OEM_1 => "Semicolon",
            VK_OEM_PLUS => "Equal",
            VK_OEM_COMMA => "Comma",
            VK_OEM_MINUS => "Minus",
            VK_OEM_PERIOD => "Period",
            VK_OEM_2 => "Slash",
            VK_OEM_3 => "Backquote",
            VK_OEM_4 => "BracketLeft",
            VK_OEM_5 => "Backslash",
            VK_OEM_6 => "BracketRight",
            VK_OEM_7 => "Quote",
            VK_MULTIPLY => "NumpadMultiply",
            VK_ADD => "NumpadAdd",
            VK_SUBTRACT => "NumpadSubtract",
            VK_DECIMAL => "NumpadDecimal",
            VK_DIVIDE => "NumpadDivide",
            _ => "Unidentified",
        }
        .to_owned(),
    }
}

#[cfg(test)]
mod tests;
