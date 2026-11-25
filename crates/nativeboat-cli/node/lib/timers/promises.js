'use strict';

// Node's `timers/promises` over the timer functions of the native runtime:
// promises settled by timers, rejected with an `AbortError` when their
// signal aborts.

const { AbortError } = require('internal/errors');

function abortable(signal, start, stop) {
  if (signal?.aborted) {
    return Promise.reject(new AbortError(undefined, { cause: signal.reason }));
  }
  return new Promise((resolve, reject) => {
    const onAbort = () => {
      stop(handle);
      reject(new AbortError(undefined, { cause: signal.reason }));
    };
    const handle = start((value) => {
      signal?.removeEventListener('abort', onAbort);
      resolve(value);
    });
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

function setTimeout(delay, value, options = {}) {
  return abortable(
    options.signal,
    (done) => globalThis.setTimeout(() => done(value), delay),
    globalThis.clearTimeout,
  );
}

function setImmediate(value, options = {}) {
  return abortable(
    options.signal,
    (done) => globalThis.setImmediate(() => done(value)),
    globalThis.clearImmediate,
  );
}

async function* setInterval(delay, value, options = {}) {
  const { signal } = options;
  if (signal?.aborted) throw new AbortError(undefined, { cause: signal.reason });
  let pending = 0;
  let wake;
  const handle = globalThis.setInterval(() => {
    pending++;
    wake?.();
  }, delay);
  const onAbort = () => wake?.();
  signal?.addEventListener('abort', onAbort, { once: true });
  try {
    while (true) {
      if (pending === 0) await new Promise((resolve) => { wake = resolve; });
      wake = undefined;
      if (signal?.aborted) throw new AbortError(undefined, { cause: signal.reason });
      while (pending > 0) {
        pending--;
        yield value;
      }
    }
  } finally {
    globalThis.clearInterval(handle);
    signal?.removeEventListener('abort', onAbort);
  }
}

const scheduler = {
  wait: (delay, options) => setTimeout(delay, undefined, options),
  yield: () => setImmediate(),
};

module.exports = { setTimeout, setImmediate, setInterval, scheduler };
