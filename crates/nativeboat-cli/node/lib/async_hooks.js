'use strict';

// The native host does not have Node's async execution resource scheduler.
// Async context travels as Node's AsyncContextFrame does: the runtime gives
// every promise job, tick, timer and I/O completion the frame that was
// current when it was queued, a Map from storages to their stores.
// AsyncResource runs callbacks in the frame it was created in; the hooks
// API itself is not available.
const {
  getContinuationPreservedEmbedderData: currentFrame,
  setContinuationPreservedEmbedderData: enterFrame,
} = internalBinding('async_context_frame');

let nextAsyncId = 1;

function runInFrame(frame, fn, thisArg, args) {
  const prior = currentFrame();
  enterFrame(frame);
  try {
    return Reflect.apply(fn, thisArg, args);
  } finally {
    enterFrame(prior);
  }
}

class AsyncResource {
  constructor(type, options) {
    this.type = typeof type === 'string' ? type : 'UNKNOWN';
    this._asyncId = nextAsyncId++;
    const triggerAsyncId = options && options.triggerAsyncId;
    this._triggerAsyncId = typeof triggerAsyncId === 'number' ? triggerAsyncId : 0;
    this._destroyed = false;
    this._frame = currentFrame();
  }

  runInAsyncScope(fn, thisArg, ...args) {
    return runInFrame(this._frame, fn, thisArg, args);
  }

  emitDestroy() {
    this._destroyed = true;
    return this;
  }

  asyncId() {
    return this._asyncId;
  }

  triggerAsyncId() {
    return this._triggerAsyncId;
  }

  bind(fn, thisArg) {
    const resource = this;
    return function (...args) {
      return resource.runInAsyncScope(fn, thisArg === undefined ? this : thisArg, ...args);
    };
  }

  static bind(fn, type, thisArg) {
    return new AsyncResource(type || fn.name || 'bound-anonymous-fn').bind(fn, thisArg);
  }
}

class AsyncLocalStorage {
  constructor(options = {}) {
    this._enabled = false;
    this._defaultValue = options.defaultValue;
    this.name = options.name;
  }

  static bind(fn) {
    return AsyncResource.bind(fn);
  }

  static snapshot() {
    const frame = currentFrame();
    return function runInAsyncScope(fn, ...args) {
      return runInFrame(frame, fn, this, args);
    };
  }

  _with(store) {
    this._enabled = true;
    const frame = new Map(currentFrame());
    frame.set(this, store);
    return frame;
  }

  disable() {
    this._enabled = false;
    const frame = currentFrame();
    if (frame !== undefined && frame.has(this)) {
      const next = new Map(frame);
      next.delete(this);
      enterFrame(next);
    }
  }

  getStore() {
    if (!this._enabled) {
      return this._defaultValue;
    }
    const frame = currentFrame();
    return frame !== undefined && frame.has(this) ? frame.get(this) : this._defaultValue;
  }

  enterWith(store) {
    enterFrame(this._with(store));
  }

  run(store, callback, ...args) {
    return runInFrame(this._with(store), callback, null, args);
  }

  exit(callback, ...args) {
    const frame = currentFrame();
    if (frame === undefined || !frame.has(this)) {
      return Reflect.apply(callback, null, args);
    }
    const next = new Map(frame);
    next.delete(this);
    return runInFrame(next, callback, null, args);
  }
}

module.exports = { AsyncLocalStorage, AsyncResource };
