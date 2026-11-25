'use strict';

// nativeboat: the WHATWG WebSocket client Node.js v22 provides as a global,
// over WinHTTP's WebSocket API (internalBinding('websocket')), which brings
// the system's TLS, proxy and certificate settings to ws: and wss: URLs.
// Messages are whole: fragments are joined before a message event.

const { Blob, Buffer } = require('buffer');
const { URL } = require('internal/url');
const { DOMException } = require('internal/per_context/domexception');
const { MessageEvent } = require('internal/worker/io');

const binding = internalBinding('websocket');

const CONNECTING = 0;
const OPEN = 1;
const CLOSING = 2;
const CLOSED = 3;

const kState = Symbol('kState');
const kHandlers = Symbol('kHandlers');

// A token, as a subprotocol name must be.
const TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

class CloseEvent extends Event {
  constructor(type, init = undefined) {
    super(type, init);
    const { wasClean = false, code = 0, reason = '' } = init ?? {};
    for (const [name, value] of [
      ['wasClean', Boolean(wasClean)],
      ['code', Number(code) & 0xffff],
      ['reason', `${reason}`],
    ]) {
      Object.defineProperty(this, name, { value, enumerable: true, configurable: true });
    }
  }
}

class ErrorEvent extends Event {
  constructor(type, init = undefined) {
    super(type, init);
    const { message = '', error } = init ?? {};
    Object.defineProperty(this, 'message', { value: `${message}`, enumerable: true, configurable: true });
    Object.defineProperty(this, 'error', { value: error, enumerable: true, configurable: true });
  }
}

function parseUrl(url) {
  let parsed;
  try {
    parsed = new URL(`${url}`);
  } catch {
    throw new DOMException(`Invalid URL: ${url}`, 'SyntaxError');
  }
  if (parsed.protocol === 'http:') parsed.protocol = 'ws:';
  else if (parsed.protocol === 'https:') parsed.protocol = 'wss:';
  if (parsed.protocol !== 'ws:' && parsed.protocol !== 'wss:') {
    throw new DOMException(`The URL's scheme must be either 'http', 'https', 'ws', or 'wss'. '${parsed.protocol}' is not allowed.`, 'SyntaxError');
  }
  if (parsed.hash !== '' || `${url}`.endsWith('#')) {
    throw new DOMException('Fragments are not allowed in a WebSocket URL', 'SyntaxError');
  }
  return parsed;
}

function parseProtocols(protocols) {
  if (protocols === undefined) return [];
  const list = typeof protocols === 'string' ? [protocols] : Array.from(protocols, (protocol) => `${protocol}`);
  const seen = new Set();
  for (const protocol of list) {
    if (!TOKEN.test(protocol) || seen.has(protocol.toLowerCase())) {
      throw new DOMException(`Invalid Sec-WebSocket-Protocol value: ${protocol}`, 'SyntaxError');
    }
    seen.add(protocol.toLowerCase());
  }
  return list;
}

// The bytes of a message to send, and whether it is text; a Blob is read
// first, so the promise keeps the order of sends.
function payload(data) {
  if (typeof data === 'string') return { text: true, bytes: Buffer.from(data, 'utf8') };
  if (data instanceof ArrayBuffer) return { text: false, bytes: new Uint8Array(data.slice(0)) };
  if (ArrayBuffer.isView(data)) {
    return { text: false, bytes: new Uint8Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength)) };
  }
  if (data instanceof Blob) {
    return { text: false, size: data.size, bytes: data.arrayBuffer().then((buffer) => new Uint8Array(buffer)) };
  }
  return { text: true, bytes: Buffer.from(`${data}`, 'utf8') };
}

class WebSocket extends EventTarget {
  constructor(url, protocols = undefined) {
    super();
    if (arguments.length === 0) {
      throw new TypeError("Failed to construct 'WebSocket': 1 argument required, but only 0 present.");
    }
    if (protocols !== null && typeof protocols === 'object' && !Array.isArray(protocols) && !(Symbol.iterator in protocols)) {
      protocols = protocols.protocols;
    }
    const parsed = parseUrl(url);
    const list = parseProtocols(protocols);
    Object.defineProperty(this, kState, {
      value: {
        url: parsed.href,
        readyState: CONNECTING,
        protocol: '',
        extensions: '',
        binaryType: 'blob',
        bufferedAmount: 0,
        socket: null,
        sending: Promise.resolve(),
        closeCode: 1005,
        closeReason: '',
        closing: false,
      },
    });
    const target = new URL(parsed.href);
    target.protocol = parsed.protocol === 'wss:' ? 'https:' : 'http:';
    const headers = [];
    if (list.length > 0) headers.push('Sec-WebSocket-Protocol', list.join(', '));
    binding.connect(target.href, headers, (error, socket, protocol, extensions) => {
      const state = this[kState];
      if (error) {
        this._fail(error);
        return;
      }
      if (state.readyState !== CONNECTING) {
        // Closed while connecting: the connection is dropped.
        socket.abort();
        return;
      }
      if (protocol && !list.includes(protocol)) {
        socket.abort();
        this._fail(new Error(`the server chose the subprotocol ${protocol}, which was not requested`));
        return;
      }
      state.socket = socket;
      state.protocol = protocol;
      state.extensions = extensions;
      state.readyState = OPEN;
      this.dispatchEvent(new Event('open'));
      this._receive();
    });
  }

