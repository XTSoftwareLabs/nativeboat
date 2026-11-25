'use strict';

// Electron's `utilityProcess`, `MessageChannelMain` and a utility process's
// `process.parentPort`. A forked module is a root of the program's main
// graph, so the child is the program's own executable, started with the
// fork's index in NATIVEBOAT_FORK_ENTRY; it connects back to a loopback
// server of its parent with a one-time token. Messages travel as
// length-prefixed frames of JSON that encodes structured-clone values, and
// transferred `MessagePortMain`s keep working across the link: each moved
// port gets a channel id, and messages for it travel on the link too.

const EventEmitter = require('events');
const net = require('net');
const { spawn } = require('child_process');
const { PassThrough } = require('stream');
const { Buffer } = require('buffer');
const { URL, fileURLToPath } = require('internal/url');

const forkBinding = internalBinding('fork');
let portsBinding;
const nativePorts = () => (portsBinding ??= internalBinding('message_ports'));

const PORT_VARIABLE = 'NATIVEBOAT_FORK_PORT';
const TOKEN_VARIABLE = 'NATIVEBOAT_FORK_TOKEN';
const KIND_VARIABLE = 'NATIVEBOAT_FORK_KIND';

function cloneError(message) {
  const error = new Error(message);
  error.name = 'DataCloneError';
  error.code = 25;
  return error;
}

// Structured-clone values as JSON: primitives JSON has stay as they are,
// everything else is a tagged object. Objects are numbered as they are met,
// so shared and cyclic references come back shared.
function encode(value, seen = new Map()) {
  switch (typeof value) {
    case 'string':
    case 'boolean':
      return value;
    case 'number':
      if (Number.isNaN(value)) return { $: 'n', v: 'NaN' };
      if (value === Infinity) return { $: 'n', v: 'Infinity' };
      if (value === -Infinity) return { $: 'n', v: '-Infinity' };
      if (Object.is(value, -0)) return { $: 'n', v: '-0' };
      return value;
    case 'undefined':
      return { $: 'u' };
    case 'bigint':
      return { $: 'g', v: value.toString() };
    case 'function':
      throw cloneError(`${value.name || 'function'} could not be cloned.`);
    case 'symbol':
      throw cloneError(`${String(value)} could not be cloned.`);
    default:
      break;
  }
  if (value === null) return null;
  if (seen.has(value)) return { $: 'r', i: seen.get(value) };
  const i = seen.size;
  seen.set(value, i);
  if (value instanceof MessagePortMain) {
    throw cloneError('A MessagePortMain could not be cloned without being transferred.');
  }
  if (Array.isArray(value)) {
    const v = new Array(value.length);
    for (let index = 0; index < value.length; index++) v[index] = encode(value[index], seen);
    return { $: 'a', i, v };
  }
  if (value instanceof Date) return { $: 'd', i, v: value.getTime() };
  if (value instanceof RegExp) return { $: 'x', i, s: value.source, f: value.flags };
  if (value instanceof Map) {
    const v = [];
    for (const [key, entry] of value) v.push([encode(key, seen), encode(entry, seen)]);
    return { $: 'm', i, v };
  }
  if (value instanceof Set) {
    const v = [];
    for (const entry of value) v.push(encode(entry, seen));
    return { $: 's', i, v };
  }
  if (value instanceof ArrayBuffer) {
    return { $: 'B', i, v: Buffer.from(value).toString('base64') };
  }
  if (ArrayBuffer.isView(value)) {
    const bytes = Buffer.from(value.buffer, value.byteOffset, value.byteLength);
    const type = value instanceof DataView ? 'DataView' :
      value instanceof Buffer ? 'Uint8Array' : value.constructor.name;
    return { $: 'T', i, t: type, v: bytes.toString('base64') };
  }
  if (value instanceof Error) {
    const encoded = { $: 'e', i, n: value.name, m: value.message, s: value.stack };
    if ('cause' in value) encoded.c = encode(value.cause, seen);
    return encoded;
  }
  if (value instanceof Boolean) return { $: 'O', i, t: 'b', v: value.valueOf() };
  if (value instanceof Number) return { $: 'O', i, t: 'n', v: encode(value.valueOf(), seen) };
  if (value instanceof String) return { $: 'O', i, t: 's', v: value.valueOf() };
  const v = {};
  for (const key of Object.keys(value)) v[key] = encode(value[key], seen);
  return { $: 'o', i, v };
}

