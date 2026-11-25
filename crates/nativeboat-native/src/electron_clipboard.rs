//! Windows clipboard text and named-buffer operations exposed by Electron.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::ffi::c_void;

use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

type Hwnd = *mut c_void;
type Hglobal = *mut c_void;

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
const GMEM_MOVEABLE: u32 = 0x0002;
const ERROR_INVALID_PARAMETER: u32 = 87;
const STATIC_CLASS: [u16; 7] = [
    b'S' as u16,
    b'T' as u16,
    b'A' as u16,
    b'T' as u16,
    b'I' as u16,
    b'C' as u16,
    0,
];
const EMPTY_TITLE: [u16; 1] = [0];

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenClipboard(owner: Hwnd) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn GetClipboardData(format: u32) -> Hglobal;
    fn SetClipboardData(format: u32, memory: Hglobal) -> Hglobal;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn CreateWindowExW(
        extended_style: u32,
        class_name: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: *mut c_void,
        parameter: *mut c_void,
    ) -> Hwnd;
    fn DestroyWindow(window: Hwnd) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn GlobalAlloc(flags: u32, bytes: usize) -> Hglobal;
    fn GlobalLock(memory: Hglobal) -> *mut c_void;
    fn GlobalUnlock(memory: Hglobal) -> i32;
    fn GlobalFree(memory: Hglobal) -> Hglobal;
    fn GlobalSize(memory: Hglobal) -> usize;
}

/// The open clipboard, closed when dropped.
pub(crate) struct Clipboard {
    owner: Hwnd,
}

