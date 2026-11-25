//! Native Node addons, as the runtime provides them in place of their
//! compiled `.node` binaries: a package's JavaScript is compiled unchanged,
//! and its request for the binary, by relative path or through the
//! `bindings` package, loads the module here. Each follows the addon's own
//! Windows implementation.
//!
//! - `@vscode/windows-registry` (`winregistry.node`): string and DWORD
//!   values.
//! - `@vscode/deviceid` (`windows.node`): the developer tools device id in
//!   the current user's registry.
//! - `@vscode/windows-mutex` (`CreateMutex`): named mutexes.
//! - `@vscode/policy-watcher` (`vscode-policy-watcher`): Group Policy
//!   values of a product, and their changes.
//! - `@vscode/windows-process-tree` (`windows_process_tree.node`): process
//!   lists and CPU use, in `process_tree`.
//! - `@vscode/windows-ca-certs` and `@vscode/os-proxy-resolver`, whole
//!   packages the runtime replaces: system certificates and proxies, in
//!   `network`.
//! - `@vscode/spdlog` (`spdlog`): rotating log files, in `spdlog`.
//! - `@vscode/sqlite3` (`vscode-sqlite3.node`): SQLite databases over the
//!   runtime's own engine, in `sqlite3`.
//! - `@parcel/watcher` (`watcher.node`): directory tree watching, in
//!   `parcel_watcher`.
//! - `node-pty`, a whole package the runtime replaces: terminals in Windows
//!   pseudoconsoles, in `pty`.
//! - `native-is-elevated` (`iselevated`): whether the process is elevated.
//! - `@vscode/native-watchdog` (`watchdog`): exits when a parent process
//!   ends.

/// Functions of system libraries most programs never load, bound when first
/// called: an executable imports only the libraries every program needs,
/// and a program loads these only when it uses an addon. A missing function
/// fails as the given value.
macro_rules! late_bound {
    ($library:literal { $(fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty = $fail:expr;)* }) => {
        $(
            #[allow(non_snake_case, clippy::too_many_arguments)]
            unsafe fn $name($($arg: $ty),*) -> $ret {
                type Procedure = unsafe extern "system" fn($($ty),*) -> $ret;
                let name = core::ffi::CStr::from_bytes_with_nul(concat!(stringify!($name), "\0").as_bytes())
                    .expect("a nul-terminated name");
                let procedure = crate::platform::system_procedure($library, name);
                if procedure.is_null() {
                    return $fail;
                }
                // SAFETY: the export has this signature, and the caller
                // upholds the function's contract.
                unsafe { core::mem::transmute::<*const c_void, Procedure>(procedure)($($arg),*) }
            }
        )*
    };
}

pub(crate) mod network;
pub(crate) mod parcel_watcher;
pub(crate) mod process_tree;
pub(crate) mod pty;
pub(crate) mod spdlog;
pub(crate) mod sqlite3;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;
use core::ffi::c_void;

const ERROR_SUCCESS: i32 = 0;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_INVALID_HANDLE: u32 = 6;
const KEY_READ: u32 = 0x2_0019;
const KEY_ALL_ACCESS: u32 = 0xF_003F;
const KEY_WOW64_64KEY: u32 = 0x0100;
const REG_OPTION_NON_VOLATILE: u32 = 0;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;
const SYNCHRONIZE: u32 = 0x0010_0000;
const HKEY_CLASSES_ROOT: isize = 0x8000_0000_u32 as i32 as isize;
const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
const HKEY_USERS: isize = 0x8000_0003_u32 as i32 as isize;
const HKEY_CURRENT_CONFIG: isize = 0x8000_0005_u32 as i32 as isize;

/// `ERROR_FILE_NOT_FOUND`, as a registry call reports a missing function.
const ERROR_FILE_NOT_FOUND: i32 = 2;

late_bound!("advapi32.dll" {
    fn RegOpenKeyExW(key: isize, path: *const u16, options: u32, access: u32, result: *mut isize) -> i32 = ERROR_FILE_NOT_FOUND;
    fn RegCreateKeyExW(
        key: isize,
        path: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        security: *const c_void,
        result: *mut isize,
        disposition: *mut u32,
    ) -> i32 = ERROR_FILE_NOT_FOUND;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32 = ERROR_FILE_NOT_FOUND;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32 = ERROR_FILE_NOT_FOUND;
    fn RegCloseKey(key: isize) -> i32 = 0;
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32 = 0;
    fn GetTokenInformation(token: *mut c_void, class: u32, information: *mut c_void, length: u32, returned: *mut u32) -> i32 = 0;
});