  get url() { return this[kState].url; }
  get readyState() { return this[kState].readyState; }
  get protocol() { return this[kState].protocol; }
  get extensions() { return this[kState].extensions; }
  get bufferedAmount() { return this[kState].bufferedAmount; }

  get binaryType() { return this[kState].binaryType; }
  set binaryType(type) {
    if (type === 'blob' || type === 'arraybuffer') this[kState].binaryType = type;
  }

  send(data) {
    const state = this[kState];
    if (arguments.length === 0) {
      throw new TypeError("Failed to execute 'send' on 'WebSocket': 1 argument required, but only 0 present.");
    }
    if (state.readyState === CONNECTING) {
      throw new DOMException("Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.", 'InvalidStateError');
    }
    const message = payload(data);
    const size = message.size ?? message.bytes.byteLength;
    state.bufferedAmount += size;
    if (state.readyState !== OPEN) return;
    state.sending = state.sending.then(async () => {
      const bytes = await message.bytes;
      if (state.readyState !== OPEN || !state.socket) return;
      await new Promise((resolve) => {
        state.socket.send(message.text, bytes, (error) => {
          state.bufferedAmount -= size;
          if (error) this._fail(error);
          resolve();
        });
      });
    });
  }

  close(code = undefined, reason = undefined) {
    const state = this[kState];
    if (code !== undefined) {
      code = Number(code);
      if (!Number.isInteger(code) || (code !== 1000 && (code < 3000 || code > 4999))) {
        throw new DOMException(`Failed to execute 'close' on 'WebSocket': The close code must be either 1000, or between 3000 and 4999. ${code} is neither.`, 'InvalidAccessError');
      }
    }
    const reasonBytes = Buffer.from(reason === undefined ? '' : `${reason}`, 'utf8');
    if (reasonBytes.length > 123) {
      throw new DOMException("Failed to execute 'close' on 'WebSocket': The close reason must not be greater than 123 UTF-8 bytes.", 'SyntaxError');
    }
    if (state.readyState === CLOSING || state.readyState === CLOSED) return;
    if (state.readyState === CONNECTING) {
      state.readyState = CLOSING;
      this._fail(new Error('WebSocket was closed before the connection was established'));
      return;
    }
    state.readyState = CLOSING;
    state.closing = true;
    // Messages already sent go first; the server's close frame then ends
    // the receive loop.
    state.sending = state.sending.then(() => new Promise((resolve) => {
      state.socket.shutdown(code ?? 1000, reasonBytes, (error) => {
        if (error) this._finish(false, 1006, '');
        resolve();
      });
    }));
  }

  _receive() {
    const state = this[kState];
    state.socket.receive((error, kind, data, code) => {
      if (state.readyState === CLOSED) return;
      if (error) {
        this._fail(error);
        return;
      }
      if (kind === 'close') {
        state.socket.abort();
        this._finish(true, code, Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString('utf8'));
        return;
      }
      if (state.readyState === OPEN) {
        let value;
        if (kind === 'text') {
          value = Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString('utf8');
        } else if (state.binaryType === 'arraybuffer') {
          value = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
        } else {
          value = new Blob([data]);
        }
        this.dispatchEvent(new MessageEvent('message', { data: value, origin: new URL(state.url).origin }));
      }
      if (state.readyState !== CLOSED) this._receive();
    });
  }

  // The connection failed: an error event, then a close event that was not
  // clean.
  _fail(error) {
    const state = this[kState];
    if (state.readyState === CLOSED) return;
    state.socket?.abort();
    state.readyState = CLOSED;
    this.dispatchEvent(new ErrorEvent('error', { message: error?.message ?? '', error }));
    this.dispatchEvent(new CloseEvent('close', { wasClean: false, code: 1006, reason: '' }));
  }

  _finish(wasClean, code, reason) {
    const state = this[kState];
    if (state.readyState === CLOSED) return;
    state.readyState = CLOSED;
    this.dispatchEvent(new CloseEvent('close', { wasClean, code, reason }));
  }
}

for (const [name, value] of [['CONNECTING', CONNECTING], ['OPEN', OPEN], ['CLOSING', CLOSING], ['CLOSED', CLOSED]]) {
  Object.defineProperty(WebSocket, name, { value, enumerable: true });
  Object.defineProperty(WebSocket.prototype, name, { value, enumerable: true });
}

// `on<type>` handler properties: one listener each, replaced when set.
for (const type of ['open', 'message', 'error', 'close']) {
  Object.defineProperty(WebSocket.prototype, `on${type}`, {
    configurable: true,
    enumerable: true,
    get() {
      return this[kHandlers]?.[type]?.handler ?? null;
    },
    set(handler) {
      if (!this[kHandlers]) Object.defineProperty(this, kHandlers, { value: {}, writable: true });
      const handlers = this[kHandlers];
      const previous = handlers[type];
      if (previous) {
        handlers[type] = undefined;
        this.removeEventListener(type, previous.wrapped);
      }
      if (typeof handler === 'function') {
        const wrapped = (event) => handler.call(this, event);
        handlers[type] = { handler, wrapped };
        this.addEventListener(type, wrapped);
      }
    },
  });
}

module.exports = {
  CloseEvent,
  WebSocket,
};
