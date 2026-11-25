'use strict';
// A stand-in for fflate's synchronous API: "gzip" prefixes gzip's magic
// bytes. Its asynchronous API would need workers.
Object.defineProperty(exports, '__esModule', { value: true });
const fail = (code, message) => {
  const error = new Error(message);
  error.code = code;
  throw error;
};
exports.strToU8 = (text) => new Uint8Array(Buffer.from(text));
exports.strFromU8 = (data) => Buffer.from(data).toString();
const gzipSync = (data) => {
  const out = new Uint8Array(data.length + 2);
  out.set([31, 139]);
  out.set(data, 2);
  return out;
};
exports.gzipSync = exports.compressSync = gzipSync;
exports.gunzipSync = (data) => {
  if (data[0] != 31 || data[1] != 139) fail(6, 'invalid gzip data');
  return data.subarray(2);
};
for (const name of ['deflateSync', 'inflateSync', 'zlibSync', 'unzlibSync', 'decompressSync', 'zipSync', 'unzipSync']) {
  exports[name] = (data) => data;
}
function Gzip(opts, ondata) {
  if (typeof opts == 'function') ondata = opts;
  this.ondata = ondata;
  this.started = false;
}
Gzip.prototype.push = function (chunk, final) {
  const out = this.started ? chunk : gzipSync(chunk);
  this.started = true;
  this.ondata(out, !!final);
};
for (const name of ['Deflate', 'Inflate', 'Gzip', 'Compress', 'Gunzip', 'Zlib', 'Unzlib', 'Decompress']) {
  exports[name] = Gzip;
}
exports.ZipDeflate = function ZipDeflate() {};
exports.UnzipInflate = function UnzipInflate() {};
exports.gzip = () => fail(0, 'fflate workers ran');
