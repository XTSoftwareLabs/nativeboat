'use strict';

// nativeboat: Node's `vm` for the native runtime, which has no JavaScript
// engine to evaluate source text with. Contexts can be made and recognized,
// and scripts constructed, but evaluating or compiling code throws
// ERR_FEATURE_UNAVAILABLE_ON_PLATFORM: an application's code is compiled
// ahead of time, and code it builds while it runs cannot be.

const contexts = new WeakSet();

function unavailable(name) {
  const error = new Error(`vm.${name} cannot evaluate code in a native build`);
  error.code = 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM';
  return error;
}

function validateObject(value, name) {
  if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
    const error = new TypeError(`The "${name}" argument must be of type object.`);
    error.code = 'ERR_INVALID_ARG_TYPE';
    throw error;
  }
}

const constants = {
  USE_MAIN_CONTEXT_DEFAULT_LOADER: Symbol('vm_dynamic_import_main_context_default'),
  DONT_CONTEXTIFY: Symbol('vm_context_no_contextify'),
};

function createContext(contextObject = {}, options = undefined) {
  if (contextObject === constants.DONT_CONTEXTIFY) contextObject = {};
  validateObject(contextObject, 'contextObject');
  void options;
  contexts.add(contextObject);
  return contextObject;
}

function isContext(object) {
  validateObject(object, 'object');
  return contexts.has(object);
}

class Script {
  constructor(code, options = undefined) {
    this.sourceMapURL = undefined;
    this.cachedDataRejected = false;
    void code;
    void options;
  }

  runInThisContext() {
    throw unavailable('Script.prototype.runInThisContext');
  }

  runInContext() {
    throw unavailable('Script.prototype.runInContext');
  }

  runInNewContext() {
    throw unavailable('Script.prototype.runInNewContext');
  }

  createCachedData() {
    throw unavailable('Script.prototype.createCachedData');
  }
}

function thrower(name) {
  return function () {
    throw unavailable(name);
  };
}

function measureMemory() {
  return Promise.reject(unavailable('measureMemory'));
}

module.exports = {
  Script,
  createContext,
  createScript(code, options) {
    return new Script(code, options);
  },
  runInContext: thrower('runInContext'),
  runInNewContext: thrower('runInNewContext'),
  runInThisContext: thrower('runInThisContext'),
  isContext,
  compileFunction: thrower('compileFunction'),
  measureMemory,
  constants,
};
