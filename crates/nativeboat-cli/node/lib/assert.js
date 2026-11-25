'use strict';

// nativeboat: Node's `assert` API written for the native runtime, with
// Node's deep equality from the bundled internal/util/comparisons. Messages
// that Node builds by reading the caller's source show the value instead,
// and failure diffs are shown as the two values. CallTracker is not
// available.

const { isDeepEqual, isDeepStrictEqual } = require('internal/util/comparisons');
const { inspect } = require('internal/util/inspect');

const show = (value) => inspect(value, { depth: 1000, colors: false, compact: false, sorted: true });

class AssertionError extends Error {
  constructor(options) {
    if (options === null || typeof options !== 'object') {
      const error = new TypeError('The "options" argument must be of type object.');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    const { message, actual, expected, operator } = options;
    super(message !== undefined ? String(message) : describe(actual, expected, operator));
    this.generatedMessage = message === undefined;
    this.code = 'ERR_ASSERTION';
    this.actual = actual;
    this.expected = expected;
    this.operator = operator;
  }

  get name() {
    return 'AssertionError';
  }

  toString() {
    return `${this.name} [${this.code}]: ${this.message}`;
  }
}

function describe(actual, expected, operator) {
  switch (operator) {
    case 'strictEqual':
      return typeof actual === 'object' && actual !== null || typeof expected === 'object' && expected !== null
        ? `Expected values to be strictly equal:\n\n${show(actual)}\n\nshould equal\n\n${show(expected)}\n`
        : `Expected values to be strictly equal:\n\n${show(actual)} !== ${show(expected)}\n`;
    case 'deepStrictEqual':
      return `Expected values to be strictly deep-equal:\n\n${show(actual)}\n\nshould equal\n\n${show(expected)}\n`;
    case 'deepEqual':
      return `Expected values to be loosely deep-equal:\n\n${show(actual)}\n\nshould loosely deep-equal\n\n${show(expected)}`;
    case 'notStrictEqual':
      return `Expected "actual" to be strictly unequal to: ${show(expected)}`;
    case 'notDeepStrictEqual':
      return `Expected "actual" not to be strictly deep-equal to: ${show(expected)}`;
    case 'notDeepEqual':
      return `Expected "actual" not to be loosely deep-equal to:\n\n${show(expected)}`;
    case '==':
    case '!=':
      return `${show(actual)} ${operator} ${show(expected)}`;
    case 'fail':
      return 'Failed';
    default:
      return `${show(actual)} ${operator} ${show(expected)}`;
  }
}

function fail(actual, expected, message, operator) {
  if (message instanceof Error) throw message;
  throw new AssertionError({ actual, expected, operator, message });
}

function innerOk(argumentCount, value, message) {
  if (value) return;
  if (message instanceof Error) throw message;
  if (argumentCount === 0) {
    message = 'No value argument passed to `assert.ok()`';
  } else if (message === undefined) {
    message = `The expression evaluated to a falsy value:\n\n  assert.ok(${show(value)})\n`;
  }
  const error = new AssertionError({ actual: value, expected: true, operator: '==', message });
  error.generatedMessage = argumentCount === 0 || arguments[2] === undefined;
  throw error;
}

function ok(...args) {
  innerOk(args.length, ...args);
}

const assert = function assert(...args) {
  innerOk(args.length, ...args);
};

assert.ok = ok;
assert.AssertionError = AssertionError;

assert.fail = function(actual, expected, message, operator) {
  if (arguments.length === 0) {
    message = 'Failed';
  } else if (arguments.length === 1) {
    message = actual;
    actual = undefined;
  }
  if (message instanceof Error) throw message;
  const error = new AssertionError({ actual, expected, operator: operator === undefined ? 'fail' : operator, message });
  if (arguments.length >= 2) error.generatedMessage = message === undefined;
  throw error;
};

assert.equal = function equal(actual, expected, message) {
  // eslint-disable-next-line eqeqeq
  if (!(actual == expected || (actual !== actual && expected !== expected))) {
    fail(actual, expected, message, '==');
  }
};

assert.notEqual = function notEqual(actual, expected, message) {
  // eslint-disable-next-line eqeqeq
  if (actual == expected || (actual !== actual && expected !== expected)) {
    fail(actual, expected, message, '!=');
  }
};

assert.deepEqual = function deepEqual(actual, expected, message) {
  if (!isDeepEqual(actual, expected)) fail(actual, expected, message, 'deepEqual');
};

assert.notDeepEqual = function notDeepEqual(actual, expected, message) {
  if (isDeepEqual(actual, expected)) fail(actual, expected, message, 'notDeepEqual');
};

assert.deepStrictEqual = function deepStrictEqual(actual, expected, message) {
  if (!isDeepStrictEqual(actual, expected)) fail(actual, expected, message, 'deepStrictEqual');
};

assert.notDeepStrictEqual = function notDeepStrictEqual(actual, expected, message) {
  if (isDeepStrictEqual(actual, expected)) fail(actual, expected, message, 'notDeepStrictEqual');
};

assert.strictEqual = function strictEqual(actual, expected, message) {
  if (!Object.is(actual, expected)) fail(actual, expected, message, 'strictEqual');
};

assert.notStrictEqual = function notStrictEqual(actual, expected, message) {
  if (Object.is(actual, expected)) fail(actual, expected, message, 'notStrictEqual');
};

assert.partialDeepStrictEqual = function partialDeepStrictEqual(actual, expected, message) {
  const { isPartialStrictEqual } = require('internal/util/comparisons');
  if (!isPartialStrictEqual(actual, expected)) fail(actual, expected, message, 'partialDeepStrictEqual');
};

// Whether a thrown value satisfies `expected`: a RegExp its text matches, a
// class it is an instance of, a validation function, or an object whose
// properties it shares.
function matches(actual, expected, message, fn) {
  if (typeof expected === 'function') {
    if (expected.prototype !== undefined && actual instanceof expected) return;
    if (Error.isPrototypeOf(expected) || expected === Error) {
      throw actual;
    }
    const result = expected.call({}, actual);
    if (result !== true) {
      if (message instanceof Error) throw message;
      throw new AssertionError({
        actual,
        expected,
        operator: fn.name,
        message: message || `The ${expected.name ? `"${expected.name}" ` : ''}validation function is expected to return "true". Received ${show(result)}`,
      });
    }
    return;
  }
  if (expected instanceof RegExp) {
    if (expected.test(String(actual))) return;
    if (message instanceof Error) throw message;
    throw new AssertionError({
      actual,
      expected,
      operator: fn.name,
      message: message || `The input did not match the regular expression ${show(expected)}. Input:\n\n${show(String(actual))}\n`,
    });
  }
  if (typeof expected === 'object' && expected !== null) {
    for (const key of Object.keys(expected)) {
      const want = expected[key];
      const have = actual?.[key];
      const same = want instanceof RegExp && typeof have === 'string'
        ? want.test(have)
        : isDeepStrictEqual(have, want);
      if (!same) {
        if (message instanceof Error) throw message;
        throw new AssertionError({ actual, expected, operator: fn.name, message: message || `Expected values to be strictly deep-equal:\n\n${show(have)}\n\nshould equal\n\n${show(want)}\n` });
      }
    }
  }
}

const NO_EXCEPTION = Symbol('no exception');

function caught(fn) {
  try {
    fn();
  } catch (error) {
    return error;
  }
  return NO_EXCEPTION;
}

async function caughtAsync(promiseOrFn) {
  let promise = promiseOrFn;
  if (typeof promiseOrFn === 'function') {
    promise = promiseOrFn();
  }
  if (!promise || typeof promise.then !== 'function') {
    const error = new TypeError('The "promiseFn" argument must be of type function or an instance of Promise.');
    error.code = 'ERR_INVALID_RETURN_VALUE';
    throw error;
  }
  try {
    await promise;
  } catch (error) {
    return error;
  }
  return NO_EXCEPTION;
}

function expectsError(fn, actual, error, message) {
  if (typeof error === 'string') {
    message = error;
    error = undefined;
  }
  if (actual === NO_EXCEPTION) {
    let details = '';
    if (error && error.name) details += ` (${error.name})`;
    details += message ? `: ${message}` : '.';
    const word = fn === assert.rejects ? 'rejection' : 'exception';
    throw new AssertionError({ actual: undefined, expected: error, operator: fn.name, message: `Missing expected ${word}${details}` });
  }
  if (error !== undefined) matches(actual, error, message, fn);
}

function expectsNoError(fn, actual, error, message) {
  if (actual === NO_EXCEPTION) return;
  if (typeof error === 'string') {
    message = error;
    error = undefined;
  }
  if (!error || (typeof error === 'function' && actual instanceof error) ||
      (error instanceof RegExp && error.test(String(actual)))) {
    const details = message ? `: ${message}` : '.';
    const word = fn === assert.doesNotReject ? 'rejection' : 'exception';
    throw new AssertionError({
      actual,
      expected: error,
      operator: fn.name,
      message: `Got unwanted ${word}${details}\nActual message: "${actual && actual.message}"`,
    });
  }
  throw actual;
}

assert.throws = function throws(promiseFn, ...args) {
  if (typeof promiseFn !== 'function') {
    const error = new TypeError('The "fn" argument must be of type function.');
    error.code = 'ERR_INVALID_ARG_TYPE';
    throw error;
  }
  expectsError(throws, caught(promiseFn), ...args);
};

assert.rejects = async function rejects(promiseFn, ...args) {
  expectsError(rejects, await caughtAsync(promiseFn), ...args);
};

assert.doesNotThrow = function doesNotThrow(fn, ...args) {
  expectsNoError(doesNotThrow, caught(fn), ...args);
};

assert.doesNotReject = async function doesNotReject(fn, ...args) {
  expectsNoError(doesNotReject, await caughtAsync(fn), ...args);
};

assert.ifError = function ifError(error) {
  if (error === null || error === undefined) return;
  let message = 'ifError got unwanted exception: ';
  message += typeof error === 'object' && typeof error.message === 'string'
    ? (error.message.length === 0 && error.constructor ? error.constructor.name : error.message)
    : show(error);
  throw new AssertionError({ actual: error, expected: null, operator: 'ifError', message });
};

function checkMatch(fn, string, regexp, message, shouldMatch) {
  if (!(regexp instanceof RegExp)) {
    const error = new TypeError('The "regexp" argument must be an instance of RegExp.');
    error.code = 'ERR_INVALID_ARG_TYPE';
    throw error;
  }
  if (typeof string !== 'string' || regexp.test(string) !== shouldMatch) {
    if (message instanceof Error) throw message;
    const generated = typeof string !== 'string'
      ? `The "string" argument must be of type string. Received type ${typeof string} (${show(string)})`
      : (shouldMatch ? 'The input did not match the regular expression ' : 'The input was expected to not match the regular expression ') +
        `${show(regexp)}. Input:\n\n${show(string)}\n`;
    throw new AssertionError({ actual: string, expected: regexp, operator: fn.name, message: message || generated });
  }
}

assert.match = function match(string, regexp, message) {
  checkMatch(match, string, regexp, message, true);
};

assert.doesNotMatch = function doesNotMatch(string, regexp, message) {
  checkMatch(doesNotMatch, string, regexp, message, false);
};

// The strict mode: equality checks are the strict ones.
function strict(...args) {
  innerOk(args.length, ...args);
}

Object.assign(strict, assert, {
  equal: assert.strictEqual,
  deepEqual: assert.deepStrictEqual,
  notEqual: assert.notStrictEqual,
  notDeepEqual: assert.notDeepStrictEqual,
});

assert.strict = strict;
assert.strict.strict = assert.strict;

module.exports = assert;