const ERRORS = {
  Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError,
};

function decode(node, made = []) {
  if (node === null || typeof node !== 'object') return node;
  switch (node.$) {
    case 'u': return undefined;
    case 'n': return node.v === '-0' ? -0 : Number(node.v);
    case 'g': return BigInt(node.v);
    case 'r': return made[node.i];
    case 'a': {
      const array = new Array(node.v.length);
      made[node.i] = array;
      for (let index = 0; index < node.v.length; index++) array[index] = decode(node.v[index], made);
      return array;
    }
    case 'd': return (made[node.i] = new Date(node.v));
    case 'x': return (made[node.i] = new RegExp(node.s, node.f));
    case 'm': {
      const map = new Map();
      made[node.i] = map;
      for (const [key, entry] of node.v) map.set(decode(key, made), decode(entry, made));
      return map;
    }
    case 's': {
      const set = new Set();
      made[node.i] = set;
      for (const entry of node.v) set.add(decode(entry, made));
      return set;
    }
    case 'B': {
      const bytes = Buffer.from(node.v, 'base64');
      const buffer = new ArrayBuffer(bytes.length);
      new Uint8Array(buffer).set(bytes);
      return (made[node.i] = buffer);
    }
    case 'T': {
      const bytes = Buffer.from(node.v, 'base64');
      const buffer = new ArrayBuffer(bytes.length);
      new Uint8Array(buffer).set(bytes);
      const Type = node.t === 'DataView' ? DataView : globalThis[node.t] || Uint8Array;
      const view = Type === DataView ? new DataView(buffer) :
        new Type(buffer, 0, bytes.length / (Type.BYTES_PER_ELEMENT || 1));
      return (made[node.i] = view);
    }
    case 'e': {
      const Type = ERRORS[node.n] || Error;
      const error = new Type(node.m);
      made[node.i] = error;
      if (node.n && error.name !== node.n) error.name = node.n;
      Object.defineProperty(error, 'stack', {
        value: node.s, writable: true, configurable: true, enumerable: false,
      });
      if ('c' in node) {
        Object.defineProperty(error, 'cause', {
          value: decode(node.c, made), writable: true, configurable: true, enumerable: false,
        });
      }
      return error;
    }
    case 'O': {
      const value = node.t === 'b' ? new Boolean(node.v) :
        node.t === 'n' ? new Number(decode(node.v, made)) : new String(node.v);
      return (made[node.i] = value);
    }
    case 'o': {
      const object = {};
      made[node.i] = object;
      for (const key of Object.keys(node.v)) object[key] = decode(node.v[key], made);
      return object;
    }
    default:
      return undefined;
  }
}

function transferList(transfer) {
  if (transfer === undefined || transfer === null) return [];
  if (!Array.isArray(transfer)) {
    throw new TypeError('The transfer list must be an array of MessagePortMain objects');
  }
  const seen = new Set();
  for (const port of transfer) {
    if (!(port instanceof MessagePortMain)) {
      throw new TypeError('Only MessagePortMain objects can be transferred');
    }
    if (seen.has(port) || port._detached) {
      throw cloneError('A MessagePortMain was transferred twice or after it was closed.');
    }
    seen.add(port);
  }
  return transfer;
}

// The far end of a port that lives in the process across `link`.
class RemoteEnd {
  constructor(link, id) {
    this.link = link;
    this.id = id;
  }

  _deliver(encoded, ports) {
    this.link._send({ t: 'c', c: this.id, m: encoded, p: this.link._movePorts(ports) });
  }

