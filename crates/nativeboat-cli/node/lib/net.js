'use strict';

// TCP sockets and servers on the native Winsock binding, and named pipes
// (`\\.\pipe\name` paths) on the native pipe binding, with Node's stream,
// event and error behavior. Handle passing is not available.

const EventEmitter = require('events');
const { Duplex } = require('stream');
const { Buffer } = require('buffer');
const { isIP, isIPv4, isIPv6 } = require('internal/net');
const tcp = internalBinding('tcp');
let pipeBinding;
const pipes = () => (pipeBinding ??= internalBinding('pipe'));

const kTimeout = Symbol('timeout');
const kReading = Symbol('reading');
const kRef = Symbol('ref');
const kSetNoDelay = Symbol('kSetNoDelay');
const kSetKeepAlive = Symbol('kSetKeepAlive');
const kSetKeepAliveInitialDelay = Symbol('kSetKeepAliveInitialDelay');
const kBytesWritten = Symbol('kBytesWritten');
const kBytesRead = Symbol('kBytesRead');

function nodeError(Base, code, message) {
  const error = new Base(message);
  error.code = code;
  return error;
}

const toNumber = (value) => (value = Number(value)) >= 0 ? value : false;

const isPipeName = (value) => typeof value === 'string' && toNumber(value) === false;

function validatePort(port, name = 'Port') {
  if ((typeof port !== 'number' && typeof port !== 'string') ||
      (typeof port === 'string' && port.trim().length === 0) ||
      +port !== (+port >>> 0) || port > 0xFFFF) {
    throw nodeError(RangeError, 'ERR_SOCKET_BAD_PORT',
                    `${name} should be >= 0 and < 65536. Received ${typeof port} (${String(port)}).`);
  }
  return +port;
}

// Node's `normalizeArgs`: (options[, listener]), (port[, host][, listener])
// or (path[, listener]), as [options, listener].
function normalizeArgs(args) {
  if (args.length === 0) return [{}, null];
  const first = args[0];
  let options = {};
  if (typeof first === 'object' && first !== null) {
    options = first;
  } else if (isPipeName(first)) {
    options.path = first;
  } else {
    options.port = first;
    if (args.length > 1 && typeof args[1] === 'string') options.host = args[1];
  }
  const listener = args[args.length - 1];
  return [options, typeof listener === 'function' ? listener : null];
}

class Socket extends Duplex {
  constructor(options = {}) {
    if (typeof options === 'number') options = { fd: options };
    else options = { ...options };
    super({
      ...options,
      allowHalfOpen: Boolean(options.allowHalfOpen),
      emitClose: false,
      autoDestroy: true,
      decodeStrings: false,
    });
    this.connecting = false;
    this._hadError = false;
    this._handle = null;
    this._host = null;
    this._peername = undefined;
    this._sockname = undefined;
    this.server = null;
    this._server = null;
    this[kTimeout] = null;
    this[kReading] = false;
    this[kRef] = true;
    this[kSetNoDelay] = Boolean(options.noDelay);
    this[kSetKeepAlive] = Boolean(options.keepAlive);
    this[kSetKeepAliveInitialDelay] = ~~(options.keepAliveInitialDelay / 1000);
    this[kBytesWritten] = 0;
    this[kBytesRead] = 0;
    if (options.fd !== undefined) {
      throw nodeError(Error, 'ERR_INVALID_ARG_VALUE', 'Sockets from file descriptors are not available');
    }
    if (options.handle) {
      this._attach(options.handle);
      if (options.readable !== false && !options.pauseOnCreate && !options.manualStart) {
        this.read(0);
      }
    }
    if (options.signal) {
      const { signal } = options;
      const abort = () => this.destroy(nodeError(Error, 'ABORT_ERR', 'The operation was aborted'));
      if (signal.aborted) process.nextTick(abort);
      else signal.addEventListener('abort', abort, { once: true });
    }
  }

  _attach(handle) {
    this._handle = handle;
    this._sockname = handle.localAddress();
    this._peername = handle.remoteAddress();
    if (!this[kRef]) handle.unref();
    if (this[kSetNoDelay]) handle.setNoDelay(true);
    if (this[kSetKeepAlive]) handle.setKeepAlive(true, this[kSetKeepAliveInitialDelay] * 1000);
  }

  get pending() {
    return !this._handle || this.connecting;
  }

