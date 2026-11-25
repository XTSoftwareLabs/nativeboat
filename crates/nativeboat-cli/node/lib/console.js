'use strict';

// nativeboat: Node's `console` for the native runtime: the global console,
// with a `Console` class whose instances write formatted lines to the
// streams they are given, as Node's do.

const { formatWithOptions, inspect } = require('util');

const kStdout = Symbol('stdout');
const kStderr = Symbol('stderr');
const kOptions = Symbol('options');
const kCounts = Symbol('counts');
const kTimers = Symbol('timers');
const kIndent = Symbol('indent');

function Console(stdout, stderr, ignoreErrors) {
  if (!new.target) return new Console(stdout, stderr, ignoreErrors);
  const options = stdout && typeof stdout.write !== 'function' ? stdout : { stdout, stderr };
  if (!options.stdout || typeof options.stdout.write !== 'function') {
    const error = new TypeError('The "stdout" argument must be an instance of WritableStream');
    error.code = 'ERR_CONSOLE_WRITABLE_STREAM';
    throw error;
  }
  this[kStdout] = options.stdout;
  this[kStderr] = options.stderr ?? options.stdout;
  this[kOptions] = options.inspectOptions ?? {};
  this[kCounts] = new Map();
  this[kTimers] = new Map();
  this[kIndent] = '';
  // Node binds the methods, so they may be passed around.
  for (const name of Object.keys(Console.prototype)) {
    if (typeof this[name] === 'function') this[name] = this[name].bind(this);
  }
}

function write(console, stream, args) {
  const text = formatWithOptions(console[kOptions], ...args);
  const indented = console[kIndent] ? text.replace(/^/gm, console[kIndent]) : text;
  try {
    stream.write(`${indented}\n`);
  } catch {
    // Node ignores write errors by default.
  }
}

Object.assign(Console.prototype, {
  log(...args) { write(this, this[kStdout], args); },
  info(...args) { write(this, this[kStdout], args); },
  debug(...args) { write(this, this[kStdout], args); },
  warn(...args) { write(this, this[kStderr], args); },
  error(...args) { write(this, this[kStderr], args); },
  trace(...args) {
    const error = new Error(formatWithOptions(this[kOptions], ...args));
    error.name = 'Trace';
    write(this, this[kStderr], [error.stack ?? `Trace: ${error.message}`]);
  },
  dir(object, options) {
    write(this, this[kStdout], [inspect(object, { ...this[kOptions], ...options })]);
  },
  dirxml(...args) { write(this, this[kStdout], args); },
  table(data) { write(this, this[kStdout], [data]); },
  assert(value, ...args) {
    if (!value) {
      args[0] = args.length ? `Assertion failed: ${args[0]}` : 'Assertion failed';
      write(this, this[kStderr], args);
    }
  },
  count(label = 'default') {
    const count = (this[kCounts].get(`${label}`) ?? 0) + 1;
    this[kCounts].set(`${label}`, count);
    write(this, this[kStdout], [`${label}: ${count}`]);
  },
  countReset(label = 'default') { this[kCounts].delete(`${label}`); },
  group(...args) {
    if (args.length) write(this, this[kStdout], args);
    this[kIndent] += '  ';
  },
  groupCollapsed(...args) { this.group(...args); },
  groupEnd() { this[kIndent] = this[kIndent].slice(0, -2); },
  time(label = 'default') { this[kTimers].set(`${label}`, globalThis.performance.now()); },
  timeLog(label = 'default', ...args) {
    const start = this[kTimers].get(`${label}`);
    if (start !== undefined) {
      write(this, this[kStdout], [`${label}: ${(globalThis.performance.now() - start).toFixed(3)}ms`, ...args]);
    }
  },
  timeEnd(label = 'default') {
    this.timeLog(label);
    this[kTimers].delete(`${label}`);
  },
});

const globalConsole = globalThis.console;
module.exports = globalConsole;
if (globalConsole && !globalConsole.Console) {
  Object.defineProperty(globalConsole, 'Console', {
    __proto__: null,
    value: Console,
    writable: true,
    configurable: true,
  });
}
