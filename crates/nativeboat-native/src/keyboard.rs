//! Native Windows keyboard layout information used by MarkText's unchanged
//! `native-keymap` import.

#[path = "keyboard/data.rs"]
mod data;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};
use core::ffi::c_void;

type Hwnd = *mut c_void;
type Hkl = *mut c_void;
type Hkey = isize;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetKeyboardLayout(thread: u32) -> Hkl;
    fn ActivateKeyboardLayout(layout: Hkl, flags: u32) -> Hkl;
    fn GetForegroundWindow() -> Hwnd;
    fn GetWindowThreadProcessId(window: Hwnd, process: *mut u32) -> u32;
    fn GetKeyboardLayoutNameW(name: *mut u16) -> i32;
    fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
    fn ToUnicode(
        virtual_key: u32,
        scan_code: u32,
        state: *const u8,
        output: *mut u16,
        output_size: i32,
        flags: u32,
    ) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: Hkey,
        subkey: *const u16,
        options: u32,
        access: u32,
        result: *mut Hkey,
    ) -> i32;
    fn RegQueryValueExW(
        key: Hkey,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: Hkey) -> i32;
}

const VK_SHIFT: usize = 0x10;
const VK_CONTROL: usize = 0x11;
const VK_MENU: usize = 0x12;
const VK_DECIMAL: u32 = 0x6e;
const MAPVK_VK_TO_VSC: u32 = 0;
const MAPVK_VSC_TO_VK: u32 = 1;
const KL_NAMELENGTH: usize = 9;
const ERROR_SUCCESS: i32 = 0;
const REG_SZ: u32 = 1;
const KEY_READ: u32 = 0x0002_0019;
const HKEY_LOCAL_MACHINE: Hkey = -2_147_483_646;

