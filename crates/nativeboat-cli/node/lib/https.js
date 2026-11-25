'use strict';

// HTTPS requests on the native client, which provides TLS and the system
// proxy, with Node's request and response classes. The response body
// streams as the native client reads it, pausing while the reader lags.
// HTTPS servers need TLS sockets, which are not available.

const EventEmitter = require('events');
const http = require('http');
const { Buffer } = require('buffer');
const { URL, urlToHttpOptions, isURL } = require('internal/url');
const client = internalBinding('http_client');

function nodeError(Base, code, message) {
  const error = new Base(message);
  error.code = code;
  return error;
}

class Agent extends http.Agent {
  constructor(options) {
    super(options);
    this.defaultPort = 443;
    this.protocol = 'https:';
  }
}

const globalAgent = new Agent({ keepAlive: true, scheduling: 'lifo', timeout: 5000 });

// What stands in for the connection: the native client owns the real one.
class NativeSocket extends EventEmitter {
  constructor(request) {
    super();
    this.connecting = false;
    this.destroyed = false;
    this.encrypted = true;
    this.readable = true;
    this.writable = true;
    this.remoteAddress = undefined;
    this.remotePort = 443;
    this._request = request;
    this._timer = null;
    this._timeout = 0;
    this._paused = false;
  }

  setTimeout(timeout, callback) {
    if (callback) this.once('timeout', callback);
    this._timeout = timeout;
    this._arm();
    return this;
  }

  _arm() {
    if (this._timer) clearTimeout(this._timer);
    this._timer = null;
    if (this._timeout && !this.destroyed) {
      this._timer = setTimeout(() => this.emit('timeout'), this._timeout);
    }
  }

  setNoDelay() { return this; }
  setKeepAlive() { return this; }
  ref() { return this; }
  unref() { return this; }
  address() { return {}; }
  pause() {
    this._paused = true;
    return this;
  }

  resume() {
    this._paused = false;
    this._request._pump();
    return this;
  }

  destroy(error) {
    if (this.destroyed) return this;
    this.destroyed = true;
    this.readable = false;
    this.writable = false;
    if (this._timer) clearTimeout(this._timer);
    this._request._socketDestroyed(error);
    process.nextTick(() => this.emit('close', Boolean(error)));
    return this;
  }
}

class ClientRequest extends http.OutgoingMessage {
  constructor(input, options, callback) {
    super();
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
    const protocol = options.protocol || 'https:';
    if (protocol !== 'https:') {
      throw nodeError(TypeError, 'ERR_INVALID_PROTOCOL', `Protocol "${protocol}" not supported. Expected "https:"`);
    }
    this.agent = options.agent === undefined ? globalAgent : options.agent;
    // Certificates the system does not trust are refused unless the
    // request, its agent or NODE_TLS_REJECT_UNAUTHORIZED=0 accepts them.
    let reject = options.rejectUnauthorized;
    if (reject === undefined && this.agent) reject = (this.agent.options || {}).rejectUnauthorized;
    this._insecure = reject === undefined ? process.env.NODE_TLS_REJECT_UNAUTHORIZED === '0' : reject === false;
    this.method = (options.method || 'GET').toUpperCase();
    this.host = options.hostname || options.host || 'localhost';
    const port = options.port || 443;
    this.path = options.path || '/';
    if (/[\u0000- ]/.test(this.path)) {
      throw nodeError(TypeError, 'ERR_UNESCAPED_CHARACTERS', 'Request path contains unescaped characters');
    }
    this.protocol = protocol;
    this.res = null;
    this.aborted = false;
    this.reusedSocket = false;
    this.timeout = options.timeout;
    this.shouldKeepAlive = false;
    const host = this.host.includes(':') ? `[${this.host}]` : this.host;
    this._url = `https://${host}${+port === 443 ? '' : `:${port}`}${this.path}`;
    this._body = [];
    if (callback) this.once('response', callback);
    const headers = options.headers;
    if (Array.isArray(headers)) {
      for (let index = 0; index < headers.length; index += 2) this.appendHeader(headers[index], headers[index + 1]);
    } else if (headers) {
      for (const name of Object.keys(headers)) this.setHeader(name, headers[name]);
    }
    if (options.auth && !this.getHeader('Authorization')) {
      this.setHeader('Authorization', `Basic ${Buffer.from(options.auth).toString('base64')}`);
    }
    if (options.signal) {
      const abort = () => this.destroy(nodeError(Error, 'ABORT_ERR', 'The operation was aborted'));
      if (options.signal.aborted) process.nextTick(abort);
      else options.signal.addEventListener('abort', abort, { once: true });
    }
    const socket = new NativeSocket(this);
    socket.on('timeout', () => this.emit('timeout'));
    if (this.timeout !== undefined) socket.setTimeout(this.timeout);
    this.socket = socket;
    process.nextTick(() => {
      if (!this.destroyed) this.emit('socket', socket);
    });
  }

