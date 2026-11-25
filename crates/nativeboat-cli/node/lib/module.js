'use strict';

// nativeboat: Node's `module` for the native runtime. A native program's
// modules are resolved and linked when it is built, so Node's loader state
// here (lookup paths, `Module._cache`, the compile cache) describes Node's
// algorithms without changing what the program loads. `require` functions
// made at run time load the built-in modules the program links, and their
// `resolve` runs the resolve hooks registered with `registerHooks`, as
// Node's does; `register()`'s off-thread hooks are accepted and never run.

const path = require('path');
const { URL, fileURLToPath, pathToFileURL } = require('url');

const builtinModules = [
  '_http_agent', '_http_client', '_http_common', '_http_incoming', '_http_outgoing',
  '_http_server', '_stream_duplex', '_stream_passthrough', '_stream_readable',
  '_stream_transform', '_stream_wrap', '_stream_writable', '_tls_common', '_tls_wrap',
  'assert', 'assert/strict', 'async_hooks', 'buffer', 'child_process', 'cluster',
  'console', 'constants', 'crypto', 'dgram', 'diagnostics_channel', 'dns', 'dns/promises',
  'domain', 'events', 'fs', 'fs/promises', 'http', 'http2', 'https', 'inspector',
  'inspector/promises', 'module', 'net', 'os', 'path', 'path/posix', 'path/win32',
  'perf_hooks', 'process', 'punycode', 'querystring', 'readline', 'readline/promises',
  'repl', 'stream', 'stream/consumers', 'stream/promises', 'stream/web', 'string_decoder',
  'sys', 'timers', 'timers/promises', 'tls', 'trace_events', 'tty', 'url', 'util',
  'util/types', 'v8', 'vm', 'wasi', 'worker_threads', 'zlib',
];

// Modules only reachable with the `node:` scheme.
const schemeOnly = ['sea', 'sqlite', 'test', 'test/reporters'];

function isBuiltin(moduleName) {
  if (typeof moduleName !== 'string') return false;
  if (moduleName.startsWith('node:')) {
    const name = moduleName.slice(5);
    return builtinModules.includes(name) || schemeOnly.includes(name);
  }
  return builtinModules.includes(moduleName);
}

const constants = {
  __proto__: null,
  compileCacheStatus: {
    __proto__: null,
    FAILED: 0,
    ENABLED: 1,
    ALREADY_ENABLED: 2,
    DISABLED: 3,
  },
};

// Hooks registered with `registerHooks`, newest last.
const hooks = [];

function moduleNotFound(request, parent) {
  const from = parent ? `\nRequire stack:\n- ${parent}` : '';
  const error = new Error(`Cannot find module '${request}'${from}`);
  error.code = 'MODULE_NOT_FOUND';
  error.requireStack = parent ? [parent] : [];
  return error;
}

// Node's default resolution for what a native program can answer: built-in
// modules. Files and packages were resolved when the program was built.
function defaultResolve(specifier, context) {
  if (isBuiltin(specifier)) {
    return {
      url: specifier.startsWith('node:') ? specifier : `node:${specifier}`,
      format: 'builtin',
      shortCircuit: true,
    };
  }
  throw moduleNotFound(specifier, context?.parentURL);
}

// Runs the resolve hooks from the newest, each able to call the next.
function runResolveHooks(specifier, context) {
  const chain = hooks.filter((hook) => typeof hook.resolve === 'function');
  const step = (index) => (nextSpecifier, nextContext) => {
    const current = nextContext ? { ...context, ...nextContext } : context;
    if (index < 0) return defaultResolve(nextSpecifier, current);
    return chain[index].resolve(nextSpecifier, current, step(index - 1));
  };
  return step(chain.length - 1)(specifier, context);
}

function registerHooks(options) {
  const entry = { resolve: options?.resolve, load: options?.load };
  hooks.push(entry);
  return {
    deregister() {
      const index = hooks.indexOf(entry);
      if (index !== -1) hooks.splice(index, 1);
    },
  };
}

function register() {}

function toPath(filename) {
  if (filename instanceof URL) filename = filename.href;
  if (typeof filename !== 'string') {
    const error = new TypeError('The argument \'filename\' must be a file URL object, file URL string, or absolute path string.');
    error.code = 'ERR_INVALID_ARG_VALUE';
    throw error;
  }
  if (filename.startsWith('file:')) return fileURLToPath(filename);
  return filename;
}

function toURL(filename) {
  return pathToFileURL(filename).href;
}

const conditions = ['require', 'node', 'node-addons'];

