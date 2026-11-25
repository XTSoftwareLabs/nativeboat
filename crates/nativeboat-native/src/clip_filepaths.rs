//! The `clip-filepaths` package for the unchanged dependency, without its
//! addon: file lists on the Windows clipboard as Explorer copies them
//! (`CF_HDROP`), with the behavior and messages of the package's Windows
//! build. Writing also puts the files' `file:///` URLs on the clipboard as
//! text; reading reports files and text separately.

use alloc::string::String;
use alloc::vec::Vec;

use crate::electron_clipboard::Clipboard;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
/// The size of `DROPFILES`, after which the file list starts.
const DROPFILES: usize = 20;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn GetLastError() -> u32;
    fn FormatMessageW(
        flags: u32,
        source: *const core::ffi::c_void,
        message: u32,
        language: u32,
        buffer: *mut u16,
        size: u32,
        arguments: *const core::ffi::c_void,
    ) -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A Windows error as Rust's `io::Error` displays it.
fn os_error(code: u32) -> String {
    let mut buffer = [0u16; 512];
    // SAFETY: the buffer is writable for its length; system messages need no
    // source or arguments (FORMAT_MESSAGE_FROM_SYSTEM | IGNORE_INSERTS).
    let length = unsafe {
        FormatMessageW(
            0x1000 | 0x200,
            core::ptr::null(),
            code,
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            core::ptr::null(),
        )
    } as usize;
    let message = String::from_utf16_lossy(&buffer[..length]);
    alloc::format!("{} (os error {code})", message.trim_end())
}

/// A `DROPFILES` block listing `paths` in UTF-16.
fn drop_files(paths: &[String]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(DROPFILES + 64 * paths.len());
    bytes.extend((DROPFILES as u32).to_le_bytes());
    bytes.extend([0; 12]); // the drop point and fNC
    bytes.extend(1u32.to_le_bytes()); // fWide
    for path in paths {
        for unit in wide(path) {
            bytes.extend(unit.to_le_bytes());
        }
    }
    bytes.extend([0, 0]);
    bytes
}

/// The paths a `DROPFILES` block lists.
fn dropped_paths(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < DROPFILES {
        return Vec::new();
    }
    let offset = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let wide = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) != 0;
    let list = bytes.get(offset..).unwrap_or_default();
    let mut paths = Vec::new();
    if wide {
        let units: Vec<u16> = list
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        for name in units.split(|unit| *unit == 0) {
            if name.is_empty() {
                break;
            }
            paths.push(String::from_utf16_lossy(name));
        }
    } else {
        for name in list.split(|byte| *byte == 0) {
            if name.is_empty() {
                break;
            }
            paths.push(name.iter().map(|byte| char::from(*byte)).collect());
        }
    }
    paths
}

/// The `file:///` URLs the package writes as text: relative paths resolve
/// against the working directory and separators become slashes.
fn file_urls(paths: &[String], directory: &str) -> String {
    let urls: Vec<String> = paths
        .iter()
        .map(|path| {
            let absolute = path.starts_with('\\')
                || path.starts_with('/')
                || path.as_bytes().get(1) == Some(&b':');
            let full = if absolute {
                path.clone()
            } else {
                alloc::format!("{}\\{path}", directory.trim_end_matches('\\'))
            };
            alloc::format!("file:///{}", full.replace('\\', "/"))
        })
        .collect();
    urls.join("\n")
}

fn clipboard_error(realm: &Realm, message: &str) -> Thrown {
    realm.throw(
        ErrorKind::Error,
        &alloc::format!("windows clipboard error: {message}"),
    )
}

fn text_bytes(text: &str) -> Vec<u8> {
    wide(text)
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect()
}

