// Explicit V8-style captures over Boa's bytecode locations. Native/async frame
// metadata and automatic prepareStackTrace on ordinary Errors are not supplied.
(function (host) {
  'use strict';
  const hidden = (target, name, value) => Object.defineProperty(target, name, {
    value, writable: true, configurable: true, enumerable: false,
  });
  class CallSite {
    #location;
    constructor(location) { this.#location = location; }
    getFileName() { return this.#location.file; }
    getScriptNameOrSourceURL() { return this.#location.file; }
    getFunctionName() { return this.#location.name === '<main>' ? null : this.#location.name || null; }
    getLineNumber() { return this.#location.line; }
    getColumnNumber() { return this.#location.column; }
    // Boa's public frames do not expose receiver, function or call-kind data.
    getThis() { return undefined; }
    getFunction() { return undefined; }
    getTypeName() { return null; }
    getMethodName() { return null; }
    getEvalOrigin() { return undefined; }
    getPromiseIndex() { return null; }
    isNative() { return false; }
    isEval() { return false; }
    isAsync() { return false; }
    isPromiseAll() { return false; }
    toString() {
      const location = this.#location;
      const file = location.file === null ? '<anonymous>' : location.file;
      const position = location.line === null ? file : `${file}:${location.line}:${location.column}`;
      const name = this.getFunctionName();
      return name ? `${name} (${position})` : position;
    }
  }
  hidden(Error, 'stackTraceLimit', 10);
  hidden(Error, 'captureStackTrace', function captureStackTrace(target, constructorOpt) {
    if (target === null || (typeof target !== 'object' && typeof target !== 'function')) {
      throw new TypeError('Error.captureStackTrace requires an object');
    }
    const limit = Error.stackTraceLimit;
    const count = typeof limit === 'number' && limit > 0 ? Math.floor(limit) : 0;
    const sites = count ? host.stackFrames(constructorOpt, count).map(location => new CallSite(location)) : [];
    let prepared = false;
    let preparing = false;
    let value;
    const defaultStack = () => {
      const name = target.name === undefined ? 'Error' : String(target.name);
      const message = target.message === undefined ? '' : String(target.message);
      const heading = name && message ? `${name}: ${message}` : name || message;
      return heading + sites.map(site => `\n    at ${site}`).join('');
    };
    Object.defineProperty(target, 'stack', {
      configurable: true,
      enumerable: false,
      get() {
        if (!prepared) {
          if (preparing) return defaultStack();
          // Resolve the hook when first read. Its result can be any value,
          // including undefined, and a throwing hook may be retried.
          preparing = true;
          try {
            const formatter = Error.prepareStackTrace;
            value = typeof formatter === 'function' ? formatter.call(Error, target, sites) : defaultStack();
            prepared = true;
          } finally {
            preparing = false;
          }
        }
        return value;
      },
      set(next) { value = next; prepared = true; },
    });
  });
})
