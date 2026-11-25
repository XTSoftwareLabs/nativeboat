//! Electron's `safeStorage` as Chromium's OSCrypt keeps secrets on Windows:
//! an AES-256-GCM key protected with DPAPI in the `Local State` file of the
//! user data directory, under `os_crypt.encrypted_key`, and ciphertexts of
//! `v10`, a 12-byte nonce, then the sealed text and its tag. Text an
//! application encrypted while it ran on Electron decrypts natively.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::crypto_platform::{
    BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt,
    BCryptGenerateSymmetricKey, BCryptOpenAlgorithmProvider, BCryptSetProperty,
};
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;
const TAG_LENGTH: usize = 16;
const VERSION: &[u8] = b"v10";
const DPAPI_PREFIX: &[u8] = b"DPAPI";
const CRYPTPROTECT_AUDIT: u32 = 0x10;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;

#[repr(C)]
struct DataBlob {
    size: u32,
    data: *mut u8,
}

/// `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO`.
#[repr(C)]
struct AuthenticatedCipherInfo {
    size: u32,
    version: u32,
    nonce: *mut u8,
    nonce_size: u32,
    auth_data: *mut u8,
    auth_data_size: u32,
    tag: *mut u8,
    tag_size: u32,
    mac_context: *mut u8,
    mac_context_size: u32,
    aad_size: u32,
    data_size: u64,
    flags: u32,
}

type ProtectFn = unsafe extern "system" fn(
    input: *const DataBlob,
    description: *const u16,
    entropy: *const DataBlob,
    reserved: *mut c_void,
    prompt: *mut c_void,
    flags: u32,
    output: *mut DataBlob,
) -> i32;
type UnprotectFn = unsafe extern "system" fn(
    input: *const DataBlob,
    description: *mut *mut u16,
    entropy: *const DataBlob,
    reserved: *mut c_void,
    prompt: *mut c_void,
    flags: u32,
    output: *mut DataBlob,
) -> i32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Protects or unprotects bytes with DPAPI for the current user, from
/// `crypt32.dll` loaded when first needed.
fn dpapi(protect: bool, input: &[u8]) -> Option<Vec<u8>> {
    let library = wide("crypt32.dll");
    // SAFETY: the name is null-terminated; system32 is searched only.
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }
    let name: &[u8] = if protect {
        b"CryptProtectData\0"
    } else {
        b"CryptUnprotectData\0"
    };
    // SAFETY: the module is loaded and the name is null-terminated.
    let function = unsafe { GetProcAddress(module, name.as_ptr()) };
    if function.is_null() {
        return None;
    }
    let blob = DataBlob {
        size: input.len() as u32,
        data: input.as_ptr().cast_mut(),
    };
    let mut output = DataBlob {
        size: 0,
        data: core::ptr::null_mut(),
    };
    // As Electron's OSCrypt describes the secrets it protects.
    let description = wide("Chromium");
    let done = if protect {
        // SAFETY: the address is CryptProtectData's; the blobs and the
        // description outlive the call.
        unsafe {
            let call: ProtectFn = core::mem::transmute(function);
            call(
                &blob,
                description.as_ptr(),
                core::ptr::null(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                CRYPTPROTECT_AUDIT,
                &mut output,
            )
        }
    } else {
        // SAFETY: the address is CryptUnprotectData's; the blobs outlive the
        // call.
        unsafe {
            let call: UnprotectFn = core::mem::transmute(function);
            call(
                &blob,
                core::ptr::null_mut(),
                core::ptr::null(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                0,
                &mut output,
            )
        }
    };
    if done == 0 || output.data.is_null() {
        return None;
    }
    // SAFETY: DPAPI returned `size` bytes at `data`, which LocalFree releases.
    let bytes = unsafe { core::slice::from_raw_parts(output.data, output.size as usize) }.to_vec();
    // SAFETY: the buffer was allocated by DPAPI for this call.
    unsafe { LocalFree(output.data.cast()) };
    Some(bytes)
}

/// Seals or opens text with AES-256-GCM. Sealed text is the ciphertext
/// followed by its tag; opening fails when the tag does not match.
fn aes_gcm(
    seal: bool,
    key: &[u8; KEY_LENGTH],
    nonce: &[u8; NONCE_LENGTH],
    input: &[u8],
) -> Option<Vec<u8>> {
    let mut algorithm = core::ptr::null_mut();
    let aes = wide("AES");
    // SAFETY: the out pointer is valid and the name null-terminated.
    if unsafe { BCryptOpenAlgorithmProvider(&mut algorithm, aes.as_ptr(), core::ptr::null(), 0) }
        < 0
    {
        return None;
    }
    let result = (|| {
        let property = wide("ChainingMode");
        let mode = wide("ChainingModeGCM");
        // SAFETY: the property value is a null-terminated string of its size.
        let set = unsafe {
            BCryptSetProperty(
                algorithm,
                property.as_ptr(),
                mode.as_ptr().cast(),
                (mode.len() * 2) as u32,
                0,
            )
        };
        if set < 0 {
            return None;
        }
        let mut handle = core::ptr::null_mut();
        // SAFETY: the provider is open and the secret is KEY_LENGTH bytes.
        let made = unsafe {
            BCryptGenerateSymmetricKey(
                algorithm,
                &mut handle,
                core::ptr::null_mut(),
                0,
                key.as_ptr(),
                KEY_LENGTH as u32,
                0,
            )
        };
        if made < 0 {
            return None;
        }
        let (text, mut tag) = if seal {
            (input, [0u8; TAG_LENGTH])
        } else {
            let split = input.len().checked_sub(TAG_LENGTH)?;
            let mut tag = [0u8; TAG_LENGTH];
            tag.copy_from_slice(&input[split..]);
            (&input[..split], tag)
        };
        let mut nonce = *nonce;
        let info = AuthenticatedCipherInfo {
            size: core::mem::size_of::<AuthenticatedCipherInfo>() as u32,
            version: 1,
            nonce: nonce.as_mut_ptr(),
            nonce_size: NONCE_LENGTH as u32,
            auth_data: core::ptr::null_mut(),
            auth_data_size: 0,
            tag: tag.as_mut_ptr(),
            tag_size: TAG_LENGTH as u32,
            mac_context: core::ptr::null_mut(),
            mac_context_size: 0,
            aad_size: 0,
            data_size: 0,
            flags: 0,
        };
        let mut output = alloc::vec![0u8; text.len()];
        let mut written = 0u32;
        // SAFETY: the key is live; input, output, nonce and tag buffers are
        // sized as passed and outlive the call.
        let status = unsafe {
            let call = if seal { BCryptEncrypt } else { BCryptDecrypt };
            call(
                handle,
                text.as_ptr(),
                text.len() as u32,
                (&raw const info).cast(),
                core::ptr::null_mut(),
                0,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut written,
                0,
            )
        };
        // SAFETY: the key was made above and is no longer used.
        unsafe { BCryptDestroyKey(handle) };
        if status < 0 {
            return None;
        }
        output.truncate(written as usize);
        if seal {
            output.extend_from_slice(&tag);
        }
        Some(output)
    })();
    // SAFETY: the provider was opened above.
    unsafe { BCryptCloseAlgorithmProvider(algorithm, 0) };
    result
}

/// `v10`, the nonce, then the sealed text.
fn encrypt(key: &[u8; KEY_LENGTH], plaintext: &[u8]) -> Option<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LENGTH];
    crate::platform::random_fill(&mut nonce);
    let sealed = aes_gcm(true, key, &nonce, plaintext)?;
    let mut ciphertext = VERSION.to_vec();
    ciphertext.extend_from_slice(&nonce);
    ciphertext.extend_from_slice(&sealed);
    Some(ciphertext)
}