  get readyState() {
    if (this.connecting) return 'opening';
    if (this.readable && this.writable) return 'open';
    if (this.readable && !this.writable) return 'readOnly';
    if (!this.readable && this.writable) return 'writeOnly';
    return 'closed';
  }

  get bufferSize() {
    return this._handle ? this.writableLength : undefined;
  }

  get bytesRead() {
    return this[kBytesRead];
  }

  get bytesWritten() {
    return this[kBytesWritten] + (this.writableLength || 0);
  }

  get remoteAddress() { return this._peername?.address; }
  get remoteFamily() { return this._peername?.family; }
  get remotePort() { return this._peername?.port; }
  get localAddress() { return this._sockname?.address; }
  get localFamily() { return this._sockname?.family; }
  get localPort() { return this._sockname?.port; }

  get _connecting() {
    return this.connecting;
  }

  address() {
    return this._sockname ? { ...this._sockname } : {};
  }

  connect(...args) {
    let normalized;
    if (Array.isArray(args[0]) && args.length === 1) normalized = args[0];
    else normalized = normalizeArgs(args);
    const [options, listener] = normalized;
    if (this.destroyed) {
      throw nodeError(Error, 'ERR_SOCKET_CLOSED', 'Socket is closed');
    }
    if (listener) this.once('connect', listener);
    if (options.timeout) this.setTimeout(options.timeout);
    if (options.path !== undefined && options.port === undefined) {
      this.connecting = true;
      this._pipeName = String(options.path);
      pipes().connect(this._pipeName, (error, handle) => this._onConnect(error, handle));
      return this;
    }
    const port = validatePort(options.port ?? 0);
    const host = options.host || 'localhost';
    this._host = host;
    this.connecting = true;
    if (options.noDelay !== undefined) this[kSetNoDelay] = Boolean(options.noDelay);
    if (options.keepAlive) {
      this[kSetKeepAlive] = true;
      this[kSetKeepAliveInitialDelay] = ~~((options.keepAliveInitialDelay || 0) / 1000);
    }
    const lookup = options.lookup;
    if (typeof lookup === 'function' && !isIP(host)) {
      lookup(host, { family: options.family || 0, hints: options.hints || 0, all: false },
             (error, address, family) => {
               this.emit('lookup', error, address, family, host);
               if (!this.connecting) return;
               if (error) {
                 this.connecting = false;
                 this.destroy(error);
                 return;
               }
               this._connectTo(address, port);
             });
    } else {
      this._connectTo(host, port);
    }
    return this;
  }

  _connectTo(host, port) {
    tcp.connect(host, port, (error, handle) => this._onConnect(error, handle));
  }

  _onConnect(error, handle) {
    if (!this.connecting || this.destroyed) {
      if (handle) handle.close();
      return;
    }
    this.connecting = false;
    if (error) {
      this.destroy(error);
      return;
    }
    this._attach(handle);
    this._refreshTimeout();
    this.emit('connect');
    this.emit('ready');
    if (this.readable && !this.isPaused()) this.read(0);
  }

  _read() {
    if (this.connecting || !this._handle) {
      this.once('connect', () => this._read());
      return;
    }
    if (this[kReading]) return;
    this[kReading] = true;
    this._handle.readStart((error, chunk) => this._onread(error, chunk));
  }

  _onread(error, chunk) {
    this._refreshTimeout();
    if (error) {
      this[kReading] = false;
      this.destroy(error);
      return;
    }
    if (chunk === null) {
      this[kReading] = false;
      this.push(null);
      this.read(0);
      return;
    }
    this[kBytesRead] += chunk.length;
    if (!this.push(Buffer.from(chunk.buffer, chunk.byteOffset, chunk.length))) {
      this[kReading] = false;
      this._handle?.readStop();
    }
  }

  _writeGeneric(data, callback) {
    if (this.connecting) {
      this.once('connect', () => this._writeGeneric(data, callback));
      return;
    }
    if (!this._handle) {
      callback(nodeError(Error, 'ERR_SOCKET_CLOSED', 'Socket is closed'));
      return;
    }
    this._refreshTimeout();
    this._handle.write(data, (error) => {
      if (error) {
        callback(error);
        return;
      }
      this[kBytesWritten] += data.length;
      this._refreshTimeout();
      callback();
    });
  }

  _write(chunk, encoding, callback) {
    this._writeGeneric(typeof chunk === 'string' ? Buffer.from(chunk, encoding) : chunk, callback);
  }

