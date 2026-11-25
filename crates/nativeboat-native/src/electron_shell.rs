//! Native Windows shell operations exposed by Electron's `shell` module.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;

#[repr(C)]
struct FileOperation {
    window: Hwnd,
    operation: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    aborted: i32,
    mappings: *mut c_void,
    progress_title: *const u16,
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ShellExecuteW(
        window: Hwnd,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> Hinstance;
    fn SHFileOperationW(operation: *mut FileOperation) -> i32;
}

const SW_SHOWNORMAL: i32 = 1;
const FO_DELETE: u32 = 3;
const FOF_SILENT: u16 = 0x0004;
const FOF_NOCONFIRMATION: u16 = 0x0010;
const FOF_ALLOWUNDO: u16 = 0x0040;
const FOF_NOERRORUI: u16 = 0x0400;

/// Builds the supported Electron `shell` exports for a native Windows host.
pub fn module(realm: &Realm) -> Object {
    let shell = realm.object();
    shell.define(
        Key::name("openExternal"),
        Property::data(Value::Object(realm.function(
            "openExternal",
            2,
            false,
            |realm, _, args| {
                let target = string_arg(realm, args, 0, "url")?;
                if target.len() > 2081 {
                    return Err(
                        realm.throw(ErrorKind::Range, "URL exceeds the Windows shell limit")
                    );
                }
                let directory = option_string(realm, args, "workingDirectory")?;
                let result = execute(&target, None, directory.as_deref());
                let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
                match result {
                    Ok(()) => {
                        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
                    }
                    Err(code) => {
                        let error = realm.error(
                            ErrorKind::Error,
                            &format!("Cannot open external target (Windows shell error {code})"),
                        );
                        ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
                    }
                }
                Ok(promise)
            },
        ))),
    );
    shell.define(
        Key::name("openPath"),
        Property::data(Value::Object(realm.function(
            "openPath",
            1,
            false,
            |realm, _, args| {
                let path = string_arg(realm, args, 0, "path")?;
                let message = execute(&path, None, None)
                    .err()
                    .map_or_else(String::new, shell_error);
                let (promise, resolve, _) = crate::promise_builtin::capability(realm);
                ops::call(realm, &resolve, Value::Undefined, &[Value::str(&message)])?;
                Ok(promise)
            },
        ))),
    );
    shell.define(
        Key::name("showItemInFolder"),
        Property::data(Value::Object(realm.function(
            "showItemInFolder",
            1,
            false,
            |realm, _, args| {
                let path = string_arg(realm, args, 0, "path")?;
                let parameters = format!("/select,\"{path}\"");
                execute("explorer.exe", Some(&parameters), None).map_err(|code| {
                    realm.throw(
                        ErrorKind::Error,
                        &format!("Cannot reveal item in Explorer (Windows shell error {code})"),
                    )
                })?;
                Ok(Value::Undefined)
            },
        ))),
    );
    shell.define(
        Key::name("trashItem"),
        Property::data(Value::Object(realm.function(
            "trashItem",
            1,
            false,
            |realm, _, args| {
                let path = string_arg(realm, args, 0, "path")?;
                let result = trash(&path);
                let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
                match result {
                    Ok(()) => {
                        ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
                    }
                    Err(code) => {
                        let error = realm.error(
                            ErrorKind::Error,
                            &format!(
                                "Cannot move item to the Recycle Bin (Windows shell error {code})"
                            ),
                        );
                        ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
                    }
                }
                Ok(promise)
            },
        ))),
    );
    shell
}

fn string_arg(
    realm: &mut Realm,
    args: &[Value],
    index: usize,
    name: &str,
) -> Result<String, Thrown> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.to_rust_lossy()),
        _ => Err(realm.throw(ErrorKind::Type, &format!("{name} must be a string"))),
    }
}

fn option_string(realm: &mut Realm, args: &[Value], name: &str) -> Result<Option<String>, Thrown> {
    let Some(options) = args.get(1) else {
        return Ok(None);
    };
    if matches!(options, Value::Undefined | Value::Null) {
        return Ok(None);
    }
    if !matches!(options, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "options must be an object"));
    }
    match ops::get(realm, options, &Key::name(name))? {
        Value::String(value) => Ok(Some(value.to_rust_lossy())),
        Value::Undefined | Value::Null => Ok(None),
        _ => Err(realm.throw(ErrorKind::Type, &format!("{name} must be a string"))),
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(core::iter::once(0)).collect()
}

fn execute(target: &str, parameters: Option<&str>, directory: Option<&str>) -> Result<(), i32> {
    let operation = wide("open");
    let target = wide(target);
    let parameters = parameters.map(wide);
    let directory = directory.map(wide);
    // SAFETY: all optional and required arguments are valid, null-terminated
    // UTF-16 strings and remain alive for the duration of this call.
    let result = unsafe {
        ShellExecuteW(
            core::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            parameters
                .as_ref()
                .map_or(core::ptr::null(), |value| value.as_ptr()),
            directory
                .as_ref()
                .map_or(core::ptr::null(), |value| value.as_ptr()),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(result as i32)
    }
}

fn trash(path: &str) -> Result<(), i32> {
    let mut source = path.encode_utf16().collect::<Vec<_>>();
    source.extend([0, 0]);
    let mut operation = FileOperation {
        window: core::ptr::null_mut(),
        operation: FO_DELETE,
        from: source.as_ptr(),
        to: core::ptr::null(),
        flags: FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_NOERRORUI,
        aborted: 0,
        mappings: core::ptr::null_mut(),
        progress_title: core::ptr::null(),
    };
    // SAFETY: the operation structure and double-null-terminated source path
    // are writable/alive for the duration of SHFileOperationW.
    let status = unsafe { SHFileOperationW(&mut operation) };
    if status == 0 && operation.aborted == 0 {
        Ok(())
    } else if status != 0 {
        Err(status)
    } else {
        Err(0x75)
    }
}

fn shell_error(code: i32) -> String {
    match code {
        2 => "The target file was not found".into(),
        3 => "The target path was not found".into(),
        5 => "Access to the target was denied".into(),
        8 => "There is not enough memory to complete the operation".into(),
        31 => "No application is associated with the target".into(),
        _ => format!("Windows shell operation failed ({code})"),
    }
}

#[cfg(test)]
#[path = "electron_shell/tests.rs"]
mod tests;
