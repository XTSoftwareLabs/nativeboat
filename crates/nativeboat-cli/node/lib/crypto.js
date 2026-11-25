'use strict';

// Node's `crypto` module on the native CNG binding: MD5 and the SHA-1 and
// SHA-2 digests, their HMACs, PBKDF2 and HKDF, AES in ECB, CBC, CTR and GCM
// modes, random bytes and integers, secret keys and X.509 certificates.
// Hashes, HMACs and ciphers are lazy transform streams, as in Node. Public
// key cryptography, Diffie-Hellman and scrypt are not available: their
// functions throw ERR_FEATURE_UNAVAILABLE_ON_PLATFORM when called.

const { Buffer } = require('buffer');
const LazyTransform = require('internal/streams/lazy_transform');
const { StringDecoder } = require('string_decoder');
const {
  codes: {
    ERR_CRYPTO_INVALID_DIGEST,
    ERR_CRYPTO_INVALID_JWK,
    ERR_FEATURE_UNAVAILABLE_ON_PLATFORM,
    ERR_INVALID_ARG_TYPE,
    ERR_INVALID_ARG_VALUE,
    ERR_OUT_OF_RANGE,
  },
} = require('internal/errors');
const {
  validateBoolean,
  validateFunction,
  validateInt32,
  validateInteger,
  validateNumber,
  validateObject,
  validateString,
  validateUint32,
} = require('internal/validators');

const binding = internalBinding('crypto');

const kHandle = Symbol('kHandle');
const kState = Symbol('kState');
const kFinalized = Symbol('kFinalized');
const kKeyMaterial = Symbol('kKeyMaterial');
const kMaxLength = 2 ** 31 - 1;

function codedError(Base, code, message) {
  const error = new Base(message);
  error.code = code;
  return error;
}

const isView = (value) => ArrayBuffer.isView(value);
const isArrayBuffer = (value) =>
  value instanceof ArrayBuffer ||
  (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer);

// The digest a name names, as OpenSSL resolves the names Node accepts, or
// undefined.
const digestAliases = {
  'sha-1': 'sha1',
  'sha-256': 'sha256',
  'sha-384': 'sha384',
  'sha-512': 'sha512',
  'sha2-256': 'sha256',
  'sha2-384': 'sha384',
  'sha2-512': 'sha512',
  'ssl3-md5': 'md5',
  'ssl3-sha1': 'sha1',
};
function digestName(name) {
  let digest = `${name}`.toLowerCase();
  if (digest.startsWith('rsa-')) digest = digest.slice(4);
  if (digest.endsWith('withrsaencryption')) digest = digest.slice(0, -17);
  digest = digestAliases[digest] ?? digest;
  return binding.digests.includes(digest) ? digest : undefined;
}
const digestLengths = { md5: 16, sha1: 20, sha256: 32, sha384: 48, sha512: 64 };

// Bytes of a string in an encoding, or of a buffer source.
function toBuffer(value, encoding, name, types) {
  if (typeof value === 'string') return Buffer.from(value, encoding || 'utf8');
  if (isView(value)) return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  if (isArrayBuffer(value)) return Buffer.from(value);
  throw new ERR_INVALID_ARG_TYPE(name, types, value);
}
const dataTypes = ['string', 'Buffer', 'TypedArray', 'DataView'];
const sourceTypes = ['string', 'ArrayBuffer', 'Buffer', 'TypedArray', 'DataView'];

function keyBytes(key, encoding, name) {
  if (key instanceof KeyObject) {
    if (key.type !== 'secret') {
      throw codedError(TypeError, 'ERR_CRYPTO_INVALID_KEY_OBJECT_TYPE',
                       `Invalid key object type ${key.type}, expected secret.`);
    }
    return key[kKeyMaterial];
  }
  return toBuffer(key, encoding, name, [...sourceTypes, 'KeyObject', 'CryptoKey']);
}

function encode(buffer, encoding) {
  return encoding && encoding !== 'buffer' ? buffer.toString(encoding) : buffer;
}