  _implicitHeader() {
    this._header = '';
  }

  _send(data, encoding, callback) {
    this._headerSent = true;
    if (data.length) this._body.push(typeof data === 'string' ? Buffer.from(data, encoding || 'latin1') : data);
    if (typeof callback === 'function') process.nextTick(callback);
    return true;
  }

  _writeBody(chunk, encoding, callback) {
    if (this.finished || this.destroyed) {
      return super._writeBody(chunk, encoding, callback);
    }
    if (!this._header) this._implicitHeader();
    return this._send(chunk, encoding, callback);
  }

  end(chunk, encoding, callback) {
    if (typeof chunk === 'function') {
      callback = chunk;
      chunk = null;
    } else if (typeof encoding === 'function') {
      callback = encoding;
      encoding = null;
    }
    if (this.finished) return this;
    if (chunk !== null && chunk !== undefined) this._writeBody(chunk, encoding, null);
    if (!this._header) this._implicitHeader();
    if (typeof callback === 'function') this.once('finish', callback);
    this.finished = true;
    process.nextTick(() => {
      this._finishEmitted = true;
      this.emit('prefinish');
      this.emit('finish');
    });
    this._start();
    return this;
  }

  _start() {
    const headers = [];
    for (const name of this.getRawHeaderNames()) {
      for (const value of [].concat(this.getHeader(name))) headers.push(name, String(value));
    }
    const body = Buffer.concat(this._body);
    this._body = [];
    client.request(this._url, this.method, headers, body, (error, status, statusText, rawHeaders, reader) => {
      if (this.destroyed) {
        if (reader) reader.close();
        return;
      }
      this.socket._arm();
      if (error) {
        this.destroyed = true;
        this.socket.destroyed = true;
        this.emit('error', error);
        this._emitClose();
        return;
      }
      this._reader = reader;
      const res = new http.IncomingMessage(this.socket);
      res.httpVersionMajor = 1;
      res.httpVersionMinor = 1;
      res.httpVersion = '1.1';
      res.statusCode = status;
      res.statusMessage = statusText;
      res.rawHeaders = rawHeaders;
      res.req = this;
      this.res = res;
      res.on('end', () => {
        this.destroyed = true;
        process.nextTick(() => {
          this._emitClose();
          res.emit('close');
        });
      });
      if (!this.emit('response', res)) res._dump();
      if (this.method === 'HEAD') this._end();
      else this._pump();
    }, this._insecure);
  }

  // Reads the next part of the body unless one is on its way or the
  // reader lags; the socket's pause and resume follow the response.
  _pump() {
    if (this._pumping || !this._reader || this.socket._paused) return;
    this._pumping = true;
    this._reader.read((error, chunk) => {
      this._pumping = false;
      if (!this._reader) return;
      if (error) {
        const aborted = nodeError(Error, 'ECONNRESET', 'aborted');
        aborted.cause = error;
        this.socket.destroy(aborted);
        return;
      }
      this.socket._arm();
      if (!chunk.length) {
        this._end();
        return;
      }
      this.res._pushBody(Buffer.from(chunk.buffer, chunk.byteOffset, chunk.length));
      this._pump();
    });
  }

  _end() {
    this._closeReader();
    this.res.complete = true;
    this.res.push(null);
  }

  _closeReader() {
    if (!this._reader) return;
    this._reader.close();
    this._reader = null;
  }

  _socketDestroyed(error) {
    this._closeReader();
    if (this.destroyed) return;
    this.destroyed = true;
    if (this.res && !this.res.complete) {
      this.res.destroy(error);
    } else if (!this.res) {
      process.nextTick(() => {
        if (error) this.emit('error', error);
        this._emitClose();
      });
    }
  }

  _emitClose() {
    if (this._closed) return;
    this._closed = true;
    this.emit('close');
  }

  destroy(error) {
    this.socket.destroy(error);
    return this;
  }

  abort() {
    if (this.aborted) return;
    this.aborted = true;
    process.nextTick(() => this.emit('abort'));
    this.destroy();
  }

  setNoDelay() {}
  setSocketKeepAlive() {}
}

function request(url, options, callback) {
  return new ClientRequest(url, options, callback);
}

function get(url, options, callback) {
  const req = request(url, options, callback);
  req.end();
  return req;
}

class Server extends http.Server {
  listen() {
    const error = nodeError(Error, 'ENOTSUP', 'HTTPS servers are not available in this application');
    error.syscall = 'listen';
    process.nextTick(() => this.emit('error', error));
    return this;
  }
}

function createServer(options, requestListener) {
  return new Server(options, requestListener);
}

module.exports = {
  Agent,
  globalAgent,
  Server,
  createServer,
  get,
  request,
};
