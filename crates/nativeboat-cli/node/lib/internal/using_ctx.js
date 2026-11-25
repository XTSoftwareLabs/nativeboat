'use strict';

// The disposal context `using` and `await using` declarations are lowered
// to, as the explicit resource management proposal disposes: `u(value)`
// and `a(value)` add a resource, `e` holds an error the block threw, and
// `d()` disposes the resources in reverse order once the block ends. Errors
// from disposal combine with earlier ones in a SuppressedError. `d()`
// returns a promise when an `await using` resource was added.

const empty = {};

function usingCtx() {
  const resources = [];
  const context = {
    e: empty,
    u: (value) => add(false, value),
    a: (value) => add(true, value),
    d: dispose,
  };

  function add(isAsync, value) {
    if (value === null || value === undefined) {
      // `await using x = null` still awaits once, at disposal.
      if (isAsync) resources.push({ value, dispose: undefined, isAsync });
      return value;
    }
    if (Object(value) !== value) {
      throw new TypeError('using declarations can only be used with objects, functions, null, or undefined.');
    }
    let method;
    let synchronous;
    if (isAsync) method = value[Symbol.asyncDispose];
    if (method === undefined) {
      method = value[Symbol.dispose];
      if (isAsync) synchronous = method;
    }
    if (typeof method !== 'function') throw new TypeError('Object is not disposable.');
    if (synchronous) {
      // An `await using` resource with only Symbol.dispose: its errors
      // reject, and its disposal is awaited.
      method = function () {
        try {
          synchronous.call(this);
        } catch (error) {
          return Promise.reject(error);
        }
        return undefined;
      };
    }
    resources.push({ value, dispose: method, isAsync });
    return value;
  }

  function dispose() {
    let error = context.e;
    // 1: a null `await using` resource is pending its await; 2: something
    // was awaited.
    let state = 0;

    function fail(thrown) {
      error = error !== empty ? new SuppressedError(thrown, error) : thrown;
      return next();
    }

    function next() {
      let resource;
      while ((resource = resources.pop())) {
        try {
          if (!resource.isAsync && state === 1) {
            state = 0;
            resources.push(resource);
            return Promise.resolve().then(next);
          }
          if (resource.dispose) {
            const result = resource.dispose.call(resource.value);
            if (resource.isAsync) {
              state |= 2;
              return Promise.resolve(result).then(next, fail);
            }
          } else {
            state |= 1;
          }
        } catch (thrown) {
          return fail(thrown);
        }
      }
      if (state === 1) return error !== empty ? Promise.reject(error) : Promise.resolve();
      if (error !== empty) throw error;
      return undefined;
    }

    return next();
  }

  return context;
}

module.exports = usingCtx;
