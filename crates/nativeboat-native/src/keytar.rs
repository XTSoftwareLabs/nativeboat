//! Engine-free Windows Credential Manager implementation for MarkText's
//! unchanged `keytar` dependency.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::slice;

type Bool = i32;
type CredentialPtr = *mut Credential;
type WriteFn = unsafe extern "system" fn(*const Credential, u32) -> Bool;
type ReadFn = unsafe extern "system" fn(*const u16, u32, u32, *mut CredentialPtr) -> Bool;
type DeleteFn = unsafe extern "system" fn(*const u16, u32, u32) -> Bool;
type EnumerateFn =
    unsafe extern "system" fn(*const u16, u32, *mut u32, *mut *mut CredentialPtr) -> Bool;
type FreeFn = unsafe extern "system" fn(*const c_void);
type LastErrorFn = unsafe extern "system" fn() -> u32;
type GetProcedureFn = unsafe extern "system" fn(*mut c_void, *const u8) -> *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct Credential {
    flags: u32,
    kind: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: FileTime,
    blob_size: u32,
    blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    target_alias: *mut u16,
    username: *mut u16,
}

impl Default for Credential {
    fn default() -> Self {
        // SAFETY: every field is an integer or nullable pointer and zero is a
        // valid empty representation for the Win32 CREDENTIALW structure.
        unsafe { core::mem::zeroed() }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
}

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_ENTERPRISE: u32 = 3;
const ERROR_NOT_FOUND: u32 = 1168;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

#[derive(Clone, Copy)]
struct Api {
    write: WriteFn,
    read: ReadFn,
    delete: DeleteFn,
    enumerate: EnumerateFn,
    free: FreeFn,
    last_error: LastErrorFn,
}

fn api() -> Result<Api, u32> {
    let library: Vec<u16> = "advapi32.dll".encode_utf16().chain([0]).collect();
    // SAFETY: the module name is terminated and the search is restricted to
    // System32. The reference remains loaded for the lifetime of this Realm.
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    // SAFETY: a non-null module came from LoadLibraryExW; a null result is
    // handled by the same resolver using the calling thread's last error.
    unsafe { resolve_api(module, GetProcAddress, GetLastError) }
}

unsafe fn resolve_api(
    module: *mut c_void,
    get_procedure: GetProcedureFn,
    last_error: LastErrorFn,
) -> Result<Api, u32> {
    if module.is_null() {
        // SAFETY: retrieves the error immediately after the failed load.
        return Err(unsafe { last_error() });
    }
    let load = |name: &'static [u8]| {
        // SAFETY: every caller passes a static NUL-terminated API name.
        unsafe { get_procedure(module, name.as_ptr()) }
    };
    let write = load(b"CredWriteW\0");
    let read = load(b"CredReadW\0");
    let delete = load(b"CredDeleteW\0");
    let enumerate = load(b"CredEnumerateW\0");
    let free = load(b"CredFree\0");
    if [write, read, delete, enumerate, free].contains(&core::ptr::null()) {
        return Err(127); // ERROR_PROC_NOT_FOUND.
    }
    // SAFETY: each address is resolved from advapi32.dll using its documented
    // Win32 ABI and exact signature; the module remains loaded for the Realm.
    Ok(unsafe {
        Api {
            write: core::mem::transmute::<*const c_void, WriteFn>(write),
            read: core::mem::transmute::<*const c_void, ReadFn>(read),
            delete: core::mem::transmute::<*const c_void, DeleteFn>(delete),
            enumerate: core::mem::transmute::<*const c_void, EnumerateFn>(enumerate),
            free: core::mem::transmute::<*const c_void, FreeFn>(free),
            last_error,
        }
    })
}

#[derive(Clone, Copy)]
enum Operation {
    SetPassword,
    GetPassword,
    DeletePassword,
    FindPassword,
    FindCredentials,
}

struct Input {
    operation: Operation,
    service: String,
    account: String,
    password: String,
}

#[derive(Debug)]
enum Output {
    Undefined,
    Password(Option<String>),
    Deleted(bool),
    Credentials(Vec<(String, String)>),
}