function getHashes() {
  return ['RSA-MD5', 'RSA-SHA1', 'RSA-SHA256', 'RSA-SHA384', 'RSA-SHA512', 'md5',
          'md5WithRSAEncryption', 'sha1', 'sha1WithRSAEncryption', 'sha256',
          'sha256WithRSAEncryption', 'sha384', 'sha384WithRSAEncryption', 'sha512',
          'sha512WithRSAEncryption', 'ssl3-md5', 'ssl3-sha1'];
}

function getCiphers() {
  return [...binding.ciphers, 'aes128', 'aes192', 'aes256', 'id-aes128-GCM', 'id-aes192-GCM',
          'id-aes256-GCM'].sort();
}

function getCurves() {
  return [];
}

// Hash

function Hash(algorithm, options) {
  if (!new.target) return new Hash(algorithm, options);
  const copy = algorithm instanceof Hash;
  if (!copy) validateString(algorithm, 'algorithm');
  if (options?.outputLength !== undefined) validateUint32(options.outputLength, 'options.outputLength');
  if (copy) {
    if (algorithm[kState][kFinalized]) throw codedError(Error, 'ERR_CRYPTO_HASH_FINALIZED', 'Digest already called');
    this[kHandle] = algorithm[kHandle].copy();
  } else {
    const digest = digestName(algorithm);
    if (digest === undefined) throw new Error('Digest method not supported');
    this[kHandle] = binding.Hash(digest);
  }
  this[kState] = { [kFinalized]: false };
  LazyTransform.call(this, options);
}
Object.setPrototypeOf(Hash.prototype, LazyTransform.prototype);
Object.setPrototypeOf(Hash, LazyTransform);

Hash.prototype.copy = function copy(options) {
  if (this[kState][kFinalized]) throw codedError(Error, 'ERR_CRYPTO_HASH_FINALIZED', 'Digest already called');
  return new Hash(this, options);
};
Hash.prototype._transform = function _transform(chunk, encoding, callback) {
  this[kHandle].update(toBuffer(chunk, encoding, 'data', dataTypes));
  callback();
};
Hash.prototype._flush = function _flush(callback) {
  this.push(this[kHandle].digest());
  callback();
};
Hash.prototype.update = function update(data, encoding) {
  if (this[kState][kFinalized]) throw codedError(Error, 'ERR_CRYPTO_HASH_FINALIZED', 'Digest already called');
  this[kHandle].update(toBuffer(data, encoding, 'data', dataTypes));
  return this;
};
Hash.prototype.digest = function digest(outputEncoding) {
  if (this[kState][kFinalized]) throw codedError(Error, 'ERR_CRYPTO_HASH_FINALIZED', 'Digest already called');
  this[kState][kFinalized] = true;
  return encode(this[kHandle].digest(), outputEncoding);
};

function Hmac(hmac, key, options) {
  if (!new.target) return new Hmac(hmac, key, options);
  validateString(hmac, 'hmac');
  const encoding = options?.encoding;
  const bytes = keyBytes(key, encoding, 'key');
  const digest = digestName(hmac);
  if (digest === undefined) throw new ERR_CRYPTO_INVALID_DIGEST(hmac);
  this[kHandle] = binding.Hash(digest, bytes);
  this[kState] = { [kFinalized]: false };
  LazyTransform.call(this, options);
}
Object.setPrototypeOf(Hmac.prototype, LazyTransform.prototype);
Object.setPrototypeOf(Hmac, LazyTransform);
Hmac.prototype.update = Hash.prototype.update;
Hmac.prototype._transform = Hash.prototype._transform;
Hmac.prototype._flush = Hash.prototype._flush;
// A finished HMAC digests to nothing, as in Node.
Hmac.prototype.digest = function digest(outputEncoding) {
  if (this[kState][kFinalized]) return encode(Buffer.alloc(0), outputEncoding);
  this[kState][kFinalized] = true;
  return encode(this[kHandle].digest(), outputEncoding);
};

function createHash(algorithm, options) {
  return new Hash(algorithm, options);
}

function createHmac(hmac, key, options) {
  return new Hmac(hmac, key, options);
}