  _peerClosed() {
    this.link._send({ t: 'x', c: this.id });
    this.link.channels.delete(this.id);
  }
}

// Repoints whatever delivers to `port` so it delivers to `target` instead.
function redirectSender(port, target) {
  const sender = port._peer;
  if (sender instanceof RemoteEnd) {
    sender.link.channels.set(sender.id, target);
  } else if (sender) {
    sender._peer = target;
  }
}

class MessagePortMain extends EventEmitter {
  constructor() {
    super();
    this._peer = null;
    this._queue = [];
    this._started = false;
    this._closed = false;
    this._detached = false;
  }

  postMessage(message, transfer) {
    if (this._closed || this._detached) return;
    const ports = transferList(transfer);
    const encoded = encode(message);
    if (this._peer) this._peer._deliver(encoded, ports);
    else for (const port of ports) port._detach();
  }

  start() {
    if (this._started) return;
    this._started = true;
    this._flush();
  }

  close() {
    if (this._closed || this._detached) return;
    this._closed = true;
    const peer = this._peer;
    this._peer = null;
    if (peer) peer._peerClosed();
  }

  // A message from the peer: data still encoded, so each receiver gets its
  // own copy, and the ports moved with it.
  _deliver(encoded, ports) {
    if (this._closed) return;
    this._queue.push({ encoded, ports });
    if (this._started) this._flush();
  }

  _flush() {
    if (this._flushing) return;
    this._flushing = true;
    setImmediate(() => {
      this._flushing = false;
      while (this._started && !this._closed && this._queue.length > 0) {
        const { encoded, ports } = this._queue.shift();
        this.emit('message', { data: decode(encoded), ports });
      }
    });
  }

  // The peer closed: what it posted before closing still arrives, then
  // 'close'.
  _peerClosed() {
    if (this._closed || this._peerGone) return;
    this._peerGone = true;
    this._peer = null;
    setImmediate(() => {
      this._closed = true;
      this.emit('close');
    });
  }

  // Moved to another holder: this object no longer sends or receives.
  _detach() {
    this._detached = true;
    this._peer = null;
    this._queue.length = 0;
  }
}

// The near end of a channel whose far end lives in another realm of this
// process, a window's preload or page: what is posted crosses as a value the
// runtime clones there. `_peer` gets what arrives from the far end;
// `redirectSender` repoints it when that port moves on.
class NativeEnd {
  constructor(handle, peer) {
    this.handle = handle;
    this._peer = peer;
    handle.bind({
      _deliver: (value, ports) => {
        if (this._peer) this._peer._deliver(encode(value), ports);
        else for (const port of ports) port.close();
      },
      _peerClosed: () => {
        const peer = this._peer;
        this._peer = null;
        if (peer) peer._peerClosed();
      },
    });
  }

  _deliver(encoded, ports) {
    this.handle.postMessage(decode(encoded), ports.map(exportPort));
  }

  _peerClosed() {
    this._peer = null;
    this.handle.close();
  }
}

// Moves a port to another realm: whatever delivered to it delivers to a new
// channel's near end, and the far end, returned, moves.
function exportPort(port) {
  if (!(port instanceof MessagePortMain)) {
    throw new TypeError('Only MessagePortMain objects can be transferred');
  }
  if (port._detached || port._closed) {
    throw cloneError('A MessagePortMain was transferred twice or after it was closed.');
  }
  const [near, far] = nativePorts().channel();
  const end = new NativeEnd(near, port._peer);
  redirectSender(port, end);
  port._detach();
  return far;
}

// A port of this realm for an end that moved here from another realm.
function adoptNativePort(handle) {
  const port = new MessagePortMain();
  port._peer = new NativeEnd(handle, port);
  return port;
}

class MessageChannelMain {
  constructor() {
    this.port1 = new MessagePortMain();
    this.port2 = new MessagePortMain();
    this.port1._peer = this.port2;
    this.port2._peer = this.port1;
  }
}

