'use strict';

// nativeboat: Node.js v22.15.0's message ports within one process, for the
// globals MessageChannel, MessagePort, MessageEvent and BroadcastChannel
// and for worker_threads. An application has no worker threads, so both
// ends of a channel, and every BroadcastChannel, live in this realm.
//
// As in Node:
// - postMessage structured-clones its value; MessagePorts move with it when
//   the transfer list names them, and arrive as new ports in its data and
//   in the event's `ports`.
// - A port delivers messages as tasks once it is started: by start(), by
//   setting onmessage, or by adding a 'message' listener. Until then they
//   wait, and receiveMessageOnPort takes them one at a time.
// - on('message') listeners receive the message itself; addEventListener
//   listeners and onmessage receive a MessageEvent.
// - Closing either end closes both: each emits 'close' after the messages
//   already sent to it.
// - A port with message listeners, and an open BroadcastChannel, keep the
//   program running unless unref()'d.
// Unlike Node, ArrayBuffers in a transfer list are copied, not detached.

const {
  codes: {
    ERR_INVALID_ARG_TYPE,
    ERR_INVALID_THIS,
    ERR_MISSING_ARGS,
  },
} = require('internal/errors');
const { DOMException } = require('internal/per_context/domexception');

const kState = Symbol('kState');
const kListeners = Symbol('kListeners');
const kHandlers = Symbol('kHandlers');
const kChannel = Symbol('kChannel');
const kPortToken = '\u0000nativeboat.port';
let constructing = false;

// Holders keep the program running while referenced ports or channels are
// open: one long timer while there are any.
let holders = 0;
let keeper = null;
function hold() {
  if (holders++ === 0) keeper = setInterval(() => {}, 0x7fffffff);
}
function release() {
  if (--holders === 0) {
    clearInterval(keeper);
    keeper = null;
  }
}

function dataCloneError(message) {
  return new DOMException(message, 'DataCloneError');
}

function hidden(object, key, value) {
  Object.defineProperty(object, key, { value, writable: true, configurable: true });
}

class MessageEvent extends Event {
  constructor(type, init = undefined) {
    if (arguments.length === 0) throw new ERR_MISSING_ARGS('type');
    super(type, init);
    const {
      data = null,
      origin = '',
      lastEventId = '',
      source = null,
      ports = [],
    } = init ?? {};
    for (const [name, value] of [
      ['data', data],
      ['origin', `${origin}`],
      ['lastEventId', `${lastEventId}`],
      ['source', source],
      ['ports', Object.freeze([...ports])],
    ]) {
      Object.defineProperty(this, name, { value, enumerable: true, configurable: true });
    }
  }
}

// The state of one end of a channel; a transfer moves it to a new port.
function endState() {
  return {
    port: null,
    peer: null,
    queue: [],
    started: false,
    closed: false,
    closing: false,
    scheduled: false,
    listeners: new Set(),
    referenced: true,
    held: false,
  };
}

// Whether the end keeps the program running: open, with message
// listeners, and not unref()'d.
function updateHold(state) {
  const wanted = !state.closed && state.listeners.size > 0 && state.referenced;
  if (wanted !== state.held) {
    state.held = wanted;
    if (wanted) hold();
    else release();
  }
}

function newPort(state) {
  constructing = true;
  let port;
  try {
    port = new MessagePort();
  } finally {
    constructing = false;
  }
  hidden(port, kState, state);
  state.port = port;
  return port;
}

// The state of a port's end; null once the end moved elsewhere.
function stateOf(port) {
  if (!(port instanceof MessagePort)) throw new ERR_INVALID_THIS('MessagePort');
  return port[kState];
}

function schedule(state) {
  if (state.scheduled || !state.started) return;
  if (state.queue.length === 0 && !state.closing) return;
  state.scheduled = true;
  setImmediate(() => deliver(state));
}

// Delivers the next message, and the next in a later task, so that
// microtasks run between messages as in Node; 'close' follows the last.
function deliver(state) {
  state.scheduled = false;
  if (state.closed || !state.started || state.port[kState] !== state) return;
  if (state.queue.length > 0) {
    const { data, ports } = state.queue.shift();
    schedule(state);
    state.port.dispatchEvent(new MessageEvent('message', { data, ports }));
    return;
  }
  if (state.closing) finishClose(state);
}