function createRequire(filename) {
  const parent = toPath(filename);
  const parentURL = toURL(parent);
  const parentModule = new Module(parent);
  parentModule.filename = parent;
  parentModule.paths = nodeModulePaths(path.dirname(parent));
  function require(id) {
    if (typeof id !== 'string') {
      const error = new TypeError('The "id" argument must be of type string');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    return Module._load(id, parentModule, false);
  }
  function resolve(request) {
    const result = runResolveHooks(request, { conditions, importAttributes: undefined, parentURL });
    const url = result?.url;
    if (typeof url !== 'string') return request;
    if (url.startsWith('node:')) return request.startsWith('node:') ? url : url.slice(5);
    if (url.startsWith('file:')) return fileURLToPath(url);
    return url;
  }
  resolve.paths = (request) => (isBuiltin(request) ? null : Module._nodeModulePaths(path.dirname(parent)));
  require.resolve = resolve;
  require.cache = Module._cache;
  require.extensions = Module._extensions;
  require.main = undefined;
  return require;
}

function Module(id = '', parent) {
  this.id = id;
  this.path = path.dirname(id);
  this.exports = {};
  this.filename = null;
  this.loaded = false;
  this.children = [];
  this.paths = [];
  this.parent = parent;
}

// Node's lookup paths on Windows: each ancestor's node_modules, nearest
// first, skipping folders that are node_modules.
function nodeModulePaths(from) {
  const resolved = path.resolve(from);
  const parts = resolved.split(/[\\/]/);
  const paths = [];
  for (let end = parts.length; end > 0; end--) {
    if (parts[end - 1] === 'node_modules') continue;
    const prefix = parts.slice(0, end).join('\\');
    paths.push(`${prefix}\\node_modules`);
  }
  return paths;
}

function globalPaths() {
  const home = process.env.USERPROFILE;
  const paths = [];
  if (home) {
    paths.push(path.resolve(home, '.node_modules'));
    paths.push(path.resolve(home, '.node_libraries'));
  }
  paths.push(path.resolve(process.execPath, '..', 'lib', 'node'));
  return paths;
}

Object.assign(Module, {
  builtinModules,
  isBuiltin,
  createRequire,
  registerHooks,
  register,
  constants,
  globalPaths: globalPaths(),
  _cache: { __proto__: null },
  _pathCache: { __proto__: null },
  _extensions: { __proto__: null, '.js'() {}, '.json'() {}, '.node'() {} },
  _nodeModulePaths: nodeModulePaths,
  _resolveLookupPaths(request, parent) {
    if (isBuiltin(request)) return null;
    if (request.startsWith('./') || request.startsWith('../') || request.startsWith('.\\') || request.startsWith('..\\') || request === '.' || request === '..') {
      return [parent?.filename ? path.dirname(parent.filename) : process.cwd()];
    }
    const own = parent?.paths?.length ? parent.paths : nodeModulePaths(parent?.filename ? path.dirname(parent.filename) : process.cwd());
    return [...own, ...Module.globalPaths];
  },
  _resolveFilename(request, parent) {
    if (isBuiltin(request)) return request;
    throw moduleNotFound(request, parent?.filename);
  },
  // Node routes every require through `_load`, which programs may wrap to
  // serve modules of their own. Requests the build compiled by name never
  // reach it; a path reaches the compiled entry registered for it.
  _load(request, parent) {
    if (isBuiltin(request)) return internalBinding('builtins').load(request);
    const compiled = internalBinding('modules').loadCompiled(request, parent?.filename);
    if (compiled !== undefined) return compiled;
    throw moduleNotFound(request, parent?.filename);
  },
  wrap(script) {
    return `(function (exports, require, module, __filename, __dirname) { ${script}\n});`;
  },
  wrapper: ['(function (exports, require, module, __filename, __dirname) { ', '\n});'],
  enableCompileCache() {
    return { status: constants.compileCacheStatus.DISABLED, message: 'native programs are compiled ahead of time' };
  },
  flushCompileCache() {},
  getCompileCacheDir() {
    return undefined;
  },
  syncBuiltinESMExports() {},
  findSourceMap() {
    return undefined;
  },
  findPackageJSON() {
    return undefined;
  },
  stripTypeScriptTypes() {
    const error = new Error('module.stripTypeScriptTypes() is not available in a native build');
    error.code = 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM';
    throw error;
  },
});

class SourceMap {
  constructor(payload) {
    this.payload = payload;
  }

  findEntry() {
    return {};
  }

  findOrigin() {
    return {};
  }
}
Module.SourceMap = SourceMap;
Module.Module = Module;

module.exports = Module;