// One socket between a parent and a forked child. `prefix` keeps the
// channel ids each side assigns apart.
class Link {
  constructor(prefix, onMessage, onClose) {
    this.prefix = prefix;
    this.next = 0;
    this.channels = new Map();
    this.socket = null;
    this.pending = [];
    this.input = Buffer.alloc(0);
    this.onMessage = onMessage;
    this.onClose = onClose;
    this.closed = false;
  }

  attach(socket) {
    this.socket = socket;
    socket.setNoDelay?.(true);
    socket.on('data', (chunk) => this._receive(chunk));
    socket.on('error', () => {});
    socket.on('close', () => this._closed());
    for (const frame of this.pending) socket.write(frame);
    this.pending.length = 0;
  }

  _send(message) {
    if (this.closed) return;
    const body = Buffer.from(JSON.stringify(message), 'utf8');
    const frame = Buffer.alloc(4 + body.length);
    frame.writeUInt32LE(body.length, 0);
    body.copy(frame, 4);
    if (this.socket) this.socket.write(frame);
    else this.pending.push(frame);
  }

  // Moves transferred ports to the other side: what delivered to each port
  // now delivers across the link, and what arrives for its channel goes to
  // where the port sent.
  _movePorts(ports) {
    const ids = [];
    for (const port of ports) {
      const id = `${this.prefix}${this.next++}`;
      const peer = port._peer;
      redirectSender(port, new RemoteEnd(this, id));
      if (peer) this.channels.set(id, peer);
      port._detach();
      ids.push(id);
    }
    return ids;
  }

  // Ports that arrived: each sends across the link on its channel, and the
  // channel's messages are delivered to it.
  _receivePorts(ids) {
    return (ids || []).map((id) => {
      const port = new MessagePortMain();
      port._peer = new RemoteEnd(this, id);
      this.channels.set(id, port);
      return port;
    });
  }

  _receive(chunk) {
    this.input = this.input.length === 0 ? chunk : Buffer.concat([this.input, chunk]);
    while (this.input.length >= 4) {
      const length = this.input.readUInt32LE(0);
      if (this.input.length < 4 + length) break;
      const text = this.input.toString('utf8', 4, 4 + length);
      this.input = this.input.subarray(4 + length);
      let frame;
      try {
        frame = JSON.parse(text);
      } catch {
        continue;
      }
      this._frame(frame);
    }
  }

  _frame(frame) {
    switch (frame.t) {
      case 'm':
        this.onMessage(frame.m, this._receivePorts(frame.p));
        break;
      case 'c': {
        const ports = this._receivePorts(frame.p);
        const target = this.channels.get(frame.c);
        if (target) target._deliver(frame.m, ports);
        break;
      }
      case 'x': {
        const target = this.channels.get(frame.c);
        this.channels.delete(frame.c);
        if (target) target._peerClosed();
        break;
      }
      default:
        break;
    }
  }

  _closed() {
    if (this.closed) return;
    this.closed = true;
    const channels = [...this.channels.values()];
    this.channels.clear();
    for (const target of channels) target._peerClosed();
    this.onClose();
  }

  postMessage(message, transfer) {
    const ports = transferList(transfer);
    this._send({ t: 'm', m: encode(message), p: this._movePorts(ports) });
  }

  close() {
    this.socket?.destroy();
  }
}

function token() {
  let text = '';
  for (let index = 0; index < 4; index++) {
    text += Math.floor(Math.random() * 0x100000000).toString(16).padStart(8, '0');
  }
  return text;
}

// A stream a caller has as soon as the child is created, which passes on
// what the child's standard handle carries once the process has started.
function relay(enabled) {
  return enabled ? new PassThrough() : null;
}

function connect(source, relayed) {
  if (!source) {
    relayed?.end();
  } else if (relayed) {
    source.on('data', (chunk) => relayed.write(chunk));
    source.on('end', () => relayed.end());
  } else {
    source.resume?.();
  }
}

