'use strict';

// nativeboat: Node's `perf_hooks` for the native runtime. `performance` is
// the runtime's global, with marks, measures and their entries. Observers
// can be created and connected, but the runtime reports no entries to them,
// and histograms record no samples: event-loop delay and function timing
// are not measured.

const {
  ObjectFreeze,
} = primordials;

const { validateFunction } = require('internal/validators');

const constants = ObjectFreeze({
  NODE_PERFORMANCE_GC_MAJOR: 4,
  NODE_PERFORMANCE_GC_MINOR: 1,
  NODE_PERFORMANCE_GC_INCREMENTAL: 8,
  NODE_PERFORMANCE_GC_WEAKCB: 16,
  NODE_PERFORMANCE_GC_FLAGS_NO: 0,
  NODE_PERFORMANCE_GC_FLAGS_CONSTRUCT_RETAINED: 2,
  NODE_PERFORMANCE_GC_FLAGS_FORCED: 4,
  NODE_PERFORMANCE_GC_FLAGS_SYNCHRONOUS_PHANTOM_PROCESSING: 8,
  NODE_PERFORMANCE_GC_FLAGS_ALL_AVAILABLE_GARBAGE: 16,
  NODE_PERFORMANCE_GC_FLAGS_ALL_EXTERNAL_MEMORY: 32,
  NODE_PERFORMANCE_GC_FLAGS_SCHEDULE_IDLE: 64,
});

class PerformanceObserverEntryList {
  getEntries() { return []; }
  getEntriesByType() { return []; }
  getEntriesByName() { return []; }
}

class PerformanceObserver {
  #callback;

  constructor(callback) {
    validateFunction(callback, 'callback');
    this.#callback = callback;
  }

  observe() {}

  disconnect() {}

  takeRecords() { return []; }

  static get supportedEntryTypes() {
    return ['mark', 'measure'];
  }
}

// A histogram without samples, as `createHistogram()` and
// `monitorEventLoopDelay()` give.
class Histogram {
  get count() { return 0; }
  get min() { return 9223372036854776000; }
  get max() { return 0; }
  get mean() { return NaN; }
  get stddev() { return NaN; }
  get exceeds() { return 0; }
  get percentiles() { return new Map(); }
  percentile() { return 0; }
  reset() {}
  enable() { return true; }
  disable() { return true; }
  record() {}
  recordDelta() {}
  add() {}
  toJSON() {
    return { count: 0, min: this.min, max: 0, mean: NaN, exceeds: 0, stddev: NaN, percentiles: {} };
  }
}

function createHistogram() {
  return new Histogram();
}

function monitorEventLoopDelay() {
  return new Histogram();
}

function timerify(fn) {
  validateFunction(fn, 'fn');
  return fn;
}

module.exports = {
  Performance: globalThis.performance?.constructor,
  PerformanceObserver,
  PerformanceObserverEntryList,
  monitorEventLoopDelay,
  createHistogram,
  performance: globalThis.performance,
  constants,
  timerify,
};
