//! `app.setAppUserModelId`, `app.setLoginItemSettings` and
//! `app.getLoginItemSettings` on Windows, as Electron keeps them. A login item
//! is a value under the current user's `Run` key, named after the settings'
//! `name` or else the AppUserModelId, holding the quoted executable path and
//! its arguments; `enabled: false` marks it disabled under Explorer's
//! `StartupApproved\Run` key, as the Task Manager's startup page does.
//!
//! `app.setAsDefaultProtocolClient`, `isDefaultProtocolClient` and
//! `removeAsDefaultProtocolClient` keep a URL scheme's handler under the
//! current user's `Software\Classes`, as Electron does on Windows: the
//! scheme's key is named `URL:<scheme>` with an empty `URL Protocol` value,
//! and its `shell\open\command` runs the quoted executable with the
//! arguments and the quoted URL.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::ffi::c_void;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Object, Property, Realm, Thrown, Value, ops};

type Hkey = isize;

const HKEY_CURRENT_USER: Hkey = 0x8000_0001_u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002_u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;
const KEY_ALL_ACCESS: u32 = 0xf_003f;
const REG_SZ: u32 = 1;
const REG_BINARY: u32 = 3;
const ERROR_SUCCESS: i32 = 0;

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const CLASSES: &str = "Software\\Classes";
const APPROVED: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: Hkey,
        path: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        security: *const c_void,
        result: *mut Hkey,
        disposition: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(
        key: Hkey,
        path: *const u16,
        options: u32,
        access: u32,
        result: *mut Hkey,
    ) -> i32;
    fn RegSetValueExW(
        key: Hkey,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> i32;
    fn RegQueryValueExW(
        key: Hkey,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegEnumValueW(
        key: Hkey,
        index: u32,
        name: *mut u16,
        name_length: *mut u32,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegDeleteValueW(key: Hkey, name: *const u16) -> i32;
    fn RegDeleteTreeW(key: Hkey, path: *const u16) -> i32;
    fn RegDeleteKeyW(key: Hkey, path: *const u16) -> i32;
    fn RegQueryInfoKeyW(
        key: Hkey,
        class: *mut u16,
        class_length: *mut u32,
        reserved: *mut u32,
        subkeys: *mut u32,
        longest_subkey: *mut u32,
        longest_class: *mut u32,
        values: *mut u32,
        longest_value_name: *mut u32,
        longest_value: *mut u32,
        security: *mut u32,
        written: *mut c_void,
    ) -> i32;
    fn RegCloseKey(key: Hkey) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
    fn CommandLineToArgvW(line: *const u16, count: *mut i32) -> *mut *mut u16;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A string value under `HKEY_CURRENT_USER`, if the key and value exist.
pub(crate) fn registry_string(path: &str, name: &str) -> Option<String> {
    let (kind, data) = RegKey::open(HKEY_CURRENT_USER, path, false)?.get(name)?;
    if kind != REG_SZ {
        return None;
    }
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// A DWORD value under `HKEY_CURRENT_USER`, if the key and value exist.
pub(crate) fn registry_dword(path: &str, name: &str) -> Option<u32> {
    const REG_DWORD: u32 = 4;
    let (kind, data) = RegKey::open(HKEY_CURRENT_USER, path, false)?.get(name)?;
    (kind == REG_DWORD && data.len() == 4)
        .then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

/// An open registry key, closed when dropped.
struct RegKey(Hkey);

impl RegKey {
    fn open(root: Hkey, path: &str, write: bool) -> Option<Self> {
        let path = wide(path);
        let mut key = 0;
        // SAFETY: the path is NUL-terminated and `key` receives the handle.
        let status = unsafe {
            if write {
                RegCreateKeyExW(
                    root,
                    path.as_ptr(),
                    0,
                    core::ptr::null(),
                    0,
                    KEY_ALL_ACCESS,
                    core::ptr::null(),
                    &mut key,
                    core::ptr::null_mut(),
                )
            } else {
                RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut key)
            }
        };
        (status == ERROR_SUCCESS).then_some(Self(key))
    }

    fn set(&self, name: &str, kind: u32, data: &[u8]) -> bool {
        let name = wide(name);
        // SAFETY: the name is NUL-terminated and the data slice is readable.
        unsafe {
            RegSetValueExW(
                self.0,
                name.as_ptr(),
                0,
                kind,
                data.as_ptr(),
                data.len() as u32,
            ) == ERROR_SUCCESS
        }
    }

    fn set_string(&self, name: &str, value: &str) -> bool {
        let bytes: Vec<u8> = wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        self.set(name, REG_SZ, &bytes)
    }

    /// Whether the key has no values and no subkeys.
    fn is_empty(&self) -> bool {
        let (mut subkeys, mut values) = (0, 0);
        // SAFETY: only the counts are asked for; the other outputs are null.
        let status = unsafe {
            RegQueryInfoKeyW(
                self.0,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut subkeys,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut values,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        status == ERROR_SUCCESS && subkeys == 0 && values == 0
    }

    fn delete(&self, name: &str) {
        let name = wide(name);
        // SAFETY: the name is NUL-terminated; a missing value is no error.
        unsafe { RegDeleteValueW(self.0, name.as_ptr()) };
    }

    fn get(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        let name = wide(name);
        let mut kind = 0;
        let mut size = 0;
        // SAFETY: a null buffer asks for the value's size.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                core::ptr::null_mut(),
                &mut kind,
                core::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut data = alloc::vec![0; size as usize];
        // SAFETY: the buffer holds `size` writable bytes.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name.as_ptr(),
                core::ptr::null_mut(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        data.truncate(size as usize);
        (status == ERROR_SUCCESS).then_some((kind, data))
    }

    /// The key's string values, by name.
    fn strings(&self) -> Vec<(String, String)> {
        let mut values = Vec::new();
        for index in 0.. {
            let mut name = alloc::vec![0u16; 16_384];
            let mut name_length = name.len() as u32;
            let mut kind = 0;
            let mut size = 0;
            // SAFETY: the name buffer holds `name_length` units; a null data
            // buffer asks for the value's size.
            let status = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    name.as_mut_ptr(),
                    &mut name_length,
                    core::ptr::null_mut(),
                    &mut kind,
                    core::ptr::null_mut(),
                    &mut size,
                )
            };
            if status != ERROR_SUCCESS {
                break;
            }
            let name = String::from_utf16_lossy(&name[..name_length as usize]);
            if let Some((REG_SZ | 2, data)) = self.get(&name) {
                values.push((name, string_value(&data)));
            }
        }
        values
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: the handle is owned here.
        unsafe { RegCloseKey(self.0) };
    }
}

fn string_value(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// A command line's program and arguments, split as Windows does.
fn split_command(line: &str) -> Vec<String> {
    let line = wide(line);
    let mut count = 0;
    // SAFETY: the line is NUL-terminated; the returned block is freed below.
    let argv = unsafe { CommandLineToArgvW(line.as_ptr(), &mut count) };
    if argv.is_null() {
        return Vec::new();
    }
    let mut parts = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        // SAFETY: CommandLineToArgvW returns `count` NUL-terminated strings.
        unsafe {
            let argument = *argv.add(index);
            let length = (0..)
                .take_while(|offset| *argument.add(*offset) != 0)
                .count();
            parts.push(String::from_utf16_lossy(core::slice::from_raw_parts(
                argument, length,
            )));
        }
    }
    // SAFETY: the block came from CommandLineToArgvW.
    unsafe { LocalFree(argv.cast()) };
    parts
}

/// Electron's `FormatCommandLineString`: the quoted program, then the
/// arguments joined by spaces.
fn command_line(path: &str, args: &[String]) -> String {
    if args.is_empty() {
        alloc::format!("\"{path}\"")
    } else {
        alloc::format!("\"{path}\" {}", args.join(" "))
    }
}

/// The registry keys login items live under; tests use their own.
#[derive(Clone)]
struct Keys {
    root: Hkey,
    /// Where URL schemes are registered: `Software\Classes`.
    classes: String,
    run: String,
    approved: String,
    machine_run: Option<String>,
}

impl Keys {
    fn user() -> Self {
        Self {
            root: HKEY_CURRENT_USER,
            classes: CLASSES.into(),
            run: RUN.into(),
            approved: APPROVED.into(),
            machine_run: Some(RUN.into()),
        }
    }
}

/// Settings the methods read from their argument.
#[derive(Default)]
struct Settings {
    open_at_login: bool,
    path: Option<String>,
    args: Vec<String>,
    name: Option<String>,
    enabled: bool,
}

fn set_login_item(keys: &Keys, name: &str, executable: &str, settings: &Settings) {
    let (Some(run), Some(approved)) = (
        RegKey::open(keys.root, &keys.run, true),
        RegKey::open(keys.root, &keys.approved, true),
    ) else {
        return;
    };
    if settings.open_at_login {
        let path = settings.path.as_deref().unwrap_or(executable);
        run.set_string(name, &command_line(path, &settings.args));
        if settings.enabled {
            approved.delete(name);
        } else {
            approved.set(name, REG_BINARY, &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        }
    } else {
        approved.delete(name);
        run.delete(name);
    }
}

struct LaunchItem {
    name: String,
    path: String,
    args: Vec<String>,
    scope: &'static str,
    enabled: bool,
}

struct LoginState {
    open_at_login: bool,
    launch_items: Vec<LaunchItem>,
}

fn login_item(keys: &Keys, name: &str, executable: &str, settings: &Settings) -> LoginState {
    let path = settings.path.as_deref().unwrap_or(executable);
    let open_at_login = RegKey::open(keys.root, &keys.run, false)
        .and_then(|run| run.get(name))
        .is_some_and(|(_, data)| string_value(&data) == command_line(path, &settings.args));
    let mut launch_items = Vec::new();
    let scopes = [
        (keys.root, Some(keys.run.as_str()), "user"),
        (HKEY_LOCAL_MACHINE, keys.machine_run.as_deref(), "machine"),
    ];
    for (root, run, scope) in scopes {
        let Some(run) = run.and_then(|run| RegKey::open(root, run, false)) else {
            continue;
        };
        let approved = RegKey::open(root, &keys.approved, false);
        for (value_name, command) in run.strings() {
            let mut parts = split_command(&command).into_iter();
            let Some(program) = parts.next() else {
                continue;
            };
            if !program.eq_ignore_ascii_case(path) {
                continue;
            }
            // Explorer marks a disabled item with an odd first byte.
            let enabled = approved
                .as_ref()
                .and_then(|approved| approved.get(&value_name))
                .is_none_or(|(_, data)| data.first().is_none_or(|byte| byte % 2 == 0));
            launch_items.push(LaunchItem {
                name: value_name,
                path: program,
                args: parts.collect(),
                scope,
                enabled,
            });
        }
    }
    LoginState {
        open_at_login,
        launch_items,
    }
}

fn settings(realm: &mut Realm, value: Option<&Value>) -> Result<Settings, Thrown> {
    let mut settings = Settings {
        enabled: true,
        ..Settings::default()
    };
    let Some(value @ Value::Object(_)) = value else {
        return Ok(settings);
    };
    let text = |realm: &mut Realm, name: &'static str| -> Result<Option<String>, Thrown> {
        match ops::get_named(realm, value, name)? {
            Value::Undefined | Value::Null => Ok(None),
            value => Ok(Some(ops::to_string(realm, &value)?.to_rust_lossy())),
        }
    };
    let flag = |realm: &mut Realm, name: &'static str| -> Result<Option<bool>, Thrown> {
        match ops::get_named(realm, value, name)? {
            Value::Undefined => Ok(None),
            value => Ok(Some(ops::to_boolean(&value))),
        }
    };
    settings.open_at_login = flag(realm, "openAtLogin")?.unwrap_or(false);
    settings.enabled = flag(realm, "enabled")?.unwrap_or(true);
    settings.path = text(realm, "path")?;
    settings.name = text(realm, "name")?;
    let args = ops::get_named(realm, value, "args")?;
    if args.as_object().is_some() {
        let count = crate::array_builtin::length_of(realm, &args)?.min(1024);
        for index in 0..count {
            let argument = ops::get(realm, &args, &Key::Index(index as u32))?;
            settings
                .args
                .push(ops::to_string(realm, &argument)?.to_rust_lossy());
        }
    }
    Ok(settings)
}

fn strings(realm: &Realm, values: &[String]) -> Result<Value, Thrown> {
    let array = crate::array_builtin::array_create(realm, values.len() as u64)?;
    for (index, value) in values.iter().enumerate() {
        array.define(Key::Index(index as u32), Property::data(Value::str(value)));
    }
    Ok(Value::Object(array))
}

fn state_object(realm: &Realm, state: &LoginState) -> Result<Value, Thrown> {
    let result = realm.object();
    let flag = |name: &str, value: bool| {
        result.define(Key::name(name), Property::data(Value::Boolean(value)));
    };
    flag("openAtLogin", state.open_at_login);
    flag("openAsHidden", false);
    flag("restoreState", false);
    flag("wasOpenedAtLogin", false);
    flag("wasOpenedAsHidden", false);
    flag(
        "executableWillLaunchAtLogin",
        state.launch_items.iter().any(|item| item.enabled),
    );
    let items = crate::array_builtin::array_create(realm, state.launch_items.len() as u64)?;
    for (index, item) in state.launch_items.iter().enumerate() {
        let entry = realm.object();
        entry.define(Key::name("name"), Property::data(Value::str(&item.name)));
        entry.define(Key::name("path"), Property::data(Value::str(&item.path)));
        entry.define(
            Key::name("args"),
            Property::data(strings(realm, &item.args)?),
        );
        entry.define(Key::name("scope"), Property::data(Value::str(item.scope)));
        entry.define(
            Key::name("enabled"),
            Property::data(Value::Boolean(item.enabled)),
        );
        items.define(
            Key::Index(index as u32),
            Property::data(Value::Object(entry)),
        );
    }
    result.define(
        Key::name("launchItems"),
        Property::data(Value::Object(items)),
    );
    Ok(Value::Object(result))
}

/// Installs the methods on `app`. The AppUserModelId defaults to Electron's
/// `electron.app.<name>`.
pub(crate) fn install(realm: &Realm, app: &Object) {
    install_with(realm, app, Keys::user());
}

/// The command Windows runs for a scheme's URLs, as Electron writes it:
/// the quoted executable, the arguments as given, and the quoted URL.
fn protocol_command(executable: &str, args: &[String]) -> String {
    if args.is_empty() {
        alloc::format!("\"{executable}\" \"%1\"")
    } else {
        alloc::format!("\"{executable}\" {} \"%1\"", args.join(" "))
    }
}

/// Registers the command as the scheme's handler.
fn set_protocol_client(keys: &Keys, protocol: &str, command: &str) -> bool {
    if protocol.is_empty() {
        return false;
    }
    let path = alloc::format!("{}\\{protocol}", keys.classes);
    let Some(key) = RegKey::open(keys.root, &path, true) else {
        return false;
    };
    if !key.set_string("", &alloc::format!("URL:{protocol}")) || !key.set_string("URL Protocol", "")
    {
        return false;
    }
    RegKey::open(
        keys.root,
        &alloc::format!("{path}\\shell\\open\\command"),
        true,
    )
    .is_some_and(|command_key| command_key.set_string("", command))
}

/// Whether the command is the scheme's registered handler.
fn is_protocol_client(keys: &Keys, protocol: &str, command: &str) -> bool {
    !protocol.is_empty()
        && RegKey::open(
            keys.root,
            &alloc::format!("{}\\{protocol}\\shell\\open\\command", keys.classes),
            false,
        )
        .and_then(|key| key.get(""))
        .is_some_and(|(kind, data)| kind == REG_SZ && string_value(&data) == command)
}

/// Removes the scheme's handler when it is the command, and the scheme's
/// key when nothing else is left in it; another program's handler stays.
fn remove_protocol_client(keys: &Keys, protocol: &str, command: &str) -> bool {
    if protocol.is_empty() {
        return false;
    }
    if !is_protocol_client(keys, protocol, command) {
        return true;
    }
    let path = alloc::format!("{}\\{protocol}", keys.classes);
    let shell = wide(&alloc::format!("{path}\\shell"));
    // SAFETY: the path is NUL-terminated.
    if unsafe { RegDeleteTreeW(keys.root, shell.as_ptr()) } != ERROR_SUCCESS {
        return false;
    }
    if let Some(key) = RegKey::open(keys.root, &path, true) {
        key.delete("URL Protocol");
        key.delete("");
        if key.is_empty() {
            drop(key);
            let path = wide(&path);
            // SAFETY: the path is NUL-terminated.
            unsafe { RegDeleteKeyW(keys.root, path.as_ptr()) };
        }
    }
    true
}

/// The scheme, executable and arguments of a protocol client call:
/// `(protocol[, path[, args]])`, the executable being the application's.
fn protocol_arguments(realm: &mut Realm, args: &[Value]) -> Result<(String, String), Thrown> {
    let protocol = match args.first() {
        Some(Value::String(protocol)) => protocol.to_rust_lossy(),
        _ => return Ok((String::new(), String::new())),
    };
    let executable = match args.get(1) {
        Some(Value::String(path)) => path.to_rust_lossy(),
        _ => crate::platform::executable_path()
            .map(|path| path.to_rust_lossy())
            .unwrap_or_default(),
    };
    let mut launch = Vec::new();
    if let Some(list @ Value::Object(_)) = args.get(2) {
        let length = ops::get_named(realm, list, "length")?;
        let length = ops::to_number(realm, &length)? as usize;
        for index in 0..length {
            let item = ops::get(realm, list, &Key::Index(index as u32))?;
            launch.push(ops::to_string(realm, &item)?.to_rust_lossy());
        }
    }
    Ok((protocol, protocol_command(&executable, &launch)))
}

fn install_protocol_clients(realm: &Realm, app: &Object, keys: &Keys) {
    type Operation = fn(&Keys, &str, &str) -> bool;
    for (name, operation) in [
        (
            "setAsDefaultProtocolClient",
            set_protocol_client as Operation,
        ),
        ("isDefaultProtocolClient", is_protocol_client),
        ("removeAsDefaultProtocolClient", remove_protocol_client),
    ] {
        let keys = keys.clone();
        let method = realm.function(name, 1, false, move |realm, _, args| {
            let (protocol, command) = protocol_arguments(realm, args)?;
            Ok(Value::Boolean(operation(&keys, &protocol, &command)))
        });
        app.define(Key::name(name), Property::data(Value::Object(method)));
    }
}

fn install_with(realm: &Realm, app: &Object, keys: Keys) {
    install_protocol_clients(realm, app, &keys);
    let model_id: Rc<RefCell<Option<String>>> = Rc::default();
    let id = model_id.clone();
    let set_id = realm.function("setAppUserModelId", 1, false, move |realm, _, args| {
        let value = args.first().cloned().unwrap_or(Value::Undefined);
        let value = ops::to_string(realm, &value)?.to_rust_lossy();
        let units = wide(&value);
        // SAFETY: the id is NUL-terminated.
        unsafe { SetCurrentProcessExplicitAppUserModelID(units.as_ptr()) };
        *id.borrow_mut() = Some(value);
        Ok(Value::Undefined)
    });
    app.define(
        Key::name("setAppUserModelId"),
        Property::data(Value::Object(set_id)),
    );
    let item_name = move |realm: &Realm, settings: &Settings| {
        settings.name.clone().unwrap_or_else(|| {
            model_id.borrow().clone().unwrap_or_else(|| {
                alloc::format!(
                    "electron.app.{}",
                    realm.electron_app_name.unwrap_or("Nativeboat App")
                )
            })
        })
    };
    let item_name = Rc::new(item_name);
    let executable = || {
        crate::platform::executable_path()
            .map(|path| path.to_rust_lossy())
            .unwrap_or_default()
    };
    let (name, set_keys) = (item_name.clone(), keys.clone());
    let set = realm.function("setLoginItemSettings", 1, false, move |realm, _, args| {
        let settings = settings(realm, args.first())?;
        set_login_item(&set_keys, &name(realm, &settings), &executable(), &settings);
        Ok(Value::Undefined)
    });
    app.define(
        Key::name("setLoginItemSettings"),
        Property::data(Value::Object(set)),
    );
    let get = realm.function("getLoginItemSettings", 0, false, move |realm, _, args| {
        let settings = settings(realm, args.first())?;
        let state = login_item(
            &keys,
            &item_name(realm, &settings),
            &executable(),
            &settings,
        );
        state_object(realm, &state)
    });
    app.define(
        Key::name("getLoginItemSettings"),
        Property::data(Value::Object(get)),
    );
}

#[cfg(test)]
mod tests;