function piped(stdio, index) {
  if (stdio === 'pipe') return true;
  return Array.isArray(stdio) && stdio[index] === 'pipe';
}

class UtilityProcess extends EventEmitter {
  constructor(entry, modulePath, args, options) {
    super();
    this._child = null;
    this._exited = false;
    this.stdout = relay(piped(options.stdio, 1));
    this.stderr = relay(piped(options.stdio, 2));
    this._link = new Link('p', (encoded) => {
      this.emit('message', decode(encoded));
    }, () => {});
    const secret = token();
    const server = net.createServer((socket) => {
      let hello = Buffer.alloc(0);
      const onData = (chunk) => {
        hello = Buffer.concat([hello, chunk]);
        const line = hello.indexOf(10);
        if (line < 0) {
          if (hello.length > 256) socket.destroy();
          return;
        }
        socket.removeListener('data', onData);
        if (hello.toString('utf8', 0, line) !== secret) {
          socket.destroy();
          return;
        }
        server.close();
        const rest = hello.subarray(line + 1);
        this._link.attach(socket);
        if (rest.length > 0) this._link._receive(rest);
      };
      socket.on('data', onData);
      socket.on('error', () => {});
    });
    this._server = server;
    server.on('error', (error) => this._fail(error));
    server.listen(0, '127.0.0.1', () => {
      const env = { ...(options.env || process.env) };
      env[forkBinding.entryVariable()] = String(entry);
      env[PORT_VARIABLE] = String(server.address().port);
      env[TOKEN_VARIABLE] = secret;
      const child = spawn(process.execPath, args, {
        cwd: options.cwd,
        env,
        windowsHide: true,
      });
      this._child = child;
      connect(child.stdout, this.stdout);
      connect(child.stderr, this.stderr);
      child.on('spawn', () => this.emit('spawn'));
      child.on('error', (error) => this._fail(error));
      child.on('exit', (code) => this._exit(typeof code === 'number' ? code : 1));
    });
    this._modulePath = modulePath;
  }

  get pid() {
    return this._exited ? undefined : this._child?.pid;
  }

  postMessage(message, transfer) {
    if (this._exited) return;
    this._link.postMessage(message, transfer);
  }

  kill() {
    if (!this._child || this._exited) return false;
    return this._child.kill();
  }

  _fail(error) {
    if (this._exited) return;
    this._server?.close();
    this.emit('error', 'FatalError', this._modulePath, String(error && error.message));
    this._exit(1);
  }

  _exit(code) {
    if (this._exited) return;
    this._exited = true;
    this._server?.close();
    this._link.close();
    this.emit('exit', code);
  }
}

const utilityProcess = {
  fork(modulePath, args, options) {
    if (typeof modulePath !== 'string') {
      throw new TypeError('The "modulePath" argument must be of type string');
    }
    if (args !== undefined && !Array.isArray(args)) {
      throw new TypeError('The "args" argument must be an array');
    }
    const entry = forkBinding.entryFor(modulePath);
    if (entry < 0) {
      throw new Error(`utilityProcess.fork: '${modulePath}' is not a module this application compiled for forking`);
    }
    return new UtilityProcess(entry, modulePath, (args || []).map(String), options || {});
  },
};

// `process.parentPort` of a forked child: connects to the parent named by
// the environment, then forgets the connection's variables so the child's
// own children do not inherit them.
class ParentPort extends EventEmitter {
  constructor() {
    super();
    this._started = false;
    this._queue = [];
    this._link = new Link('c', (encoded, ports) => {
      this._queue.push({ encoded, ports });
      this._flush();
    }, () => process.exit(0));
    this.on('newListener', (event) => {
      if (event === 'message') this.start();
    });
  }

  start() {
    if (this._started) return;
    this._started = true;
    this._flush();
  }

  _flush() {
    if (!this._started || this._flushing) return;
    this._flushing = true;
    setImmediate(() => {
      this._flushing = false;
      while (this._queue.length > 0) {
        const { encoded, ports } = this._queue.shift();
        this.emit('message', { data: decode(encoded), ports });
      }
    });
  }

