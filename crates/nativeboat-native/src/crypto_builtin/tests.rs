// Expected values are Node 22.15's for the same inputs.
use super::*;
use crate::{Key, Realm, ops};
use alloc::vec;

fn method(realm: &mut Realm, object: &Value, name: &str) -> Value {
    ops::get(realm, object, &Key::name(name)).unwrap()
}

fn call(realm: &mut Realm, object: &Value, name: &str, args: &[Value]) -> Completion {
    let function = method(realm, object, name);
    ops::call(realm, &function, object.clone(), args)
}

fn bytes(realm: &Realm, data: &[u8]) -> Value {
    Value::Object(typed_array_builtin::new_bytes(realm, data.to_vec(), None))
}

fn hex(value: &Value) -> alloc::string::String {
    use core::fmt::Write;
    let mut text = alloc::string::String::new();
    for byte in typed_array_builtin::view_bytes(value).unwrap() {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn code(realm: &mut Realm, thrown: Thrown) -> (alloc::string::String, Value) {
    let message = ops::get_named(realm, &thrown.0, "message").unwrap();
    let code = ops::get_named(realm, &thrown.0, "code").unwrap();
    let Value::String(message) = message else {
        panic!("errors have messages");
    };
    (message.to_rust_lossy(), code)
}

#[test]
fn hashes_and_hmacs_update_copy_and_finish_once() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let hash = call(&mut realm, &exports, "Hash", &[Value::str("sha256")]).unwrap();
    let ab = bytes(&realm, b"ab");
    call(&mut realm, &hash, "update", &[ab]).unwrap();
    let copy = call(&mut realm, &hash, "copy", &[]).unwrap();
    let c = bytes(&realm, b"c");
    call(&mut realm, &hash, "update", &[c]).unwrap();
    let digest = call(&mut realm, &hash, "digest", &[]).unwrap();
    assert!(hex(&digest).starts_with("ba7816bf"));
    let copied = call(&mut realm, &copy, "digest", &[]).unwrap();
    assert!(hex(&copied).starts_with("fb8e20fc"));
    for name in ["update", "digest", "copy"] {
        let data = bytes(&realm, b"x");
        let thrown = call(&mut realm, &hash, name, &[data]).unwrap_err();
        let (message, code) = code(&mut realm, thrown);
        assert_eq!(message, "Digest already called");
        assert!(ops::strict_equals(
            &code,
            &Value::str("ERR_CRYPTO_HASH_FINALIZED")
        ));
    }

    let key = bytes(&realm, b"key");
    let hmac = call(&mut realm, &exports, "Hash", &[Value::str("sha1"), key]).unwrap();
    let data = bytes(&realm, b"data");
    call(&mut realm, &hmac, "update", &[data]).unwrap();
    let digest = call(&mut realm, &hmac, "digest", &[]).unwrap();
    assert_eq!(hex(&digest), "104152c5bfdca07bc633eebd46199f0255c9f49d");

    let thrown = call(&mut realm, &exports, "Hash", &[Value::str("sha224")]).unwrap_err();
    assert_eq!(code(&mut realm, thrown).0, "Digest method not supported");
    let text = Value::str("text");
    assert!(call(&mut realm, &hash, "update", &[text]).is_err());
}

#[test]
fn key_derivation_random_fill_and_comparison() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let password = bytes(&realm, b"pw");
    let salt = bytes(&realm, b"salt");
    let key = call(
        &mut realm,
        &exports,
        "pbkdf2",
        &[
            password,
            salt,
            Value::Number(1000.0),
            Value::Number(20.0),
            Value::str("sha256"),
        ],
    )
    .unwrap();
    assert_eq!(hex(&key), "0a38253555ce37f5c72a6b703f996814ebf241f2");

    let target = Value::Object(typed_array_builtin::new_bytes(&realm, vec![0; 64], None));
    let filled = call(
        &mut realm,
        &exports,
        "randomFill",
        core::slice::from_ref(&target),
    )
    .unwrap();
    assert!(ops::strict_equals(&filled, &target));
    assert!(
        typed_array_builtin::view_bytes(&target)
            .unwrap()
            .iter()
            .any(|byte| *byte != 0)
    );
    assert!(call(&mut realm, &exports, "randomFill", &[Value::Null]).is_err());

    let (left, same, other) = (
        bytes(&realm, b"abc"),
        bytes(&realm, b"abc"),
        bytes(&realm, b"abd"),
    );
    let equal = call(
        &mut realm,
        &exports,
        "timingSafeEqual",
        &[left.clone(), same],
    )
    .unwrap();
    assert!(matches!(equal, Value::Boolean(true)));
    let equal = call(&mut realm, &exports, "timingSafeEqual", &[left, other]).unwrap();
    assert!(matches!(equal, Value::Boolean(false)));
}

