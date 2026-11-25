//! The Web Crypto API of every realm's `crypto` global, on Windows CNG:
//! `getRandomValues`, `randomUUID` and `subtle` with SHA digests, HMAC
//! signatures, AES-GCM, AES-CBC and AES-CTR encryption, and PBKDF2 and HKDF
//! derivation, for secret keys in raw and JWK formats. Other algorithms
//! reject with `NotSupportedError`. Each `subtle` method finishes before it
//! returns and settles its promise with the result.

use crate::crypto_cipher::{self, Cipher, Mode};
use crate::crypto_platform::{self, AesKey, CryptoHash};
use crate::typed_array_builtin::{self, Element};
use crate::{
    Completion, ErrorKind, Key, Object, Property, Realm, Slot, Symbol, Thrown, Value, ops,
};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub(crate) fn install(realm: &mut Realm) {
    let crypto = realm.object();
    realm.method(&crypto, "randomUUID", 0, |_, _, _| {
        let uuid = crate::platform::random_uuid();
        Ok(Value::str(&uuid))
    });
    realm.method(&crypto, "getRandomValues", 1, |realm, _, args| {
        get_random_values(realm, args.first().cloned().unwrap_or_default())
    });
    let key_class = realm.function("CryptoKey", 0, true, |realm, _, _| {
        Err(realm.throw(ErrorKind::Type, "Illegal constructor"))
    });
    let key_prototype = match ops::get_named(realm, &Value::Object(key_class.clone()), "prototype")
    {
        Ok(Value::Object(prototype)) => prototype,
        _ => realm.object(),
    };
    let subtle = subtle(realm, &key_prototype);
    crypto.define(Key::name("subtle"), Property::hidden(Value::Object(subtle)));
    realm.set_global("crypto", Value::Object(crypto));
    realm.set_global("CryptoKey", Value::Object(key_class));
}

/// `crypto.getRandomValues(view)`: an integer typed array filled in place,
/// up to 65536 bytes.
fn get_random_values(realm: &mut Realm, target: Value) -> Completion {
    let integer = typed_array_builtin::view_of(&target)
        .is_some_and(|view| !matches!(view.element, Element::Float32 | Element::Float64));
    if !integer {
        return Err(dom_exception(
            realm,
            "The data argument must be an integer-type TypedArray",
            "TypeMismatchError",
        ));
    }
    let (bytes, offset, length) =
        typed_array_builtin::view_range(&target).expect("typed arrays have ranges");
    if length > 65536 {
        return Err(dom_exception(
            realm,
            "The requested length exceeds 65,536 bytes",
            "QuotaExceededError",
        ));
    }
    crate::platform::random_fill(&mut bytes.borrow_mut()[offset..offset + length]);
    Ok(target)
}

/// A `DOMException` with a name, or an `Error` carrying the name where
/// the realm has no `DOMException`.
fn dom_exception(realm: &mut Realm, message: &str, name: &str) -> Thrown {
    let global = Value::Object(realm.global.clone());
    if let Ok(constructor) = ops::get(realm, &global, &Key::name("DOMException"))
        && crate::array_builtin::is_callable(&constructor)
        && let Ok(error) = ops::construct(
            realm,
            &constructor,
            &[Value::str(message), Value::str(name)],
        )
    {
        return Thrown(error);
    }
    let error = realm.error(ErrorKind::Error, message);
    error.define(Key::name("name"), Property::hidden(Value::str(name)));
    Thrown(Value::Object(error))
}

fn not_supported(realm: &mut Realm) -> Thrown {
    dom_exception(realm, "Unrecognized algorithm name", "NotSupportedError")
}

fn operation_failed(realm: &mut Realm) -> Thrown {
    dom_exception(
        realm,
        "The operation failed for an operation-specific reason",
        "OperationError",
    )
}

fn invalid_access(realm: &mut Realm) -> Thrown {
    dom_exception(
        realm,
        "The requested operation is not valid for the provided key",
        "InvalidAccessError",
    )
}