impl Clipboard {
    pub(crate) fn open() -> Result<Self, u32> {
        // A real owner is required for EmptyClipboard/SetClipboardData. STATIC
        // is a built-in class, so this hidden zero-sized owner needs no class
        // registration or message procedure.
        // SAFETY: the class string is static and all optional handles are null.
        let owner = unsafe {
            CreateWindowExW(
                0,
                STATIC_CLASS.as_ptr(),
                EMPTY_TITLE.as_ptr(),
                0x8000_0000,
                0,
                0,
                0,
                0,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if owner.is_null() {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        // SAFETY: `owner` is a live HWND created above.
        if unsafe { OpenClipboard(owner) } == 0 {
            // SAFETY: owner remains live and is owned by this guard.
            unsafe { DestroyWindow(owner) };
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        Ok(Self { owner })
    }

    pub(crate) fn read(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: this object keeps the clipboard open.
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: clipboard-owned global memory may be locked while open.
        let data = unsafe { GlobalLock(handle) };
        if data.is_null() {
            return None;
        }
        // SAFETY: the lock remains held while the clipboard allocation is copied.
        let bytes = unsafe {
            let length = GlobalSize(handle);
            core::slice::from_raw_parts(data.cast::<u8>(), length).to_vec()
        };
        // SAFETY: releases the lock obtained above.
        unsafe { GlobalUnlock(handle) };
        Some(bytes)
    }

    pub(crate) fn replace(&self, format: u32, bytes: &[u8]) -> Result<(), u32> {
        self.empty()?;
        self.set(format, bytes)
    }

    /// Clears the clipboard and takes ownership of it.
    pub(crate) fn empty(&self) -> Result<(), u32> {
        // SAFETY: this object owns the opened clipboard.
        if unsafe { EmptyClipboard() } == 0 {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        Ok(())
    }

    /// Adds one format to the clipboard's current contents.
    pub(crate) fn set(&self, format: u32, bytes: &[u8]) -> Result<(), u32> {
        // SAFETY: movable global memory is the required clipboard payload form.
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) };
        if memory.is_null() {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        // SAFETY: the newly allocated memory is owned and writable.
        let destination = unsafe { GlobalLock(memory) };
        if destination.is_null() {
            // SAFETY: no clipboard operation took ownership of this allocation.
            unsafe { GlobalFree(memory) };
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        if !bytes.is_empty() {
            // SAFETY: the allocation is at least `bytes.len()` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast(), bytes.len())
            };
        }
        // SAFETY: releases the lock before transferring ownership to Windows.
        unsafe { GlobalUnlock(memory) };
        // SAFETY: on success the system clipboard owns `memory`.
        if unsafe { SetClipboardData(format, memory) }.is_null() {
            // SAFETY: SetClipboardData did not take ownership on failure.
            unsafe { GlobalFree(memory) };
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        Ok(())
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: this guard successfully opened the clipboard and owns `owner`.
        unsafe {
            CloseClipboard();
            DestroyWindow(self.owner);
        }
    }
}

trait ClipboardBackend {
    fn read(&self, format: u32) -> Result<Option<Vec<u8>>, u32>;
    fn replace(&self, format: u32, bytes: &[u8]) -> Result<(), u32>;
    fn register_format(&self, name: &str) -> Result<u32, u32>;
    fn available(&self, format: u32) -> bool;
}

struct WindowsClipboardBackend;

impl ClipboardBackend for WindowsClipboardBackend {
    fn read(&self, format: u32) -> Result<Option<Vec<u8>>, u32> {
        let clipboard = Clipboard::open()?;
        Ok(clipboard.read(format))
    }

    fn replace(&self, format: u32, bytes: &[u8]) -> Result<(), u32> {
        Clipboard::open()?.replace(format, bytes)
    }

    fn register_format(&self, name: &str) -> Result<u32, u32> {
        register_format(name)
    }

    fn available(&self, format: u32) -> bool {
        // SAFETY: format availability can be queried without opening the clipboard.
        unsafe { IsClipboardFormatAvailable(format) != 0 }
    }
}

/// Creates the supported Electron `clipboard` module for native Windows apps.
pub fn module(realm: &Realm) -> Object {
    module_with(realm, Rc::new(WindowsClipboardBackend))
}

fn module_with(realm: &Realm, backend: Rc<dyn ClipboardBackend>) -> Object {
    let clipboard = realm.object();
    let read_backend = backend.clone();
    clipboard.define(
        Key::name("readText"),
        Property::data(Value::Object(realm.function(
            "readText",
            1,
            false,
            move |_, _, _| {
                let text = read_backend
                    .read(CF_UNICODETEXT)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                Ok(Value::String(crate::string::JsStr::from_units(
                    &decode_utf16(&text),
                )))
            },
        ))),
    );
    let write_backend = backend.clone();
    clipboard.define(
        Key::name("writeText"),
        Property::data(Value::Object(realm.function(
            "writeText",
            2,
            false,
            move |realm, _, args| {
                let text = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;
                let bytes = encode_utf16(text.units());
                write_backend
                    .replace(CF_UNICODETEXT, &bytes)
                    .map_err(|code| clipboard_error(realm, "writeText", code))?;
                Ok(Value::Undefined)
            },
        ))),
    );
    let image_backend = backend.clone();
    clipboard.define(
        Key::name("writeImage"),
        Property::data(Value::Object(realm.function(
            "writeImage",
            1,
            false,
            move |realm, _, args| {
                let image = args.first().unwrap_or(&Value::Undefined);
                let bitmap = crate::electron_native_image::dibv5(realm, image)?;
                image_backend
                    .replace(CF_DIBV5, &bitmap)
                    .map_err(|code| clipboard_error(realm, "writeImage", code))?;
                Ok(Value::Undefined)
            },
        ))),
    );
    let read_image_backend = backend.clone();
    clipboard.define(
        Key::name("readImage"),
        Property::data(Value::Object(realm.function(
            "readImage",
            1,
            false,
            move |realm, _, _| {
                let png = read_image_backend
                    .register_format("PNG")
                    .ok()
                    .and_then(|format| read_image_backend.read(format).ok().flatten());
                let dib = read_image_backend
                    .read(CF_DIBV5)
                    .ok()
                    .flatten()
                    .or_else(|| read_image_backend.read(CF_DIB).ok().flatten());
                Ok(Value::Object(
                    crate::electron_native_image::clipboard_image(realm, png, dib),
                ))
            },
        ))),
    );
    let buffer_backend = backend.clone();
    clipboard.define(
        Key::name("readBuffer"),
        Property::data(Value::Object(realm.function(
            "readBuffer",
            1,
            false,
            move |realm, _, args| {
                let name = format_name(realm, args.first().unwrap_or(&Value::Undefined))?;
                let format = buffer_backend
                    .register_format(&name)
                    .map_err(|code| clipboard_error(realm, "readBuffer", code))?;
                let bytes = buffer_backend
                    .read(format)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                buffer_value(realm, bytes)
            },
        ))),
    );
    let has_backend = backend;
    clipboard.define(
        Key::name("has"),
        Property::data(Value::Object(realm.function(
            "has",
            1,
            false,
            move |realm, _, args| {
                let name = format_name(realm, args.first().unwrap_or(&Value::Undefined))?;
                let available = if name == "text/plain" || name == "Text" {
                    has_backend.available(CF_UNICODETEXT)
                } else {
                    let format = has_backend
                        .register_format(&name)
                        .map_err(|code| clipboard_error(realm, "has", code))?;
                    has_backend.available(format)
                };
                Ok(Value::Boolean(available))
            },
        ))),
    );
    clipboard
}

fn format_name(realm: &mut Realm, value: &Value) -> Result<alloc::string::String, Thrown> {
    Ok(ops::to_string(realm, value)?.to_rust_lossy())
}

fn register_format(name: &str) -> Result<u32, u32> {
    if name.is_empty() || name.contains('\0') {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    // SAFETY: `wide` is NUL-terminated and lives through registration.
    let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if format == 0 {
        // SAFETY: GetLastError has no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(format)
    }
}

/// A Node `Buffer` of `bytes`, or a `Uint8Array` without Node's library.
pub(crate) fn buffer_value(realm: &mut Realm, bytes: Vec<u8>) -> Result<Value, Thrown> {
    if realm.buffer_prototype.is_none()
        && let Some(require) = realm.node_library
    {
        let _ = require(realm, "buffer")?;
    }
    let prototype = realm.buffer_prototype.clone();
    Ok(Value::Object(crate::typed_array_builtin::new_bytes(
        realm, bytes, prototype,
    )))
}

fn clipboard_error(realm: &Realm, operation: &str, code: u32) -> Thrown {
    realm.throw(
        ErrorKind::Error,
        &alloc::format!("clipboard.{operation} failed (Windows error {code})"),
    )
}

fn encode_utf16(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((units.len() + 1) * 2);
    bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    bytes.extend_from_slice(&[0, 0]);
    bytes
}

fn decode_utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect()
}

#[cfg(test)]
#[path = "electron_clipboard/tests.rs"]
mod tests;