function finishClose(state) {
  state.closed = true;
  state.closing = false;
  state.queue = [];
  updateHold(state);
  state.port.dispatchEvent(new Event('close'));
}

// Closes an end and its peer: each emits 'close' once it has delivered
// what was sent to it, or at once if it is not started.
function closeEnd(state) {
  const peer = state.peer;
  state.peer = null;
  if (peer) peer.peer = null;
  for (const end of [state, peer]) {
    if (!end || end.closed || end.closing) continue;
    end.closing = true;
    if (end.started) {
      schedule(end);
    } else {
      setImmediate(() => {
        if (!end.closed && end.port[kState] === end) finishClose(end);
      });
    }
  }
}

function startEnd(state) {
  if (state.started || state.closed) return;
  state.started = true;
  schedule(state);
}

// The MessagePorts a transfer list names, checked as Node checks them.
function transferredPorts(transfer, source) {
  if (transfer === undefined || transfer === null) return [];
  let list = transfer;
  if (!Array.isArray(list) && typeof list === 'object') {
    if (list.transfer === undefined || list.transfer === null) return [];
    list = list.transfer;
  }
  if (typeof list !== 'object' || list === null || !(Symbol.iterator in list)) {
    throw new ERR_INVALID_ARG_TYPE('transferList', 'Array', list);
  }
  const ports = [];
  for (const item of list) {
    if (item instanceof MessagePort) {
      if (item === source) throw dataCloneError('Transfer list contains source port');
      if (ports.includes(item)) {
        throw dataCloneError('Transfer list contains duplicate MessagePort');
      }
      const state = item[kState];
      if (!state || state.closed || state.closing) {
        throw dataCloneError('MessagePort in transfer list is already detached');
      }
      ports.push(item);
    } else if (!(item instanceof ArrayBuffer)) {
      throw dataCloneError('Found invalid value in transferList.');
    }
  }
  return ports;
}