/// The algorithms the API provides, by their registered names.
const ALGORITHMS: [&str; 10] = [
    "SHA-1", "SHA-256", "SHA-384", "SHA-512", "HMAC", "AES-GCM", "AES-CBC", "AES-CTR", "PBKDF2",
    "HKDF",
];

/// The CNG digest name of a Web Crypto hash name.
fn digest_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "SHA-1" => "sha1",
        "SHA-256" => "sha256",
        "SHA-384" => "sha384",
        "SHA-512" => "sha512",
        _ => return None,
    })
}

/// The registered name of an algorithm identifier: a name or an object
/// with one, matched without regard to case.
fn algorithm_name(realm: &mut Realm, algorithm: &Value) -> Result<&'static str, Thrown> {
    let name = match algorithm {
        Value::String(name) => name.to_rust_lossy(),
        Value::Object(_) => {
            let name = ops::get_named(realm, algorithm, "name")?;
            if matches!(name, Value::Undefined) {
                return Err(
                    realm.throw(ErrorKind::Type, "Algorithm: name: Missing or not a string")
                );
            }
            ops::to_string(realm, &name)?.to_rust_lossy()
        }
        _ => {
            return Err(realm.throw(ErrorKind::Type, "Algorithm: Not an object or a string"));
        }
    };
    ALGORITHMS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(&name))
        .copied()
        .ok_or_else(|| not_supported(realm))
}

fn member(realm: &mut Realm, object: &Value, name: &'static str) -> Completion {
    match object {
        Value::Object(_) => ops::get_named(realm, object, name),
        _ => Ok(Value::Undefined),
    }
}

/// The hash an algorithm names in its `hash` member.
fn hash_member(realm: &mut Realm, algorithm: &Value) -> Result<&'static str, Thrown> {
    let hash = member(realm, algorithm, "hash")?;
    if matches!(hash, Value::Undefined) {
        return Err(realm.throw(ErrorKind::Type, "Algorithm: hash: Missing"));
    }
    let name = algorithm_name(realm, &hash)?;
    digest_of(name)
        .map(|_| name)
        .ok_or_else(|| not_supported(realm))
}

/// The bytes of a `BufferSource`.
fn buffer_source(realm: &mut Realm, value: &Value, what: &str) -> Result<Vec<u8>, Thrown> {
    if let Some(bytes) = typed_array_builtin::view_bytes(value) {
        return Ok(bytes);
    }
    if let Some(bytes) = typed_array_builtin::buffer_of(value) {
        return Ok(bytes.borrow().clone());
    }
    Err(realm.throw(
        ErrorKind::Type,
        &alloc::format!("{what} is not of type 'ArrayBuffer', 'ArrayBufferView'"),
    ))
}

fn array_buffer(realm: &mut Realm, bytes: Vec<u8>) -> Completion {
    let array = Value::Object(typed_array_builtin::new_bytes(realm, bytes, None));
    ops::get_named(realm, &array, "buffer")
}

/// A numeric member of an algorithm, or `None` when it is absent.
fn number_member(
    realm: &mut Realm,
    object: &Value,
    name: &'static str,
) -> Result<Option<f64>, Thrown> {
    let value = member(realm, object, name)?;
    number(realm, &value)
}

fn number(realm: &mut Realm, value: &Value) -> Result<Option<f64>, Thrown> {
    match value {
        Value::Undefined => Ok(None),
        value => ops::to_number(realm, value).map(Some),
    }
}

/// The hidden parts of a key: its bytes, algorithm name and hash name.
struct Material {
    bytes: Vec<u8>,
    algorithm: &'static str,
    hash: Option<&'static str>,
}

#[derive(Clone)]
struct Keys {
    prototype: Object,
    bytes: Symbol,
    algorithm: Symbol,
    hash: Symbol,
}

impl Keys {
    fn private(object: &Object, symbol: &Symbol) -> Option<Value> {
        match object.private_property(symbol)?.slot {
            Slot::Data { value, .. } => Some(value),
            Slot::Accessor { .. } => None,
        }
    }