  _writev(chunks, callback) {
    const buffers = chunks.map(({ chunk, encoding }) =>
      typeof chunk === 'string' ? Buffer.from(chunk, encoding) : chunk);
    this._writeGeneric(Buffer.concat(buffers), callback);
  }

  _final(callback) {
    if (this.connecting) {
      this.once('connect', () => this._final(callback));
      return;
    }
    this._handle?.shutdown();
    callback();
  }

  _destroy(error, callback) {
    this._clearTimeout();
    this.connecting = false;
    const handle = this._handle;
    this._handle = null;
    if (handle) handle.close();
    this._hadError = Boolean(error);
    callback(error);
    process.nextTick(() => {
      this.emit('close', Boolean(error));
      const server = this._server;
      if (server) {
        this._server = null;
        server._connections--;
        server._emitCloseIfDrained();
      }
    });
  }

  end(data, encoding, callback) {
    super.end(data, encoding, callback);
    return this;
  }

  destroySoon() {
    if (this.writable) this.end();
    if (this.writableFinished) this.destroy();
    else this.once('finish', () => this.destroy());
  }

  resetAndDestroy() {
    return this.destroy();
  }

  setTimeout(timeout, callback) {
    timeout = Number(timeout);
    if (!Number.isFinite(timeout) || timeout < 0) {
      throw nodeError(RangeError, 'ERR_OUT_OF_RANGE', `The value of "timeout" is out of range. Received ${timeout}`);
    }
    this.timeout = timeout;
    this._clearTimeout();
    if (timeout === 0) {
      if (callback) this.removeListener('timeout', callback);
      return this;
    }
    this[kTimeout] = { ms: timeout, timer: null };
    this._refreshTimeout();
    if (callback) this.once('timeout', callback);
    return this;
  }

  _clearTimeout() {
    if (this[kTimeout]?.timer) clearTimeout(this[kTimeout].timer);
    this[kTimeout] = null;
  }

  _refreshTimeout() {
    const timeout = this[kTimeout];
    if (!timeout) return;
    if (timeout.timer) clearTimeout(timeout.timer);
    timeout.timer = setTimeout(() => {
      timeout.timer = null;
      this.emit('timeout');
    }, timeout.ms);
    timeout.timer.unref?.();
  }

  _unrefTimer() {
    this._refreshTimeout();
  }

  setNoDelay(enable) {
    this[kSetNoDelay] = enable === undefined ? true : Boolean(enable);
    this._handle?.setNoDelay(this[kSetNoDelay]);
    return this;
  }

  setKeepAlive(enable, initialDelay = 0) {
    this[kSetKeepAlive] = Boolean(enable);
    this[kSetKeepAliveInitialDelay] = ~~(initialDelay / 1000);
    this._handle?.setKeepAlive(this[kSetKeepAlive], this[kSetKeepAliveInitialDelay] * 1000);
    return this;
  }

  ref() {
    this[kRef] = true;
    this._handle?.ref();
    return this;
  }

  unref() {
    this[kRef] = false;
    this._handle?.unref();
    return this;
  }
}

class Server extends EventEmitter {
  constructor(options, connectionListener) {
    super();
    if (typeof options === 'function') {
      connectionListener = options;
      options = {};
    } else if (options == null || typeof options === 'object') {
      options = { ...options };
    } else {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be of type object.');
    }
    if (typeof connectionListener === 'function') this.on('connection', connectionListener);
    this._connections = 0;
    this._handle = null;
    this._unref = false;
    this.allowHalfOpen = options.allowHalfOpen || false;
    this.pauseOnConnect = Boolean(options.pauseOnConnect);
    this.noDelay = Boolean(options.noDelay);
    this.keepAlive = Boolean(options.keepAlive);
    this.keepAliveInitialDelay = ~~((options.keepAliveInitialDelay || 0) / 1000);
    this.highWaterMark = options.highWaterMark;
  }

  get listening() {
    return Boolean(this._handle);
  }

