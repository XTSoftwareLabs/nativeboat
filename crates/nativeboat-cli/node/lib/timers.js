'use strict';

// Node's `timers` module over the timer functions of the native runtime,
// which schedules timers itself; the exports match Node's.

const timers = {
  setTimeout: globalThis.setTimeout,
  clearTimeout: globalThis.clearTimeout,
  setInterval: globalThis.setInterval,
  clearInterval: globalThis.clearInterval,
  setImmediate: globalThis.setImmediate,
  clearImmediate: globalThis.clearImmediate,
};

let promises;
Object.defineProperty(timers, 'promises', {
  __proto__: null,
  configurable: true,
  enumerable: true,
  get() {
    promises ??= require('timers/promises');
    return promises;
  },
});

module.exports = timers;