    /// The material of a `CryptoKey` this API made.
    fn material(&self, realm: &mut Realm, key: &Value) -> Result<Material, Thrown> {
        let object = key
            .as_object()
            .filter(|object| object.private_property(&self.bytes).is_some())
            .ok_or_else(|| realm.throw(ErrorKind::Type, "parameter is not of type 'CryptoKey'"))?;
        let bytes = Self::private(object, &self.bytes)
            .and_then(|value| typed_array_builtin::view_bytes(&value))
            .unwrap_or_default();
        let name = |value: Option<Value>| match value {
            Some(Value::String(name)) => {
                let name = name.to_rust_lossy();
                ALGORITHMS.iter().find(|known| **known == name).copied()
            }
            _ => None,
        };
        Ok(Material {
            bytes,
            algorithm: name(Self::private(object, &self.algorithm)).unwrap_or("HMAC"),
            hash: name(Self::private(object, &self.hash)),
        })
    }

    /// Whether a key allows a usage.
    fn allows(realm: &mut Realm, key: &Value, usage: &str) -> Result<bool, Thrown> {
        let usages = ops::get_named(realm, key, "usages")?;
        Ok(strings(realm, &usages)?.iter().any(|known| known == usage))
    }

    /// A new `CryptoKey`.
    fn create(
        &self,
        realm: &mut Realm,
        material: Material,
        extractable: bool,
        usages: Vec<String>,
    ) -> Result<Value, Thrown> {
        let key = realm.object();
        key.data_mut().prototype = Some(self.prototype.clone());
        let algorithm = realm.object();
        algorithm.define(
            Key::name("name"),
            Property::data(Value::str(material.algorithm)),
        );
        if let Some(hash) = material.hash {
            let hash_object = realm.object();
            hash_object.define(Key::name("name"), Property::data(Value::str(hash)));
            algorithm.define(
                Key::name("hash"),
                Property::data(Value::Object(hash_object)),
            );
        }
        if matches!(
            material.algorithm,
            "AES-GCM" | "AES-CBC" | "AES-CTR" | "HMAC"
        ) {
            algorithm.define(
                Key::name("length"),
                Property::data(Value::Number((material.bytes.len() * 8) as f64)),
            );
        }
        let fixed = |value| Property {
            slot: Slot::Data {
                value,
                writable: false,
            },
            enumerable: true,
            configurable: false,
        };
        key.define(Key::name("type"), fixed(Value::str("secret")));
        key.define(Key::name("extractable"), fixed(Value::Boolean(extractable)));
        key.define(Key::name("algorithm"), fixed(Value::Object(algorithm)));
        let usages = usages.iter().map(|usage| Value::str(usage)).collect();
        let usages = realm.array(usages);
        key.define(Key::name("usages"), fixed(Value::Object(usages)));
        let bytes = typed_array_builtin::new_bytes(realm, material.bytes, None);
        key.set_private(&self.bytes, Property::hidden(Value::Object(bytes)));
        key.set_private(
            &self.algorithm,
            Property::hidden(Value::str(material.algorithm)),
        );
        if let Some(hash) = material.hash {
            key.set_private(&self.hash, Property::hidden(Value::str(hash)));
        }
        Ok(Value::Object(key))
    }
}

/// The strings of an array-like value.
fn strings(realm: &mut Realm, value: &Value) -> Result<Vec<String>, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "keyUsages is not a sequence"));
    }
    let length = ops::get_named(realm, value, "length")?;
    let length = crate::array_builtin::to_length(realm, &length)?;
    let mut result = Vec::new();
    for index in 0..length {
        let item = ops::get(realm, value, &Key::Index(index as u32))?;
        result.push(ops::to_string(realm, &item)?.to_rust_lossy());
    }
    Ok(result)
}

