//! Windows CNG primitives used only by the native `crypto` binding: digests
//! and HMACs, PBKDF2 and the AES block cipher. Keeping these imports in their
//! own module prevents unrelated process and filesystem executables from
//! importing bcrypt.dll.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::ffi::c_void;

#[link(name = "bcrypt", kind = "raw-dylib")]
unsafe extern "system" {
    pub(crate) fn BCryptOpenAlgorithmProvider(
        algorithm: *mut *mut c_void,
        name: *const u16,
        implementation: *const u16,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptCloseAlgorithmProvider(algorithm: *mut c_void, flags: u32) -> i32;
    pub(crate) fn BCryptCreateHash(
        algorithm: *mut c_void,
        hash: *mut *mut c_void,
        object: *mut u8,
        object_length: u32,
        secret: *mut u8,
        secret_length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptHashData(hash: *mut c_void, data: *mut u8, length: u32, flags: u32) -> i32;
    pub(crate) fn BCryptFinishHash(
        hash: *mut c_void,
        output: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptDestroyHash(hash: *mut c_void) -> i32;
    pub(crate) fn BCryptDuplicateHash(
        hash: *mut c_void,
        duplicate: *mut *mut c_void,
        object: *mut u8,
        object_length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptDeriveKeyPBKDF2(
        prf: *mut c_void,
        password: *mut u8,
        password_length: u32,
        salt: *mut u8,
        salt_length: u32,
        iterations: u64,
        output: *mut u8,
        output_length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptSetProperty(
        object: *mut c_void,
        property: *const u16,
        input: *const u8,
        input_length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptGenerateSymmetricKey(
        algorithm: *mut c_void,
        key: *mut *mut c_void,
        object: *mut u8,
        object_length: u32,
        secret: *const u8,
        secret_length: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptEncrypt(
        key: *mut c_void,
        input: *const u8,
        input_length: u32,
        padding: *const c_void,
        iv: *mut u8,
        iv_length: u32,
        output: *mut u8,
        output_length: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptDecrypt(
        key: *mut c_void,
        input: *const u8,
        input_length: u32,
        padding: *const c_void,
        iv: *mut u8,
        iv_length: u32,
        output: *mut u8,
        output_length: u32,
        result: *mut u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn BCryptDestroyKey(key: *mut c_void) -> i32;
}

/// The status reported for a digest name CNG does not provide.
pub(crate) const UNSUPPORTED: i32 = i32::MIN;

/// The digests the binding provides, by their canonical lowercase names.
pub(crate) const DIGESTS: [&str; 5] = ["md5", "sha1", "sha256", "sha384", "sha512"];

/// An open CNG algorithm provider, closed when its last user is dropped.
struct Provider(*mut c_void);

impl Provider {
    fn open(name: &str, flags: u32) -> Result<Rc<Self>, i32> {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut algorithm = core::ptr::null_mut();
        // SAFETY: name is a null-terminated UTF-16 algorithm ID and
        // algorithm points to writable storage for the returned handle.
        let status = unsafe {
            BCryptOpenAlgorithmProvider(&mut algorithm, name.as_ptr(), core::ptr::null(), flags)
        };
        if status < 0 {
            return Err(status);
        }
        Ok(Rc::new(Self(algorithm)))
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        // SAFETY: the provider was opened by `open` and is closed once.
        unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}

/// The CNG algorithm ID and output length of a digest name.
fn digest(name: &str) -> Result<(&'static str, usize), i32> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "md5" => ("MD5", 16),
        "sha1" => ("SHA1", 20),
        "sha256" => ("SHA256", 32),
        "sha384" => ("SHA384", 48),
        "sha512" => ("SHA512", 64),
        _ => return Err(UNSUPPORTED),
    })
}

const BCRYPT_ALG_HANDLE_HMAC_FLAG: u32 = 0x8;

/// An incremental Windows CNG digest. The source stream can feed chunks
/// directly, without retaining whole files in application memory.
pub(crate) struct CryptoHash {
    provider: Rc<Provider>,
    hash: *mut c_void,
    output_length: usize,
}

impl CryptoHash {
    /// Opens one of the digest algorithms used by Node's crypto module.
    pub(crate) fn new(name: &str) -> Result<Self, i32> {
        Self::open(name, None)
    }

    /// Opens an HMAC of one of those digests with a secret key.
    pub(crate) fn hmac(name: &str, key: &[u8]) -> Result<Self, i32> {
        Self::open(name, Some(key))
    }

    /// The output length of a digest name.
    pub(crate) fn output_length(name: &str) -> Result<usize, i32> {
        digest(name).map(|(_, length)| length)
    }

    fn open(name: &str, key: Option<&[u8]>) -> Result<Self, i32> {
        let (name, output_length) = digest(name)?;
        let flags = if key.is_some() {
            BCRYPT_ALG_HANDLE_HMAC_FLAG
        } else {
            0
        };
        let provider = Provider::open(name, flags)?;
        let mut hash = core::ptr::null_mut();
        // CNG reads the secret during the call and keeps its own copy.
        let mut secret = key.map(<[u8]>::to_vec).unwrap_or_default();
        // SAFETY: the provider is live and both handle output and optional
        // object storage satisfy CNG's documented contract; the secret
        // buffer holds its length.
        let status = unsafe {
            BCryptCreateHash(
                provider.0,
                &mut hash,
                core::ptr::null_mut(),
                0,
                if key.is_some() {
                    secret.as_mut_ptr()
                } else {
                    core::ptr::null_mut()
                },
                secret.len() as u32,
                0,
            )
        };
        if status < 0 {
            return Err(status);
        }
        Ok(Self {
            provider,
            hash,
            output_length,
        })
    }

    /// Adds one input chunk to the digest.
    pub(crate) fn update(&mut self, mut data: &[u8]) -> Result<(), i32> {
        while !data.is_empty() {
            let length = data.len().min(u32::MAX as usize);
            // SAFETY: data is valid for length bytes and the hash is live.
            let status =
                unsafe { BCryptHashData(self.hash, data.as_ptr() as *mut u8, length as u32, 0) };
            if status < 0 {
                return Err(status);
            }
            data = &data[length..];
        }
        Ok(())
    }

    /// An independent digest with the same input so far.
    pub(crate) fn duplicate(&self) -> Result<Self, i32> {
        let mut hash = core::ptr::null_mut();
        // SAFETY: the source hash is live; CNG allocates the duplicate's
        // object itself when no storage is given.
        let status =
            unsafe { BCryptDuplicateHash(self.hash, &mut hash, core::ptr::null_mut(), 0, 0) };
        if status < 0 {
            return Err(status);
        }
        Ok(Self {
            provider: self.provider.clone(),
            hash,
            output_length: self.output_length,
        })
    }

    /// Finishes the digest into an exactly sized output buffer.
    pub(crate) fn finish(self) -> Result<Vec<u8>, i32> {
        let mut output = alloc::vec![0; self.output_length];
        // SAFETY: output is writable for the digest's exact fixed length.
        let status = unsafe {
            BCryptFinishHash(self.hash, output.as_mut_ptr(), self.output_length as u32, 0)
        };
        if status < 0 {
            return Err(status);
        }
        Ok(output)
    }
}

impl Drop for CryptoHash {
    fn drop(&mut self) {
        // SAFETY: the hash is owned by this value and destroyed once; the
        // provider outlives it.
        unsafe { BCryptDestroyHash(self.hash) };
    }
}

/// PBKDF2 with an HMAC of a digest, as Node's `pbkdf2` derives keys.
pub(crate) fn pbkdf2(
    digest_name: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u64,
    length: usize,
) -> Result<Vec<u8>, i32> {
    let (name, _) = digest(digest_name)?;
    let provider = Provider::open(name, BCRYPT_ALG_HANDLE_HMAC_FLAG)?;
    let mut output = alloc::vec![0; length];
    let mut password = password.to_vec();
    let mut salt = salt.to_vec();
    // SAFETY: every buffer is valid for the length passed with it and the
    // HMAC provider is live for the call.
    let status = unsafe {
        BCryptDeriveKeyPBKDF2(
            provider.0,
            password.as_mut_ptr(),
            password.len() as u32,
            salt.as_mut_ptr(),
            salt.len() as u32,
            iterations,
            output.as_mut_ptr(),
            length as u32,
            0,
        )
    };
    if status < 0 {
        return Err(status);
    }
    Ok(output)
}

/// An AES key whose block function runs through CNG in ECB mode, for the
/// cipher modes `crypto_cipher` builds on it.
pub(crate) struct AesKey {
    _provider: Rc<Provider>,
    key: *mut c_void,
}

impl AesKey {
    /// A 16, 24 or 32 byte AES key.
    pub(crate) fn new(secret: &[u8]) -> Result<Self, i32> {
        let provider = Provider::open("AES", 0)?;
        let mode: Vec<u16> = "ChainingModeECB".encode_utf16().chain([0]).collect();
        let property: Vec<u16> = "ChainingMode".encode_utf16().chain([0]).collect();
        // SAFETY: property and mode are null-terminated UTF-16 strings and
        // the length counts the mode's bytes including its terminator.
        let status = unsafe {
            BCryptSetProperty(
                provider.0,
                property.as_ptr(),
                mode.as_ptr() as *const u8,
                (mode.len() * 2) as u32,
                0,
            )
        };
        if status < 0 {
            return Err(status);
        }
        let mut key = core::ptr::null_mut();
        // SAFETY: CNG allocates the key object itself and copies the secret,
        // which is valid for its length.
        let status = unsafe {
            BCryptGenerateSymmetricKey(
                provider.0,
                &mut key,
                core::ptr::null_mut(),
                0,
                secret.as_ptr(),
                secret.len() as u32,
                0,
            )
        };
        if status < 0 {
            return Err(status);
        }
        Ok(Self {
            _provider: provider,
            key,
        })
    }

    /// Encrypts whole 16-byte blocks in place.
    pub(crate) fn encrypt(&self, blocks: &mut [u8]) {
        self.run(blocks, true);
    }

    /// Decrypts whole 16-byte blocks in place.
    pub(crate) fn decrypt(&self, blocks: &mut [u8]) {
        self.run(blocks, false);
    }

    fn run(&self, blocks: &mut [u8], encrypt: bool) {
        debug_assert!(blocks.len().is_multiple_of(16));
        for chunk in blocks.chunks_mut(1 << 24) {
            let mut written = 0u32;
            let pointer = chunk.as_mut_ptr();
            let length = chunk.len() as u32;
            // SAFETY: the key is live, input and output are the same valid
            // buffer of whole blocks (CNG encrypts symmetric blocks in
            // place) and ECB takes no IV or padding.
            unsafe {
                if encrypt {
                    BCryptEncrypt(
                        self.key,
                        pointer,
                        length,
                        core::ptr::null(),
                        core::ptr::null_mut(),
                        0,
                        pointer,
                        length,
                        &mut written,
                        0,
                    )
                } else {
                    BCryptDecrypt(
                        self.key,
                        pointer,
                        length,
                        core::ptr::null(),
                        core::ptr::null_mut(),
                        0,
                        pointer,
                        length,
                        &mut written,
                        0,
                    )
                }
            };
        }
    }
}

impl Drop for AesKey {
    fn drop(&mut self) {
        // SAFETY: the key is owned by this value and destroyed once, before
        // its provider.
        unsafe { BCryptDestroyKey(self.key) };
    }
}
