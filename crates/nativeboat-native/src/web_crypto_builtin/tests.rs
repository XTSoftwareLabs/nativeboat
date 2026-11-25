use super::*;
use crate::{Key, ops};

#[test]
fn web_crypto_random_uuid_has_rfc_4122_v4_shape_and_version_bits() {
    let mut realm = Realm::new();
    let global = Value::Object(realm.global.clone());
    let crypto = ops::get(&mut realm, &global, &Key::name("crypto")).unwrap();
    let random_uuid = ops::get(&mut realm, &crypto, &Key::name("randomUUID")).unwrap();
    let first = ops::call(&mut realm, &random_uuid, Value::Undefined, &[]).unwrap();
    let Value::String(first) = first else {
        panic!("crypto.randomUUID must return a string");
    };
    let first = first.to_rust_lossy();
    assert_eq!(first.len(), 36);
    assert!(first.chars().enumerate().all(|(index, character)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            character == '-'
        } else if index == 14 {
            character == '4'
        } else if index == 19 {
            matches!(character, '8' | '9' | 'a' | 'b')
        } else {
            character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
        }
    }));
}

// Expected values below are Node 22.15's Web Crypto results for the same
// calls.

fn subtle_call(realm: &mut Realm, method: &str, args: &[Value]) -> Result<Value, Value> {
    let global = Value::Object(realm.global.clone());
    let crypto = ops::get(realm, &global, &Key::name("crypto")).unwrap();
    let subtle = ops::get(realm, &crypto, &Key::name("subtle")).unwrap();
    let function = ops::get(realm, &subtle, &Key::name(method)).unwrap();
    let promise = ops::call(realm, &function, subtle, args).unwrap();
    let Value::Object(promise) = promise else {
        panic!("{method} returns a promise");
    };
    let crate::object::Kind::Promise(state) = &promise.data().kind else {
        panic!("{method} returns a promise");
    };
    match state.borrow().state.clone() {
        crate::promise_builtin::State::Fulfilled(value) => Ok(value),
        crate::promise_builtin::State::Rejected(error) => Err(error),
        _ => panic!("{method} settles before it returns"),
    }
}

fn bytes(realm: &Realm, data: &[u8]) -> Value {
    Value::Object(crate::typed_array_builtin::new_bytes(
        realm,
        data.to_vec(),
        None,
    ))
}