/// The usages a key of an algorithm may have.
fn allowed_usages(algorithm: &str) -> &'static [&'static str] {
    match algorithm {
        "HMAC" => &["sign", "verify"],
        "PBKDF2" | "HKDF" => &["deriveBits", "deriveKey"],
        _ => &["encrypt", "decrypt", "wrapKey", "unwrapKey"],
    }
}

fn check_usages(realm: &mut Realm, algorithm: &str, usages: &[String]) -> Result<(), Thrown> {
    let allowed = allowed_usages(algorithm);
    if usages
        .iter()
        .any(|usage| !allowed.contains(&usage.as_str()))
    {
        let kind = match algorithm {
            "HMAC" => "an HMAC",
            "PBKDF2" => "a PBKDF2",
            "HKDF" => "a HKDF",
            _ => "an AES",
        };
        let message = alloc::format!("Unsupported key usage for {kind} key");
        return Err(dom_exception(realm, &message, "SyntaxError"));
    }
    if usages.is_empty() {
        return Err(dom_exception(
            realm,
            "Usages cannot be empty when creating a key.",
            "SyntaxError",
        ));
    }
    Ok(())
}

/// The block length of a hash, the default HMAC key length.
fn block_length(hash: &str) -> usize {
    if matches!(hash, "SHA-384" | "SHA-512") {
        128
    } else {
        64
    }
}

fn digest(name: &str, data: &[u8]) -> Result<Vec<u8>, i32> {
    let mut hash = CryptoHash::new(digest_of(name).expect("a digest name"))?;
    hash.update(data)?;
    hash.finish()
}