/// Returns one realm's native module object. Layout change callbacks are
/// called from the owning GUI thread by the app host's Win32 message loop.
pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    for (name, function) in [
        (
            "getKeyMap",
            realm.function("getKeyMap", 0, false, |realm, _, _| {
                Ok(Value::Object(keymap(realm)))
            }),
        ),
        (
            "getCurrentKeyboardLayout",
            realm.function("getCurrentKeyboardLayout", 0, false, |realm, _, _| {
                Ok(current_layout(realm).map_or(Value::Null, Value::Object))
            }),
        ),
        (
            "onDidChangeKeyboardLayout",
            realm.function("onDidChangeKeyboardLayout", 1, false, |realm, _, args| {
                let Some(callback @ Value::Object(_)) = args.first() else {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "Wrong type of arguments. Expects a function as first argument.",
                    ));
                };
                if !matches!(callback, Value::Object(object) if matches!(&object.data().kind, crate::Kind::Function { .. })) {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "Wrong type of arguments. Expects a function as first argument.",
                    ));
                }
                realm.keyboard_layout_callbacks.push(callback.clone());
                Ok(Value::Undefined)
            }),
        ),
        (
            "isISOKeyboard",
            realm.function("isISOKeyboard", 1, false, |_, _, _| {
                // The Windows native-keymap binding has no ISO detector.
                Ok(Value::Undefined)
            }),
        ),
    ] {
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

fn keymap(realm: &Realm) -> Object {
    let _layout = ForegroundLayout::new();
    // SAFETY: decimal is a valid virtual key and is used only to flush dead-key
    // state after each translation.
    let clear_scan = unsafe { MapVirtualKeyW(VK_DECIMAL, MAPVK_VK_TO_VSC) };
    let result = realm.object();
    for &(scan, code) in data::CODES {
        // SAFETY: each scan code is a static entry in native-keymap's table.
        let key = unsafe { MapVirtualKeyW(scan, MAPVK_VSC_TO_VK) };
        let label = data::VKEYS
            .iter()
            .find(|(value, _)| *value == key)
            .map_or("VK_UNKNOWN", |(_, label)| *label);
        let entry = realm.object();
        for (name, value) in [
            ("vkey", label.to_owned()),
            ("value", translated(key, 0, clear_scan)),
            ("withShift", translated(key, 1, clear_scan)),
            ("withAltGr", translated(key, 6, clear_scan)),
            ("withShiftAltGr", translated(key, 7, clear_scan)),
        ] {
            entry.define(Key::name(name), Property::data(Value::str(&value)));
        }
        result.define(Key::name(code), Property::data(Value::Object(entry)));
    }
    result
}

struct ForegroundLayout(Hkl);

impl ForegroundLayout {
    fn new() -> Self {
        // SAFETY: this temporarily activates the foreground window's layout on
        // the calling GUI thread. Drop restores the original layout.
        unsafe {
            let original = GetKeyboardLayout(0);
            let foreground = GetForegroundWindow();
            if !foreground.is_null() {
                let thread = GetWindowThreadProcessId(foreground, core::ptr::null_mut());
                ActivateKeyboardLayout(GetKeyboardLayout(thread), 0);
            }
            Self(original)
        }
    }
}

impl Drop for ForegroundLayout {
    fn drop(&mut self) {
        // SAFETY: restores the keyboard layout saved by `new`.
        unsafe { ActivateKeyboardLayout(self.0, 0) };
    }
}

fn translated(key: u32, modifiers: u8, clear_scan: u32) -> String {
    let mut state = [0_u8; 256];
    for (mask, key) in [(1, VK_SHIFT), (2, VK_CONTROL), (4, VK_MENU)] {
        if modifiers & mask != 0 {
            state[key] = 0x80;
        }
    }
    let mut chars = [0_u16; 4];
    // SAFETY: buffers have the sizes passed to ToUnicode and the foreground
    // keyboard layout is active for the duration of this call.
    let count = unsafe {
        ToUnicode(
            key,
            MapVirtualKeyW(key, MAPVK_VK_TO_VSC),
            state.as_ptr(),
            chars.as_mut_ptr(),
            chars.len() as i32,
            0,
        )
    };
    let empty = [0_u8; 256];
    let mut cleared = [0_u16; 4];
    loop {
        // SAFETY: the scan code and output buffer are valid; this clears any
        // dead-key state left by the preceding translation.
        let result = unsafe {
            ToUnicode(
                VK_DECIMAL,
                clear_scan,
                empty.as_ptr(),
                cleared.as_mut_ptr(),
                cleared.len() as i32,
                0,
            )
        };
        if result >= 0 {
            break;
        }
    }
    let count = if count < 0 { 1 } else { count as usize };
    if count == 0 || (count == 1 && is_control(chars[0])) {
        return String::new();
    }
    String::from_utf16_lossy(&chars[..count.min(chars.len())])
}

fn is_control(unit: u16) -> bool {
    unit < 0x20 || (0x7f..=0x9f).contains(&unit)
}

fn current_layout(realm: &Realm) -> Option<Object> {
    let _layout = ForegroundLayout::new();
    let mut name = [0_u16; KL_NAMELENGTH];
    // SAFETY: the output is the documented KL_NAMELENGTH UTF-16 buffer.
    if unsafe { GetKeyboardLayoutNameW(name.as_mut_ptr()) } == 0 {
        return None;
    }
    let name = narrow(&name);
    let object = realm.object();
    object.define(
        Key::name("id"),
        Property::data(Value::str(&registry(&name, "Layout Id"))),
    );
    object.define(
        Key::name("text"),
        Property::data(Value::str(&registry(&name, "Layout Text"))),
    );
    object.define(Key::name("name"), Property::data(Value::str(&name)));
    Some(object)
}

fn registry(layout: &str, value_name: &str) -> String {
    let path = wide(&format!(
        "SYSTEM\\CurrentControlSet\\Control\\Keyboard Layouts\\{layout}"
    ));
    let mut key = 0;
    // SAFETY: the path is null-terminated and key is writable storage.
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &mut key) }
        != ERROR_SUCCESS
    {
        return String::new();
    }
    let value_name = wide(value_name);
    let mut value = [0_u16; 512];
    let mut size = (value.len() * 2) as u32;
    let mut kind = 0;
    // SAFETY: key remains open, value is aligned/writable and size is in bytes.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            core::ptr::null_mut(),
            &mut kind,
            value.as_mut_ptr().cast(),
            &mut size,
        )
    };
    // SAFETY: key is the handle returned by the successful RegOpenKeyExW.
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS || kind != REG_SZ {
        return String::new();
    }
    narrow(&value[..(size as usize / 2).min(value.len())])
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn narrow(value: &[u16]) -> String {
    String::from_utf16_lossy(
        &value[..value
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(value.len())],
    )
}

/// Dispatches `WM_INPUTLANGCHANGE` into the current main Realm.
///
/// # Safety
/// `context` must point to the live Realm registered by the generated app.
pub unsafe extern "system" fn layout_changed(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // SAFETY: the app host invokes this on the GUI thread after main evaluation.
    let realm = unsafe { &mut *context.cast::<Realm>() };
    let callbacks = realm.keyboard_layout_callbacks.clone();
    for callback in callbacks {
        if ops::call(realm, &callback, Value::Undefined, &[]).is_err() {
            crate::app_host::request_quit(1);
            break;
        }
    }
}

#[cfg(test)]
mod tests;