  postMessage(message, transfer) {
    this._link.postMessage(message, transfer);
  }
}

// Node's `child_process.fork`: the same child, with Node's IPC channel
// instead of a parent port. Messages are JSON unless `serialization` is
// 'advanced', which is the structured clone.
function channelClosed() {
  const error = new Error('Channel closed');
  error.code = 'ERR_IPC_CHANNEL_CLOSED';
  return error;
}

function ipcMessage(message, advanced) {
  if (message === undefined) {
    const error = new TypeError('The "message" argument must be specified');
    error.code = 'ERR_MISSING_ARGS';
    throw error;
  }
  return advanced ? message : JSON.parse(JSON.stringify(message));
}

// `send(message[, sendHandle][, options][, callback])`: handles are not
// passed between processes.
function sendArguments(handle, options, callback) {
  if (typeof handle === 'function') return handle;
  if (typeof options === 'function') return options;
  return typeof callback === 'function' ? callback : undefined;
}

function ipcSend(owner, link, advanced, message, handle, options, callback) {
  const done = sendArguments(handle, options, callback);
  const value = ipcMessage(message, advanced);
  if (!owner.connected) {
    const error = channelClosed();
    if (done) process.nextTick(done, error);
    else process.nextTick(() => owner.emit('error', error));
    return false;
  }
  link.postMessage(value);
  if (done) process.nextTick(done, null);
  return true;
}

class ChildProcess extends EventEmitter {
  constructor(entry, modulePath, args, options) {
    super();
    this.connected = true;
    this.exitCode = null;
    this.signalCode = null;
    this.killed = false;
    this.spawnfile = process.execPath;
    this.spawnargs = [process.execPath, modulePath, ...args];
    this.stdin = null;
    this.stdout = null;
    this.stderr = null;
    this._child = null;
    this._exited = false;
    this._advanced = options.serialization === 'advanced';
    this._link = new Link('p', (encoded) => {
      this.emit('message', decode(encoded), undefined);
    }, () => this._disconnected());
    const silent = options.silent === true;
    if (entry >= 0) {
      this.stdin = relay(silent || piped(options.stdio, 0));
      this.stdout = relay(silent || piped(options.stdio, 1));
      this.stderr = relay(silent || piped(options.stdio, 2));
    }
    if (entry < 0) {
      // Node starts and fails to find the module: the child exits at once.
      process.nextTick(() => {
        this._disconnected();
        this._exit(1, null);
      });
      return;
    }
    const secret = token();
    const server = net.createServer((socket) => {
      let hello = Buffer.alloc(0);
      const onData = (chunk) => {
        hello = Buffer.concat([hello, chunk]);
        const line = hello.indexOf(10);
        if (line < 0) {
          if (hello.length > 256) socket.destroy();
          return;
        }
        socket.removeListener('data', onData);
        if (hello.toString('utf8', 0, line) !== secret) {
          socket.destroy();
          return;
        }
        server.close();
        const rest = hello.subarray(line + 1);
        this._link.attach(socket);
        if (rest.length > 0) this._link._receive(rest);
      };
      socket.on('data', onData);
      socket.on('error', () => {});
    });
    this._server = server;
    server.on('error', (error) => this.emit('error', error));
    server.listen(0, '127.0.0.1', () => {
      const env = { ...(options.env || process.env) };
      env[forkBinding.entryVariable()] = String(entry);
      env[PORT_VARIABLE] = String(server.address().port);
      env[TOKEN_VARIABLE] = secret;
      env[KIND_VARIABLE] = this._advanced ? 'node-advanced' : 'node';
      const child = spawn(process.execPath, args, { cwd: options.cwd, env, windowsHide: true });
      this._child = child;
      if (this.stdin && child.stdin) {
        this.stdin.on('data', (chunk) => child.stdin.write(chunk));
        this.stdin.on('end', () => child.stdin.end());
      }
      connect(child.stdout, this.stdout);
      connect(child.stderr, this.stderr);
      child.on('spawn', () => this.emit('spawn'));
      child.on('error', (error) => this.emit('error', error));
      child.on('exit', (code, signal) => this._exit(code, signal));
    });
  }

