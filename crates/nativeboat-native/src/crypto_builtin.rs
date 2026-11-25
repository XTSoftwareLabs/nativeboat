//! The native binding of Node's `crypto` library module, on Windows CNG and
//! ProcessPrng: digests and HMACs that copy, random bytes written into a
//! view, PBKDF2, AES ciphers and constant-time comparison. The library
//! module validates arguments and converts encodings; these functions take
//! bytes as `ArrayBufferView`s and return Buffers.

use crate::crypto_cipher::{self, Cipher, Failure};
use crate::crypto_platform::{self, CryptoHash};
use crate::typed_array_builtin;
use crate::{Completion, ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

/// `internalBinding('crypto')`.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let digests = crypto_platform::DIGESTS
        .iter()
        .map(|name| Value::str(name))
        .collect();
    let digests = realm.array(digests);
    exports.define(Key::name("digests"), Property::data(Value::Object(digests)));
    let ciphers = crypto_cipher::CIPHERS
        .iter()
        .map(|(name, _, _)| Value::str(name))
        .collect();
    let ciphers = realm.array(ciphers);
    exports.define(Key::name("ciphers"), Property::data(Value::Object(ciphers)));
    realm.method(&exports, "Hash", 2, |realm, _, args| {
        let algorithm = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let hash = match arg(args, 1) {
            Value::Undefined => CryptoHash::new(&algorithm),
            key => {
                let key = view(realm, &key)?;
                CryptoHash::hmac(&algorithm, &key)
            }
        }
        .map_err(|status| crypto_error(realm, status))?;
        Ok(Value::Object(hash_handle(realm, hash)))
    });
    realm.method(&exports, "randomFill", 1, |realm, _, args| {
        let target = arg(args, 0);
        let (bytes, offset, length) = typed_array_builtin::view_range(&target)
            .ok_or_else(|| realm.throw(ErrorKind::Type, "randomFill needs an ArrayBufferView"))?;
        crate::platform::random_fill(&mut bytes.borrow_mut()[offset..offset + length]);
        Ok(target)
    });
    realm.method(&exports, "pbkdf2", 5, |realm, _, args| {
        let password = view(realm, &arg(args, 0))?;
        let salt = view(realm, &arg(args, 1))?;
        let iterations = ops::to_number(realm, &arg(args, 2))? as u64;
        let length = ops::to_number(realm, &arg(args, 3))? as usize;
        let digest = ops::to_string(realm, &arg(args, 4))?.to_rust_lossy();
        let key = crypto_platform::pbkdf2(&digest, &password, &salt, iterations, length)
            .map_err(|status| crypto_error(realm, status))?;
        buffer_value(realm, key)
    });
    realm.method(&exports, "Cipher", 5, |realm, _, args| {
        let name = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let (_, mode) = crypto_cipher::lookup(&name)
            .ok_or_else(|| realm.throw(ErrorKind::Error, "Unknown cipher"))?;
        let key = view(realm, &arg(args, 1))?;
        let iv = match arg(args, 2) {
            Value::Undefined | Value::Null => Vec::new(),
            iv => view(realm, &iv)?,
        };
        let decrypt = ops::to_boolean(&arg(args, 3));
        let tag_length = match arg(args, 4) {
            Value::Number(length) => length as usize,
            _ => 16,
        };
        let cipher = Cipher::new(mode, &key, &iv, decrypt, tag_length)
            .map_err(|status| crypto_error(realm, status))?;
        Ok(Value::Object(cipher_handle(realm, cipher)))
    });
    realm.method(&exports, "timingSafeEqual", 2, |realm, _, args| {
        let left = view(realm, &arg(args, 0))?;
        let right = view(realm, &arg(args, 1))?;
        Ok(Value::Boolean(crypto_cipher::constant_time_equal(
            &left, &right,
        )))
    });
    exports
}

type Digest = Rc<RefCell<Option<CryptoHash>>>;

/// A digest's handle: `update(view)`, `digest()` and `copy()`. A finished
/// digest throws `ERR_CRYPTO_HASH_FINALIZED`, as Node's does.
fn hash_handle(realm: &mut Realm, hash: CryptoHash) -> Object {
    let state: Digest = Rc::new(RefCell::new(Some(hash)));
    let object = realm.object();
    let update_state = state.clone();
    realm.method(&object, "update", 1, move |realm, this, args| {
        let bytes = view(realm, &arg(args, 0))?;
        let mut state = update_state.borrow_mut();
        let hash = state.as_mut().ok_or_else(|| finalized(realm))?;
        hash.update(&bytes)
            .map_err(|status| crypto_error(realm, status))?;
        Ok(this)
    });
    let copy_state = state.clone();
    realm.method(&object, "copy", 0, move |realm, _, _| {
        let copy = {
            let state = copy_state.borrow();
            let hash = state.as_ref().ok_or_else(|| finalized(realm))?;
            hash.duplicate()
                .map_err(|status| crypto_error(realm, status))?
        };
        Ok(Value::Object(hash_handle(realm, copy)))
    });
    realm.method(&object, "digest", 0, move |realm, _, _| {
        let hash = state.borrow_mut().take().ok_or_else(|| finalized(realm))?;
        let bytes = hash
            .finish()
            .map_err(|status| crypto_error(realm, status))?;
        buffer_value(realm, bytes)
    });
    object
}