// `crypto.hash(algorithm, data[, outputEncoding])`: a one-shot digest.
function hash(algorithm, input, outputEncoding = 'hex') {
  validateString(algorithm, 'algorithm');
  if (typeof input !== 'string' && !isView(input)) {
    throw new ERR_INVALID_ARG_TYPE('input', ['Buffer', 'TypedArray', 'DataView', 'string'], input);
  }
  if (outputEncoding !== 'buffer') validateString(outputEncoding, 'outputEncoding');
  const digest = digestName(algorithm);
  if (digest === undefined) throw new Error('Digest method not supported');
  const handle = binding.Hash(digest);
  handle.update(toBuffer(input, 'utf8', 'input', dataTypes));
  return encode(handle.digest(), outputEncoding);
}

// Ciphers

const cipherInfo = {
  'aes-128-ecb': [418, 16, 'ecb'],
  'aes-128-cbc': [419, 16, 'cbc'],
  'aes-192-ecb': [422, 24, 'ecb'],
  'aes-192-cbc': [423, 24, 'cbc'],
  'aes-256-ecb': [426, 32, 'ecb'],
  'aes-256-cbc': [427, 32, 'cbc'],
  'aes-128-gcm': [895, 16, 'gcm'],
  'aes-192-gcm': [898, 24, 'gcm'],
  'aes-256-gcm': [901, 32, 'gcm'],
  'aes-128-ctr': [904, 16, 'ctr'],
  'aes-192-ctr': [905, 24, 'ctr'],
  'aes-256-ctr': [906, 32, 'ctr'],
};
const cipherAliases = {
  aes128: 'aes-128-cbc',
  aes192: 'aes-192-cbc',
  aes256: 'aes-256-cbc',
  'id-aes128-gcm': 'aes-128-gcm',
  'id-aes192-gcm': 'aes-192-gcm',
  'id-aes256-gcm': 'aes-256-gcm',
};
function cipherName(name) {
  const lower = `${name}`.toLowerCase();
  const canonical = cipherAliases[lower] ?? lower;
  return cipherInfo[canonical] ? canonical : undefined;
}

function getCipherInfo(nameOrNid, options) {
  let name;
  if (typeof nameOrNid === 'number') {
    name = Object.keys(cipherInfo).find((known) => cipherInfo[known][0] === nameOrNid);
  } else {
    validateString(nameOrNid, 'nameOrNid');
    name = cipherName(nameOrNid);
  }
  if (options !== undefined) validateObject(options, 'options');
  if (name === undefined) return undefined;
  const [nid, keyLength, mode] = cipherInfo[name];
  const ivLength = mode === 'gcm' ? 12 : mode === 'ecb' ? undefined : 16;
  if (options?.keyLength !== undefined) {
    validateInt32(options.keyLength, 'options.keyLength');
    if (options.keyLength !== keyLength) return undefined;
  }
  if (options?.ivLength !== undefined) {
    validateInt32(options.ivLength, 'options.ivLength');
    if (mode === 'gcm' ? options.ivLength < 1 : options.ivLength !== ivLength) return undefined;
  }
  const info = {
    mode,
    name: mode === 'gcm' ? `id-aes${keyLength * 8}-gcm` : name,
    nid,
    blockSize: mode === 'cbc' || mode === 'ecb' ? 16 : 1,
  };
  if (ivLength !== undefined) info.ivLength = mode === 'gcm' && options?.ivLength ? options.ivLength : ivLength;
  info.keyLength = keyLength;
  return info;
}

const validTagLengths = [4, 8, 12, 13, 14, 15, 16];
function invalidTag(length) {
  return codedError(TypeError, 'ERR_CRYPTO_INVALID_AUTH_TAG', `Invalid authentication tag length: ${length}`);
}