#[test]
fn ciphers_stream_through_handles_with_node_errors() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let key = bytes(&realm, &[9; 32]);
    let iv = bytes(&realm, &[4; 12]);
    let cipher = call(
        &mut realm,
        &exports,
        "Cipher",
        &[
            Value::str("aes-256-gcm"),
            key,
            iv,
            Value::Boolean(false),
            Value::Number(8.0),
        ],
    )
    .unwrap();
    let tag = call(&mut realm, &cipher, "getAuthTag", &[]).unwrap();
    assert!(matches!(tag, Value::Undefined));
    let text = bytes(&realm, b"abc");
    let output = call(&mut realm, &cipher, "update", &[text]).unwrap();
    assert_eq!(hex(&output), "86dbdb");
    let aad = bytes(&realm, b"late");
    let thrown = call(&mut realm, &cipher, "setAAD", &[aad]).unwrap_err();
    let (message, error_code) = code(&mut realm, thrown);
    assert_eq!(message, "Invalid state for operation setAAD");
    assert!(ops::strict_equals(
        &error_code,
        &Value::str("ERR_CRYPTO_INVALID_STATE")
    ));
    call(&mut realm, &cipher, "final", &[]).unwrap();
    let tag = call(&mut realm, &cipher, "getAuthTag", &[]).unwrap();
    assert_eq!(hex(&tag), "91b47f9595e0c325");
    let more = bytes(&realm, b"x");
    let thrown = call(&mut realm, &cipher, "update", &[more]).unwrap_err();
    let (message, error_code) = code(&mut realm, thrown);
    assert_eq!(message, "Trying to add data in unsupported state");
    assert!(matches!(error_code, Value::Undefined));
    let thrown = call(&mut realm, &cipher, "final", &[]).unwrap_err();
    assert_eq!(code(&mut realm, thrown).0, "Invalid state");

    let key = bytes(&realm, &[1; 16]);
    let iv = bytes(&realm, &[2; 16]);
    let decipher = call(
        &mut realm,
        &exports,
        "Cipher",
        &[Value::str("AES-128-CBC"), key, iv, Value::Boolean(true)],
    )
    .unwrap();
    call(
        &mut realm,
        &decipher,
        "setAutoPadding",
        &[Value::Boolean(true)],
    )
    .unwrap();
    let block = bytes(&realm, &[0; 16]);
    call(&mut realm, &decipher, "update", &[block]).unwrap();
    let thrown = call(&mut realm, &decipher, "final", &[]).unwrap_err();
    let (message, error_code) = code(&mut realm, thrown);
    assert_eq!(message, "error:1C800064:Provider routines::bad decrypt");
    assert!(ops::strict_equals(
        &error_code,
        &Value::str("ERR_OSSL_BAD_DECRYPT")
    ));

    let key = bytes(&realm, &[1; 16]);
    let cipher = call(
        &mut realm,
        &exports,
        "Cipher",
        &[
            Value::str("aes-128-ecb"),
            key,
            Value::Null,
            Value::Boolean(false),
        ],
    )
    .unwrap();
    call(
        &mut realm,
        &cipher,
        "setAutoPadding",
        &[Value::Boolean(false)],
    )
    .unwrap();
    let partial = bytes(&realm, b"abc");
    call(&mut realm, &cipher, "update", &[partial]).unwrap();
    let thrown = call(&mut realm, &cipher, "final", &[]).unwrap_err();
    let (_, error_code) = code(&mut realm, thrown);
    assert!(ops::strict_equals(
        &error_code,
        &Value::str("ERR_OSSL_WRONG_FINAL_BLOCK_LENGTH")
    ));

    let key = bytes(&realm, &[1; 16]);
    let iv = bytes(&realm, &[2; 12]);
    let decipher = call(
        &mut realm,
        &exports,
        "Cipher",
        &[Value::str("aes-128-gcm"), key, iv, Value::Boolean(true)],
    )
    .unwrap();
    let tag = bytes(&realm, &[0; 16]);
    call(&mut realm, &decipher, "setAuthTag", &[tag]).unwrap();
    let data = bytes(&realm, &[0; 10]);
    call(&mut realm, &decipher, "update", &[data]).unwrap();
    let thrown = call(&mut realm, &decipher, "final", &[]).unwrap_err();
    assert_eq!(
        code(&mut realm, thrown).0,
        "Unsupported state or unable to authenticate data"
    );

    let key = bytes(&realm, &[1; 16]);
    let thrown = call(
        &mut realm,
        &exports,
        "Cipher",
        &[Value::str("aes-128-ofb"), key],
    )
    .unwrap_err();
    assert_eq!(code(&mut realm, thrown).0, "Unknown cipher");
}

