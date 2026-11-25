'use strict';

// HTTP/1.1 servers and clients over `net` sockets, with Node's classes,
// events and header rules. `https` requests use the native client; TLS
// sockets are not available.

const EventEmitter = require('events');
const net = require('net');
const { Buffer } = require('buffer');
const { Readable, Stream } = require('stream');
const { URL, urlToHttpOptions, isURL } = require('internal/url');

const METHODS = [
  'ACL', 'BIND', 'CHECKOUT', 'CONNECT', 'COPY', 'DELETE', 'GET', 'HEAD', 'LINK', 'LOCK',
  'M-SEARCH', 'MERGE', 'MKACTIVITY', 'MKCALENDAR', 'MKCOL', 'MOVE', 'NOTIFY', 'OPTIONS',
  'PATCH', 'POST', 'PROPFIND', 'PROPPATCH', 'PURGE', 'PUT', 'QUERY', 'REBIND', 'REPORT',
  'SEARCH', 'SOURCE', 'SUBSCRIBE', 'TRACE', 'UNBIND', 'UNLINK', 'UNLOCK', 'UNSUBSCRIBE',
];

const STATUS_CODES = {
  100: 'Continue', 101: 'Switching Protocols', 102: 'Processing', 103: 'Early Hints',
  200: 'OK', 201: 'Created', 202: 'Accepted', 203: 'Non-Authoritative Information',
  204: 'No Content', 205: 'Reset Content', 206: 'Partial Content', 207: 'Multi-Status',
  208: 'Already Reported', 226: 'IM Used', 300: 'Multiple Choices', 301: 'Moved Permanently',
  302: 'Found', 303: 'See Other', 304: 'Not Modified', 305: 'Use Proxy',
  307: 'Temporary Redirect', 308: 'Permanent Redirect', 400: 'Bad Request',
  401: 'Unauthorized', 402: 'Payment Required', 403: 'Forbidden', 404: 'Not Found',
  405: 'Method Not Allowed', 406: 'Not Acceptable', 407: 'Proxy Authentication Required',
  408: 'Request Timeout', 409: 'Conflict', 410: 'Gone', 411: 'Length Required',
  412: 'Precondition Failed', 413: 'Payload Too Large', 414: 'URI Too Long',
  415: 'Unsupported Media Type', 416: 'Range Not Satisfiable', 417: 'Expectation Failed',
  418: 'I\'m a Teapot', 421: 'Misdirected Request', 422: 'Unprocessable Entity',
  423: 'Locked', 424: 'Failed Dependency', 425: 'Too Early', 426: 'Upgrade Required',
  428: 'Precondition Required', 429: 'Too Many Requests',
  431: 'Request Header Fields Too Large', 451: 'Unavailable For Legal Reasons',
  500: 'Internal Server Error', 501: 'Not Implemented', 502: 'Bad Gateway',
  503: 'Service Unavailable', 504: 'Gateway Timeout', 505: 'HTTP Version Not Supported',
  506: 'Variant Also Negotiates', 507: 'Insufficient Storage', 508: 'Loop Detected',
  509: 'Bandwidth Limit Exceeded', 510: 'Not Extended',
  511: 'Network Authentication Required',
};

const maxHeaderSize = 16 * 1024;
const kOutHeaders = Symbol('kOutHeaders');
const kNeedDrain = Symbol('kNeedDrain');
const kCorked = Symbol('corked');

function nodeError(Base, code, message) {
  const error = new Base(message);
  error.code = code;
  return error;
}

function connResetException(message) {
  const error = new Error(message);
  error.code = 'ECONNRESET';
  return error;
}