function createCipherBase(self, cipher, key, iv, options, decipher) {
  validateString(cipher, 'cipher');
  const keyBuffer = keyBytes(key, undefined, 'key');
  const ivBuffer = iv === null ? null : toBuffer(iv, 'utf8', 'iv', [...sourceTypes, 'null']);
  const name = cipherName(cipher);
  if (name === undefined) throw codedError(Error, 'ERR_CRYPTO_UNKNOWN_CIPHER', 'Unknown cipher');
  const [, keyLength, mode] = cipherInfo[name];
  if (keyBuffer.length !== keyLength) throw codedError(RangeError, 'ERR_CRYPTO_INVALID_KEYLEN', 'Invalid key length');
  const ivLength = ivBuffer === null ? 0 : ivBuffer.length;
  const ivValid = mode === 'ecb' ? ivLength === 0 : mode === 'gcm' ? ivLength > 0 : ivLength === 16;
  if (!ivValid) throw codedError(TypeError, 'ERR_CRYPTO_INVALID_IV', 'Invalid initialization vector');
  let tagLength = 16;
  if (options?.authTagLength !== undefined) {
    validateUint32(options.authTagLength, 'options.authTagLength');
    if (mode === 'gcm' && !validTagLengths.includes(options.authTagLength)) {
      throw invalidTag(options.authTagLength);
    }
    tagLength = options.authTagLength;
  }
  self[kHandle] = binding.Cipher(name, keyBuffer, ivBuffer, decipher, tagLength);
  self[kState] = { mode, tagLength: options?.authTagLength, decoder: null };
  LazyTransform.call(self, options);
}

function decoderFor(self, encoding) {
  const state = self[kState];
  if (!state.decoder) {
    state.decoder = new StringDecoder(encoding);
    state.encoding = encoding;
  } else if (state.encoding !== encoding) {
    throw new Error('Cannot change encoding');
  }
  return state.decoder;
}

function Cipheriv(cipher, key, iv, options) {
  if (!new.target) return new Cipheriv(cipher, key, iv, options);
  createCipherBase(this, cipher, key, iv, options, false);
}
function Decipheriv(cipher, key, iv, options) {
  if (!new.target) return new Decipheriv(cipher, key, iv, options);
  createCipherBase(this, cipher, key, iv, options, true);
}
for (const Class of [Cipheriv, Decipheriv]) {
  Object.setPrototypeOf(Class.prototype, LazyTransform.prototype);
  Object.setPrototypeOf(Class, LazyTransform);
  Class.prototype._transform = function _transform(chunk, encoding, callback) {
    this.push(this[kHandle].update(toBuffer(chunk, encoding, 'data', dataTypes)));
    callback();
  };
  Class.prototype._flush = function _flush(callback) {
    try {
      this.push(this.final());
    } catch (error) {
      callback(error);
      return;
    }
    callback();
  };
  Class.prototype.update = function update(data, inputEncoding, outputEncoding) {
    if (typeof data === 'string') {
      data = Buffer.from(data, inputEncoding || 'utf8');
    } else if (!isView(data)) {
      throw new ERR_INVALID_ARG_TYPE('data', dataTypes, data);
    }
    const output = this[kHandle].update(data);
    if (outputEncoding && outputEncoding !== 'buffer') {
      return decoderFor(this, outputEncoding).write(output);
    }
    return output;
  };
  Class.prototype.final = function final(outputEncoding) {
    const output = this[kHandle].final();
    if (outputEncoding && outputEncoding !== 'buffer') {
      return decoderFor(this, outputEncoding).end(output);
    }
    return output;
  };
  Class.prototype.setAutoPadding = function setAutoPadding(autoPadding = true) {
    this[kHandle].setAutoPadding(!!autoPadding);
    return this;
  };
  Class.prototype.setAAD = function setAAD(buffer, options) {
    if (!isView(buffer)) throw new ERR_INVALID_ARG_TYPE('buffer', ['Buffer', 'TypedArray', 'DataView'], buffer);
    if (options !== undefined) validateObject(options, 'options');
    this[kHandle].setAAD(buffer);
    return this;
  };
}
Cipheriv.prototype.getAuthTag = function getAuthTag() {
  const tag = this[kHandle].getAuthTag();
  if (tag === undefined) throw codedError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation getAuthTag');
  return tag;
};
Decipheriv.prototype.setAuthTag = function setAuthTag(tagbuf, encoding) {
  const tag = toBuffer(tagbuf, encoding, 'buffer', ['string', 'Buffer', 'TypedArray', 'DataView']);
  const state = this[kState];
  if (state.mode !== 'gcm') throw codedError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation setAuthTag');
  if (!validTagLengths.includes(tag.length) ||
      (state.tagLength !== undefined && state.tagLength !== tag.length)) {
    throw invalidTag(tag.length);
  }
  this[kHandle].setAuthTag(tag);
  return this;
};