// The value with each MessagePort replaced by a token naming its place in
// the transfer list; containers are copied, other values kept.
function tokenize(value, ports, seen = new Map()) {
  if (value instanceof MessagePort) {
    const index = ports.indexOf(value);
    if (index < 0) {
      throw dataCloneError(
        'Object that needs transfer was found in message but not listed in transferList');
    }
    return { [kPortToken]: index };
  }
  if (typeof value !== 'object' || value === null) return value;
  if (seen.has(value)) return seen.get(value);
  if (Array.isArray(value)) {
    const copy = new Array(value.length);
    seen.set(value, copy);
    for (let index = 0; index < value.length; index++) {
      if (index in value) copy[index] = tokenize(value[index], ports, seen);
    }
    return copy;
  }
  if (value instanceof Map) {
    const copy = new Map();
    seen.set(value, copy);
    for (const [key, item] of value) {
      copy.set(tokenize(key, ports, seen), tokenize(item, ports, seen));
    }
    return copy;
  }
  if (value instanceof Set) {
    const copy = new Set();
    seen.set(value, copy);
    for (const item of value) copy.add(tokenize(item, ports, seen));
    return copy;
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return value;
  const copy = {};
  seen.set(value, copy);
  for (const key of Object.keys(value)) copy[key] = tokenize(value[key], ports, seen);
  return copy;
}

// The cloned value with each token replaced by the port that arrived.
function detokenize(value, arrived, seen = new Set()) {
  if (typeof value !== 'object' || value === null || seen.has(value)) return value;
  if (Object.prototype.hasOwnProperty.call(value, kPortToken)) return arrived[value[kPortToken]];
  seen.add(value);
  if (value instanceof Map) {
    const entries = [...value];
    value.clear();
    for (const [key, item] of entries) {
      value.set(detokenize(key, arrived, seen), detokenize(item, arrived, seen));
    }
  } else if (value instanceof Set) {
    const items = [...value];
    value.clear();
    for (const item of items) value.add(detokenize(item, arrived, seen));
  } else if (Array.isArray(value) || Object.getPrototypeOf(value) === Object.prototype) {
    for (const key of Object.keys(value)) value[key] = detokenize(value[key], arrived, seen);
  }
  return value;
}

// structuredClone, failing with Node's DataCloneError.
function cloneValue(value) {
  try {
    return structuredClone(value);
  } catch (error) {
    if (error instanceof TypeError) throw dataCloneError(error.message);
    throw error;
  }
}

// Moves an end to a new port: the port it leaves stops working, and the
// new one starts unstarted, without listeners, as Node's arriving ports do.
function movePort(port) {
  const state = port[kState];
  hidden(port, kState, null);
  if (state.held) {
    state.held = false;
    release();
  }
  const moved = {
    ...state,
    started: false,
    scheduled: false,
    listeners: new Set(),
    referenced: true,
    held: false,
  };
  if (moved.peer) moved.peer.peer = moved;
  return newPort(moved);
}

// Clones a message and moves the ports it transfers.
function cloneMessage(value, transfer, source) {
  const ports = transferredPorts(transfer, source);
  if (ports.length === 0) return { data: cloneValue(value), ports: [] };
  const data = cloneValue(tokenize(value, ports));
  const arrived = ports.map(movePort);
  return { data: detokenize(data, arrived), ports: arrived };
}

function nodeStyle(type, listener) {
  if (type === 'message' || type === 'messageerror') return (event) => listener(event.data);
  return () => listener();
}

// Node's EventEmitter-style methods of MessagePort and BroadcastChannel:
// 'message' listeners receive the message itself.
const emitterMethods = {
  on(type, listener) {
    return this.addListener(type, listener);
  },
  addListener(type, listener) {
    const wrapped = nodeStyle(type, listener);
    if (!this[kListeners]) hidden(this, kListeners, []);
    this[kListeners].push({ type, listener, wrapped });
    this.addEventListener(type, wrapped);
    return this;
  },
  once(type, listener) {
    const once = (...args) => {
      this.removeListener(type, once);
      return listener.apply(this, args);
    };
    once.listener = listener;
    return this.addListener(type, once);
  },
  off(type, listener) {
    return this.removeListener(type, listener);
  },
  removeListener(type, listener) {
    const listeners = this[kListeners] ?? [];
    const index = listeners.findIndex((entry) =>
      entry.type === type && (entry.listener === listener || entry.listener.listener === listener));
    if (index >= 0) {
      const [entry] = listeners.splice(index, 1);
      this.removeEventListener(type, entry.wrapped);
    }
    return this;
  },
  removeAllListeners(type) {
    for (const entry of [...(this[kListeners] ?? [])]) {
      if (type === undefined || entry.type === type) this.removeListener(entry.type, entry.listener);
    }
    return this;
  },
  emit(type, arg) {
    const event = type === 'message' || type === 'messageerror' ?
      new MessageEvent(type, { data: arg }) : new Event(type);
    return this.dispatchEvent(event);
  },
  listenerCount(type) {
    return (this[kListeners] ?? []).filter((entry) => entry.type === type).length;
  },
  eventNames() {
    return [...new Set((this[kListeners] ?? []).map((entry) => entry.type))];
  },
  setMaxListeners() {
    return this;
  },
  getMaxListeners() {
    return Infinity;
  },
};

// `on<type>` handler properties: one listener each, replaced when set.
function defineHandlers(prototype, types) {
  for (const type of types) {
    Object.defineProperty(prototype, `on${type}`, {
      configurable: true,
      enumerable: true,
      get() {
        return this[kHandlers]?.[type]?.handler ?? null;
      },
      set(handler) {
        if (!this[kHandlers]) hidden(this, kHandlers, {});
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
}

class MessagePort extends EventTarget {
  constructor() {
    if (!constructing) throw new TypeError('Illegal constructor');
    super();
  }

  addEventListener(type, listener, options) {
    super.addEventListener(type, listener, options);
    const state = this[kState];
    if (state && `${type}` === 'message' && listener != null) {
      state.listeners.add(listener);
      updateHold(state);
      startEnd(state);
    }
  }

  removeEventListener(type, listener, options) {
    super.removeEventListener(type, listener, options);
    const state = this[kState];
    if (state && `${type}` === 'message' && state.listeners.delete(listener)) {
      updateHold(state);
    }
  }

  postMessage(value, transfer = undefined) {
    if (arguments.length === 0) throw new ERR_MISSING_ARGS('value');
    const state = stateOf(this);
    const message = cloneMessage(value, transfer, this);
    if (!state || state.closed || state.closing || !state.peer) return;
    state.peer.queue.push(message);
    schedule(state.peer);
  }

  start() {
    const state = stateOf(this);
    if (state) startEnd(state);
  }

  close() {
    const state = stateOf(this);
    if (state && !state.closed && !state.closing) closeEnd(state);
  }

  ref() {
    const state = stateOf(this);
    if (state) {
      state.referenced = true;
      updateHold(state);
    }
    return this;
  }

  unref() {
    const state = stateOf(this);
    if (state) {
      state.referenced = false;
      updateHold(state);
    }
    return this;
  }

  hasRef() {
    const state = stateOf(this);
    return !!state && state.held;
  }
}
Object.assign(MessagePort.prototype, emitterMethods);
defineHandlers(MessagePort.prototype, ['message', 'messageerror', 'close']);

class MessageChannel {
  constructor() {
    const first = endState();
    const second = endState();
    first.peer = second;
    second.peer = first;
    Object.defineProperty(this, 'port1', { value: newPort(first), enumerable: true });
    Object.defineProperty(this, 'port2', { value: newPort(second), enumerable: true });
  }
}

// `receiveMessageOnPort(port)`: the next message waiting on a port, as
// `{ message }`, or undefined.
function receiveMessageOnPort(port) {
  if (!(port instanceof MessagePort)) {
    throw new ERR_INVALID_ARG_TYPE('port', 'MessagePort', port);
  }
  const state = port[kState];
  if (!state || state.queue.length === 0) return undefined;
  return { message: state.queue.shift().data };
}

// The open BroadcastChannels by name.
const channels = new Map();

function channelOf(channel) {
  if (!(channel instanceof BroadcastChannel)) throw new ERR_INVALID_THIS('BroadcastChannel');
  return channel[kChannel];
}

class BroadcastChannel extends EventTarget {
  constructor(name) {
    if (arguments.length === 0) throw new ERR_MISSING_ARGS('name');
    super();
    const state = { name: `${name}`, closed: false, held: true };
    hidden(this, kChannel, state);
    let open = channels.get(state.name);
    if (!open) channels.set(state.name, open = new Set());
    open.add(this);
    hold();
  }

  get name() {
    return channelOf(this).name;
  }

  // Delivers a clone to every other open channel of the same name.
  postMessage(message) {
    const state = channelOf(this);
    if (arguments.length === 0) throw new ERR_MISSING_ARGS('message');
    if (state.closed) throw new DOMException('BroadcastChannel is closed.', 'InvalidStateError');
    for (const channel of channels.get(state.name) ?? []) {
      if (channel === this) continue;
      const data = cloneValue(message);
      setImmediate(() => {
        if (!channel[kChannel].closed) channel.dispatchEvent(new MessageEvent('message', { data }));
      });
    }
  }

  close() {
    const state = channelOf(this);
    if (state.closed) return;
    state.closed = true;
    const open = channels.get(state.name);
    open.delete(this);
    if (open.size === 0) channels.delete(state.name);
    if (state.held) {
      state.held = false;
      release();
    }
  }

  ref() {
    const state = channelOf(this);
    if (!state.closed && !state.held) {
      state.held = true;
      hold();
    }
    return this;
  }

  unref() {
    const state = channelOf(this);
    if (state.held) {
      state.held = false;
      release();
    }
    return this;
  }
}
Object.assign(BroadcastChannel.prototype, emitterMethods);
defineHandlers(BroadcastChannel.prototype, ['message', 'messageerror']);

module.exports = {
  BroadcastChannel,
  MessageChannel,
  MessageEvent,
  MessagePort,
  receiveMessageOnPort,
};
