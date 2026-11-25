'use strict';
// `fflate/lib/node.cjs` without workers. fflate runs its asynchronous
// operations in workers whose code it builds from its functions' source text,
// which a compiled program has no engine to run. Here they run the matching
// synchronous operations on the main thread on a later turn, with the same
// results, errors and callbacks; the synchronous API is fflate's own.
const fflate = require('./index.cjs');

Object.defineProperty(exports, '__esModule', { value: true });
for (const key of Object.keys(fflate)) exports[key] = fflate[key];

// fflate's errors: an Error with the message of its code, and that code.
const flateError = (code) => {
  const error = new Error(errorMessages[code]);
  error.code = code;
  if (Error.captureStackTrace) Error.captureStackTrace(error, flateError);
  return error;
};
const errorMessages = [
  'unexpected EOF',
  'invalid block type',
  'invalid length/literal',
  'invalid distance',
  'stream finished',
  'no stream handler',
  ,
  'no callback',
  'invalid UTF-8 data',
  'extra field too long',
  'date not in range 1980-2099',
  'filename too long',
  'stream finishing',
  'invalid zip data',
];

// A one-shot operation, as `gzip(data, opts?, callback)`: its result or error
// reaches the callback on a later turn, unless the returned function
// terminated it first.
const oneShot = (operation) => (data, opts, callback) => {
  if (!callback) (callback = opts), (opts = {});
  if (typeof callback != 'function') throw flateError(7);
  let terminated = false;
  setImmediate(() => {
    if (terminated) return;
    let result;
    try {
      result = operation(data, opts);
    } catch (error) {
      callback(error, null);
      return;
    }
    callback(null, result);
  });
  return () => {
    terminated = true;
  };
};

for (const [name, operation] of [
  ['deflate', fflate.deflateSync],
  ['inflate', fflate.inflateSync],
  ['gzip', fflate.gzipSync],
  ['compress', fflate.gzipSync],
  ['gunzip', fflate.gunzipSync],
  ['zlib', fflate.zlibSync],
  ['unzlib', fflate.unzlibSync],
  ['decompress', fflate.decompressSync],
  ['zip', fflate.zipSync],
  ['unzip', fflate.unzipSync],
]) {
  exports[name] = oneShot(operation);
}

// A stream, as `new AsyncGzip(opts?, ondata)`: chunks pushed reach the
// synchronous stream in order on later turns, and its output reaches
// `ondata(error, data, final)`; `ondrain(size)` follows each chunk.
const asyncStream = (Stream) =>
  function (opts, ondata) {
    if (typeof opts == 'function') (ondata = opts), (opts = {});
    this.ondata = ondata;
    this.queuedSize = 0;
    let finished = false;
    let terminated = false;
    const stream = new Stream(opts, (data, final) => this.ondata(null, data, final));
    stream.onmember = (offset) => {
      if (this.onmember) this.onmember(offset);
    };
    this.push = (chunk, final) => {
      if (!this.ondata) throw flateError(5);
      if (finished) {
        this.ondata(flateError(4), null, !!final);
        return;
      }
      finished = !!final;
      this.queuedSize += chunk.length;
      setImmediate(() => {
        if (terminated) return;
        this.queuedSize -= chunk.length;
        try {
          stream.push(chunk, final);
        } catch (error) {
          terminated = true;
          this.ondata(error);
          return;
        }
        if (this.ondrain) this.ondrain(chunk.length);
      });
    };
    if (typeof stream.flush == 'function') {
      this.flush = () => {
        setImmediate(() => {
          if (!terminated) stream.flush();
        });
      };
    }
    this.terminate = () => {
      terminated = true;
    };
  };

for (const [name, stream] of [
  ['AsyncDeflate', fflate.Deflate],
  ['AsyncInflate', fflate.Inflate],
  ['AsyncGzip', fflate.Gzip],
  ['AsyncCompress', fflate.Gzip],
  ['AsyncGunzip', fflate.Gunzip],
  ['AsyncZlib', fflate.Zlib],
  ['AsyncUnzlib', fflate.Unzlib],
  ['AsyncDecompress', fflate.Decompress],
]) {
  exports[name] = asyncStream(stream);
}

// Zip entries compress, and unzipped entries inflate, as they are pushed;
// their handlers take the same arguments.
exports.AsyncZipDeflate = fflate.ZipDeflate;
exports.AsyncUnzipInflate = fflate.UnzipInflate;