fn decrypt(key: &[u8; KEY_LENGTH], ciphertext: &[u8]) -> Option<Vec<u8>> {
    let rest = ciphertext.strip_prefix(VERSION)?;
    if rest.len() < NONCE_LENGTH {
        return None;
    }
    let mut nonce = [0u8; NONCE_LENGTH];
    nonce.copy_from_slice(&rest[..NONCE_LENGTH]);
    aes_gcm(false, key, &nonce, &rest[NONCE_LENGTH..])
}

/// The key DPAPI protects in the `Local State` text, if it holds one.
fn stored_key(encrypted: &str) -> Option<[u8; KEY_LENGTH]> {
    let units: Vec<u16> = encrypted.encode_utf16().collect();
    let decoded = crate::encoding::forgiving_base64(&units).ok()?;
    let protected = decoded.strip_prefix(DPAPI_PREFIX)?;
    dpapi(false, protected)?.try_into().ok()
}

/// The process's key, read from or added to `Local State` once.
static KEY: AtomicPtr<[u8; KEY_LENGTH]> = AtomicPtr::new(core::ptr::null_mut());

fn key(realm: &mut Realm) -> Result<Option<[u8; KEY_LENGTH]>, Thrown> {
    let cached = KEY.load(Ordering::Acquire);
    if !cached.is_null() {
        // SAFETY: a stored key is leaked and never freed.
        return Ok(Some(unsafe { *cached }));
    }
    let Value::String(directory) = crate::electron_builtin::app_path(realm, "userData")? else {
        return Ok(None);
    };
    let directory = directory.to_rust_lossy();
    let path = alloc::format!("{}\\Local State", directory.trim_end_matches(['\\', '/']));
    // The preferences Electron keeps there stay as they are.
    let text = crate::platform::read_file(&path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let parsed = if text.trim().is_empty() {
        None
    } else {
        crate::json_builtin::parse(realm, Value::Undefined, &[Value::str(&text)])
            .ok()
            .filter(|value| value.as_object().is_some())
    };
    let state = parsed.unwrap_or_else(|| Value::Object(realm.object()));
    let os_crypt = match ops::get_named(realm, &state, "os_crypt")? {
        value @ Value::Object(_) => value,
        _ => {
            let object = Value::Object(realm.object());
            ops::set(realm, &state, &Key::name("os_crypt"), object.clone())?;
            object
        }
    };
    let stored = match ops::get_named(realm, &os_crypt, "encrypted_key")? {
        Value::String(encrypted) => stored_key(&encrypted.to_rust_lossy()),
        _ => None,
    };
    let key = match stored {
        Some(key) => key,
        None => {
            let mut key = [0u8; KEY_LENGTH];
            crate::platform::random_fill(&mut key);
            let Some(protected) = dpapi(true, &key) else {
                return Ok(None);
            };
            let mut encrypted = DPAPI_PREFIX.to_vec();
            encrypted.extend_from_slice(&protected);
            let encoded = crate::encoding::base64_encode(&encrypted, false);
            // Electron notes that the key was protected with auditing.
            ops::set(
                realm,
                &os_crypt,
                &Key::name("audit_enabled"),
                Value::Boolean(true),
            )?;
            ops::set(
                realm,
                &os_crypt,
                &Key::name("encrypted_key"),
                Value::str(&encoded),
            )?;
            let Value::String(json) =
                crate::json_builtin::stringify(realm, Value::Undefined, &[state])?
            else {
                return Ok(None);
            };
            let directory_wide = wide(&directory);
            let _ = crate::fs_win::mkdir(&directory_wide);
            if !crate::platform::write_file(&path, json.to_rust_lossy().as_bytes()) {
                return Ok(None);
            }
            key
        }
    };
    let leaked = alloc::boxed::Box::leak(alloc::boxed::Box::new(key));
    KEY.store(leaked, Ordering::Release);
    Ok(Some(key))
}

fn failure(realm: &Realm, message: &str) -> Thrown {
    realm.throw(ErrorKind::Error, message)
}

/// Electron's `safeStorage` module.
pub(crate) fn module(realm: &Realm) -> Object {
    let storage = realm.object();
    let available = realm.function("isEncryptionAvailable", 0, false, |realm, _, _| {
        Ok(Value::Boolean(key(realm)?.is_some()))
    });
    let encrypt_string = realm.function("encryptString", 1, false, |realm, _, args| {
        let Some(Value::String(plaintext)) = args.first() else {
            return Err(realm.throw(
                ErrorKind::Type,
                "Error processing argument at index 0, conversion failure from undefined",
            ));
        };
        let plaintext = plaintext.to_rust_lossy();
        let Some(key) = key(realm)? else {
            return Err(failure(
                realm,
                "Error while encrypting the text provided to safeStorage.encryptString. Encryption is not available.",
            ));
        };
        let Some(ciphertext) = encrypt(&key, plaintext.as_bytes()) else {
            return Err(failure(
                realm,
                "Error while encrypting the text provided to safeStorage.encryptString.",
            ));
        };
        crate::electron_clipboard::buffer_value(realm, ciphertext)
    });
    let decrypt_string = realm.function("decryptString", 1, false, |realm, _, args| {
        let Some(key) = key(realm)? else {
            return Err(failure(
                realm,
                "Error while decrypting the ciphertext provided to safeStorage.decryptString. Decryption is not available.",
            ));
        };
        let Some(ciphertext) = args.first().and_then(crate::typed_array_builtin::view_bytes) else {
            return Err(failure(
                realm,
                "Expected the first argument of decryptString() to be a buffer",
            ));
        };
        if ciphertext.is_empty() {
            return Ok(Value::str(""));
        }
        if !ciphertext.starts_with(b"v10") && !ciphertext.starts_with(b"v11") {
            return Err(failure(
                realm,
                "Error while decrypting the ciphertext provided to safeStorage.decryptString. Ciphertext does not appear to be encrypted.",
            ));
        }
        match decrypt(&key, &ciphertext) {
            Some(plaintext) => Ok(Value::str(&String::from_utf8_lossy(&plaintext))),
            None => Err(failure(
                realm,
                "Error while decrypting the ciphertext provided to safeStorage.decryptString.",
            )),
        }
    });
    // Linux's choice of a plain-text backend; nothing to choose on Windows.
    let plain_text = realm.function("setUsePlainTextEncryption", 1, false, |_, _, _| {
        Ok(Value::Undefined)
    });
    for (name, function) in [
        ("isEncryptionAvailable", available),
        ("encryptString", encrypt_string),
        ("decryptString", decrypt_string),
        ("setUsePlainTextEncryption", plain_text),
    ] {
        storage.define(Key::name(name), Property::data(Value::Object(function)));
    }
    storage
}

#[cfg(test)]
#[path = "electron_safe_storage/tests.rs"]
mod tests;
