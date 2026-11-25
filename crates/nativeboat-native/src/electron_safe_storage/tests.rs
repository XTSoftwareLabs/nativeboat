use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

#[test]
fn cipher_info_matches_the_windows_layout() {
    assert_eq!(core::mem::size_of::<AuthenticatedCipherInfo>(), 88);
}

#[test]
fn aes_gcm_matches_the_nist_vectors_and_rejects_altered_text() {
    let key = [0u8; KEY_LENGTH];
    let nonce = [0u8; NONCE_LENGTH];
    // NIST GCM test cases 13 and 14: a zero key and nonce.
    assert_eq!(
        aes_gcm(true, &key, &nonce, &[]).unwrap(),
        hex("530f8afbc74536b9a963b4f1c4cb738b")
    );
    let sealed = aes_gcm(true, &key, &nonce, &[0u8; 16]).unwrap();
    assert_eq!(
        sealed,
        hex("cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919")
    );
    assert_eq!(aes_gcm(false, &key, &nonce, &sealed).unwrap(), [0u8; 16]);
    let mut altered = sealed.clone();
    altered[0] ^= 1;
    assert!(aes_gcm(false, &key, &nonce, &altered).is_none());
    assert!(aes_gcm(false, &key, &nonce, &sealed[..4]).is_none());
}

#[test]
fn ciphertexts_carry_the_version_and_nonce() {
    let key = [7u8; KEY_LENGTH];
    let ciphertext = encrypt(&key, "密码 secret".as_bytes()).unwrap();
    assert!(ciphertext.starts_with(b"v10"));
    assert_eq!(
        ciphertext.len(),
        3 + NONCE_LENGTH + "密码 secret".len() + TAG_LENGTH
    );
    assert_eq!(
        decrypt(&key, &ciphertext).unwrap(),
        "密码 secret".as_bytes()
    );
    // Fresh nonces: the same text encrypts differently.
    assert_ne!(
        encrypt(&key, b"same").unwrap(),
        encrypt(&key, b"same").unwrap()
    );
    assert!(decrypt(&[8u8; KEY_LENGTH], &ciphertext).is_none());
    assert!(decrypt(&key, b"v10short").is_none());
    assert!(decrypt(&key, b"plain").is_none());
}

#[test]
fn keys_are_protected_with_dpapi_behind_its_prefix() {
    let key = [3u8; KEY_LENGTH];
    let protected = dpapi(true, &key).unwrap();
    assert_ne!(protected.as_slice(), key.as_slice());
    assert_eq!(dpapi(false, &protected).unwrap(), key);
    let mut stored = DPAPI_PREFIX.to_vec();
    stored.extend_from_slice(&protected);
    let encoded = crate::encoding::base64_encode(&stored, false);
    assert_eq!(stored_key(&encoded), Some(key));
    // Without the prefix, or not base64, there is no key.
    assert_eq!(
        stored_key(&crate::encoding::base64_encode(&protected, false)),
        None
    );
    assert_eq!(stored_key("not base64!"), None);
}

#[test]
fn safe_storage_keeps_its_key_in_local_state_and_round_trips_text() {
    let directory = std::env::temp_dir().join(alloc::format!(
        "nativeboat-safe-storage-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let local_state = directory.join("Local State");
    // Other preferences in the file are kept.
    std::fs::write(
        &local_state,
        br#"{"browser":{"enabled_labs_experiments":[]}}"#,
    )
    .unwrap();
    let mut realm = Realm::new();
    realm.electron_app_paths =
        alloc::vec![("userData".into(), directory.to_string_lossy().into_owned())];
    let storage = module(&realm);
    let call = |realm: &mut Realm, name: &str, args: &[Value]| {
        let method = ops::get(realm, &Value::Object(storage.clone()), &Key::name(name)).unwrap();
        ops::call(realm, &method, Value::Object(storage.clone()), args)
    };
    assert!(matches!(
        call(&mut realm, "isEncryptionAvailable", &[]).unwrap(),
        Value::Boolean(true)
    ));
    let state = std::fs::read_to_string(&local_state).unwrap();
    assert!(state.contains("\"enabled_labs_experiments\""), "{state}");
    assert!(
        state.contains("\"os_crypt\":{\"audit_enabled\":true,\"encrypted_key\":\""),
        "{state}"
    );
    let encrypted = call(&mut realm, "encryptString", &[Value::str("hunter2")]).unwrap();
    let bytes = crate::typed_array_builtin::view_bytes(&encrypted).unwrap();
    assert!(bytes.starts_with(b"v10"));
    let decrypted = call(&mut realm, "decryptString", &[encrypted]).unwrap();
    assert!(matches!(decrypted, Value::String(text) if text.to_rust_lossy() == "hunter2"));
    // Errors as Electron gives them.
    assert!(call(&mut realm, "encryptString", &[Value::Number(1.0)]).is_err());
    assert!(call(&mut realm, "decryptString", &[Value::str("text")]).is_err());
    let plain = crate::electron_clipboard::buffer_value(&mut realm, b"plain".to_vec()).unwrap();
    assert!(call(&mut realm, "decryptString", &[plain]).is_err());
    let empty = crate::electron_clipboard::buffer_value(&mut realm, Vec::new()).unwrap();
    assert!(matches!(
        call(&mut realm, "decryptString", &[empty]).unwrap(),
        Value::String(text) if text.to_rust_lossy().is_empty()
    ));
    assert!(matches!(
        call(
            &mut realm,
            "setUsePlainTextEncryption",
            &[Value::Boolean(true)]
        )
        .unwrap(),
        Value::Undefined
    ));
    let _ = std::fs::remove_dir_all(&directory);
}