late_bound!("userenv.dll" {
    fn RegisterGPNotification(event: *mut c_void, machine: i32) -> i32 = 0;
    fn UnregisterGPNotification(event: *mut c_void) -> i32 = 0;
});

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn Sleep(milliseconds: u32);
    fn CreateThread(
        security: *mut c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        parameter: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> *mut c_void;
    fn ExitProcess(code: u32);
    fn CreateEventW(
        security: *mut c_void,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> *mut c_void;
    fn SetEvent(event: *mut c_void) -> i32;
    fn WaitForMultipleObjects(
        count: u32,
        handles: *const *mut c_void,
        all: i32,
        milliseconds: u32,
    ) -> u32;
    fn CreateMutexW(security: *const c_void, owned: i32, name: *const u16) -> *mut c_void;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn method(
    realm: &Realm,
    target: &Object,
    name: &'static str,
    length: u32,
    body: impl Fn(&mut Realm, &[Value]) -> crate::Completion + 'static,
) {
    let function = realm.function(name, length, false, move |realm, _, args| body(realm, args));
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

/// An `Error` with the addon's message, and the code it gives, if any.
fn addon_error(realm: &mut Realm, code: Option<&str>, message: &str) -> Thrown {
    let error = realm.error(ErrorKind::Error, message);
    if let Some(code) = code {
        error.define(Key::name("code"), Property::data(Value::str(code)));
    }
    Thrown(Value::Object(error))
}

/// The open registry key, closed when dropped.
struct RegistryKey(isize);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: the key was opened by this process and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_key(hive: isize, path: &str, access: u32) -> Option<RegistryKey> {
    let path = wide(path);
    let mut key = 0;
    // SAFETY: the path is nul-terminated and the result is writable.
    let status = unsafe { RegOpenKeyExW(hive, path.as_ptr(), 0, access, &mut key) };
    (status == ERROR_SUCCESS).then_some(RegistryKey(key))
}

/// A value's type and bytes, or nothing when it is missing or larger than
/// `limit` bytes.
fn query_value(key: &RegistryKey, name: &str, limit: usize) -> Option<(u32, Vec<u8>)> {
    let name = wide(name);
    let mut kind = 0;
    let mut data = alloc::vec![0_u8; limit];
    let mut size = limit as u32;
    // SAFETY: the name is nul-terminated and the buffer holds `size` bytes.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            core::ptr::null_mut(),
            &mut kind,
            data.as_mut_ptr(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    data.truncate(size as usize);
    Some((kind, data))
}

/// Registry string data as text, up to its first nul.
fn registry_text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn hive(name: &str) -> Option<isize> {
    Some(match name {
        "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKEY_USERS" => HKEY_USERS,
        "HKEY_CURRENT_CONFIG" => HKEY_CURRENT_CONFIG,
        _ => return None,
    })
}

/// The three string arguments of a registry read, as the addon checks them.
fn registry_arguments(realm: &mut Realm, args: &[Value]) -> Result<[String; 3], Thrown> {
    if args.len() < 3 {
        return Err(addon_error(
            realm,
            Some("EINVAL"),
            "Wrong number of arguments",
        ));
    }
    let mut strings: [String; 3] = Default::default();
    for (index, slot) in strings.iter_mut().enumerate() {
        let Value::String(text) = &args[index] else {
            return Err(addon_error(realm, Some("EINVAL"), "Expected string"));
        };
        let text = text.to_rust_lossy();
        if text.len() + 1 > 16383 {
            return Err(addon_error(realm, Some("EINVAL"), "Arguments too long"));
        }
        *slot = text;
    }
    Ok(strings)
}

/// `@vscode/windows-registry`'s binding: `GetStringRegKey(hive, path,
/// name)` and `GetDWORDRegKey(hive, path, name)` give the value, or
/// `undefined` when it is missing or of another type, and throw when the
/// hive or key cannot be opened.
pub(crate) fn windows_registry(realm: &mut Realm) -> Object {
    let exports = realm.object();
    for (name, dword) in [("GetStringRegKey", false), ("GetDWORDRegKey", true)] {
        method(realm, &exports, name, 3, move |realm, args| {
            let [hive_name, path, value] = registry_arguments(realm, args)?;
            let Some(hive) = hive(&hive_name) else {
                return Err(addon_error(realm, None, "Unable to open registry hive"));
            };
            let Some(key) = open_key(hive, &path, KEY_READ) else {
                return Err(addon_error(realm, None, "Unable to open registry key"));
            };
            // The addon reads into a 512-byte buffer: longer values are
            // missing.
            Ok(match query_value(&key, &value, 512) {
                Some((REG_SZ | REG_EXPAND_SZ, data)) if !dword => Value::str(&registry_text(&data)),
                Some((REG_DWORD, data)) if dword && data.len() >= 4 => {
                    Value::Number(f64::from(u32::from_le_bytes([
                        data[0], data[1], data[2], data[3],
                    ])))
                }
                _ => Value::Undefined,
            })
        });
    }
    exports
}

const DEVICE_ID_PATH: &str = "SOFTWARE\\Microsoft\\DeveloperTools";
const DEVICE_ID_NAME: &str = "deviceid";

/// `@vscode/deviceid`'s Windows binding: `GetDeviceId()` reads the current
/// user's developer tools device id, and `SetDeviceId(id)` stores one,
/// throwing as the addon does when the key or value is unavailable.
pub(crate) fn device_id(realm: &mut Realm) -> Object {
    let exports = realm.object();
    method(realm, &exports, "GetDeviceId", 0, |realm, _| {
        let value = open_key(
            HKEY_CURRENT_USER,
            DEVICE_ID_PATH,
            KEY_READ | KEY_WOW64_64KEY,
        )
        .and_then(|key| query_value(&key, DEVICE_ID_NAME, 16383 * 2));
        match value {
            Some((_, data)) => Ok(Value::str(&registry_text(&data))),
            None => Err(addon_error(realm, None, "Unable to open registry key")),
        }
    });
    method(realm, &exports, "SetDeviceId", 1, |realm, args| {
        let id = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        if id.len() + 1 > 16383 {
            return Err(addon_error(realm, None, "String too long"));
        }
        let path = wide(DEVICE_ID_PATH);
        let mut key = 0;
        // SAFETY: the path is nul-terminated and the result is writable.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                core::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS | KEY_WOW64_64KEY,
                core::ptr::null(),
                &mut key,
                core::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(addon_error(
                realm,
                None,
                "Unable to create or open registry key",
            ));
        }
        let key = RegistryKey(key);
        let name = wide(DEVICE_ID_NAME);
        let data = wide(&id);
        // SAFETY: the name is nul-terminated and the data holds the string
        // and its nul, which the size includes.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(addon_error(realm, None, "Unable to set registry value"));
        }
        Ok(Value::Undefined)
    });
    exports
}

/// `@vscode/windows-mutex`'s binding: `new Mutex(name)` creates and owns a
/// named mutex, throwing when one of that name exists; `release()` closes
/// it and `isActive()` says whether it is held. `isActive(name)` says
/// whether any process holds a mutex of that name.
pub(crate) fn windows_mutex(realm: &mut Realm) -> Object {
    let exports = realm.object();
    method(realm, &exports, "isActive", 1, |realm, args| {
        let Value::String(name) = arg(args, 0) else {
            return Err(addon_error(realm, None, "Provide a mutex name"));
        };
        let name = wide(&name.to_rust_lossy());
        // SAFETY: the name is nul-terminated; an opened handle is closed.
        let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
        if !handle.is_null() {
            // SAFETY: the handle was opened above.
            unsafe { CloseHandle(handle) };
        }
        Ok(Value::Boolean(!handle.is_null()))
    });
    let prototype = realm.object();
    let constructor_prototype = prototype.clone();
    let constructor = realm.function("Mutex", 1, true, move |realm, _, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructor Mutex cannot be invoked without 'new'",
            ));
        }
        let Value::String(name) = arg(args, 0) else {
            return Err(addon_error(realm, None, "Provide a mutex name"));
        };
        let name = wide(&name.to_rust_lossy());
        // SAFETY: the name is nul-terminated; the error is read at once.
        let (handle, error) = unsafe {
            let handle = CreateMutexW(core::ptr::null(), 1, name.as_ptr());
            (handle, GetLastError())
        };
        if handle.is_null() {
            let message = if error == ERROR_INVALID_HANDLE {
                "Error provided name is already used for another HANDLE"
            } else {
                "Error creating mutex"
            };
            return Err(addon_error(realm, None, message));
        }
        if error == ERROR_ALREADY_EXISTS {
            // SAFETY: the handle was returned above and is closed once.
            unsafe { CloseHandle(handle) };
            return Err(addon_error(realm, None, "Error mutex already exists"));
        }
        let held = Rc::new(Cell::new(handle));
        let mutex = realm.object();
        mutex.set_prototype(Some(constructor_prototype.clone()));
        let release = held.clone();
        method(realm, &mutex, "release", 0, move |_, _| {
            let handle = release.replace(core::ptr::null_mut());
            if !handle.is_null() {
                // SAFETY: the handle is this mutex's, closed once.
                unsafe { CloseHandle(handle) };
            }
            Ok(Value::Undefined)
        });
        let active = held;
        method(realm, &mutex, "isActive", 0, move |_, _| {
            Ok(Value::Boolean(!active.get().is_null()))
        });
        Ok(Value::Object(mutex))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    exports.define(
        Key::name("Mutex"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

/// The `bindings` package: `bindings(name)` (or `bindings({ bindings:
/// name })`) loads a package's compiled addon by its binary's name. The
/// runtime's addons are known by package and binary; a name another addon
/// uses is not compiled, and fails as `bindings` does when it finds no
/// binary.
pub(crate) fn bindings(realm: &mut Realm) -> Object {
    realm.function("bindings", 1, false, |realm, _, args| {
        let requested = match arg(args, 0) {
            Value::Object(options) => ops::get_named(realm, &Value::Object(options), "bindings")?,
            other => other,
        };
        let name = ops::to_string(realm, &requested)?.to_rust_lossy();
        let file = if name.ends_with(".node") {
            name.clone()
        } else {
            alloc::format!("{name}.node")
        };
        let suffix = alloc::format!("/{file}");
        let Some(module) = crate::modules::NATIVE_BINDINGS
            .iter()
            .find(|binding| binding.ends_with(&suffix))
        else {
            return Err(addon_error(
                realm,
                None,
                &alloc::format!("Could not locate the bindings file for {file}: native addons are compiled into the application, and this one is not"),
            ));
        };
        crate::modules::load(realm, module)
    })
}

const REG_MULTI_SZ: u32 = 7;
const REG_QWORD: u32 = 11;
const INFINITE: u32 = u32::MAX;

/// The types a policy accepts.
#[derive(Clone, Copy, Default)]
struct PolicyTypes {
    string: bool,
    number: bool,
    boolean: bool,
}

#[derive(Clone, PartialEq)]
enum PolicyValue {
    String(String),
    Number(f64),
    Boolean(bool),
}

/// Registry string data, without its terminating nul.
fn policy_string(data: &[u8]) -> String {
    let mut units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16_lossy(&units)
}

/// `REG_MULTI_SZ` data as lines, up to the empty string that ends it.
fn policy_lines(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    units
        .split(|unit| *unit == 0)
        .take_while(|line| !line.is_empty())
        .map(String::from_utf16_lossy)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A policy's value under `root`, as the addon parses each type: a single
/// type reads only its registry types, a union takes the first it accepts.
fn read_policy(
    root: isize,
    path: &str,
    name: &str,
    types: PolicyTypes,
    union: bool,
) -> Option<PolicyValue> {
    let key = open_key(root, path, KEY_READ)?;
    let (kind, data) = query_value(&key, name, 1 << 20)?;
    let dword = (kind == REG_DWORD && data.len() == 4)
        .then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
    let qword = (kind == REG_QWORD && data.len() == 8).then(|| {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&data);
        i64::from_le_bytes(bytes)
    });
    if union {
        if let Some(value) = dword {
            if types.boolean {
                return Some(PolicyValue::Boolean(value != 0));
            }
            if types.number {
                return Some(PolicyValue::Number(f64::from(value)));
            }
        }
        if let Some(value) = qword
            && types.number
        {
            return Some(PolicyValue::Number(value as f64));
        }
        if types.string && kind == REG_SZ {
            return Some(PolicyValue::String(registry_text(&data)));
        }
        if types.string && kind == REG_MULTI_SZ {
            return Some(PolicyValue::String(policy_lines(&data)));
        }
        return None;
    }
    if types.string {
        return match kind {
            REG_SZ => Some(PolicyValue::String(policy_string(&data))),
            REG_MULTI_SZ => Some(PolicyValue::String(policy_lines(&data))),
            _ => None,
        };
    }
    if types.number {
        return dword
            .map(f64::from)
            .or_else(|| qword.map(|value| value as f64))
            .map(PolicyValue::Number);
    }
    dword.map(|value| PolicyValue::Boolean(value != 0))
}

struct Policy {
    name: String,
    types: PolicyTypes,
    union: bool,
    value: Option<PolicyValue>,
}

struct PolicyWatcher {
    path: String,
    policies: alloc::vec::Vec<Policy>,
    callback: Value,
    disposed: Cell<bool>,
    /// The dispose event and the machine and user Group Policy events.
    events: [usize; 3],
}

impl PolicyWatcher {
    /// Reads every policy, machine values first: the policies that are set
    /// or were just removed, and whether any changed.
    fn refresh(&mut self) -> (Vec<(String, Option<PolicyValue>)>, bool) {
        let mut reported = Vec::new();
        let mut changed = false;
        for policy in &mut self.policies {
            let next = read_policy(
                HKEY_LOCAL_MACHINE,
                &self.path,
                &policy.name,
                policy.types,
                policy.union,
            )
            .or_else(|| {
                read_policy(
                    HKEY_CURRENT_USER,
                    &self.path,
                    &policy.name,
                    policy.types,
                    policy.union,
                )
            });
            if next != policy.value {
                changed = true;
                reported.push((policy.name.clone(), next.clone()));
            } else if next.is_some() {
                reported.push((policy.name.clone(), next.clone()));
            }
            policy.value = next;
        }
        (reported, changed)
    }
}

impl Drop for PolicyWatcher {
    fn drop(&mut self) {
        for (index, event) in self.events.iter().enumerate() {
            let event = *event as *mut c_void;
            if event.is_null() {
                continue;
            }
            // SAFETY: the events were created by this watcher; the Group
            // Policy ones were registered, and each is closed once.
            unsafe {
                if index > 0 {
                    UnregisterGPNotification(event);
                }
                CloseHandle(event);
            }
        }
    }
}

fn notify_policies(
    realm: &mut Realm,
    watcher: &Rc<core::cell::RefCell<PolicyWatcher>>,
    reported: Vec<(String, Option<PolicyValue>)>,
) -> Result<(), Thrown> {
    let update = realm.object();
    for (name, value) in reported {
        let value = match value {
            None => Value::Undefined,
            Some(PolicyValue::String(text)) => Value::str(&text),
            Some(PolicyValue::Number(number)) => Value::Number(number),
            Some(PolicyValue::Boolean(flag)) => Value::Boolean(flag),
        };
        ops::set(
            realm,
            &Value::Object(update.clone()),
            &Key::String(name.as_str().into()),
            value,
        )?;
    }
    let callback = watcher.borrow().callback.clone();
    ops::call(realm, &callback, Value::Undefined, &[Value::Object(update)]).map(drop)
}

/// Waits on a pool thread for a Group Policy change or disposal, then
/// re-reads the policies and reports them if any changed.
fn watch_policies(realm: &mut Realm, watcher: Rc<core::cell::RefCell<PolicyWatcher>>) {
    let events = watcher.borrow().events;
    let referenced = Rc::new(Cell::new(false));
    let submitted = realm.submit_io_with_ref(
        move || {
            let handles = events.map(|event| event as *mut c_void);
            // SAFETY: the three events stay open until the watcher, which
            // outlives this wait, is dropped after disposal wakes it.
            unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, INFINITE) }
        },
        referenced,
        move |realm, woken| {
            // WAIT_OBJECT_0 is the dispose event; failures stop watching.
            if woken == 0 || woken > 2 || watcher.borrow().disposed.get() {
                return Ok(());
            }
            let (reported, changed) = watcher.borrow_mut().refresh();
            if changed {
                notify_policies(realm, &watcher, reported)?;
            }
            watch_policies(realm, watcher);
            Ok(())
        },
    );
    debug_assert!(submitted.is_ok());
}

fn policy_types(realm: &mut Realm, declared: &Value) -> Result<(PolicyTypes, bool), Thrown> {
    let Value::Object(policy) = declared else {
        return Err(realm.throw(ErrorKind::Type, "Expected policy to be object"));
    };
    let kind = ops::get_named(realm, &Value::Object(policy.clone()), "type")?;
    let mut names = Vec::new();
    match &kind {
        Value::String(name) => names.push(name.to_rust_lossy()),
        Value::Object(array) if array.is_array() => {
            let length = ops::get_named(realm, &kind, "length")?;
            let length = match length {
                Value::Number(length) => length as u32,
                _ => 0,
            };
            if length == 0 {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "Expected policy type array to be non-empty",
                ));
            }
            for index in 0..length {
                let Value::String(name) = ops::get(realm, &kind, &Key::Index(index))? else {
                    return Err(realm.throw(
                        ErrorKind::Type,
                        "Expected policy type array entries to be strings",
                    ));
                };
                let name = name.to_rust_lossy();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        _ => {
            return Err(realm.throw(
                ErrorKind::Type,
                "Expected policy type to be a string or non-empty string array",
            ));
        }
    }
    let mut types = PolicyTypes::default();
    for name in &names {
        match name.as_str() {
            "string" => types.string = true,
            "number" => types.number = true,
            "boolean" => types.boolean = true,
            other => {
                return Err(realm.throw(
                    ErrorKind::Type,
                    &alloc::format!("Unknown policy type '{other}'"),
                ));
            }
        }
    }
    Ok((types, names.len() > 1))
}

/// `@vscode/policy-watcher`'s binding: `createWatcher(productName,
/// policies, onDidChange[, { registryPath }])` reads each policy from
/// `Software\Policies\Microsoft\<productName>` (or the given path), the
/// machine's value before the user's, reports them once, and again when a
/// Group Policy change alters any. `dispose()` stops it.
pub(crate) fn policy_watcher(realm: &mut Realm) -> Object {
    realm.function("createWatcher", 4, false, |realm, _, args| {
        if args.len() < 3 {
            return Err(realm.throw(ErrorKind::Type, "Expected 3 arguments"));
        }
        let Value::String(product) = &args[0] else {
            return Err(realm.throw(ErrorKind::Type, "Expected first arg to be string"));
        };
        let Value::Object(declared) = &args[1] else {
            return Err(realm.throw(ErrorKind::Type, "Expected second arg to be object"));
        };
        if !crate::array_builtin::is_callable(&args[2]) {
            return Err(realm.throw(ErrorKind::Type, "Expected third arg to be function"));
        }
        let mut path = alloc::format!("Software\\Policies\\Microsoft\\{}", product.to_rust_lossy());
        match args.get(3) {
            None | Some(Value::Undefined) => {}
            Some(Value::Object(options)) => {
                match ops::get_named(realm, &Value::Object(options.clone()), "registryPath")? {
                    Value::Undefined => {}
                    Value::String(custom) => path = custom.to_rust_lossy(),
                    _ => {
                        return Err(realm.throw(
                            ErrorKind::Type,
                            "Expected options.registryPath to be string",
                        ));
                    }
                }
            }
            Some(_) => return Err(realm.throw(ErrorKind::Type, "Expected fourth arg to be object")),
        }
        let mut policies = Vec::new();
        for key in ops::own_property_keys(realm, declared)? {
            let Key::String(name) = &key else {
                continue;
            };
            let value = ops::get(realm, &Value::Object(declared.clone()), &key)?;
            let (types, union) = policy_types(realm, &value)?;
            policies.push(Policy {
                name: name.to_rust_lossy(),
                types,
                union,
                value: None,
            });
        }
        let mut events = [0_usize; 3];
        for (index, event) in events.iter_mut().enumerate() {
            // SAFETY: an unnamed auto-reset event, closed by the watcher.
            let handle = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
            if handle.is_null() {
                return Err(addon_error(realm, None, "Failed to create watcher event"));
            }
            *event = handle as usize;
            // SAFETY: the event is live; machine first, then user.
            if index > 0 && unsafe { RegisterGPNotification(handle, i32::from(index == 1)) } == 0 {
                let message = if index == 1 {
                    "Failed to register machine GP event"
                } else {
                    "Failed to register user GP event"
                };
                // SAFETY: the event was created above and is not registered.
                unsafe { CloseHandle(handle) };
                *event = 0;
                let watcher = PolicyWatcher {
                    path,
                    policies,
                    callback: Value::Undefined,
                    disposed: Cell::new(true),
                    events,
                };
                drop(watcher);
                return Err(addon_error(realm, None, message));
            }
        }
        let watcher = Rc::new(core::cell::RefCell::new(PolicyWatcher {
            path,
            policies,
            callback: args[2].clone(),
            disposed: Cell::new(false),
            events,
        }));
        // The first reading is reported whether or not any policy is set.
        let first = watcher.clone();
        realm.enqueue(alloc::boxed::Box::new(move |realm| {
            if first.borrow().disposed.get() {
                return Ok(());
            }
            let (reported, _) = first.borrow_mut().refresh();
            notify_policies(realm, &first, reported)?;
            watch_policies(realm, first);
            Ok(())
        }));
        let handle = realm.object();
        let disposed = watcher;
        method(realm, &handle, "dispose", 0, move |_, _| {
            let watcher = disposed.borrow();
            if !watcher.disposed.replace(true) {
                // SAFETY: the dispose event is live until the watcher drops.
                unsafe { SetEvent(watcher.events[0] as *mut c_void) };
            }
            Ok(Value::Undefined)
        });
        Ok(Value::Object(handle))
    })
}

/// `native-is-elevated`'s binding: `isElevated()` says whether the
/// process's token is elevated.
pub(crate) fn is_elevated(realm: &mut Realm) -> Object {
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION: u32 = 20;
    let exports = realm.object();
    method(realm, &exports, "isElevated", 0, |_, _| {
        let mut token = core::ptr::null_mut();
        // SAFETY: the pseudo-handle names this process and the token out
        // pointer is writable.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Ok(Value::Boolean(false));
        }
        let mut elevated = 0_u32;
        let mut returned = 0;
        // SAFETY: the token is open and TOKEN_ELEVATION is one DWORD.
        let read = unsafe {
            GetTokenInformation(
                token,
                TOKEN_ELEVATION,
                (&raw mut elevated).cast(),
                4,
                &mut returned,
            )
        };
        // SAFETY: the token was opened above.
        unsafe { CloseHandle(token) };
        Ok(Value::Boolean(read != 0 && elevated != 0))
    });
    exports
}