function createCipheriv(cipher, key, iv, options) {
  return new Cipheriv(cipher, key, iv, options);
}

function createDecipheriv(cipher, key, iv, options) {
  return new Decipheriv(cipher, key, iv, options);
}

// Random values

function assertSize(size, elementSize, offset, length) {
  validateNumber(size, 'size');
  size *= elementSize;
  if (Number.isNaN(size) || size > kMaxLength || size < 0) {
    throw new ERR_OUT_OF_RANGE('size', `>= 0 && <= ${kMaxLength}`, size);
  }
  if (size + offset > length) {
    throw new ERR_OUT_OF_RANGE('size + offset', `<= ${length}`, size + offset);
  }
  return size >>> 0;
}

function assertOffset(offset, elementSize, length) {
  validateNumber(offset, 'offset');
  offset *= elementSize;
  const maxLength = Math.min(length, kMaxLength);
  if (Number.isNaN(offset) || offset > maxLength || offset < 0) {
    throw new ERR_OUT_OF_RANGE('offset', `>= 0 && <= ${maxLength}`, offset);
  }
  return offset >>> 0;
}

function randomBytes(size, callback) {
  validateNumber(size, 'size');
  if (Number.isNaN(size) || size > kMaxLength || size < 0) {
    throw new ERR_OUT_OF_RANGE('size', `>= 0 && <= ${kMaxLength}`, size);
  }
  if (callback !== undefined) validateFunction(callback, 'callback');
  const buffer = Buffer.alloc(size >>> 0);
  binding.randomFill(buffer);
  if (callback === undefined) return buffer;
  setImmediate(() => callback(null, buffer));
}

function bytesOf(buf) {
  if (isArrayBuffer(buf)) return new Uint8Array(buf);
  if (isView(buf)) return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  throw new ERR_INVALID_ARG_TYPE('buf', ['ArrayBuffer', 'ArrayBufferView'], buf);
}

function randomFillSync(buf, offset = 0, size) {
  const bytes = bytesOf(buf);
  const elementSize = buf.BYTES_PER_ELEMENT || 1;
  offset = assertOffset(offset, elementSize, bytes.byteLength);
  size = size === undefined ? bytes.byteLength - offset : assertSize(size, elementSize, offset, bytes.byteLength);
  if (size > 0) binding.randomFill(bytes.subarray(offset, offset + size));
  return buf;
}

function randomFill(buf, offset, size, callback) {
  const bytes = bytesOf(buf);
  const elementSize = buf.BYTES_PER_ELEMENT || 1;
  if (typeof offset === 'function') {
    callback = offset;
    offset = 0;
    size = bytes.byteLength;
  } else if (typeof size === 'function') {
    callback = size;
    offset = assertOffset(offset, elementSize, bytes.byteLength);
    size = bytes.byteLength - offset;
  } else {
    validateFunction(callback, 'callback');
    offset = assertOffset(offset, elementSize, bytes.byteLength);
    size = size === undefined ? bytes.byteLength - offset : assertSize(size, elementSize, offset, bytes.byteLength);
  }
  validateFunction(callback, 'callback');
  if (size > 0) binding.randomFill(bytes.subarray(offset, offset + size));
  setImmediate(() => callback(null, buf));
}