fn write(realm: &mut Realm, args: &[Value]) -> Result<Value, Thrown> {
    let list = args.first().cloned().unwrap_or(Value::Undefined);
    if list.as_object().is_none() {
        return Err(realm.throw(
            ErrorKind::Type,
            "Failed to convert JavaScript value `Undefined` into rust type `Vec<String>`",
        ));
    }
    let count = crate::array_builtin::length_of(realm, &list)?;
    let mut paths = Vec::new();
    for index in 0..count {
        let path = ops::get(realm, &list, &Key::Index(index as u32))?;
        paths.push(ops::to_string(realm, &path)?.to_rust_lossy());
    }
    // Every path must exist, as the package checks before writing.
    let mut missing = Vec::new();
    for path in &paths {
        let name = wide(path);
        // SAFETY: the path is NUL-terminated.
        if unsafe { GetFileAttributesW(name.as_ptr()) } == INVALID_FILE_ATTRIBUTES {
            // SAFETY: reads this thread's latest Windows error.
            let code = unsafe { GetLastError() };
            missing.push(alloc::format!(
                "Failed to canonicalize path {path}: {}",
                os_error(code)
            ));
        }
    }
    if !missing.is_empty() {
        return Err(clipboard_error(
            realm,
            &alloc::format!("Some paths could not be processed: {}", missing.join("; ")),
        ));
    }
    let clipboard = Clipboard::open().map_err(|code| {
        clipboard_error(realm, &alloc::format!("Failed to open clipboard: {code}"))
    })?;
    clipboard.empty().map_err(|code| {
        clipboard_error(realm, &alloc::format!("Failed to empty clipboard: {code}"))
    })?;
    if paths.is_empty() {
        return Ok(Value::Undefined);
    }
    clipboard
        .set(CF_HDROP, &drop_files(&paths))
        .map_err(|code| {
            clipboard_error(
                realm,
                &alloc::format!("Failed to set clipboard data: {code}"),
            )
        })?;
    let directory = crate::platform::current_directory()
        .map(|directory| directory.to_rust_lossy())
        .unwrap_or_default();
    // The text form is a convenience; failing to add it is not an error.
    let _ = clipboard.set(CF_UNICODETEXT, &text_bytes(&file_urls(&paths, &directory)));
    Ok(Value::Undefined)
}

fn read(realm: &mut Realm) -> Result<Value, Thrown> {
    // Only a clipboard that cannot be opened fails both reads.
    let clipboard = Clipboard::open().map_err(|code| {
        let reason = alloc::format!("windows clipboard error: Failed to open clipboard: {code}");
        realm.throw(
            ErrorKind::Error,
            &alloc::format!(
                "Failed to read clipboard content: file paths error: {reason}, text error: {reason}"
            ),
        )
    })?;
    let paths = clipboard
        .read(CF_HDROP)
        .map(|bytes| dropped_paths(&bytes))
        .unwrap_or_default();
    let text = clipboard.read(CF_UNICODETEXT).map(|bytes| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    });
    drop(clipboard);
    let result = realm.object();
    let list: Vec<Value> = paths.iter().map(|path| Value::str(path)).collect();
    result.define(
        Key::name("filePaths"),
        Property::data(Value::Object(realm.array(list))),
    );
    if let Some(text) = text {
        result.define(Key::name("text"), Property::data(Value::str(&text)));
    }
    Ok(Value::Object(result))
}

/// The package's exports.
pub(crate) fn module(realm: &Realm) -> Object {
    let exports = realm.object();
    let hello = realm.function("helloWorld", 0, false, |_, _, _| {
        Ok(Value::str("Hello from Rust 0.3.0 on Windows!"))
    });
    exports.define(
        Key::name("helloWorld"),
        Property::data(Value::Object(hello)),
    );
    let write = realm.function("writeClipboardFilePaths", 1, false, |realm, _, args| {
        write(realm, args)
    });
    exports.define(
        Key::name("writeClipboardFilePaths"),
        Property::data(Value::Object(write)),
    );
    let read = realm.function("readClipboardFilePaths", 0, false, |realm, _, _| {
        read(realm)
    });
    exports.define(
        Key::name("readClipboardFilePaths"),
        Property::data(Value::Object(read)),
    );
    exports
}

#[cfg(test)]
mod tests;