#[test]
fn lists_and_buffer_prototype_initialization() {
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let digests = method(&mut realm, &exports, "digests");
    let length = ops::get_named(&mut realm, &digests, "length").unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(5.0)));
    let ciphers = method(&mut realm, &exports, "ciphers");
    let length = ops::get_named(&mut realm, &ciphers, "length").unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(12.0)));

    realm.node_library = Some(initialize_buffer);
    let hash = call(&mut realm, &exports, "Hash", &[Value::str("md5")]).unwrap();
    let digest = call(&mut realm, &hash, "digest", &[]).unwrap();
    assert!(ops::strict_equals(
        &Value::Object(
            digest
                .as_object()
                .unwrap()
                .data()
                .prototype
                .as_ref()
                .unwrap()
                .clone()
        ),
        &Value::Object(realm.buffer_prototype.clone().unwrap())
    ));
}

fn initialize_buffer(realm: &mut Realm, name: &str) -> Completion {
    assert_eq!(name, "buffer");
    realm.buffer_prototype = Some(realm.object());
    Ok(Value::Undefined)
}

#[test]
fn native_failures_reach_the_javascript_caller() {
    use crate::crypto_platform::{BCryptFinishHash, BCryptHashData};
    use core::ffi::c_void;
    let mut realm = Realm::new();
    let exports = Value::Object(binding(&mut realm));
    let hash = call(&mut realm, &exports, "Hash", &[Value::str("sha1")]).unwrap();
    {
        let mut session = shimforge::Session::new();
        let fail = shimforge::mock!(
            session,
            BCryptHashData,
            unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32
        );
        fail.expect().once().returning(|_, _, _, _| -1);
        let data = bytes(&realm, b"data");
        let thrown = call(&mut realm, &hash, "update", &[data]).unwrap_err();
        assert_eq!(
            code(&mut realm, thrown).0,
            "cryptography failure (0xFFFFFFFF)"
        );
    }
    let mut session = shimforge::Session::new();
    let fail = shimforge::mock!(
        session,
        BCryptFinishHash,
        unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32
    );
    fail.expect().once().returning(|_, _, _, _| -1);
    assert!(call(&mut realm, &hash, "digest", &[]).is_err());
}