fn hex(buffer: &Value) -> alloc::string::String {
    use core::fmt::Write;
    let bytes = crate::typed_array_builtin::buffer_of(buffer)
        .expect("an ArrayBuffer")
        .borrow()
        .clone();
    let mut text = alloc::string::String::new();
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn object(realm: &mut Realm, entries: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (name, value) in entries {
        object.define(Key::name(name), crate::Property::data(value.clone()));
    }
    Value::Object(object)
}

fn usages(realm: &mut Realm, names: &[&str]) -> Value {
    let names = names.iter().map(|name| Value::str(name)).collect();
    Value::Object(realm.array(names))
}

fn error_name(realm: &mut Realm, error: &Value) -> alloc::string::String {
    let name = ops::get(realm, error, &Key::name("name")).unwrap();
    ops::to_string(realm, &name).unwrap().to_rust_lossy()
}

fn import(
    realm: &mut Realm,
    raw: &[u8],
    algorithm: Value,
    extractable: bool,
    names: &[&str],
) -> Result<Value, Value> {
    let raw = bytes(realm, raw);
    let key_usages = usages(realm, names);
    subtle_call(
        realm,
        "importKey",
        &[
            Value::str("raw"),
            raw,
            algorithm,
            Value::Boolean(extractable),
            key_usages,
        ],
    )
}

#[test]
fn subtle_digests_and_hmac_signatures() {
    let mut realm = Realm::new();
    let data = bytes(&realm, b"abc");
    let digest = subtle_call(&mut realm, "digest", &[Value::str("SHA-256"), data.clone()]).unwrap();
    assert_eq!(
        hex(&digest),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let sha1 = object(&mut realm, &[("name", Value::str("sha-1"))]);
    let digest = subtle_call(&mut realm, "digest", &[sha1, data.clone()]).unwrap();
    assert_eq!(hex(&digest), "a9993e364706816aba3e25717850c26c9cd0d89d");
    let error = subtle_call(&mut realm, "digest", &[Value::str("MD5"), data]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "NotSupportedError");

    let algorithm = object(
        &mut realm,
        &[
            ("name", Value::str("HMAC")),
            ("hash", Value::str("SHA-256")),
        ],
    );
    let key = import(&mut realm, b"key", algorithm, true, &["sign", "verify"]).unwrap();
    let message = bytes(&realm, b"data");
    let mac = subtle_call(
        &mut realm,
        "sign",
        &[Value::str("HMAC"), key.clone(), message.clone()],
    )
    .unwrap();
    assert_eq!(
        hex(&mac),
        "5031fe3d989c6d1537a013fa6e739da23463fdaec3b70137d828e36ace221bd0"
    );
    let mac_view = Value::Object(crate::typed_array_builtin::new_bytes(
        &realm,
        crate::typed_array_builtin::buffer_of(&mac)
            .unwrap()
            .borrow()
            .clone(),
        None,
    ));
    let valid = subtle_call(
        &mut realm,
        "verify",
        &[Value::str("HMAC"), key.clone(), mac_view.clone(), message],
    )
    .unwrap();
    assert!(matches!(valid, Value::Boolean(true)));
    let other = bytes(&realm, b"datb");
    let valid = subtle_call(
        &mut realm,
        "verify",
        &[Value::str("HMAC"), key.clone(), mac_view, other],
    )
    .unwrap();
    assert!(matches!(valid, Value::Boolean(false)));
    let jwk = subtle_call(&mut realm, "exportKey", &[Value::str("jwk"), key.clone()]).unwrap();
    let k = ops::get(&mut realm, &jwk, &Key::name("k")).unwrap();
    let alg = ops::get(&mut realm, &jwk, &Key::name("alg")).unwrap();
    assert!(ops::strict_equals(&k, &Value::str("a2V5")));
    assert!(ops::strict_equals(&alg, &Value::str("HS256")));
    let algorithm = ops::get(&mut realm, &key, &Key::name("algorithm")).unwrap();
    let length = ops::get(&mut realm, &algorithm, &Key::name("length")).unwrap();
    assert!(ops::strict_equals(&length, &Value::Number(24.0)));

    let aes = import(
        &mut realm,
        &[0; 16],
        Value::str("AES-GCM"),
        false,
        &["encrypt"],
    )
    .unwrap();
    let data = bytes(&realm, b"x");
    let error = subtle_call(&mut realm, "sign", &[Value::str("HMAC"), aes, data]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "InvalidAccessError");
}

#[test]
fn subtle_aes_modes_match_node() {
    let mut realm = Realm::new();
    let key = import(
        &mut realm,
        &[9; 32],
        Value::str("AES-GCM"),
        true,
        &["encrypt", "decrypt"],
    )
    .unwrap();
    let iv = bytes(&realm, &[4; 12]);
    let aad = bytes(&realm, b"header");
    let params = object(
        &mut realm,
        &[
            ("name", Value::str("AES-GCM")),
            ("iv", iv),
            ("additionalData", aad),
        ],
    );
    let text = bytes(&realm, b"secret message");
    let sealed = subtle_call(&mut realm, "encrypt", &[params.clone(), key.clone(), text]).unwrap();
    assert_eq!(
        hex(&sealed),
        "94dcdb0b8bc14b42e3371153733d582b68d5ad3c2dd3dc6500148bab6c44"
    );
    let opened = subtle_call(
        &mut realm,
        "decrypt",
        &[params, key.clone(), sealed.clone()],
    )
    .unwrap();
    assert_eq!(hex(&opened), "736563726574206d657373616765");
    let wrong_iv = bytes(&realm, &[0; 12]);
    let params = object(
        &mut realm,
        &[("name", Value::str("AES-GCM")), ("iv", wrong_iv)],
    );
    let error = subtle_call(&mut realm, "decrypt", &[params, key.clone(), sealed]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "OperationError");
    let jwk = subtle_call(&mut realm, "exportKey", &[Value::str("jwk"), key]).unwrap();
    let alg = ops::get(&mut realm, &jwk, &Key::name("alg")).unwrap();
    assert!(ops::strict_equals(&alg, &Value::str("A256GCM")));

    let key = import(
        &mut realm,
        &[1; 16],
        Value::str("AES-CBC"),
        false,
        &["encrypt"],
    )
    .unwrap();
    let iv = bytes(&realm, &[2; 16]);
    let params = object(&mut realm, &[("name", Value::str("AES-CBC")), ("iv", iv)]);
    let text = bytes(&realm, b"0123456789abcdef");
    let sealed = subtle_call(&mut realm, "encrypt", &[params.clone(), key.clone(), text]).unwrap();
    assert_eq!(
        hex(&sealed),
        "2744fc9b2379f312df9c9491361209cf3cf34ae22ace271023f19f7a651debfe"
    );
    let block = bytes(&realm, &[0; 16]);
    let error = subtle_call(&mut realm, "decrypt", &[params, key.clone(), block]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "InvalidAccessError");
    let error = subtle_call(&mut realm, "exportKey", &[Value::str("raw"), key]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "InvalidAccessError");

    let key = import(
        &mut realm,
        &[1; 16],
        Value::str("AES-CTR"),
        false,
        &["encrypt"],
    )
    .unwrap();
    let mut counter = [0xab; 16];
    counter[15] = 0xfe;
    let counter = bytes(&realm, &counter);
    let params = object(
        &mut realm,
        &[
            ("name", Value::str("AES-CTR")),
            ("counter", counter),
            ("length", Value::Number(4.0)),
        ],
    );
    let zeros = bytes(&realm, &[0; 40]);
    let output = subtle_call(&mut realm, "encrypt", &[params, key, zeros]).unwrap();
    assert_eq!(
        hex(&output),
        "f4e80d674fcf452007f07f04ef465224dbe37de72cbf01221ff8dfe3b0557cc24310e60d6390fa7d"
    );
}

#[test]
fn subtle_derives_bits_and_keys_and_generates_keys() {
    let mut realm = Realm::new();
    let base = import(
        &mut realm,
        b"pw",
        Value::str("PBKDF2"),
        false,
        &["deriveBits", "deriveKey"],
    )
    .unwrap();
    let salt = bytes(&realm, b"salt");
    let params = object(
        &mut realm,
        &[
            ("name", Value::str("PBKDF2")),
            ("salt", salt.clone()),
            ("iterations", Value::Number(1000.0)),
            ("hash", Value::str("SHA-256")),
        ],
    );
    let derived = subtle_call(
        &mut realm,
        "deriveBits",
        &[params, base.clone(), Value::Number(160.0)],
    )
    .unwrap();
    assert_eq!(hex(&derived), "0a38253555ce37f5c72a6b703f996814ebf241f2");
    let params = object(
        &mut realm,
        &[
            ("name", Value::str("PBKDF2")),
            ("salt", salt.clone()),
            ("iterations", Value::Number(1.0)),
            ("hash", Value::str("SHA-1")),
        ],
    );
    let target = object(
        &mut realm,
        &[
            ("name", Value::str("AES-GCM")),
            ("length", Value::Number(128.0)),
        ],
    );
    let key_usages = usages(&mut realm, &["encrypt"]);
    let key = subtle_call(
        &mut realm,
        "deriveKey",
        &[params, base, target, Value::Boolean(true), key_usages],
    )
    .unwrap();
    let raw = subtle_call(&mut realm, "exportKey", &[Value::str("raw"), key]).unwrap();
    assert_eq!(hex(&raw), "95c9338ea3a049dc400b5ed6617b5c45");

    let base = import(
        &mut realm,
        b"key",
        Value::str("HKDF"),
        false,
        &["deriveBits"],
    )
    .unwrap();
    let info = bytes(&realm, b"info");
    let params = object(
        &mut realm,
        &[
            ("name", Value::str("HKDF")),
            ("hash", Value::str("SHA-256")),
            ("salt", salt),
            ("info", info),
        ],
    );
    let derived = subtle_call(
        &mut realm,
        "deriveBits",
        &[params, base, Value::Number(336.0)],
    )
    .unwrap();
    assert_eq!(
        hex(&derived),
        "9ca0d662557439e3b83365f2da4626d35da195c6d9d1779f09838cf9e408966ece99106e2585ace6f083"
    );

    let algorithm = object(
        &mut realm,
        &[
            ("name", Value::str("HMAC")),
            ("hash", Value::str("SHA-512")),
        ],
    );
    let key_usages = usages(&mut realm, &["sign"]);
    let key = subtle_call(
        &mut realm,
        "generateKey",
        &[algorithm, Value::Boolean(true), key_usages],
    )
    .unwrap();
    let raw = subtle_call(&mut realm, "exportKey", &[Value::str("raw"), key]).unwrap();
    assert_eq!(hex(&raw).len(), 256);
}

#[test]
fn subtle_and_random_values_reject_what_web_crypto_rejects() {
    let mut realm = Realm::new();
    let error = import(&mut realm, b"k", Value::str("AES-GCM"), false, &["encrypt"]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "DataError");
    let error = import(
        &mut realm,
        &[0; 16],
        Value::str("AES-GCM"),
        false,
        &["sign"],
    )
    .unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "SyntaxError");
    let error = import(
        &mut realm,
        b"pw",
        Value::str("PBKDF2"),
        true,
        &["deriveBits"],
    )
    .unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "SyntaxError");
    let error = subtle_call(&mut realm, "wrapKey", &[]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error), "NotSupportedError");

    let global = Value::Object(realm.global.clone());
    let crypto = ops::get(&mut realm, &global, &Key::name("crypto")).unwrap();
    let random = ops::get(&mut realm, &crypto, &Key::name("getRandomValues")).unwrap();
    let floats = Value::Object(crate::typed_array_builtin::new_typed_array(
        &realm,
        crate::typed_array_builtin::Element::Float64,
        2,
    ));
    let error = ops::call(&mut realm, &random, crypto.clone(), &[floats]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error.0), "TypeMismatchError");
    let large = bytes(&realm, &alloc::vec![0; 65537]);
    let error = ops::call(&mut realm, &random, crypto.clone(), &[large]).unwrap_err();
    assert_eq!(error_name(&mut realm, &error.0), "QuotaExceededError");
    let words = Value::Object(crate::typed_array_builtin::new_typed_array(
        &realm,
        crate::typed_array_builtin::Element::Uint32,
        8,
    ));
    let filled = ops::call(&mut realm, &random, crypto, core::slice::from_ref(&words)).unwrap();
    assert!(ops::strict_equals(&filled, &words));
    assert!(
        crate::typed_array_builtin::view_bytes(&words)
            .unwrap()
            .iter()
            .any(|byte| *byte != 0)
    );
    let key_class = ops::get(&mut realm, &global, &Key::name("CryptoKey")).unwrap();
    assert!(ops::construct(&mut realm, &key_class, &[]).is_err());
}