const RAND_MAX = 0xFFFFFFFFFFFF;
function randomInt(min, max, callback) {
  const minNotSpecified = typeof max === 'undefined' || typeof max === 'function';
  if (minNotSpecified) {
    callback = max;
    max = min;
    min = 0;
  }
  const isSync = typeof callback === 'undefined';
  if (!isSync) validateFunction(callback, 'callback');
  if (!Number.isSafeInteger(min)) throw new ERR_INVALID_ARG_TYPE('min', 'a safe integer', min);
  if (!Number.isSafeInteger(max)) throw new ERR_INVALID_ARG_TYPE('max', 'a safe integer', max);
  if (max <= min) {
    throw new ERR_OUT_OF_RANGE('max', `greater than the value of "min" (${min})`, max);
  }
  const range = max - min;
  if (!(range <= RAND_MAX)) {
    throw new ERR_OUT_OF_RANGE(`max${minNotSpecified ? '' : ' - min'}`, `<= ${RAND_MAX}`, range);
  }
  // Rejection sampling over 48-bit values, as Node draws them.
  const limit = RAND_MAX - (RAND_MAX % range);
  const bytes = Buffer.alloc(6);
  for (;;) {
    binding.randomFill(bytes);
    const value = bytes.readUIntBE(0, 6);
    if (value < limit) {
      const n = (value % range) + min;
      if (isSync) return n;
      process.nextTick(() => callback(undefined, n));
      return;
    }
  }
}

