//! The system clipboard: text, HTML, images and named binary formats.

#[allow(unused_imports)]
use crate::prelude::*;
mod image;
pub use image::{read_clipboard_image, write_clipboard_image};

use crate::Error;
use crate::wide::wide;
use core::ptr;
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

/// A message-only window lets EmptyClipboard assign ownership to this process.
struct Owner(HWND);

impl Owner {
    fn new() -> Result<Self, Error> {
        let class = wide("STATIC");
        // SAFETY: STATIC is a system class; the parent makes this an invisible
        // message-only window owned by the current thread, with no user data.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(Error::last("CreateWindowExW"));
        }
        Ok(Self(window))
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: this scope owns the window and closes the clipboard first.
        unsafe { DestroyWindow(self.0) };
    }
}

/// The clipboard, held open until dropped.
struct Open;

impl Open {
    fn new(owner: HWND) -> Result<Open, Error> {
        // Another program may hold the clipboard for a moment.
        for _ in 0..10 {
            // SAFETY: owner is null for reads or this thread's owned window.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Open);
            }
            // SAFETY: Sleep takes a scalar duration.
            unsafe { windows_sys::Win32::System::Threading::Sleep(10) };
        }
        Err(Error::last("OpenClipboard"))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard.
        unsafe { CloseClipboard() };
    }
}

pub(crate) fn html_format() -> u32 {
    let name = wide("HTML Format");
    // SAFETY: the name is NUL-terminated and outlives the call.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn registered_format(name: &str) -> Result<u32, Error> {
    if name.is_empty() || name.contains('\0') {
        return Err(Error {
            message: "Clipboard format must be nonempty and contain no NUL".into(),
        });
    }
    let name = wide(name);
    // SAFETY: name is a nonempty NUL-terminated buffer that outlives the call.
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        return Err(Error::last("RegisterClipboardFormatW"));
    }
    Ok(format)
}

/// The bytes of a registered clipboard format, without text conversion.
pub fn read_clipboard_buffer(name: &str) -> Vec<u8> {
    registered_format(name)
        .ok()
        .and_then(read)
        .unwrap_or_default()
}

/// Whether a registered format is on the clipboard.
pub fn has_clipboard_format(name: &str) -> bool {
    let Ok(format) = registered_format(name) else {
        return false;
    };
    // SAFETY: the registered format ID is valid; querying needs no open owner.
    unsafe { IsClipboardFormatAvailable(format) != 0 }
}

/// Replaces the clipboard with an unmodified named binary payload.
pub fn write_clipboard_buffer(name: &str, data: &[u8]) -> Result<(), Error> {
    write(&[(registered_format(name)?, data.to_vec())])
}

/// The bytes of a clipboard format, if it holds any.
fn read(format: u32) -> Option<Vec<u8>> {
    let _open = Open::new(ptr::null_mut()).ok()?;
    read_open(format)
}

/// Reads one format while this thread already holds the clipboard open.
fn read_open(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is open; the handle stays owned by it.
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: clipboard data handles are global memory handles.
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the memory is locked and holds `GlobalSize` bytes.
    let bytes = unsafe {
        let size = GlobalSize(handle);
        core::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec()
    };
    // SAFETY: unlocks what was locked above.
    unsafe { GlobalUnlock(handle) };
    Some(bytes)
}

/// Replaces the clipboard contents with the given formats.
fn write(entries: &[(u32, Vec<u8>)]) -> Result<(), Error> {
    let owner = Owner::new()?;
    let _open = Open::new(owner.0)?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(Error::last("EmptyClipboard"));
    }
    for (format, data) in entries {
        // SAFETY: allocates movable global memory for the clipboard.
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) };
        if memory.is_null() {
            return Err(Error::last("GlobalAlloc"));
        }
        if !data.is_empty() {
            // SAFETY: locks the owned allocation before writing its bytes.
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                let error = Error::last("GlobalLock");
                // SAFETY: no clipboard operation has taken this allocation.
                unsafe { GlobalFree(memory) };
                return Err(error);
            }
            // SAFETY: the successful lock holds at least `data.len()` bytes.
            unsafe {
                ptr::copy_nonoverlapping(data.as_ptr(), pointer.cast::<u8>(), data.len());
                GlobalUnlock(memory);
            }
        }
        // SAFETY: on success the clipboard owns the memory.
        if unsafe { SetClipboardData(*format, memory) }.is_null() {
            let error = Error::last("SetClipboardData");
            // SAFETY: the clipboard did not take the memory.
            unsafe { GlobalFree(memory) };
            return Err(error);
        }
    }
    Ok(())
}