const tokenRegExp = /^[\^_`a-zA-Z\-0-9!#$%&'*+.|~]+$/;
const headerCharRegex = /[^\t\x20-\x7e\x80-\xff]/;

const checkIsHttpToken = (value) => tokenRegExp.test(value);
const checkInvalidHeaderChar = (value) => headerCharRegex.test(value);

function validateHeaderName(name, label = 'Header name') {
  if (typeof name !== 'string' || !name || !checkIsHttpToken(name)) {
    throw nodeError(TypeError, 'ERR_INVALID_HTTP_TOKEN', `${label} must be a valid HTTP token ["${name}"]`);
  }
}

function validateHeaderValue(name, value) {
  if (value === undefined) {
    throw nodeError(TypeError, 'ERR_HTTP_INVALID_HEADER_VALUE', `Invalid value "${value}" for header "${name}"`);
  }
  if (checkInvalidHeaderChar(String(value))) {
    throw nodeError(TypeError, 'ERR_INVALID_CHAR', `Invalid character in header content ["${name}"]`);
  }
}

// Incremental HTTP/1.x message parsing. Callbacks: onHeaders(info) returns
// true to skip a body, onBody(chunk), onComplete(trailers) and onError(error);
// `upgrade` stops parsing and leaves the rest of the input to the caller.
class Parser {
  constructor(kind, maxSize = maxHeaderSize) {
    this.kind = kind;
    this.maxSize = maxSize;
    this.buffer = null;
    this.state = 'head';
    this.remaining = 0;
    this.paused = false;
    this.stopped = false;
    this.rest = null;
  }

  // Bytes before the end of a header block, or -1.
  static headEnd(buffer, from) {
    for (let index = from; index < buffer.length; index++) {
      if (buffer[index] !== 10) continue;
      if (buffer[index + 1] === 10) return [index, 2];
      if (buffer[index + 1] === 13 && buffer[index + 2] === 10) return [index, 3];
    }
    return null;
  }

  execute(chunk) {
    this.buffer = this.buffer && this.buffer.length ? Buffer.concat([this.buffer, chunk]) : chunk;
    try {
      while (!this.stopped && this.buffer.length) {
        if (!this.step()) break;
      }
    } catch (error) {
      this.stopped = true;
      this.onError(error);
    }
  }

  // Input ends: a body that runs to the end of the stream is complete.
  finish() {
    if (this.stopped) return;
    if (this.state === 'eof') {
      this.state = 'head';
      this.onComplete([]);
    } else if (this.state !== 'head' || (this.buffer && this.buffer.length)) {
      this.stopped = true;
      this.onError(this.error('HPE_INVALID_EOF_STATE', 'Parse Error: Invalid EOF state'));
    }
  }

  error(code, message, reason) {
    const error = new Error(message);
    error.code = code;
    if (reason) error.reason = reason;
    if (this.buffer) error.rawPacket = this.buffer;
    return error;
  }

  take(count) {
    const taken = this.buffer.subarray(0, count);
    this.buffer = this.buffer.subarray(count);
    return taken;
  }

  step() {
    switch (this.state) {
      case 'head': return this.head();
      case 'length': {
        const body = this.take(Math.min(this.remaining, this.buffer.length));
        this.remaining -= body.length;
        if (body.length) this.onBody(body);
        if (this.remaining === 0) this.complete([]);
        return true;
      }
      case 'eof':
        this.onBody(this.take(this.buffer.length));
        return true;
      case 'chunk-size': {
        const end = this.buffer.indexOf(10);
        if (end < 0) {
          if (this.buffer.length > 256) throw this.error('HPE_INVALID_CHUNK_SIZE', 'Parse Error: Invalid character in chunk size');
          return false;
        }
        const line = this.take(end + 1).toString('latin1').replace(/\r?\n$/, '');
        const size = line.split(';')[0].trim();
        if (!/^[0-9a-fA-F]+$/.test(size)) {
          throw this.error('HPE_INVALID_CHUNK_SIZE', 'Parse Error: Invalid character in chunk size');
        }
        this.remaining = parseInt(size, 16);
        this.state = this.remaining === 0 ? 'trailers' : 'chunk-data';
        return true;
      }
      case 'chunk-data': {
        const body = this.take(Math.min(this.remaining, this.buffer.length));
        this.remaining -= body.length;
        if (body.length) this.onBody(body);
        if (this.remaining === 0) this.state = 'chunk-end';
        return true;
      }
      case 'chunk-end': {
        if (this.buffer[0] === 13) {
          if (this.buffer.length < 2) return false;
          this.take(2);
        } else if (this.buffer[0] === 10) {
          this.take(1);
        } else {
          throw this.error('HPE_STRICT', 'Parse Error: Expected LF after chunk data');
        }
        this.state = 'chunk-size';
        return true;
      }
      case 'trailers': {
        if (this.buffer[0] === 10 || (this.buffer[0] === 13 && this.buffer[1] === 10)) {
          this.take(this.buffer[0] === 10 ? 1 : 2);
          this.complete([]);
          return true;
        }
        const found = Parser.headEnd(this.buffer, 0);
        if (!found) {
          if (this.buffer.length > this.maxSize) throw this.error('HPE_HEADER_OVERFLOW', 'Parse Error: Header overflow');
          return false;
        }
        const text = this.take(found[0] + found[1]).toString('latin1');
        this.complete(this.headerLines(text.split(/\r?\n/).filter(Boolean)));
        return true;
      }
    }
    return false;
  }

  complete(trailers) {
    this.state = 'head';
    this.onComplete(trailers);
  }

  headerLines(lines) {
    const raw = [];
    for (const line of lines) {
      if ((line[0] === ' ' || line[0] === '\t') && raw.length) {
        raw[raw.length - 1] += ` ${line.trim()}`;
        continue;
      }
      const colon = line.indexOf(':');
      const name = colon > 0 ? line.slice(0, colon) : '';
      if (!checkIsHttpToken(name)) {
        throw this.error('HPE_INVALID_HEADER_TOKEN', 'Parse Error: Invalid header value char');
      }
      raw.push(name, line.slice(colon + 1).trim());
    }
    return raw;
  }

  head() {
    // Line breaks between messages are ignored.
    while (this.buffer.length && (this.buffer[0] === 13 || this.buffer[0] === 10)) this.take(1);
    if (!this.buffer.length) return false;
    const found = Parser.headEnd(this.buffer, 0);
    if (!found) {
      if (this.buffer.length > this.maxSize) throw this.error('HPE_HEADER_OVERFLOW', 'Parse Error: Header overflow');
      return false;
    }
    if (found[0] > this.maxSize) throw this.error('HPE_HEADER_OVERFLOW', 'Parse Error: Header overflow');
    const text = this.take(found[0] + found[1]).toString('latin1');
    const lines = text.split(/\r?\n/);
    const start = lines.shift();
    const info = { versionMajor: 1, versionMinor: 1, upgrade: false };
    let match;
    if (this.kind === 'request') {
      match = /^([A-Z-]+) (\S+) HTTP\/(\d)\.(\d)$/.exec(start);
      if (!match) throw this.error('HPE_INVALID_METHOD', 'Parse Error: Invalid method encountered', 'Invalid method encountered');
      info.method = match[1];
      info.url = match[2];
      info.versionMajor = +match[3];
      info.versionMinor = +match[4];
    } else {
      match = /^HTTP\/(\d)\.(\d) (\d{3})(?: (.*))?$/.exec(start);
      if (!match) throw this.error('HPE_INVALID_CONSTANT', 'Parse Error: Expected HTTP/');
      info.versionMajor = +match[1];
      info.versionMinor = +match[2];
      info.statusCode = +match[3];
      info.statusMessage = match[4] || '';
    }
    info.rawHeaders = this.headerLines(lines.filter(Boolean));
    let length;
    let chunked = false;
    let connection = '';
    let upgradeHeader = false;
    for (let index = 0; index < info.rawHeaders.length; index += 2) {
      const name = info.rawHeaders[index].toLowerCase();
      const value = info.rawHeaders[index + 1];
      if (name === 'content-length') {
        if (!/^\d+$/.test(value) || (length !== undefined && length !== +value)) {
          throw this.error('HPE_UNEXPECTED_CONTENT_LENGTH', 'Parse Error: Invalid character in Content-Length');
        }
        length = +value;
      } else if (name === 'transfer-encoding') {
        chunked = /(?:^|,)\s*chunked\s*$/i.test(value);
      } else if (name === 'connection') {
        connection += `${value.toLowerCase()},`;
      } else if (name === 'upgrade') {
        upgradeHeader = true;
      }
    }
    if (chunked && length !== undefined) {
      throw this.error('HPE_UNEXPECTED_CONTENT_LENGTH', 'Parse Error: Content-Length can\'t be present with Transfer-Encoding');
    }
    const http11 = info.versionMajor === 1 && info.versionMinor >= 1;
    info.shouldKeepAlive = http11 ? !/(?:^|[\s,])close(?:,|$)/.test(connection)
      : /(?:^|[\s,])keep-alive(?:,|$)/.test(connection);
    info.upgrade = this.kind === 'request'
      ? info.method === 'CONNECT' || (upgradeHeader && /(?:^|[\s,])upgrade(?:,|$)/.test(connection))
      : info.statusCode === 101;
    const skipBody = this.onHeaders(info);
    if (info.upgrade && (this.kind === 'request' ? skipBody !== false : true)) {
      this.stopped = true;
      this.rest = this.buffer;
      this.buffer = Buffer.alloc(0);
      return false;
    }
    if (this.kind === 'response' && info.statusCode < 200) return true;
    if (skipBody === true || (this.kind === 'response' && (info.statusCode === 204 || info.statusCode === 304))) {
      this.complete([]);
    } else if (chunked) {
      this.state = 'chunk-size';
    } else if (length !== undefined) {
      this.remaining = length;
      this.state = 'length';
      if (length === 0) this.complete([]);
    } else if (this.kind === 'response') {
      this.state = 'eof';
    } else {
      this.complete([]);
    }
    return true;
  }
}

// Node's rules for repeated headers: some keep their first value, cookies
// join with '; ', set-cookie collects an array and others join with ', '.
const firstValueOnly = new Set([
  'content-type', 'content-length', 'user-agent', 'referer', 'host', 'authorization',
  'proxy-authorization', 'if-modified-since', 'if-unmodified-since', 'from', 'location',
  'max-forwards', 'retry-after', 'etag', 'last-modified', 'server', 'age', 'expires',
]);

function addHeaderLine(headers, field, value, joinDuplicates) {
  const name = field.toLowerCase();
  if (!(name in headers)) {
    headers[name] = name === 'set-cookie' ? [value] : value;
  } else if (name === 'set-cookie') {
    headers[name].push(value);
  } else if (joinDuplicates) {
    headers[name] += `, ${value}`;
  } else if (name === 'cookie') {
    headers[name] += `; ${value}`;
  } else if (!firstValueOnly.has(name)) {
    headers[name] += `, ${value}`;
  }
}

class IncomingMessage extends Readable {
  constructor(socket) {
    super(socket?.readableHighWaterMark ? { highWaterMark: socket.readableHighWaterMark } : {});
    this._readableState.readingMore = true;
    this.socket = socket;
    this.httpVersionMajor = null;
    this.httpVersionMinor = null;
    this.httpVersion = null;
    this.complete = false;
    this.rawHeaders = [];
    this.rawTrailers = [];
    this.joinDuplicateHeaders = false;
    this.aborted = false;
    this.upgrade = null;
    this.url = '';
    this.method = null;
    this.statusCode = null;
    this.statusMessage = null;
    this.client = socket;
    this._consuming = false;
    this._dumped = false;
    this._headers = null;
    this._trailers = null;
    this._pausedSocket = false;
  }

  get connection() { return this.socket; }
  set connection(value) { this.socket = value; }

  get headers() {
    if (!this._headers) {
      this._headers = {};
      for (let index = 0; index < this.rawHeaders.length; index += 2) {
        addHeaderLine(this._headers, this.rawHeaders[index], this.rawHeaders[index + 1], this.joinDuplicateHeaders);
      }
    }
    return this._headers;
  }

  set headers(value) { this._headers = value; }

  get headersDistinct() {
    const distinct = {};
    for (let index = 0; index < this.rawHeaders.length; index += 2) {
      const name = this.rawHeaders[index].toLowerCase();
      (distinct[name] ||= []).push(this.rawHeaders[index + 1]);
    }
    return distinct;
  }

  get trailers() {
    if (!this._trailers) {
      this._trailers = {};
      for (let index = 0; index < this.rawTrailers.length; index += 2) {
        addHeaderLine(this._trailers, this.rawTrailers[index], this.rawTrailers[index + 1], this.joinDuplicateHeaders);
      }
    }
    return this._trailers;
  }

  set trailers(value) { this._trailers = value; }

  setTimeout(timeout, callback) {
    if (callback) this.on('timeout', callback);
    this.socket?.setTimeout(timeout);
    return this;
  }

  _read() {
    if (!this._consuming) {
      this._readableState.readingMore = false;
      this._consuming = true;
    }
    if (this._pausedSocket && this.socket) {
      this._pausedSocket = false;
      this.socket.resume();
    }
  }

  // A body chunk from the parser; the socket pauses while the reader lags.
  _pushBody(chunk) {
    if (!this.push(chunk) && this.socket && !this._pausedSocket) {
      this._pausedSocket = true;
      this.socket.pause();
    }
  }

  _destroy(error, callback) {
    if (!this.readableEnded || !this.complete) {
      this.aborted = true;
      this.emit('aborted');
    }
    if (this.socket && !this.socket.destroyed && this.aborted) {
      this.socket.destroy(error);
    }
    // As in Node, the error is emitted only to listeners for it.
    callback(this.listenerCount('error') === 0 ? undefined : error);
  }

  _dump() {
    if (!this._dumped) {
      this._dumped = true;
      this.removeAllListeners('data');
      this.resume();
    }
  }
}

function headerEntries(headers) {
  if (Array.isArray(headers)) {
    if (headers.length && Array.isArray(headers[0])) return headers;
    const pairs = [];
    for (let index = 0; index < headers.length; index += 2) pairs.push([headers[index], headers[index + 1]]);
    return pairs;
  }
  return Object.keys(headers || {}).map((name) => [name, headers[name]]);
}

class OutgoingMessage extends Stream {
  constructor(options) {
    super();
    this.outputData = [];
    this.outputSize = 0;
    this.writable = true;
    this.destroyed = false;
    this._last = false;
    this.chunkedEncoding = false;
    this.shouldKeepAlive = true;
    this.maxRequestsOnConnectionReached = false;
    this._defaultKeepAlive = true;
    this.useChunkedEncodingByDefault = true;
    this.sendDate = false;
    this._removedConnection = false;
    this._removedContLen = false;
    this._removedTE = false;
    this.strictContentLength = false;
    this._contentLength = null;
    this._hasBody = true;
    this._trailer = '';
    this.finished = false;
    this._headerSent = false;
    this._header = null;
    this.socket = null;
    this._closed = false;
    this[kCorked] = 0;
    this[kOutHeaders] = null;
    this[kNeedDrain] = false;
    this._keepAliveTimeout = 0;
    this._highWaterMark = options?.highWaterMark ?? 16 * 1024;
  }

  get connection() { return this.socket; }
  set connection(value) { this.socket = value; }
  get headersSent() { return Boolean(this._header); }
  get writableEnded() { return this.finished; }
  get writableObjectMode() { return false; }
  get writableCorked() { return this[kCorked]; }
  get writableHighWaterMark() { return this.socket ? this.socket.writableHighWaterMark : this._highWaterMark; }
  get writableLength() { return this.outputSize + (this.socket ? this.socket.writableLength : 0); }
  get writableNeedDrain() { return this[kNeedDrain]; }

  get writableFinished() {
    return this.finished && this.outputSize === 0 && (!this.socket || this.socket.writableLength === 0);
  }

  cork() {
    this[kCorked]++;
    this.socket?.cork();
  }

  uncork() {
    if (this[kCorked]) this[kCorked]--;
    this.socket?.uncork();
  }

  setTimeout(timeout, callback) {
    if (callback) this.on('timeout', callback);
    if (this.socket) this.socket.setTimeout(timeout);
    else this.once('socket', (socket) => socket.setTimeout(timeout));
    return this;
  }

  destroy(error) {
    if (this.destroyed) return this;
    this.destroyed = true;
    if (this.socket) this.socket.destroy(error);
    else this.once('socket', (socket) => socket.destroy(error));
    return this;
  }

  setHeader(name, value) {
    if (this._header) throw nodeError(Error, 'ERR_HTTP_HEADERS_SENT', 'Cannot set headers after they are sent to the client');
    validateHeaderName(name);
    validateHeaderValue(name, value);
    (this[kOutHeaders] ||= Object.create(null))[name.toLowerCase()] = [name, value];
    return this;
  }

  setHeaders(headers) {
    for (const [name, value] of headers instanceof Map || (headers && typeof headers.entries === 'function' && !Array.isArray(headers))
      ? headers.entries() : headerEntries(headers)) {
      this.setHeader(name, value);
    }
    return this;
  }

  appendHeader(name, value) {
    if (this._header) throw nodeError(Error, 'ERR_HTTP_HEADERS_SENT', 'Cannot append headers after they are sent to the client');
    validateHeaderName(name);
    validateHeaderValue(name, value);
    const key = name.toLowerCase();
    const headers = (this[kOutHeaders] ||= Object.create(null));
    if (headers[key]) {
      const existing = headers[key][1];
      headers[key][1] = [].concat(existing, value);
    } else {
      headers[key] = [name, value];
    }
    return this;
  }

  getHeader(name) {
    return this[kOutHeaders]?.[String(name).toLowerCase()]?.[1];
  }

  getHeaders() {
    const headers = Object.create(null);
    for (const key of Object.keys(this[kOutHeaders] || {})) headers[key] = this[kOutHeaders][key][1];
    return headers;
  }

  getHeaderNames() {
    return Object.keys(this[kOutHeaders] || {});
  }

  getRawHeaderNames() {
    return Object.values(this[kOutHeaders] || {}).map(([name]) => name);
  }

  hasHeader(name) {
    return Boolean(this[kOutHeaders]?.[String(name).toLowerCase()]);
  }

  removeHeader(name) {
    if (this._header) throw nodeError(Error, 'ERR_HTTP_HEADERS_SENT', 'Cannot remove headers after they are sent to the client');
    const key = String(name).toLowerCase();
    if (key === 'connection') this._removedConnection = true;
    else if (key === 'content-length') this._removedContLen = true;
    else if (key === 'transfer-encoding') this._removedTE = true;
    else if (key === 'date') this.sendDate = false;
    if (this[kOutHeaders]) delete this[kOutHeaders][key];
  }

  // The header block for `firstLine` and the given or stored headers, with
  // the connection, date and framing headers Node adds.
  _storeHeader(firstLine, headers) {
    const state = { connection: false, contLen: false, te: false, date: false, expect: false, trailer: false, header: firstLine };
    const add = (name, value) => {
      if (Array.isArray(value) && name.toLowerCase() !== 'set-cookie' && value.length > 1) {
        // Repeated header lines, as Node writes arrays.
        for (const item of value) add(name, item);
        return;
      }
      if (Array.isArray(value)) {
        for (const item of value) add(name, item);
        return;
      }
      validateHeaderName(name);
      validateHeaderValue(name, value);
      state.header += `${name}: ${value}\r\n`;
      const key = name.toLowerCase();
      if (key === 'connection') {
        state.connection = true;
        if (/(?:^|\W)close(?:$|\W)/i.test(String(value))) this._last = true;
        else this.shouldKeepAlive = true;
      } else if (key === 'transfer-encoding') {
        state.te = true;
        if (/(?:^|\W)chunked(?:$|\W)/i.test(String(value))) this.chunkedEncoding = true;
      } else if (key === 'content-length') {
        state.contLen = true;
        this._contentLength = +value;
        this._removedContLen = false;
      } else if (key === 'date') {
        state.date = true;
      } else if (key === 'expect') {
        state.expect = true;
      } else if (key === 'trailer') {
        state.trailer = true;
      }
    };
    if (headers) {
      for (const [name, value] of headerEntries(headers)) add(name, value);
    } else if (this[kOutHeaders]) {
      for (const key of Object.keys(this[kOutHeaders])) add(...this[kOutHeaders][key]);
    }
    if (this.sendDate && !state.date) state.header += `Date: ${new Date().toUTCString()}\r\n`;
    if (this.chunkedEncoding && (this.statusCode === 204 || this.statusCode === 304)) {
      this._hasBody = false;
    }
    if (!state.connection && !this._removedConnection) {
      const keepAlive = this.shouldKeepAlive && (state.contLen || this.useChunkedEncodingByDefault || this.agent);
      if (keepAlive && this.maxRequestsOnConnectionReached) {
        state.header += 'Connection: close\r\n';
        this._last = true;
      } else if (keepAlive) {
        state.header += 'Connection: keep-alive\r\n';
        if (this._keepAliveTimeout && this._defaultKeepAlive) {
          state.header += `Keep-Alive: timeout=${Math.floor(this._keepAliveTimeout / 1000)}\r\n`;
        }
      } else {
        this._last = true;
        state.header += 'Connection: close\r\n';
      }
    }
    if (!state.contLen && !state.te) {
      if (!this._hasBody) {
        this.chunkedEncoding = false;
      } else if (!this.useChunkedEncodingByDefault) {
        this._last = true;
      } else if (!state.trailer && !this._removedContLen && typeof this._contentLength === 'number') {
        state.header += `Content-Length: ${this._contentLength}\r\n`;
      } else if (!this._removedTE) {
        state.header += 'Transfer-Encoding: chunked\r\n';
        this.chunkedEncoding = true;
      } else {
        this._last = true;
      }
    }
    this._header = `${state.header}\r\n`;
    this._headerSent = false;
  }

  _writeRaw(data, encoding, callback) {
    const socket = this.socket;
    if (socket?.destroyed) {
      if (typeof callback === 'function') process.nextTick(callback);
      return false;
    }
    if (socket && socket._httpMessage === this && socket.writable) {
      if (this.outputData.length) this._flushOutput(socket);
      return socket.write(data, encoding, callback);
    }
    this.outputData.push({ data, encoding, callback });
    this.outputSize += data.length;
    return this.outputSize < this.writableHighWaterMark;
  }

  _send(data, encoding, callback) {
    if (!this._headerSent && this._header !== null) {
      if (typeof data === 'string' && (encoding === 'utf8' || encoding === 'latin1' || !encoding)) {
        data = this._header + data;
      } else {
        this.outputData.unshift({ data: this._header, encoding: 'latin1', callback: null });
        this.outputSize += this._header.length;
      }
      this._headerSent = true;
    }
    return this._writeRaw(data, encoding, callback);
  }

  _flushOutput(socket) {
    const pending = this.outputData;
    this.outputData = [];
    this.outputSize = 0;
    let ret = true;
    for (const { data, encoding, callback } of pending) {
      ret = socket.write(data, encoding, callback || undefined);
    }
    return ret;
  }

  _flush() {
    const socket = this.socket;
    if (socket && socket.writable) {
      const ret = this._flushOutput(socket);
      if (this.finished) this._finish();
      else if (ret && this[kNeedDrain]) {
        this[kNeedDrain] = false;
        this.emit('drain');
      }
    }
  }

  flushHeaders() {
    if (!this._header) this._implicitHeader();
    this._send('');
  }

  write(chunk, encoding, callback) {
    if (typeof encoding === 'function') {
      callback = encoding;
      encoding = null;
    }
    const ret = this._writeBody(chunk, encoding, callback);
    if (!ret) this[kNeedDrain] = true;
    return ret;
  }

  _writeBody(chunk, encoding, callback) {
    if (this.finished || this.destroyed) {
      const error = this.finished
        ? nodeError(Error, 'ERR_STREAM_WRITE_AFTER_END', 'write after end')
        : nodeError(Error, 'ERR_STREAM_DESTROYED', 'Cannot call write after a stream was destroyed');
      process.nextTick(() => {
        if (typeof callback === 'function') callback(error);
        this.emit('error', error);
      });
      return false;
    }
    if (chunk === null) throw nodeError(TypeError, 'ERR_STREAM_NULL_VALUES', 'May not write null values to stream');
    if (typeof chunk !== 'string' && !(chunk instanceof Uint8Array)) {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "chunk" argument must be of type string or an instance of Buffer or Uint8Array');
    }
    if (!this._header) this._implicitHeader();
    if (!this._hasBody) {
      if (typeof callback === 'function') process.nextTick(callback);
      return true;
    }
    const length = typeof chunk === 'string' ? Buffer.byteLength(chunk, encoding) : chunk.length;
    if (length === 0) {
      if (typeof callback === 'function') process.nextTick(callback);
      return true;
    }
    if (this.chunkedEncoding) {
      this._send(`${length.toString(16)}\r\n`, 'latin1', null);
      this._send(chunk, encoding, null);
      return this._send('\r\n', 'latin1', callback);
    }
    return this._send(chunk, encoding, callback);
  }

  addTrailers(headers) {
    this._trailer = '';
    for (const [name, value] of headerEntries(headers)) {
      validateHeaderName(name, 'Trailer name');
      validateHeaderValue(name, value);
      this._trailer += `${name}: ${value}\r\n`;
    }
  }

  end(chunk, encoding, callback) {
    if (typeof chunk === 'function') {
      callback = chunk;
      chunk = null;
      encoding = null;
    } else if (typeof encoding === 'function') {
      callback = encoding;
      encoding = null;
    }
    if (chunk !== null && chunk !== undefined) {
      if (this.finished) {
        this._writeBody(chunk, encoding, callback);
        return this;
      }
      if (!this._header) {
        this._contentLength = typeof chunk === 'string' ? Buffer.byteLength(chunk, encoding) : chunk.length;
      }
      this._writeBody(chunk, encoding, null);
    } else if (this.finished) {
      if (typeof callback === 'function') {
        if (!this.writableFinished) this.once('finish', callback);
        else process.nextTick(callback, nodeError(Error, 'ERR_STREAM_ALREADY_FINISHED', 'Cannot call end after a stream was finished'));
      }
      return this;
    } else if (!this._header) {
      this._contentLength = 0;
      this._implicitHeader();
    }
    if (typeof callback === 'function') this.once('finish', callback);
    const finish = () => this._finish();
    if (this._hasBody && this.chunkedEncoding) {
      this._send(`0\r\n${this._trailer}\r\n`, 'latin1', finish);
    } else if (!this._headerSent || this.writableLength || chunk) {
      this._send('', 'latin1', finish);
    } else {
      process.nextTick(finish);
    }
    while (this[kCorked]) this.uncork();
    this.finished = true;
    return this;
  }

  _finish() {
    if (this._finishEmitted || !this.finished) return;
    if (this.outputData.length) return;
    this._finishEmitted = true;
    this.emit('prefinish');
    this.emit('finish');
  }
}

class ServerResponse extends OutgoingMessage {
  constructor(req, options) {
    super(options);
    this.statusCode = 200;
    this.statusMessage = undefined;
    this.req = req;
    this.sendDate = true;
    this._sent100 = false;
    this._expect_continue = false;
    if (req.method === 'HEAD') this._hasBody = false;
    if (req.httpVersionMajor < 1 || req.httpVersionMinor < 1) {
      this.useChunkedEncodingByDefault = /(?:^|\W)chunked(?:$|\W)/i.test(req.headers.te || '');
      this.shouldKeepAlive = false;
    }
  }

  assignSocket(socket) {
    socket._httpMessage = this;
    socket.on('close', onServerResponseClose);
    this.socket = socket;
    this.emit('socket', socket);
    this._flush();
  }

  detachSocket(socket) {
    socket.removeListener('close', onServerResponseClose);
    socket._httpMessage = null;
    this.socket = null;
  }

  writeContinue(callback) {
    this._writeRaw('HTTP/1.1 100 Continue\r\n\r\n', 'ascii', callback);
    this._sent100 = true;
  }

  writeProcessing(callback) {
    this._writeRaw('HTTP/1.1 102 Processing\r\n\r\n', 'ascii', callback);
  }

  writeEarlyHints(hints, callback) {
    let head = 'HTTP/1.1 103 Early Hints\r\n';
    for (const [name, value] of headerEntries(hints)) {
      for (const item of [].concat(value)) head += `${name === 'link' ? 'Link' : name}: ${item}\r\n`;
    }
    this._writeRaw(`${head}\r\n`, 'ascii', callback);
  }

  _implicitHeader() {
    this.writeHead(this.statusCode);
  }

  writeHead(statusCode, reason, headers) {
    if (this._header) throw nodeError(Error, 'ERR_HTTP_HEADERS_SENT', 'Cannot write headers after they are sent to the client');
    statusCode |= 0;
    if (statusCode < 100 || statusCode > 999) {
      throw nodeError(RangeError, 'ERR_HTTP_INVALID_STATUS_CODE', `Invalid status code: ${statusCode}`);
    }
    if (typeof reason === 'string') {
      this.statusMessage = reason;
    } else {
      this.statusMessage ||= STATUS_CODES[statusCode] || 'unknown';
      headers ??= reason;
    }
    this.statusCode = statusCode;
    let block;
    if (this[kOutHeaders]) {
      if (headers) {
        for (const [name, value] of headerEntries(headers)) this.setHeader(name, value);
      }
      block = null;
    } else {
      block = headers;
    }
    if (checkInvalidHeaderChar(this.statusMessage)) {
      throw nodeError(TypeError, 'ERR_INVALID_CHAR', 'Invalid character in statusMessage');
    }
    if (statusCode === 204 || statusCode === 304 || (statusCode >= 100 && statusCode <= 199)) {
      this._hasBody = false;
    }
    if (this._expect_continue && !this._sent100) this.shouldKeepAlive = false;
    this._storeHeader(`HTTP/1.1 ${statusCode} ${this.statusMessage}\r\n`, block);
    return this;
  }
}

ServerResponse.prototype.writeHeader = ServerResponse.prototype.writeHead;

function onServerResponseClose() {
  const message = this._httpMessage;
  if (message) {
    message.destroyed = true;
    message._closed = true;
    message.emit('close');
  }
}

class Server extends net.Server {
  constructor(options, requestListener) {
    if (typeof options === 'function') {
      requestListener = options;
      options = {};
    } else {
      options = { ...options };
    }
    super({
      noDelay: options.noDelay ?? true,
      keepAlive: options.keepAlive,
      keepAliveInitialDelay: options.keepAliveInitialDelay,
      highWaterMark: options.highWaterMark,
    });
    this[kIncomingMessage] = options.IncomingMessage || IncomingMessage;
    this[kServerResponse] = options.ServerResponse || ServerResponse;
    this.maxHeaderSize = options.maxHeaderSize;
    this.joinDuplicateHeaders = options.joinDuplicateHeaders;
    this.requireHostHeader = options.requireHostHeader ?? true;
    this.timeout = 0;
    this.maxHeadersCount = null;
    this.maxRequestsPerSocket = 0;
    this.headersTimeout = options.headersTimeout ?? 60 * 1000;
    this.keepAliveTimeout = options.keepAliveTimeout ?? 5000;
    this.requestTimeout = options.requestTimeout ?? 300 * 1000;
    this.httpAllowHalfOpen = false;
    this[kConnections] = new Set();
    if (requestListener) this.on('request', requestListener);
    this.on('connection', connectionListener);
  }

  setTimeout(timeout, callback) {
    this.timeout = timeout;
    if (callback) this.on('timeout', callback);
    return this;
  }

  close(callback) {
    this.closeIdleConnections();
    return super.close(callback);
  }

  closeAllConnections() {
    for (const socket of this[kConnections]) socket.destroy();
  }

  closeIdleConnections() {
    for (const socket of this[kConnections]) {
      if (!socket._httpMessage) socket.destroy();
    }
  }

  [Symbol.asyncDispose]() {
    return new Promise((resolve, reject) => {
      if (!this.listening) return resolve();
      this.close((error) => error ? reject(error) : resolve());
    });
  }
}

const kIncomingMessage = Symbol('IncomingMessage');
const kServerResponse = Symbol('ServerResponse');
const kConnections = Symbol('http.server.connections');

function connectionListener(socket) {
  const server = this;
  const state = { incoming: [], outgoing: [], requests: 0, keepAliveTimeoutSet: false };
  const parser = new Parser('request', server.maxHeaderSize || maxHeaderSize);
  server[kConnections].add(socket);
  if (server.timeout && typeof socket.setTimeout === 'function') socket.setTimeout(server.timeout);
  socket.on('timeout', () => {
    const req = socket.parser?.incoming;
    let handled = req ? req.emit('timeout', socket) : false;
    const res = socket._httpMessage;
    handled = (res ? res.emit('timeout', socket) : false) || handled;
    handled = server.emit('timeout', socket) || handled;
    if (!handled) socket.destroy();
  });

  const onData = (chunk) => parser.execute(chunk);
  const abortIncoming = () => {
    while (state.incoming.length) {
      const req = state.incoming.shift();
      req.destroy(connResetException('aborted'));
    }
  };
  socket.on('data', onData);
  socket.on('end', () => {
    parser.finish();
    if (!server.httpAllowHalfOpen) {
      abortIncoming();
      if (socket.writable) socket.end();
    } else if (state.outgoing.length) {
      state.outgoing[state.outgoing.length - 1]._last = true;
    } else if (socket._httpMessage) {
      socket._httpMessage._last = true;
    } else if (socket.writable) {
      socket.end();
    }
  });
  socket.on('error', (error) => {
    if (server.listenerCount('clientError')) {
      server.emit('clientError', error, socket);
    } else {
      socket.destroy();
    }
  });
  socket.on('close', () => {
    server[kConnections].delete(socket);
    abortIncoming();
  });

  parser.onError = (error) => {
    socket.removeListener('data', onData);
    if (server.listenerCount('clientError')) {
      server.emit('clientError', error, socket);
      return;
    }
    if (socket.writable && (!socket._httpMessage || !socket._httpMessage._headerSent)) {
      const status = error.code === 'HPE_HEADER_OVERFLOW' ? '431 Request Header Fields Too Large' : '400 Bad Request';
      socket.write(`HTTP/1.1 ${status}\r\nConnection: close\r\n\r\n`);
    }
    socket.destroy(error);
  };

  parser.onHeaders = (info) => {
    if (state.keepAliveTimeoutSet) {
      socket.setTimeout(server.timeout || 0);
      state.keepAliveTimeoutSet = false;
    }
    const req = new server[kIncomingMessage](socket);
    req.joinDuplicateHeaders = Boolean(server.joinDuplicateHeaders);
    req.httpVersionMajor = info.versionMajor;
    req.httpVersionMinor = info.versionMinor;
    req.httpVersion = `${info.versionMajor}.${info.versionMinor}`;
    req.method = info.method;
    req.url = info.url;
    req.rawHeaders = info.rawHeaders;
    req.upgrade = info.upgrade;
    parser.incoming = req;
    socket.parser = parser;
    if (info.upgrade) {
      const event = info.method === 'CONNECT' ? 'connect' : 'upgrade';
      if (server.listenerCount(event)) {
        socket.removeListener('data', onData);
        process.nextTick(() => {
          const head = parser.rest || Buffer.alloc(0);
          server.emit(event, req, socket, head);
        });
        return true;
      }
      if (event === 'connect') {
        socket.destroy();
        return true;
      }
      req.upgrade = false;
    }
    state.incoming.push(req);
    state.requests++;
    const res = new server[kServerResponse](req, { highWaterMark: socket.writableHighWaterMark });
    res.shouldKeepAlive = info.shouldKeepAlive;
    res._keepAliveTimeout = server.keepAliveTimeout;
    if (server.maxRequestsPerSocket && state.requests >= server.maxRequestsPerSocket) {
      res.maxRequestsOnConnectionReached = true;
    }
    if (socket._httpMessage) state.outgoing.push(res);
    else res.assignSocket(socket);
    res.on('finish', () => {
      const finishedReq = state.incoming.shift();
      if (finishedReq && !finishedReq._consuming && !finishedReq._readableState.resumeScheduled) {
        finishedReq._dump();
      }
      res.detachSocket(socket);
      process.nextTick(() => {
        res.destroyed = true;
        res._closed = true;
        res.emit('close');
      });
      if (res._last) {
        socket.destroySoon();
      } else if (state.outgoing.length === 0) {
        if (server.keepAliveTimeout) {
          socket.setTimeout(server.keepAliveTimeout);
          state.keepAliveTimeoutSet = true;
        }
      } else {
        state.outgoing.shift().assignSocket(socket);
      }
    });
    const expect = req.headers.expect;
    if (expect !== undefined && req.httpVersionMajor === 1 && req.httpVersionMinor === 1) {
      if (/(?:^|\W)100-continue(?:$|\W)/i.test(expect)) {
        res._expect_continue = true;
        if (server.listenerCount('checkContinue')) {
          server.emit('checkContinue', req, res);
        } else {
          res.writeContinue();
          server.emit('request', req, res);
        }
      } else if (server.listenerCount('checkExpectation')) {
        server.emit('checkExpectation', req, res);
      } else {
        res.writeHead(417);
        res.end();
      }
    } else {
      server.emit('request', req, res);
    }
    return false;
  };
  parser.onBody = (chunk) => {
    const req = parser.incoming;
    if (req && !req._dumped) req._pushBody(chunk);
  };
  parser.onComplete = (trailers) => {
    const req = parser.incoming;
    if (!req) return;
    req.rawTrailers = trailers;
    req.complete = true;
    req.push(null);
  };
}

function createServer(options, requestListener) {
  return new Server(options, requestListener);
}

// Connection pooling for client requests, as Node's agent: sockets per
// host and port, kept free between requests when `keepAlive` is set.
class Agent extends EventEmitter {
  constructor(options) {
    super();
    this.defaultPort = 80;
    this.protocol = 'http:';
    this.options = { __proto__: null, ...options };
    if (this.options.noDelay === undefined) this.options.noDelay = true;
    this.options.path = null;
    this.requests = Object.create(null);
    this.sockets = Object.create(null);
    this.freeSockets = Object.create(null);
    this.keepAliveMsecs = this.options.keepAliveMsecs || 1000;
    this.keepAlive = this.options.keepAlive || false;
    this.maxSockets = this.options.maxSockets || Agent.defaultMaxSockets;
    this.maxFreeSockets = this.options.maxFreeSockets || 256;
    this.scheduling = this.options.scheduling || 'lifo';
    this.maxTotalSockets = this.options.maxTotalSockets ?? Infinity;
    this.totalSocketCount = 0;
    this.on('free', (socket, options) => {
      const name = this.getName(options);
      if (!socket.writable) {
        socket.destroy();
        return;
      }
      const requests = this.requests[name];
      if (requests && requests.length) {
        const req = requests.shift();
        if (!requests.length) delete this.requests[name];
        this.reuseSocket(socket, req);
        req.onSocket(socket);
        return;
      }
      const req = socket._httpMessage;
      if (!req || !req.shouldKeepAlive || !this.keepAlive) {
        socket.destroy();
        return;
      }
      const free = this.freeSockets[name] || [];
      if (free.length >= this.maxFreeSockets || !this.keepSocketAlive(socket)) {
        socket.destroy();
        return;
      }
      this.freeSockets[name] = free;
      socket._httpMessage = null;
      this._removeSocket(socket, options, true);
      free.push(socket);
    });
  }

  getName(options = {}) {
    let name = options.host || 'localhost';
    name += ':';
    if (options.port) name += options.port;
    name += ':';
    if (options.localAddress) name += options.localAddress;
    if (options.family === 4 || options.family === 6) name += `:${options.family}`;
    if (options.socketPath) name += `:${options.socketPath}`;
    return name;
  }

  createConnection(...args) {
    return net.createConnection(...args);
  }

  keepSocketAlive(socket) {
    socket.setKeepAlive(true, this.keepAliveMsecs);
    socket.unref();
    if (this.options.timeout) socket.setTimeout(this.options.timeout);
    return true;
  }

  reuseSocket(socket, req) {
    req.reusedSocket = true;
    socket.ref();
  }

  addRequest(req, options, port, localAddress) {
    if (typeof options === 'string') options = { __proto__: null, host: options, port, localAddress };
    options = { __proto__: null, ...options, ...this.options };
    if (options.socketPath) options.path = options.socketPath;
    options.servername ||= options.host;
    // The socket returns to the pool it came from.
    req._agentOptions = options;
    const name = this.getName(options);
    const free = this.freeSockets[name];
    while (free && free.length) {
      const socket = this.scheduling === 'fifo' ? free.shift() : free.pop();
      if (!free.length) delete this.freeSockets[name];
      if (socket.destroyed) continue;
      (this.sockets[name] ||= []).push(socket);
      this.reuseSocket(socket, req);
      req.onSocket(socket);
      return;
    }
    const count = (this.sockets[name] || []).length;
    if (count < this.maxSockets && this.totalSocketCount < this.maxTotalSockets) {
      this.createSocket(req, options, (error, socket) => {
        if (error) req.onSocket(socket, error);
        else req.onSocket(socket);
      });
    } else {
      (this.requests[name] ||= []).push(req);
    }
  }

  createSocket(req, options, callback) {
    options = { __proto__: null, ...options, ...this.options };
    const name = this.getName(options);
    options._agentKey = name;
    let called = false;
    const done = (error, socket) => {
      if (called) return;
      called = true;
      if (error) return callback(error);
      (this.sockets[name] ||= []).push(socket);
      this.totalSocketCount++;
      this._installListeners(socket, options);
      callback(null, socket);
    };
    const socket = this.createConnection(options, done);
    if (socket) done(null, socket);
  }

  _installListeners(socket, options) {
    const onRemove = () => {
      this.totalSocketCount--;
      this._removeSocket(socket, options, false);
      socket.removeListener('close', onRemove);
      socket.removeListener('timeout', onTimeout);
      socket.removeListener('agentRemove', onRemove);
    };
    const onTimeout = () => {
      const free = this.freeSockets[this.getName(options)];
      if (free && free.includes(socket)) socket.destroy();
    };
    socket.on('close', onRemove);
    socket.on('timeout', onTimeout);
    socket.on('agentRemove', onRemove);
  }

  _removeSocket(socket, options, keepFree) {
    const name = this.getName(options);
    for (const sets of keepFree ? [this.sockets] : [this.sockets, this.freeSockets]) {
      const list = sets[name];
      if (!list) continue;
      const index = list.indexOf(socket);
      if (index !== -1) {
        list.splice(index, 1);
        if (!list.length) delete sets[name];
      }
    }
    if (!keepFree && this.requests[name]?.length) {
      const req = this.requests[name][0];
      this.createSocket(req, options, (error, created) => {
        if (error) return;
        const queued = this.requests[name];
        const next = queued?.shift();
        if (queued && !queued.length) delete this.requests[name];
        if (next) next.onSocket(created);
        else created.destroy();
      });
    }
  }

  destroy() {
    for (const sets of [this.freeSockets, this.sockets]) {
      for (const name of Object.keys(sets)) {
        for (const socket of sets[name]) socket.destroy();
      }
    }
  }
}

Agent.defaultMaxSockets = Infinity;

const globalAgent = new Agent({ keepAlive: true, scheduling: 'lifo', timeout: 5000 });

// Arguments as Node's ClientRequest takes them: (url[, options][, cb]) or
// (options[, cb]), as merged options and the response listener.
function requestArguments(input, options, callback) {
  if (typeof input === 'string') {
    input = urlToHttpOptions(new URL(input));
  } else if (isURL(input)) {
    input = urlToHttpOptions(input);
  } else {
    callback = options;
    options = input;
    input = null;
  }
  if (typeof options === 'function') {
    callback = options;
    options = input || {};
  } else {
    options = { ...input, ...options };
  }
  return [options, callback];
}

class ClientRequest extends OutgoingMessage {
  constructor(input, options, callback) {
    super();
    [options, callback] = requestArguments(input, options, callback);
    const defaultAgent = options._defaultAgent || globalAgent;
    let agent = options.agent;
    if (agent === false) {
      agent = new defaultAgent.constructor();
    } else if (agent === null || agent === undefined) {
      if (typeof options.createConnection !== 'function') agent = defaultAgent;
    } else if (typeof agent.addRequest !== 'function') {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options.agent" property must be one of Agent-like Object, undefined, or false.');
    }
    this.agent = agent;
    const protocol = options.protocol || defaultAgent.protocol;
    let expectedProtocol = defaultAgent.protocol;
    if (this.agent?.protocol) expectedProtocol = this.agent.protocol;
    if (protocol !== expectedProtocol) {
      throw nodeError(TypeError, 'ERR_INVALID_PROTOCOL', `Protocol "${protocol}" not supported. Expected "${expectedProtocol}"`);
    }
    const defaultPort = options.defaultPort || this.agent?.defaultPort || 80;
    const port = options.port = options.port || defaultPort || 80;
    const host = options.host = options.hostname?.startsWith?.('[') ? options.hostname.slice(1, -1)
      : (options.hostname || options.host || 'localhost');
    const path = options.path || '/';
    if (/[\u0000- ]/.test(path)) {
      throw nodeError(TypeError, 'ERR_UNESCAPED_CHARACTERS', 'Request path contains unescaped characters');
    }
    this.path = path;
    this.method = (options.method || 'GET').toUpperCase();
    if (!checkIsHttpToken(this.method)) {
      throw nodeError(TypeError, 'ERR_INVALID_HTTP_TOKEN', `Method must be a valid HTTP token ["${options.method}"]`);
    }
    this.host = host;
    this.protocol = protocol;
    this.timeout = options.timeout;
    this.res = null;
    this.aborted = false;
    this.reusedSocket = false;
    this.maxHeaderSize = options.maxHeaderSize;
    this.joinDuplicateHeaders = options.joinDuplicateHeaders;
    this.insecureHTTPParser = options.insecureHTTPParser;
    this._ended = false;
    if (callback) this.once('response', callback);
    if (['GET', 'HEAD', 'DELETE', 'OPTIONS', 'TRACE', 'CONNECT'].includes(this.method)) {
      this.useChunkedEncodingByDefault = false;
    }
    this.shouldKeepAlive = Boolean(this.agent?.keepAlive);
    this._defaultKeepAlive = this.shouldKeepAlive;
    const headers = options.headers;
    const raw = Array.isArray(headers);
    if (!raw) {
      if (headers) {
        for (const name of Object.keys(headers)) this.setHeader(name, headers[name]);
      }
      if (host && !this.getHeader('host') && options.setHost !== false) {
        let hostHeader = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
        if (+port !== +defaultPort) hostHeader += `:${port}`;
        this.setHeader('Host', hostHeader);
      }
      if (options.auth && !this.getHeader('Authorization')) {
        this.setHeader('Authorization', `Basic ${Buffer.from(options.auth).toString('base64')}`);
      }
      if (this.getHeader('expect')) this._storeHeader(`${this.method} ${path} HTTP/1.1\r\n`, null);
    } else {
      this._storeHeader(`${this.method} ${path} HTTP/1.1\r\n`, headers);
    }
    if (options.signal) {
      const abort = () => this.destroy(nodeError(Error, 'ABORT_ERR', 'The operation was aborted'));
      if (options.signal.aborted) process.nextTick(abort);
      else options.signal.addEventListener('abort', abort, { once: true });
    }
    const connectOptions = { ...options, host, port, path: null };
    if (this.agent) {
      this.agent.addRequest(this, connectOptions);
    } else if (typeof options.createConnection === 'function') {
      let called = false;
      const created = options.createConnection(connectOptions, (error, socket) => {
        if (called) return;
        called = true;
        this.onSocket(socket, error);
      });
      if (created && !called) {
        called = true;
        this.onSocket(created);
      }
    } else {
      this.onSocket(net.createConnection(connectOptions));
    }
  }

  get path() { return this._path; }
  set path(value) { this._path = value; }

  _implicitHeader() {
    if (this._header) throw nodeError(Error, 'ERR_HTTP_HEADERS_SENT', 'Cannot render headers after they are sent to the client');
    this._storeHeader(`${this.method} ${this.path} HTTP/1.1\r\n`, null);
  }

  abort() {
    if (this.aborted) return;
    this.aborted = true;
    process.nextTick(() => this.emit('abort'));
    this.destroy();
  }

  destroy(error) {
    if (this.destroyed) return this;
    this.destroyed = true;
    if (this.res) this.res._dump();
    this[kError] = error;
    if (this.socket) {
      this.socket.destroy(error);
    } else if (error) {
      process.nextTick(() => {
        this.emit('error', error);
        this._emitClose();
      });
    } else {
      process.nextTick(() => this._emitClose());
    }
    return this;
  }

  _emitClose() {
    if (this._closed) return;
    this._closed = true;
    this.emit('close');
  }

  onSocket(socket, error) {
    process.nextTick(() => {
      if (error) {
        this.destroyed = true;
        this.emit('error', error);
        this._emitClose();
        return;
      }
      if (this.destroyed) {
        this.agent?.emit?.('free', socket, {});
        socket.destroy(this[kError]);
        return;
      }
      this._attachSocket(socket);
    });
  }

  _attachSocket(socket) {
    const req = this;
    const parser = new Parser('response', this.maxHeaderSize || maxHeaderSize);
    this.socket = socket;
    socket._httpMessage = this;
    socket.parser = parser;
    let res = null;
    let errored = false;
    const onData = (chunk) => parser.execute(chunk);
    const detach = () => {
      socket.removeListener('data', onData);
      socket.removeListener('end', onEnd);
      socket.removeListener('close', onClose);
      socket.removeListener('error', onError);
      socket.removeListener('timeout', onTimeout);
      socket.parser = null;
    };
    const fail = (error) => {
      if (errored) return;
      errored = true;
      if (req.res && !req.res.complete) {
        req.res.destroy(connResetException('aborted'));
      } else if (!req.res) {
        req.destroyed = true;
        req.emit('error', error);
      }
    };
    const onError = (error) => fail(error);
    const onEnd = () => parser.finish();
    const onClose = () => {
      detach();
      if (req.res && req.res.readable && !req.res.complete) {
        req.res.destroy(connResetException('aborted'));
      } else if (!req.res && !errored && !req.destroyed) {
        fail(connResetException('socket hang up'));
      }
      req.destroyed = true;
      if (req.res) {
        req.res.once('close', () => req._emitClose());
        if (!req.res.readable || req.res.destroyed) req._emitClose();
      } else {
        req._emitClose();
      }
    };
    const onTimeout = () => req.emit('timeout');
    socket.on('data', onData);
    socket.on('end', onEnd);
    socket.on('close', onClose);
    socket.on('error', onError);
    if (this.timeout !== undefined || this.listenerCount('timeout')) socket.on('timeout', onTimeout);
    if (this.timeout !== undefined) socket.setTimeout(this.timeout);
    parser.onError = (error) => {
      detach();
      fail(error);
      socket.destroy();
    };
    parser.onHeaders = (info) => {
      res = new IncomingMessage(socket);
      res.joinDuplicateHeaders = Boolean(this.joinDuplicateHeaders);
      res.httpVersionMajor = info.versionMajor;
      res.httpVersionMinor = info.versionMinor;
      res.httpVersion = `${info.versionMajor}.${info.versionMinor}`;
      res.statusCode = info.statusCode;
      res.statusMessage = info.statusMessage;
      res.rawHeaders = info.rawHeaders;
      res.req = req;
      if (info.statusCode < 200 && info.statusCode !== 101) {
        if (info.statusCode === 100) req.emit('continue');
        req.emit('information', {
          statusCode: info.statusCode,
          statusMessage: info.statusMessage,
          httpVersion: res.httpVersion,
          httpVersionMajor: info.versionMajor,
          httpVersionMinor: info.versionMinor,
          headers: res.headers,
          rawHeaders: info.rawHeaders,
        });
        return true;
      }
      if (req.method === 'CONNECT' || info.statusCode === 101) {
        const event = req.method === 'CONNECT' ? 'connect' : 'upgrade';
        res.upgrade = true;
        detach();
        process.nextTick(() => {
          const head = parser.rest || Buffer.alloc(0);
          if (req.listenerCount(event)) {
            req.res = res;
            socket._httpMessage = null;
            req.emit(event, res, socket, head);
            req.destroyed = true;
            req._emitClose();
          } else {
            socket.destroy();
          }
        });
        parser.stopped = true;
        parser.rest = parser.buffer;
        return true;
      }
      req.res = res;
      if (!info.shouldKeepAlive) req.shouldKeepAlive = false;
      res.on('end', () => {
        if (!req.shouldKeepAlive) {
          if (socket.writable) socket.destroySoon();
        } else if (req.writableFinished || req.finished) {
          responseKeepAlive();
        } else {
          req.once('finish', responseKeepAlive);
        }
      });
      if (!req.emit('response', res)) res._dump();
      return req.method === 'HEAD';
    };
    const responseKeepAlive = () => {
      detach();
      socket._httpMessage = req;
      req.destroyed = true;
      process.nextTick(() => {
        req._emitClose();
        if (req.res) {
          req.res.destroyed = true;
          req.res.emit?.('close');
        }
        if (this.agent) this.agent.emit('free', socket, this._agentOptions || {});
        else socket.destroy();
      });
    };
    parser.onBody = (chunk) => {
      if (res && !res._dumped) res._pushBody(chunk);
    };
    parser.onComplete = (trailers) => {
      if (!res) return;
      res.rawTrailers = trailers;
      res.complete = true;
      res.push(null);
    };
    this.emit('socket', socket);
    this._flush();
  }
}

const kError = Symbol('kError');

function request(url, options, callback) {
  return new ClientRequest(url, options, callback);
}

function get(url, options, callback) {
  const req = request(url, options, callback);
  req.end();
  return req;
}

module.exports = {
  _connectionListener: connectionListener,
  METHODS: METHODS.slice().sort(),
  STATUS_CODES,
  Agent,
  ClientRequest,
  IncomingMessage,
  OutgoingMessage,
  Server,
  ServerResponse,
  createServer,
  validateHeaderName,
  validateHeaderValue,
  get,
  request,
  setMaxIdleHTTPParsers() {},
  maxHeaderSize,
  globalAgent,
};