  listen(...args) {
    const [options, listener] = normalizeArgs(args);
    if (listener) this.once('listening', listener);
    if (this._handle) {
      throw nodeError(Error, 'ERR_SERVER_ALREADY_LISTEN', 'Listen method has been called more than once without closing.');
    }
    if (options.path !== undefined && options.port === undefined) {
      try {
        this._handle = pipes().listen(String(options.path));
      } catch (error) {
        process.nextTick(() => this.emit('error', error));
        return this;
      }
      this._pipeName = String(options.path);
      if (this._unref) this._handle.unref();
      process.nextTick(() => {
        if (this._handle) this.emit('listening');
      });
      this._accept();
      return this;
    }
    const port = options.port === undefined || options.port === null ? 0 : validatePort(options.port, 'options.port');
    const backlog = toNumber(options.backlog) ||
      toNumber(args.length > 1 && args[1]) || toNumber(args.length > 2 && args[2]) || 511;
    try {
      this._handle = tcp.listen(options.host || '', port, backlog, Boolean(options.ipv6Only));
    } catch (error) {
      process.nextTick(() => this.emit('error', error));
      return this;
    }
    if (this._unref) this._handle.unref();
    process.nextTick(() => {
      if (this._handle) this.emit('listening');
    });
    this._accept();
    return this;
  }

  _accept() {
    const handle = this._handle;
    if (!handle) return;
    handle.accept((error, connection) => {
      if (handle !== this._handle) {
        connection?.close();
        return;
      }
      this._accept();
      if (error) {
        this.emit('error', error);
        return;
      }
      if (this.maxConnections != null && this._connections >= this.maxConnections) {
        const local = connection.localAddress() || {};
        const remote = connection.remoteAddress() || {};
        connection.close();
        this.emit('drop', {
          localAddress: local.address,
          localPort: local.port,
          localFamily: local.family,
          remoteAddress: remote.address,
          remotePort: remote.port,
          remoteFamily: remote.family,
        });
        return;
      }
      const socket = new Socket({
        handle: connection,
        allowHalfOpen: this.allowHalfOpen,
        pauseOnCreate: this.pauseOnConnect,
        readable: true,
        writable: true,
        readableHighWaterMark: this.highWaterMark,
        writableHighWaterMark: this.highWaterMark,
      });
      if (this.noDelay) socket.setNoDelay(true);
      if (this.keepAlive) socket.setKeepAlive(true, this.keepAliveInitialDelay * 1000);
      this._connections++;
      socket.server = this;
      socket._server = this;
      this.emit('connection', socket);
    });
  }

  address() {
    if (!this._handle) return null;
    // A pipe server's address is its name, as in Node.
    if (this._pipeName !== undefined) return this._pipeName;
    return this._handle.address();
  }

  close(callback) {
    if (typeof callback === 'function') {
      if (!this._handle) {
        this.once('close', () => callback(nodeError(Error, 'ERR_SERVER_NOT_RUNNING', 'Server is not running.')));
      } else {
        this.once('close', callback);
      }
    }
    if (this._handle) {
      this._handle.close();
      this._handle = null;
    }
    this._emitCloseIfDrained();
    return this;
  }

  _emitCloseIfDrained() {
    if (this._handle || this._connections) return;
    process.nextTick(() => this.emit('close'));
  }

  getConnections(callback) {
    process.nextTick(callback, null, this._connections);
    return this;
  }

  ref() {
    this._unref = false;
    this._handle?.ref();
    return this;
  }

  unref() {
    this._unref = true;
    this._handle?.unref();
    return this;
  }

  [Symbol.asyncDispose]() {
    if (!this._handle) return Promise.resolve();
    return new Promise((resolve, reject) => this.close((error) => error ? reject(error) : resolve()));
  }
}

function connect(...args) {
  const normalized = normalizeArgs(args);
  const [options] = normalized;
  const socket = new Socket(options);
  if (options.timeout) socket.setTimeout(options.timeout);
  return socket.connect(normalized);
}

function createServer(options, connectionListener) {
  return new Server(options, connectionListener);
}

let autoSelectFamily = true;
let autoSelectFamilyAttemptTimeout = 250;

module.exports = {
  _normalizeArgs: normalizeArgs,
  Socket,
  Stream: Socket,
  Server,
  connect,
  createConnection: connect,
  createServer,
  isIP,
  isIPv4,
  isIPv6,
  getDefaultAutoSelectFamily() { return autoSelectFamily; },
  setDefaultAutoSelectFamily(value) { autoSelectFamily = Boolean(value); },
  getDefaultAutoSelectFamilyAttemptTimeout() { return autoSelectFamilyAttemptTimeout; },
  setDefaultAutoSelectFamilyAttemptTimeout(value) { autoSelectFamilyAttemptTimeout = value; },
};