  get pid() {
    return this._child?.pid;
  }

  send(message, handle, options, callback) {
    return ipcSend(this, this._link, this._advanced, message, handle, options, callback);
  }

  disconnect() {
    if (!this.connected) {
      this.emit('error', channelClosed());
      return;
    }
    this._link.close();
    this._disconnected();
  }

  kill(signal) {
    if (!this._child || this._exited) return false;
    const killed = this._child.kill(signal);
    if (killed) this.killed = true;
    return killed;
  }

  ref() {
    this._child?.ref?.();
  }

  unref() {
    this._child?.unref?.();
  }

  _disconnected() {
    if (!this.connected) return;
    this.connected = false;
    process.nextTick(() => this.emit('disconnect'));
  }

  _exit(code, signal) {
    if (this._exited) return;
    this._exited = true;
    this.exitCode = typeof code === 'number' ? code : null;
    this.signalCode = signal || null;
    this._server?.close();
    this._link.close();
    this._disconnected();
    process.nextTick(() => {
      this.emit('exit', this.exitCode, this.signalCode);
      this.emit('close', this.exitCode, this.signalCode);
    });
  }
}

// `fork(modulePath[, args][, options])`.
function fork(modulePath, args, options) {
  if (typeof modulePath !== 'string' && !(modulePath instanceof URL)) {
    const error = new TypeError('The "modulePath" argument must be of type string or an instance of URL');
    error.code = 'ERR_INVALID_ARG_TYPE';
    throw error;
  }
  const path = typeof modulePath === 'string' ? modulePath : fileURLToPath(modulePath);
  if (args !== undefined && args !== null && !Array.isArray(args)) {
    options = args;
    args = [];
  }
  const entry = forkBinding.entryFor(path);
  return new ChildProcess(entry, path, (args || []).map(String), options || {});
}

// The child side of Node's channel: `process.send`, `process.connected`,
// `process.disconnect()`, and `message` and `disconnect` events on
// `process`.
function connectNodeParent(link, advanced) {
  process.connected = true;
  process.send = (message, handle, options, callback) =>
    ipcSend(process, link, advanced, message, handle, options, callback);
  process.disconnect = () => {
    if (!process.connected) return;
    link.close();
  };
}

// Connects a forked child to its parent, as the environment names it, and
// forgets the connection's variables so the child's own children do not
// inherit them. A utility process gets `process.parentPort`; a Node child
// gets Node's channel.
function connectParent() {
  const port = Number(process.env[PORT_VARIABLE]);
  const secret = process.env[TOKEN_VARIABLE] || '';
  const kind = process.env[KIND_VARIABLE];
  delete process.env[PORT_VARIABLE];
  delete process.env[TOKEN_VARIABLE];
  delete process.env[KIND_VARIABLE];
  delete process.env[forkBinding.entryVariable()];
  const socket = net.connect(port, '127.0.0.1');
  socket.write(`${secret}\n`);
  if (kind === 'node' || kind === 'node-advanced') {
    const link = new Link('c', (encoded) => {
      process.emit('message', decode(encoded), undefined);
    }, () => {
      process.connected = false;
      process.emit('disconnect');
    });
    connectNodeParent(link, kind === 'node-advanced');
    link.attach(socket);
    return;
  }
  const parentPort = new ParentPort();
  parentPort._link.attach(socket);
  Object.defineProperty(process, 'parentPort', {
    value: parentPort,
    writable: true,
    configurable: true,
    enumerable: true,
  });
}

module.exports = {
  utilityProcess,
  MessageChannelMain,
  MessagePortMain,
  fork,
  connectParent,
  encode,
  decode,
  _exportPort: exportPort,
  _adoptNativePort: adoptNativePort,
};