fn hmac(name: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, i32> {
    let mut hash = CryptoHash::hmac(digest_of(name).expect("a digest name"), key)?;
    hash.update(data)?;
    hash.finish()
}

/// HKDF (RFC 5869) with an HMAC of `hash`.
fn hkdf(hash: &str, ikm: &[u8], salt: &[u8], info: &[u8], length: usize) -> Result<Vec<u8>, i32> {
    let size = CryptoHash::output_length(digest_of(hash).expect("a digest name"))?;
    let zeros = alloc::vec![0; size];
    let prk = hmac(hash, if salt.is_empty() { &zeros } else { salt }, ikm)?;
    let mut output = Vec::with_capacity(length);
    let mut previous = Vec::new();
    let mut counter = 1u8;
    while output.len() < length {
        let mut input = previous.clone();
        input.extend_from_slice(info);
        input.push(counter);
        previous = hmac(hash, &prk, &input)?;
        let take = (length - output.len()).min(previous.len());
        output.extend_from_slice(&previous[..take]);
        counter = counter.wrapping_add(1);
    }
    Ok(output)
}

/// AES-CTR whose counter is the low `bits` bits of the counter block, as
/// Web Crypto defines it: they wrap without carrying into the rest.
fn ctr(key: &[u8], counter: &[u8], bits: u32, data: &[u8]) -> Result<Vec<u8>, i32> {
    let key = AesKey::new(key)?;
    let start = u128::from_be_bytes(counter.try_into().expect("16-byte counter"));
    let mask = if bits == 128 {
        u128::MAX
    } else {
        (1u128 << bits) - 1
    };
    let mut output = data.to_vec();
    for (index, chunk) in output.chunks_mut(16).enumerate() {
        let low = (start & mask).wrapping_add(index as u128) & mask;
        let mut block = ((start & !mask) | low).to_be_bytes();
        key.encrypt(&mut block);
        for (byte, stream) in chunk.iter_mut().zip(block) {
            *byte ^= stream;
        }
    }
    Ok(output)
}

fn subtle(realm: &mut Realm, key_prototype: &Object) -> Object {
    let keys = Keys {
        prototype: key_prototype.clone(),
        bytes: Symbol::private("nativeboat.cryptokey.bytes"),
        algorithm: Symbol::private("nativeboat.cryptokey.algorithm"),
        hash: Symbol::private("nativeboat.cryptokey.hash"),
    };
    let subtle = realm.object();
    promise_method(realm, &subtle, "digest", 2, |realm, args| {
        let name = algorithm_name(realm, &arg(args, 0))?;
        if digest_of(name).is_none() {
            return Err(not_supported(realm));
        }
        let data = buffer_source(realm, &arg(args, 1), "data")?;
        let bytes = digest(name, &data).map_err(|_| operation_failed(realm))?;
        array_buffer(realm, bytes)
    });
    let generate_keys = keys.clone();
    promise_method(realm, &subtle, "generateKey", 3, move |realm, args| {
        let algorithm = arg(args, 0);
        let name = algorithm_name(realm, &algorithm)?;
        let usages = strings(realm, &arg(args, 2))?;
        let (length, hash) = match name {
            "AES-GCM" | "AES-CBC" | "AES-CTR" => {
                let length = number_member(realm, &algorithm, "length")?;
                match length {
                    Some(bits) if bits == 128.0 || bits == 192.0 || bits == 256.0 => {
                        ((bits as usize) / 8, None)
                    }
                    _ => {
                        return Err(dom_exception(
                            realm,
                            "AES key length must be 128, 192, or 256 bits",
                            "OperationError",
                        ));
                    }
                }
            }
            "HMAC" => {
                let hash = hash_member(realm, &algorithm)?;
                let length = number_member(realm, &algorithm, "length")?;
                let bytes = match length {
                    None => block_length(hash),
                    Some(bits) if bits > 0.0 && bits % 8.0 == 0.0 => (bits as usize) / 8,
                    Some(_) => return Err(operation_failed(realm)),
                };
                (bytes, Some(hash))
            }
            _ => return Err(not_supported(realm)),
        };
        check_usages(realm, name, &usages)?;
        let mut bytes = alloc::vec![0; length];
        crate::platform::random_fill(&mut bytes);
        let extractable = ops::to_boolean(&arg(args, 1));
        let material = Material {
            bytes,
            algorithm: name,
            hash,
        };
        generate_keys.create(realm, material, extractable, usages)
    });
    let import_keys = keys.clone();
    promise_method(realm, &subtle, "importKey", 5, move |realm, args| {
        let format = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let algorithm = arg(args, 2);
        let name = algorithm_name(realm, &algorithm)?;
        if digest_of(name).is_some() {
            return Err(not_supported(realm));
        }
        let extractable = ops::to_boolean(&arg(args, 3));
        let usages = strings(realm, &arg(args, 4))?;
        let bytes = match format.as_str() {
            "raw" => buffer_source(realm, &arg(args, 1), "keyData")?,
            "jwk" if !matches!(name, "PBKDF2" | "HKDF") => {
                let jwk = arg(args, 1);
                let kty = member(realm, &jwk, "kty")?;
                let k = member(realm, &jwk, "k")?;
                let (Value::String(kty), Value::String(k)) = (kty, k) else {
                    return Err(dom_exception(realm, "Invalid keyData", "DataError"));
                };
                if kty.to_rust_lossy() != "oct" {
                    return Err(dom_exception(
                        realm,
                        "Invalid JWK \"kty\" Parameter",
                        "DataError",
                    ));
                }
                let units: Vec<u16> = k
                    .units()
                    .iter()
                    .map(|&unit| match unit {
                        0x2d => u16::from(b'+'),
                        0x5f => u16::from(b'/'),
                        unit => unit,
                    })
                    .collect();
                crate::encoding::forgiving_base64(&units)
                    .map_err(|_| dom_exception(realm, "Invalid keyData", "DataError"))?
            }
            _ => {
                let message = alloc::format!("Unable to import {name} key with format {format}");
                return Err(dom_exception(realm, &message, "NotSupportedError"));
            }
        };
        let hash = match name {
            "AES-GCM" | "AES-CBC" | "AES-CTR" => {
                if !matches!(bytes.len(), 16 | 24 | 32) {
                    return Err(dom_exception(realm, "Invalid key length", "DataError"));
                }
                None
            }
            "HMAC" => {
                if bytes.is_empty() {
                    return Err(dom_exception(
                        realm,
                        "Zero-length key is not supported",
                        "DataError",
                    ));
                }
                Some(hash_member(realm, &algorithm)?)
            }
            _ => {
                if extractable {
                    let message = alloc::format!("{name} keys are not extractable");
                    return Err(dom_exception(realm, &message, "SyntaxError"));
                }
                None
            }
        };
        check_usages(realm, name, &usages)?;
        let material = Material {
            bytes,
            algorithm: name,
            hash,
        };
        import_keys.create(realm, material, extractable, usages)
    });
    let export_keys = keys.clone();
    promise_method(realm, &subtle, "exportKey", 2, move |realm, args| {
        let format = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let key = arg(args, 1);
        let material = export_keys.material(realm, &key)?;
        let extractable = ops::get_named(realm, &key, "extractable")?;
        if !ops::to_boolean(&extractable) {
            return Err(dom_exception(
                realm,
                "key is not extractable",
                "InvalidAccessError",
            ));
        }
        match format.as_str() {
            "raw" => array_buffer(realm, material.bytes),
            "jwk" => {
                let jwk = realm.object();
                let k = crate::encoding::base64_encode(&material.bytes, true)
                    .trim_end_matches('=')
                    .to_string();
                let bits = material.bytes.len() * 8;
                let alg = match (material.algorithm, material.hash) {
                    ("HMAC", Some(hash)) => alloc::format!("HS{}", &hash[4..]),
                    (aes, _) => alloc::format!("A{bits}{}", &aes[4..]),
                };
                let usages = ops::get_named(realm, &key, "usages")?;
                let usages = strings(realm, &usages)?
                    .iter()
                    .map(|usage| Value::str(usage))
                    .collect();
                let usages = realm.array(usages);
                for (name, value) in [
                    ("key_ops", Value::Object(usages)),
                    ("ext", Value::Boolean(true)),
                    ("kty", Value::str("oct")),
                    ("k", Value::str(&k)),
                    ("alg", Value::str(&alg)),
                ] {
                    jwk.define(Key::name(name), Property::data(value));
                }
                Ok(Value::Object(jwk))
            }
            _ => {
                let message = alloc::format!("Unable to export a raw {} key", material.algorithm);
                Err(dom_exception(realm, &message, "NotSupportedError"))
            }
        }
    });
    for (method, decrypt) in [("encrypt", false), ("decrypt", true)] {
        let cipher_keys = keys.clone();
        promise_method(realm, &subtle, method, 3, move |realm, args| {
            let algorithm = arg(args, 0);
            let name = algorithm_name(realm, &algorithm)?;
            let key = arg(args, 1);
            let material = cipher_keys.material(realm, &key)?;
            if material.algorithm != name || !Keys::allows(realm, &key, method)? {
                return Err(invalid_access(realm));
            }
            let data = buffer_source(realm, &arg(args, 2), "data")?;
            let output = match name {
                "AES-GCM" => {
                    let iv = member(realm, &algorithm, "iv")?;
                    let iv = buffer_source(realm, &iv, "iv")?;
                    let aad = member(realm, &algorithm, "additionalData")?;
                    let aad = match aad {
                        Value::Undefined => Vec::new(),
                        aad => buffer_source(realm, &aad, "additionalData")?,
                    };
                    let tag_bits = number_member(realm, &algorithm, "tagLength")?.unwrap_or(128.0);
                    if ![32.0, 64.0, 96.0, 104.0, 112.0, 120.0, 128.0].contains(&tag_bits)
                        || iv.is_empty()
                    {
                        return Err(operation_failed(realm));
                    }
                    let tag_length = tag_bits as usize / 8;
                    gcm(&material.bytes, &iv, &aad, &data, tag_length, decrypt)
                        .ok_or_else(|| operation_failed(realm))?
                }
                "AES-CBC" => {
                    let iv = member(realm, &algorithm, "iv")?;
                    let iv = buffer_source(realm, &iv, "iv")?;
                    if iv.len() != 16 {
                        return Err(dom_exception(
                            realm,
                            "algorithm.iv must contain exactly 16 bytes",
                            "OperationError",
                        ));
                    }
                    let mut cipher = Cipher::new(Mode::Cbc, &material.bytes, &iv, decrypt, 16)
                        .map_err(|_| operation_failed(realm))?;
                    let mut output = cipher.update(&data).map_err(|_| operation_failed(realm))?;
                    output.extend(cipher.finish().map_err(|_| operation_failed(realm))?);
                    output
                }
                "AES-CTR" => {
                    let counter = member(realm, &algorithm, "counter")?;
                    let counter = buffer_source(realm, &counter, "counter")?;
                    let bits = number_member(realm, &algorithm, "length")?.unwrap_or(0.0);
                    if counter.len() != 16 || !(1.0..=128.0).contains(&bits) {
                        return Err(operation_failed(realm));
                    }
                    let bits = bits as u32;
                    let blocks = data.len().div_ceil(16) as u128;
                    if bits < 128 && blocks > (1u128 << bits) {
                        return Err(operation_failed(realm));
                    }
                    ctr(&material.bytes, &counter, bits, &data)
                        .map_err(|_| operation_failed(realm))?
                }
                _ => return Err(not_supported(realm)),
            };
            array_buffer(realm, output)
        });
    }
    for (method, verify) in [("sign", false), ("verify", true)] {
        let sign_keys = keys.clone();
        promise_method(realm, &subtle, method, 3, move |realm, args| {
            let name = algorithm_name(realm, &arg(args, 0))?;
            let key = arg(args, 1);
            let material = sign_keys.material(realm, &key)?;
            if name != "HMAC" {
                return Err(not_supported(realm));
            }
            if material.algorithm != "HMAC" || !Keys::allows(realm, &key, method)? {
                let message = alloc::format!("Unable to use this key to {method}");
                return Err(dom_exception(realm, &message, "InvalidAccessError"));
            }
            let hash = material.hash.unwrap_or("SHA-256");
            let data = buffer_source(realm, &arg(args, if verify { 3 } else { 2 }), "data")?;
            let mac = hmac(hash, &material.bytes, &data).map_err(|_| operation_failed(realm))?;
            if verify {
                let signature = buffer_source(realm, &arg(args, 2), "signature")?;
                return Ok(Value::Boolean(crypto_cipher::constant_time_equal(
                    &signature, &mac,
                )));
            }
            array_buffer(realm, mac)
        });
    }
    let derive_keys = keys.clone();
    promise_method(realm, &subtle, "deriveBits", 3, move |realm, args| {
        let length = number(realm, &arg(args, 2))?;
        let Some(bits) = length.filter(|bits| *bits >= 0.0 && bits % 8.0 == 0.0) else {
            return Err(operation_failed(realm));
        };
        let bytes = derive(
            realm,
            &derive_keys,
            &arg(args, 0),
            &arg(args, 1),
            "deriveBits",
            bits as usize / 8,
        )?;
        array_buffer(realm, bytes)
    });
    let derive_key_keys = keys.clone();
    promise_method(realm, &subtle, "deriveKey", 5, move |realm, args| {
        let target = arg(args, 2);
        let name = algorithm_name(realm, &target)?;
        let (length, hash) = match name {
            "AES-GCM" | "AES-CBC" | "AES-CTR" => {
                let bits = number_member(realm, &target, "length")?;
                match bits {
                    Some(bits) if bits == 128.0 || bits == 192.0 || bits == 256.0 => {
                        (bits as usize / 8, None)
                    }
                    _ => return Err(operation_failed(realm)),
                }
            }
            "HMAC" => {
                let hash = hash_member(realm, &target)?;
                let bits = number_member(realm, &target, "length")?;
                (
                    bits.map_or(block_length(hash), |bits| bits as usize / 8),
                    Some(hash),
                )
            }
            _ => return Err(not_supported(realm)),
        };
        let usages = strings(realm, &arg(args, 4))?;
        check_usages(realm, name, &usages)?;
        let bytes = derive(
            realm,
            &derive_key_keys,
            &arg(args, 0),
            &arg(args, 1),
            "deriveKey",
            length,
        )?;
        let material = Material {
            bytes,
            algorithm: name,
            hash,
        };
        let extractable = ops::to_boolean(&arg(args, 3));
        derive_key_keys.create(realm, material, extractable, usages)
    });
    for method in ["wrapKey", "unwrapKey"] {
        promise_method(realm, &subtle, method, 4, |realm, _| {
            Err(not_supported(realm))
        });
    }
    subtle
}

/// PBKDF2 or HKDF bits of a base key.
fn derive(
    realm: &mut Realm,
    keys: &Keys,
    algorithm: &Value,
    key: &Value,
    usage: &str,
    length: usize,
) -> Result<Vec<u8>, Thrown> {
    let name = algorithm_name(realm, algorithm)?;
    let material = keys.material(realm, key)?;
    if material.algorithm != name || !Keys::allows(realm, key, usage)? {
        return Err(invalid_access(realm));
    }
    let hash = hash_member(realm, algorithm)?;
    let salt = member(realm, algorithm, "salt")?;
    let salt = buffer_source(realm, &salt, "salt")?;
    match name {
        "PBKDF2" => {
            let iterations = number_member(realm, algorithm, "iterations")?.unwrap_or(0.0);
            if iterations < 1.0 {
                return Err(operation_failed(realm));
            }
            if length == 0 {
                return Ok(Vec::new());
            }
            crypto_platform::pbkdf2(
                digest_of(hash).expect("a digest name"),
                &material.bytes,
                &salt,
                iterations as u64,
                length,
            )
            .map_err(|_| operation_failed(realm))
        }
        "HKDF" => {
            let info = member(realm, algorithm, "info")?;
            let info = buffer_source(realm, &info, "info")?;
            let size = CryptoHash::output_length(digest_of(hash).expect("a digest name"))
                .map_err(|_| operation_failed(realm))?;
            if length > 255 * size {
                return Err(operation_failed(realm));
            }
            hkdf(hash, &material.bytes, &salt, &info, length).map_err(|_| operation_failed(realm))
        }
        _ => Err(not_supported(realm)),
    }
}

/// AES-GCM as Web Crypto applies it: the tag follows the ciphertext.
fn gcm(
    key: &[u8],
    iv: &[u8],
    aad: &[u8],
    data: &[u8],
    tag_length: usize,
    decrypt: bool,
) -> Option<Vec<u8>> {
    let mut cipher = Cipher::new(Mode::Gcm, key, iv, decrypt, tag_length).ok()?;
    cipher.add_aad(aad).ok()?;
    if decrypt {
        let split = data.len().checked_sub(tag_length)?;
        cipher.set_tag(&data[split..]).ok()?;
        let mut output = cipher.update(&data[..split]).ok()?;
        output.extend(cipher.finish().ok()?);
        Some(output)
    } else {
        let mut output = cipher.update(data).ok()?;
        output.extend(cipher.finish().ok()?);
        output.extend_from_slice(cipher.tag()?);
        Some(output)
    }
}

/// Defines a method whose result or error settles the promise it returns.
fn promise_method(
    realm: &mut Realm,
    target: &Object,
    name: &str,
    length: u32,
    code: impl Fn(&mut Realm, &[Value]) -> Completion + 'static,
) {
    realm.method(target, name, length, move |realm, _, args| {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        match code(realm, args) {
            Ok(value) => ops::call(realm, &resolve, Value::Undefined, &[value])?,
            Err(Thrown(error)) => ops::call(realm, &reject, Value::Undefined, &[error])?,
        };
        Ok(promise)
    });
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

#[cfg(test)]
mod tests;