const WAIT_TIMEOUT: u32 = 0x102;

/// Whether a process is running, as the watchdog checks it.
fn process_running(pid: u32) -> bool {
    // SAFETY: OpenProcess takes plain values; a null handle makes the wait
    // fail, as the addon's check does.
    unsafe {
        let process = OpenProcess(SYNCHRONIZE, 0, pid);
        let result = WaitForSingleObject(process, 0);
        if !process.is_null() {
            CloseHandle(process);
        }
        result == WAIT_TIMEOUT
    }
}

/// The watchdog's thread: once a second it checks the parent, and five
/// seconds after the parent ends it exits the process with code 87.
unsafe extern "system" fn watch_parent(parameter: *mut c_void) -> u32 {
    let pid = parameter as usize as u32;
    loop {
        if !process_running(pid) {
            // SAFETY: Sleep and ExitProcess take plain values.
            unsafe {
                Sleep(5000);
                ExitProcess(87);
            }
            return 87;
        }
        // SAFETY: as above.
        unsafe { Sleep(1000) };
    }
}

/// `@vscode/native-watchdog`'s binding: `start(pid)` watches a parent
/// process from a thread of its own, and `exit(code)` exits at once.
pub(crate) fn watchdog(realm: &mut Realm) -> Object {
    let exports = realm.object();
    method(realm, &exports, "start", 1, |realm, args| {
        if args.len() != 1 {
            return Err(addon_error(
                realm,
                None,
                "Wrong number of arguments, expected 1.",
            ));
        }
        let Value::Number(pid) = args[0] else {
            return Err(addon_error(realm, None, "Wrong argument, number expected."));
        };
        // SAFETY: the thread takes the pid by value and never returns
        // anything it borrows.
        let thread = unsafe {
            CreateThread(
                core::ptr::null_mut(),
                0,
                watch_parent,
                pid as i64 as u32 as usize as *mut c_void,
                0,
                core::ptr::null_mut(),
            )
        };
        if !thread.is_null() {
            // SAFETY: the thread keeps running without its handle.
            unsafe { CloseHandle(thread) };
        }
        Ok(Value::Undefined)
    });
    method(realm, &exports, "exit", 1, |realm, args| {
        let Value::Number(code) = arg(args, 0) else {
            return Err(addon_error(realm, None, "Wrong argument, number expected."));
        };
        // SAFETY: ExitProcess takes a plain value and does not return.
        unsafe { ExitProcess(code as i32 as u32) };
        Ok(Value::Undefined)
    });
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(realm: &mut Realm, target: &Object, name: &str, args: &[Value]) -> crate::Completion {
        let function = ops::get(realm, &Value::Object(target.clone()), &Key::name(name)).unwrap();
        ops::call(realm, &function, Value::Object(target.clone()), args)
    }

    fn message(realm: &mut Realm, thrown: Thrown) -> String {
        let message = ops::get_named(realm, &thrown.0, "message").unwrap();
        ops::to_string(realm, &message).unwrap().to_rust_lossy()
    }

    #[test]
    fn registry_values_are_read_as_the_addon_reads_them() {
        let mut realm = Realm::new();
        let registry = windows_registry(&mut realm);
        let path = Value::str("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        let product = call(
            &mut realm,
            &registry,
            "GetStringRegKey",
            &[
                Value::str("HKEY_LOCAL_MACHINE"),
                path.clone(),
                Value::str("ProductName"),
            ],
        )
        .unwrap();
        assert!(
            matches!(&product, Value::String(name) if name.to_rust_lossy().starts_with("Windows"))
        );
        let missing = call(
            &mut realm,
            &registry,
            "GetStringRegKey",
            &[
                Value::str("HKEY_LOCAL_MACHINE"),
                path.clone(),
                Value::str("NoSuchValue"),
            ],
        )
        .unwrap();
        assert!(matches!(missing, Value::Undefined));
        let major = call(
            &mut realm,
            &registry,
            "GetDWORDRegKey",
            &[
                Value::str("HKEY_LOCAL_MACHINE"),
                path.clone(),
                Value::str("CurrentMajorVersionNumber"),
            ],
        )
        .unwrap();
        assert!(matches!(major, Value::Number(version) if version >= 10.0));
        // A string value read as a DWORD is missing.
        let mismatched = call(
            &mut realm,
            &registry,
            "GetDWORDRegKey",
            &[
                Value::str("HKEY_LOCAL_MACHINE"),
                path.clone(),
                Value::str("ProductName"),
            ],
        )
        .unwrap();
        assert!(matches!(mismatched, Value::Undefined));
        let hive = call(
            &mut realm,
            &registry,
            "GetStringRegKey",
            &[
                Value::str("HKEY_NOWHERE"),
                path.clone(),
                Value::str("ProductName"),
            ],
        )
        .unwrap_err();
        assert_eq!(message(&mut realm, hive), "Unable to open registry hive");
        let key = call(
            &mut realm,
            &registry,
            "GetStringRegKey",
            &[
                Value::str("HKEY_LOCAL_MACHINE"),
                Value::str("SOFTWARE\\NoSuchNativeboatKey"),
                Value::str("x"),
            ],
        )
        .unwrap_err();
        assert_eq!(message(&mut realm, key), "Unable to open registry key");
        let arguments =
            call(&mut realm, &registry, "GetStringRegKey", &[Value::str("x")]).unwrap_err();
        let code = ops::get_named(&mut realm, &arguments.0, "code").unwrap();
        assert!(matches!(code, Value::String(code) if code.to_rust_lossy() == "EINVAL"));
    }

    #[test]
    fn named_mutexes_are_created_held_and_released() {
        let mut realm = Realm::new();
        let module = windows_mutex(&mut realm);
        let name = Value::str(&alloc::format!(
            "nativeboat-test-mutex-{}",
            crate::platform::process_id()
        ));
        let is_active = |realm: &mut Realm| {
            call(realm, &module, "isActive", core::slice::from_ref(&name)).unwrap()
        };
        assert!(matches!(is_active(&mut realm), Value::Boolean(false)));
        let constructor =
            ops::get_named(&mut realm, &Value::Object(module.clone()), "Mutex").unwrap();
        let mutex = ops::construct(&mut realm, &constructor, core::slice::from_ref(&name)).unwrap();
        let Value::Object(mutex) = mutex else {
            panic!("a mutex object");
        };
        assert!(matches!(is_active(&mut realm), Value::Boolean(true)));
        assert!(matches!(
            call(&mut realm, &mutex, "isActive", &[]).unwrap(),
            Value::Boolean(true)
        ));
        let again =
            ops::construct(&mut realm, &constructor, core::slice::from_ref(&name)).unwrap_err();
        assert_eq!(message(&mut realm, again), "Error mutex already exists");
        call(&mut realm, &mutex, "release", &[]).unwrap();
        assert!(matches!(
            call(&mut realm, &mutex, "isActive", &[]).unwrap(),
            Value::Boolean(false)
        ));
        assert!(matches!(is_active(&mut realm), Value::Boolean(false)));
        let unnamed = ops::construct(&mut realm, &constructor, &[]).unwrap_err();
        assert_eq!(message(&mut realm, unnamed), "Provide a mutex name");
    }

    #[test]
    fn policy_watchers_report_their_first_reading_and_dispose() {
        let mut realm = Realm::new();
        let create = Value::Object(policy_watcher(&mut realm));
        let seen = Rc::new(core::cell::RefCell::new(Vec::new()));
        let record = seen.clone();
        let callback = realm.function("onDidChange", 1, false, move |realm, _, args| {
            let update = args.first().cloned().unwrap_or(Value::Undefined);
            let Value::Object(update) = &update else {
                panic!("an update object");
            };
            let keys = ops::own_property_keys(realm, update).unwrap().len();
            record.borrow_mut().push(keys);
            Ok(Value::Undefined)
        });
        let policies = realm.object();
        let declared = realm.object();
        declared.define(Key::name("type"), Property::data(Value::str("string")));
        policies.define(
            Key::name("UpdateMode"),
            Property::data(Value::Object(declared)),
        );
        let product = Value::str("NativeboatTestProductWithoutPolicies");
        let handle = ops::call(
            &mut realm,
            &create,
            Value::Undefined,
            &[
                product.clone(),
                Value::Object(policies.clone()),
                Value::Object(callback.clone()),
            ],
        )
        .unwrap();
        realm.run_jobs().unwrap();
        // The first reading is reported, with no policy set.
        assert_eq!(*seen.borrow(), [0]);
        let Value::Object(handle) = handle else {
            panic!("a watcher handle");
        };
        call(&mut realm, &handle, "dispose", &[]).unwrap();
        let bad = realm.object();
        bad.define(Key::name("type"), Property::data(Value::str("date")));
        let invalid = realm.object();
        invalid.define(Key::name("X"), Property::data(Value::Object(bad)));
        let error = ops::call(
            &mut realm,
            &create,
            Value::Undefined,
            &[product, Value::Object(invalid), Value::Object(callback)],
        )
        .unwrap_err();
        assert_eq!(message(&mut realm, error), "Unknown policy type 'date'");
        assert_eq!(policy_lines(&[b'a', 0, 0, 0, b'b', 0, 0, 0, 0, 0]), "a\nb");
        assert_eq!(policy_string(&[b'x', 0, 0, 0]), "x");
    }

    #[test]
    fn elevation_and_parent_processes_are_read() {
        let mut realm = Realm::new();
        let module = is_elevated(&mut realm);
        assert!(matches!(
            call(&mut realm, &module, "isElevated", &[]).unwrap(),
            Value::Boolean(_)
        ));
        assert!(process_running(crate::platform::process_id()));
        assert!(!process_running(0xFFFF_FFF0));
        let watchdog = watchdog(&mut realm);
        let error = call(&mut realm, &watchdog, "start", &[Value::str("x")]).unwrap_err();
        assert_eq!(
            message(&mut realm, error),
            "Wrong argument, number expected."
        );
    }

    #[test]
    fn bindings_load_the_runtime_addons_by_binary_name() {
        let mut realm = Realm::new();
        let function = Value::Object(bindings(&mut realm));
        for name in ["CreateMutex", "CreateMutex.node"] {
            let module =
                ops::call(&mut realm, &function, Value::Undefined, &[Value::str(name)]).unwrap();
            let mutex = ops::get_named(&mut realm, &module, "Mutex").unwrap();
            assert!(crate::array_builtin::is_callable(&mutex));
        }
        let missing = ops::call(
            &mut realm,
            &function,
            Value::Undefined,
            &[Value::str("other")],
        )
        .unwrap_err();
        assert!(
            message(&mut realm, missing)
                .starts_with("Could not locate the bindings file for other.node")
        );
        // The device id addon's functions exist; reading it does not write
        // the registry.
        let device = device_id(&mut realm);
        let get =
            ops::get_named(&mut realm, &Value::Object(device.clone()), "GetDeviceId").unwrap();
        assert!(crate::array_builtin::is_callable(&get));
        let set = ops::get_named(&mut realm, &Value::Object(device), "SetDeviceId").unwrap();
        assert!(crate::array_builtin::is_callable(&set));
    }
}
