'use strict';

const { Buffer } = require('buffer');
const { Transform, finished } = require('stream');
const native = internalBinding('zlib');

// Node's zlib constants.
const constants = Object.freeze(internalBinding('constants').zlib);

// Node's error codes by number and number by name.
const codes = {
  Z_OK: constants.Z_OK,
  Z_STREAM_END: constants.Z_STREAM_END,
  Z_NEED_DICT: constants.Z_NEED_DICT,
  Z_ERRNO: constants.Z_ERRNO,
  Z_STREAM_ERROR: constants.Z_STREAM_ERROR,
  Z_DATA_ERROR: constants.Z_DATA_ERROR,
  Z_MEM_ERROR: constants.Z_MEM_ERROR,
  Z_BUF_ERROR: constants.Z_BUF_ERROR,
  Z_VERSION_ERROR: constants.Z_VERSION_ERROR,
};
for (const name of Object.keys(codes)) codes[codes[name]] = name;
Object.freeze(codes);

const compresses = (mode) => mode === 'gzip' || mode.startsWith('deflate');

const bytes = (data) => {
  if (typeof data === 'string') return Buffer.from(data);
  if (data instanceof ArrayBuffer ||
      (typeof SharedArrayBuffer === 'function' && data instanceof SharedArrayBuffer)) {
    return Buffer.from(data);
  }
  return data;
};

// The whole of `data` compressed or decompressed. Decompression that ends
// with a flush other than Z_FINISH gives what input that stops early holds,
// as zlib does. Errors carry zlib's code and number.
function run(mode, data, options) {
  const partial = !compresses(mode) && options?.finishFlush !== undefined &&
    options.finishFlush !== constants.Z_FINISH;
  let output;
  try {
    output = native.zlibProcess(mode, bytes(data), options?.level, partial);
  } catch (error) {
    if (!(error instanceof TypeError)) {
      error.code = error.message === 'unexpected end of file' ? 'Z_BUF_ERROR' : 'Z_DATA_ERROR';
      error.errno = constants[error.code];
    }
    throw error;
  }
  return Buffer.from(output.buffer, output.byteOffset, output.length);
}

const zlibSync = (mode) => (data, options) => run(mode, data, options);

const zlibCallback = (mode) => (data, options, callback) => {
  if (typeof options === 'function') [callback, options] = [options, undefined];
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  let result;
  let error;
  try {
    result = run(mode, data, options);
  } catch (cause) {
    error = cause;
  }
  process.nextTick(() => callback(error, result));
};

// The transform streams buffer their input and write its whole output when
// it ends.
function Zlib(mode, options) {
  Reflect.apply(Transform, this, [{ autoDestroy: true, ...options }]);
  this._mode = mode;
  this._options = options;
  this._chunks = [];
  this.bytesWritten = 0;
}
Object.setPrototypeOf(Zlib.prototype, Transform.prototype);
Object.setPrototypeOf(Zlib, Transform);

Zlib.prototype._transform = function(chunk, encoding, callback) {
  this._chunks.push(chunk);
  this.bytesWritten += chunk.length;
  callback();
};

Zlib.prototype._flush = function(callback) {
  const input = Buffer.concat(this._chunks);
  this._chunks = [];
  if (input.length === 0 && !compresses(this._mode)) return callback();
  let output;
  try {
    output = run(this._mode, input, this._options);
  } catch (error) {
    return callback(error);
  }
  if (output.length) this.push(output);
  callback();
};

Zlib.prototype.flush = function(kind, callback) {
  if (typeof kind === 'function') callback = kind;
  if (typeof callback === 'function') process.nextTick(callback);
};

Zlib.prototype.params = function(level, strategy, callback) {
  this._options = { ...this._options, level, strategy };
  if (typeof callback === 'function') process.nextTick(callback);
};

Zlib.prototype.reset = function() {
  this._chunks = [];
};

Zlib.prototype.close = function(callback) {
  if (typeof callback === 'function') finished(this, callback);
  this.destroy();
};

Object.defineProperty(Zlib.prototype, '_closed', {
  configurable: true,
  enumerable: true,
  get() {
    return this.destroyed;
  },
});

function subclass(name, mode) {
  const Class = function(options) {
    if (!(this instanceof Class)) return new Class(options);
    Reflect.apply(Zlib, this, [mode, options]);
  };
  Object.defineProperty(Class, 'name', { value: name });
  Object.setPrototypeOf(Class.prototype, Zlib.prototype);
  Object.setPrototypeOf(Class, Zlib);
  return Class;
}

const classes = {
  Deflate: 'deflate',
  Inflate: 'inflate',
  Gzip: 'gzip',
  Gunzip: 'gunzip',
  DeflateRaw: 'deflateRaw',
  InflateRaw: 'inflateRaw',
  Unzip: 'unzip',
};
for (const [name, mode] of Object.entries(classes)) {
  const Class = subclass(name, mode);
  exports[name] = Class;
  exports[`create${name}`] = (options) => new Class(options);
  exports[mode] = zlibCallback(mode);
  exports[`${mode}Sync`] = zlibSync(mode);
}

exports.crc32 = (data, value = 0) => {
  if (typeof value !== 'number') throw new TypeError('The "value" argument must be of type number');
  return native.crc32(bytes(data), value >>> 0);
};

exports.codes = codes;
exports.constants = constants;

// Deprecated: the constants on the module itself, as Node keeps them.
for (const name of Object.keys(constants)) {
  if (name.startsWith('BROTLI') || name.startsWith('ZSTD')) continue;
  Object.defineProperty(exports, name, {
    enumerable: false,
    value: constants[name],
    writable: false,
  });
}