function randomUUID(options) {
  if (options !== undefined) validateObject(options, 'options');
  if (options?.disableEntropyCache !== undefined) {
    validateBoolean(options.disableEntropyCache, 'options.disableEntropyCache');
  }
  const bytes = Buffer.alloc(16);
  binding.randomFill(bytes);
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = bytes.toString('hex');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function getRandomValues(data) {
  if (globalThis.crypto && typeof globalThis.crypto.getRandomValues === 'function') {
    return globalThis.crypto.getRandomValues(data);
  }
  if (!isView(data) || data instanceof DataView || data instanceof Float32Array ||
      data instanceof Float64Array) {
    throw new DOMException('The data argument must be an integer-type TypedArray', 'TypeMismatchError');
  }
  if (data.byteLength > 65536) {
    throw new DOMException(`The ArrayBufferView's byte length (${data.byteLength}) exceeds the number of bytes of entropy available via this API (65536)`, 'QuotaExceededError');
  }
  binding.randomFill(data);
  return data;
}

function timingSafeEqual(buf1, buf2) {
  const types = ['ArrayBuffer', 'Buffer', 'TypedArray', 'DataView'];
  if (!isView(buf1) && !isArrayBuffer(buf1)) throw new ERR_INVALID_ARG_TYPE('buf1', types, buf1);
  if (!isView(buf2) && !isArrayBuffer(buf2)) throw new ERR_INVALID_ARG_TYPE('buf2', types, buf2);
  if (buf1.byteLength !== buf2.byteLength) {
    throw codedError(RangeError, 'ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH', 'Input buffers must have the same byte length');
  }
  return binding.timingSafeEqual(buf1, buf2);
}

// Key derivation

function pbkdf2Sync(password, salt, iterations, keylen, digest) {
  password = toBuffer(password, 'utf8', 'password', sourceTypes);
  salt = toBuffer(salt, 'utf8', 'salt', sourceTypes);
  validateInt32(iterations, 'iterations', 1);
  validateInt32(keylen, 'keylen', 0);
  validateString(digest, 'digest');
  const name = digestName(digest);
  if (name === undefined) throw new ERR_CRYPTO_INVALID_DIGEST(digest);
  if (keylen === 0) return Buffer.alloc(0);
  return binding.pbkdf2(password, salt, iterations, keylen, name);
}

function pbkdf2(password, salt, iterations, keylen, digest, callback) {
  if (typeof digest === 'function') {
    callback = digest;
    digest = undefined;
  }
  validateFunction(callback, 'callback');
  const key = pbkdf2Sync(password, salt, iterations, keylen, digest);
  setImmediate(() => callback(null, key));
}

function hkdfSync(digest, ikm, salt, info, keylen) {
  validateString(digest, 'digest');
  ikm = keyBytes(ikm, 'utf8', 'ikm');
  salt = toBuffer(salt, 'utf8', 'salt', sourceTypes);
  info = toBuffer(info, 'utf8', 'info', sourceTypes);
  validateInteger(keylen, 'length', 0, kMaxLength);
  if (info.byteLength > 1024) {
    throw new ERR_OUT_OF_RANGE('info', 'must not contain more than 1024 bytes', info.byteLength);
  }
  const name = digestName(digest);
  if (name === undefined) throw new ERR_CRYPTO_INVALID_DIGEST(digest);
  const size = digestLengths[name];
  if (keylen > 255 * size) throw codedError(RangeError, 'ERR_CRYPTO_INVALID_KEYLEN', 'Invalid key length');
  // RFC 5869: extract with the salt (zeros of the digest's length when
  // empty), then expand with the info.
  const prk = binding.Hash(name, salt.length ? salt : Buffer.alloc(size)).update(ikm).digest();
  const output = new Uint8Array(keylen);
  let previous = Buffer.alloc(0);
  for (let offset = 0, counter = 1; offset < keylen; counter++) {
    previous = binding.Hash(name, prk).update(previous).update(info).update(Buffer.from([counter])).digest();
    output.set(previous.subarray(0, Math.min(size, keylen - offset)), offset);
    offset += size;
  }
  return output.buffer;
}

function hkdf(digest, ikm, salt, info, keylen, callback) {
  validateFunction(callback, 'callback');
  const key = hkdfSync(digest, ikm, salt, info, keylen);
  setImmediate(() => callback(null, key));
}

// Keys

class KeyObject {
  constructor(type, handle) {
    if (type !== 'secret' && type !== 'public' && type !== 'private') {
      throw new ERR_INVALID_ARG_VALUE('type', type);
    }
    this[kKeyMaterial] = handle;
    Object.defineProperty(this, 'type', { value: type, enumerable: true, configurable: false });
  }

  static from(key) {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('KeyObject.from');
  }

  get symmetricKeySize() {
    return this.type === 'secret' ? this[kKeyMaterial].length : undefined;
  }

  equals(otherKeyObject) {
    if (!(otherKeyObject instanceof KeyObject)) {
      throw new ERR_INVALID_ARG_TYPE('otherKeyObject', 'KeyObject', otherKeyObject);
    }
    return otherKeyObject.type === this.type &&
      this[kKeyMaterial].length === otherKeyObject[kKeyMaterial].length &&
      binding.timingSafeEqual(this[kKeyMaterial], otherKeyObject[kKeyMaterial]);
  }

  export(options) {
    if (options !== undefined) validateObject(options, 'options');
    if (options?.format === 'jwk') {
      return { kty: 'oct', k: this[kKeyMaterial].toString('base64url') };
    }
    if (options?.format !== undefined && options.format !== 'buffer') {
      throw new ERR_INVALID_ARG_VALUE('options.format', options.format);
    }
    return Buffer.from(this[kKeyMaterial]);
  }
}

function createSecretKey(key, encoding) {
  if (key !== null && typeof key === 'object' && !isView(key) && !isArrayBuffer(key) &&
      !(key instanceof KeyObject) && key.kty !== undefined) {
    if (key.kty !== 'oct' || typeof key.k !== 'string') throw new ERR_CRYPTO_INVALID_JWK();
    return new KeyObject('secret', Buffer.from(key.k, 'base64url'));
  }
  const bytes = toBuffer(key, encoding, 'key', sourceTypes);
  return new KeyObject('secret', Buffer.from(bytes));
}

function generateKeySync(type, options) {
  validateString(type, 'type');
  validateObject(options, 'options');
  const { length } = options;
  if (type === 'hmac') {
    validateInteger(length, 'options.length', 8, kMaxLength);
  } else if (type === 'aes') {
    if (length !== 128 && length !== 192 && length !== 256) {
      throw new ERR_INVALID_ARG_VALUE('options.length', length, 'must be one of: 128, 192, 256');
    }
  } else {
    throw new ERR_INVALID_ARG_VALUE('type', type, "must be a supported key type");
  }
  return createSecretKey(randomBytes(Math.floor(length / 8)));
}

function generateKey(type, options, callback) {
  validateFunction(callback, 'callback');
  const key = generateKeySync(type, options);
  setImmediate(() => callback(null, key));
}

function unavailable(name) {
  return function () {
    throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM(`crypto.${name}`);
  };
}

let X509Certificate;
function x509() {
  X509Certificate ??= require('internal/crypto/x509').X509Certificate;
  return X509Certificate;
}

const constants = {
  OPENSSL_VERSION_NUMBER: 805306624,
  RSA_PKCS1_PADDING: 1,
  RSA_NO_PADDING: 3,
  RSA_PKCS1_OAEP_PADDING: 4,
  RSA_X931_PADDING: 5,
  RSA_PKCS1_PSS_PADDING: 6,
  RSA_PSS_SALTLEN_DIGEST: -1,
  RSA_PSS_SALTLEN_MAX_SIGN: -2,
  RSA_PSS_SALTLEN_AUTO: -2,
  TLS1_VERSION: 769,
  TLS1_1_VERSION: 770,
  TLS1_2_VERSION: 771,
  TLS1_3_VERSION: 772,
  POINT_CONVERSION_COMPRESSED: 2,
  POINT_CONVERSION_UNCOMPRESSED: 4,
  POINT_CONVERSION_HYBRID: 6,
};

module.exports = {
  checkPrime: unavailable('checkPrime'),
  checkPrimeSync: unavailable('checkPrimeSync'),
  createCipheriv,
  createDecipheriv,
  createDiffieHellman: unavailable('createDiffieHellman'),
  createDiffieHellmanGroup: unavailable('createDiffieHellmanGroup'),
  createECDH: unavailable('createECDH'),
  createHash,
  createHmac,
  createPrivateKey: unavailable('createPrivateKey'),
  createPublicKey: unavailable('createPublicKey'),
  createSecretKey,
  createSign: unavailable('createSign'),
  createVerify: unavailable('createVerify'),
  diffieHellman: unavailable('diffieHellman'),
  generatePrime: unavailable('generatePrime'),
  generatePrimeSync: unavailable('generatePrimeSync'),
  getCiphers,
  getCipherInfo,
  getCurves,
  getDiffieHellman: unavailable('getDiffieHellman'),
  getHashes,
  hkdf,
  hkdfSync,
  pbkdf2,
  pbkdf2Sync,
  generateKeyPair: unavailable('generateKeyPair'),
  generateKeyPairSync: unavailable('generateKeyPairSync'),
  generateKey,
  generateKeySync,
  privateDecrypt: unavailable('privateDecrypt'),
  privateEncrypt: unavailable('privateEncrypt'),
  publicDecrypt: unavailable('publicDecrypt'),
  publicEncrypt: unavailable('publicEncrypt'),
  randomBytes,
  randomFill,
  randomFillSync,
  randomInt,
  randomUUID,
  scrypt: unavailable('scrypt'),
  scryptSync: unavailable('scryptSync'),
  sign: unavailable('sign'),
  setEngine: unavailable('setEngine'),
  timingSafeEqual,
  getFips() {
    return 0;
  },
  setFips(value) {
    if (value) throw new ERR_FEATURE_UNAVAILABLE_ON_PLATFORM('crypto.setFips');
  },
  verify: unavailable('verify'),
  hash,
  Certificate: unavailable('Certificate'),
  Cipheriv,
  Decipheriv,
  DiffieHellman: unavailable('DiffieHellman'),
  DiffieHellmanGroup: unavailable('DiffieHellmanGroup'),
  ECDH: unavailable('ECDH'),
  Hash,
  Hmac,
  KeyObject,
  Sign: unavailable('Sign'),
  Verify: unavailable('Verify'),
  secureHeapUsed() {
    return { total: 0, min: 0, used: 0, utilization: 0 };
  },
  constants,
  getRandomValues,
  pseudoRandomBytes: randomBytes,
  prng: randomBytes,
  rng: randomBytes,
};

Object.defineProperties(module.exports, {
  X509Certificate: { enumerable: true, configurable: true, get: x509 },
  webcrypto: { enumerable: true, configurable: true, get: () => globalThis.crypto },
  subtle: { enumerable: true, configurable: true, get: () => globalThis.crypto?.subtle },
});