fn utf16(text: &str) -> Vec<u8> {
    wide(text)
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

/// The clipboard's plain text, or an empty string.
pub fn read_text() -> String {
    decode_text(&read(u32::from(CF_UNICODETEXT)).unwrap_or_default())
}

pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The HTML fragment on the clipboard, or an empty string.
pub fn read_html() -> String {
    decode_html(&read(html_format()).unwrap_or_default())
}

pub(crate) fn decode_html(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let offset = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.trim().parse::<usize>().ok())
    };
    match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) if start <= end && end <= bytes.len() => {
            String::from_utf8_lossy(&bytes[start..end]).into_owned()
        }
        _ => String::new(),
    }
}

/// A consistent snapshot of the supported page clipboard formats. Missing
/// formats are omitted; an empty value remains distinct from an absent format.
/// `None` means the clipboard could not be opened.
pub fn read_clipboard_contents() -> Option<Vec<(&'static str, String)>> {
    let _open = Open::new(ptr::null_mut()).ok()?;
    let mut contents = Vec::new();
    if let Some(bytes) = read_open(u32::from(CF_UNICODETEXT)) {
        contents.push(("text/plain", decode_text(&bytes)));
    }
    if let Some(bytes) = read_open(html_format()) {
        contents.push(("text/html", decode_html(&bytes)));
    }
    Some(contents)
}

/// HTML in the clipboard's `HTML Format`: a header of byte offsets, then
/// the document with the fragment marked.
pub(crate) fn cf_html(fragment: &str) -> Vec<u8> {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let prefix = "<html><body>\r\n<!--StartFragment-->";
    let suffix = "<!--EndFragment-->\r\n</body></html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + prefix.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + suffix.len();
    format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{prefix}{fragment}{suffix}"
    )
    .into_bytes()
}

/// Replaces the clipboard with text and, optionally, HTML.
pub fn write_clipboard(text: Option<&str>, html: Option<&str>) -> Result<(), Error> {
    write(&text_entries(text, html))
}

fn text_entries(text: Option<&str>, html: Option<&str>) -> Vec<(u32, Vec<u8>)> {
    let mut entries = Vec::new();
    if let Some(text) = text {
        entries.push((u32::from(CF_UNICODETEXT), utf16(text)));
    }
    if let Some(html) = html {
        entries.push((html_format(), cf_html(html)));
    }
    entries
}

/// The MIME types the clipboard holds.
pub fn clipboard_formats() -> Vec<&'static str> {
    let mut formats = Vec::new();
    // SAFETY: format queries need no open clipboard.
    if unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) } != 0 {
        formats.push("text/plain");
    }
    // SAFETY: as above.
    if unsafe { IsClipboardFormatAvailable(html_format()) } != 0 {
        formats.push("text/html");
    }
    let png = registered_format("PNG").unwrap_or_default();
    // SAFETY: these format queries need no open clipboard.
    if unsafe {
        (png != 0 && IsClipboardFormatAvailable(png) != 0)
            || IsClipboardFormatAvailable(u32::from(windows_sys::Win32::System::Ole::CF_DIB)) != 0
            || IsClipboardFormatAvailable(u32::from(windows_sys::Win32::System::Ole::CF_DIBV5)) != 0
    } {
        formats.push("image/png");
    }
    formats
}

#[cfg(test)]
mod tests;