struct Allocation {
    pointer: *const c_void,
    free: unsafe extern "system" fn(*const c_void),
}

impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: successful CredRead/Enumerate returned this allocation, and
        // this owner releases it exactly once after copying nested values.
        unsafe { (self.free)(self.pointer) };
    }
}

/// The module exports the same Promise-returning functions as keytar.
pub(crate) fn module(realm: &mut Realm) -> Object {
    module_with_api(realm, api())
}

fn module_with_api(realm: &mut Realm, api: Result<Api, u32>) -> Object {
    let exports = realm.object();
    for (name, operation, length) in [
        ("setPassword", Operation::SetPassword, 3),
        ("getPassword", Operation::GetPassword, 2),
        ("deletePassword", Operation::DeletePassword, 2),
        ("findPassword", Operation::FindPassword, 1),
        ("findCredentials", Operation::FindCredentials, 1),
    ] {
        let function = realm.function(name, length, false, move |realm, _, args| {
            let service = argument(realm, args, 0)?;
            let account = match operation {
                Operation::SetPassword | Operation::GetPassword | Operation::DeletePassword => {
                    argument(realm, args, 1)?
                }
                Operation::FindPassword | Operation::FindCredentials => String::new(),
            };
            let password = if matches!(operation, Operation::SetPassword) {
                argument(realm, args, 2)?
            } else {
                String::new()
            };
            run(
                realm,
                api,
                Input {
                    operation,
                    service,
                    account,
                    password,
                },
            )
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports
}

fn argument(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, crate::Thrown> {
    let value = args.get(index).cloned().unwrap_or(Value::Undefined);
    Ok(ops::to_string(realm, &value)?.to_rust_lossy())
}

fn run(realm: &mut Realm, api: Result<Api, u32>, input: Input) -> crate::Completion {
    let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
    let failure_resolver = reject.clone();
    let api = match api {
        Ok(api) => api,
        Err(code) => {
            let error = realm.error(
                ErrorKind::Error,
                &format!("Could not load Windows Credential Manager (error {code})"),
            );
            ops::call(realm, &reject, Value::Undefined, &[Value::Object(error)])?;
            return Ok(promise);
        }
    };
    let submitted = realm.submit_io(
        move || perform(api, input),
        move |realm, result| {
            let (settler, value) = match result {
                Ok(output) => (resolve, to_value(realm, output)),
                Err(message) => {
                    let error = realm.error(ErrorKind::Error, &message);
                    (reject, Value::Object(error))
                }
            };
            ops::call(realm, &settler, Value::Undefined, &[value]).map(drop)
        },
    );
    if let Err(code) = submitted {
        let error = realm.error(
            ErrorKind::Error,
            &format!("Could not queue Windows credential operation (error {code})"),
        );
        ops::call(
            realm,
            &failure_resolver,
            Value::Undefined,
            &[Value::Object(error)],
        )?;
    }
    Ok(promise)
}

fn to_value(realm: &Realm, output: Output) -> Value {
    match output {
        Output::Undefined => Value::Undefined,
        Output::Password(password) => password.map_or(Value::Null, |text| Value::str(&text)),
        Output::Deleted(deleted) => Value::Boolean(deleted),
        Output::Credentials(credentials) => Value::Object(
            realm.array(
                credentials
                    .into_iter()
                    .map(|(account, password)| {
                        let value = realm.object();
                        value.define(Key::name("account"), Property::data(Value::str(&account)));
                        value.define(Key::name("password"), Property::data(Value::str(&password)));
                        Value::Object(value)
                    })
                    .collect(),
            ),
        ),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16()
        .take_while(|unit| *unit != 0)
        .chain([0])
        .collect()
}

fn failure(api: Api) -> String {
    // SAFETY: the API is queried on the same worker thread as the failed call.
    format!("Windows Credential Manager error {}", unsafe {
        (api.last_error)()
    })
}

fn not_found(api: Api) -> Result<bool, String> {
    // SAFETY: immediately reads the failure from the worker's last API call.
    let code = unsafe { (api.last_error)() };
    if code == ERROR_NOT_FOUND {
        Ok(false)
    } else {
        Err(format!("Windows Credential Manager error {code}"))
    }
}

fn blob_size(length: usize) -> Result<u32, String> {
    u32::try_from(length).map_err(|_| "Credential password exceeds 4 GiB".to_string())
}

fn perform(api: Api, input: Input) -> Result<Output, String> {
    let target = wide(&format!("{}/{}", input.service, input.account));
    match input.operation {
        Operation::SetPassword => {
            let username = wide(&input.account);
            let credential = Credential {
                kind: CRED_TYPE_GENERIC,
                target_name: target.as_ptr().cast_mut(),
                username: username.as_ptr().cast_mut(),
                blob: input.password.as_ptr().cast_mut(),
                blob_size: blob_size(input.password.len())?,
                persist: CRED_PERSIST_ENTERPRISE,
                ..Credential::default()
            };
            // SAFETY: all pointers reference buffers alive for this call;
            // CredWriteW copies the credential fields before returning.
            if unsafe { (api.write)(&credential, 0) } == 0 {
                return Err(failure(api));
            }
            Ok(Output::Undefined)
        }
        Operation::GetPassword => {
            let mut credential = core::ptr::null_mut();
            // SAFETY: target is terminated and the output pointer is writable.
            if unsafe { (api.read)(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
                let _ = not_found(api)?;
                return Ok(Output::Password(None));
            }
            let _allocation = Allocation {
                pointer: credential.cast(),
                free: api.free,
            };
            // SAFETY: a successful read returns a live credential allocation.
            Ok(Output::Password(Some(unsafe { password(&*credential) })))
        }
        Operation::DeletePassword => {
            // SAFETY: target is terminated and the reserved flags are zero.
            if unsafe { (api.delete)(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
                Ok(Output::Deleted(true))
            } else {
                not_found(api).map(Output::Deleted)
            }
        }
        Operation::FindPassword | Operation::FindCredentials => {
            let filter = wide(&format!("{}*", input.service));
            let mut count = 0_u32;
            let mut credentials = core::ptr::null_mut();
            // SAFETY: the filter is terminated and both output pointers are valid.
            if unsafe { (api.enumerate)(filter.as_ptr(), 0, &mut count, &mut credentials) } == 0 {
                let _ = not_found(api)?;
                return Ok(match input.operation {
                    Operation::FindPassword => Output::Password(None),
                    _ => Output::Credentials(Vec::new()),
                });
            }
            let _allocation = Allocation {
                pointer: credentials.cast(),
                free: api.free,
            };
            let mut result = Vec::new();
            for index in 0..count as usize {
                // SAFETY: successful enumeration returns `count` pointers in
                // the allocation held until all nested strings are copied.
                let credential = unsafe { &**credentials.add(index) };
                if matches!(input.operation, Operation::FindPassword) {
                    // SAFETY: the credential and its password remain allocated.
                    return Ok(Output::Password(Some(unsafe { password(credential) })));
                }
                if !credential.username.is_null() && credential.blob_size != 0 {
                    // SAFETY: both buffers are owned by the live enumeration.
                    let account = unsafe { account(credential.username) };
                    // SAFETY: the credential blob stays owned by the active
                    // enumeration allocation until this copy completes.
                    let password = unsafe { password(credential) };
                    result.push((account, password));
                }
            }
            Ok(Output::Credentials(result))
        }
    }
}

unsafe fn password(credential: &Credential) -> String {
    if credential.blob_size == 0 {
        return String::new();
    }
    // SAFETY: the caller owns the credential allocation and the declared blob
    // size bounds the byte range.
    let bytes = unsafe { slice::from_raw_parts(credential.blob, credential.blob_size as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

unsafe fn account(pointer: *const u16) -> String {
    let mut length = 0;
    // SAFETY: the caller owns this NUL-terminated username within the live
    // CredEnumerateW allocation.
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: the scan established the readable UTF-16 range.
    String::from_utf16_lossy(unsafe { slice::from_raw_parts(pointer, length) })
}

#[cfg(test)]
mod tests;