/// A cipher's handle: `update(view)`, `final()`, `setAAD(view)`,
/// `setAutoPadding(on)`, `setAuthTag(view)` and `getAuthTag()`, which is
/// undefined until a GCM encryption ends.
fn cipher_handle(realm: &mut Realm, cipher: Cipher) -> Object {
    let state = Rc::new(RefCell::new(cipher));
    let object = realm.object();
    let update_state = state.clone();
    realm.method(&object, "update", 1, move |realm, _, args| {
        let bytes = view(realm, &arg(args, 0))?;
        let output = update_state
            .borrow_mut()
            .update(&bytes)
            .map_err(|failure| cipher_error(realm, failure, "update"))?;
        buffer_value(realm, output)
    });
    let final_state = state.clone();
    realm.method(&object, "final", 0, move |realm, _, _| {
        let output = final_state
            .borrow_mut()
            .finish()
            .map_err(|failure| cipher_error(realm, failure, "final"))?;
        buffer_value(realm, output)
    });
    let aad_state = state.clone();
    realm.method(&object, "setAAD", 1, move |realm, _, args| {
        let bytes = view(realm, &arg(args, 0))?;
        aad_state
            .borrow_mut()
            .add_aad(&bytes)
            .map_err(|failure| cipher_error(realm, failure, "setAAD"))?;
        Ok(Value::Undefined)
    });
    let padding_state = state.clone();
    realm.method(&object, "setAutoPadding", 1, move |realm, _, args| {
        padding_state
            .borrow_mut()
            .set_padding(ops::to_boolean(&arg(args, 0)))
            .map_err(|failure| cipher_error(realm, failure, "setAutoPadding"))?;
        Ok(Value::Undefined)
    });
    let tag_state = state.clone();
    realm.method(&object, "setAuthTag", 1, move |realm, _, args| {
        let bytes = view(realm, &arg(args, 0))?;
        tag_state
            .borrow_mut()
            .set_tag(&bytes)
            .map_err(|failure| cipher_error(realm, failure, "setAuthTag"))?;
        Ok(Value::Undefined)
    });
    realm.method(&object, "getAuthTag", 0, move |realm, _, _| {
        let tag = state.borrow().tag().map(<[u8]>::to_vec);
        match tag {
            Some(tag) => buffer_value(realm, tag),
            None => Ok(Value::Undefined),
        }
    });
    object
}

/// The error OpenSSL's failure gives in Node: a coded provider error for
/// block lengths and padding, the authentication message for GCM tags, and
/// `ERR_CRYPTO_INVALID_STATE` for calls out of order (data after `final`
/// has Node's own message).
fn cipher_error(realm: &mut Realm, failure: Failure, operation: &str) -> Thrown {
    let (message, code) = match failure {
        Failure::WrongFinalBlockLength => (
            alloc::string::String::from(
                "error:1C80006B:Provider routines::wrong final block length",
            ),
            Some("ERR_OSSL_WRONG_FINAL_BLOCK_LENGTH"),
        ),
        Failure::BadDecrypt => (
            "error:1C800064:Provider routines::bad decrypt".into(),
            Some("ERR_OSSL_BAD_DECRYPT"),
        ),
        Failure::Authentication => (
            "Unsupported state or unable to authenticate data".into(),
            None,
        ),
        Failure::State if operation == "update" => {
            ("Trying to add data in unsupported state".into(), None)
        }
        Failure::State if operation == "final" => {
            ("Invalid state".into(), Some("ERR_CRYPTO_INVALID_STATE"))
        }
        Failure::State => (
            alloc::format!("Invalid state for operation {operation}"),
            Some("ERR_CRYPTO_INVALID_STATE"),
        ),
    };
    let error = realm.error(ErrorKind::Error, &message);
    if let Some(code) = code {
        error.define(Key::name("code"), Property::data(Value::str(code)));
    }
    Thrown(Value::Object(error))
}

fn finalized(realm: &Realm) -> Thrown {
    let error = realm.error(ErrorKind::Error, "Digest already called");
    error.define(
        Key::name("code"),
        Property::data(Value::str("ERR_CRYPTO_HASH_FINALIZED")),
    );
    Thrown(Value::Object(error))
}

/// The bytes of an `ArrayBufferView` or `ArrayBuffer`.
fn view(realm: &Realm, value: &Value) -> Result<Vec<u8>, Thrown> {
    if let Some(bytes) = typed_array_builtin::view_bytes(value) {
        return Ok(bytes);
    }
    if let Some(bytes) = typed_array_builtin::buffer_of(value) {
        return Ok(bytes.borrow().clone());
    }
    Err(realm.throw(
        ErrorKind::Type,
        "The crypto binding takes an ArrayBufferView or ArrayBuffer",
    ))
}

fn buffer_value(realm: &mut Realm, bytes: Vec<u8>) -> Completion {
    if realm.buffer_prototype.is_none()
        && let Some(require) = realm.node_library
    {
        let _ = require(realm, "buffer")?;
    }
    let prototype = realm.buffer_prototype.clone();
    Ok(Value::Object(typed_array_builtin::new_bytes(
        realm, bytes, prototype,
    )))
}

fn crypto_error(realm: &Realm, status: i32) -> Thrown {
    if status == crypto_platform::UNSUPPORTED {
        realm.throw(ErrorKind::Error, "Digest method not supported")
    } else {
        realm.throw(
            ErrorKind::Error,
            &alloc::format!("cryptography failure (0x{:08X})", status as u32),
        )
    }
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

#[cfg(test)]
mod tests;
