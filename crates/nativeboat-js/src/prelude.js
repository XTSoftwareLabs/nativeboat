// The nativeboat JavaScript host prelude: module interop and loading, timers,
// microtasks, console output and error reporting. It evaluates to a function
// that the Rust host calls once per context with its `host` object:
//
//   host.compile(code, path, params) returns a module factory function
//   host.print(level, text)     writes console output
//   host.now()                  milliseconds since the host started
//   host.timerSchedule/Cancel/Refresh/Next/Run  native timer storage
//   host.random(size)           returns a Uint8Array of random bytes
//   host.taskId()               returns a task id unique in the process
//
// Later layers may set `host.onerror(error, inPromise)`; returning `true`
// marks an uncaught error as handled.
(function (host, node) {
  'use strict';
  const global = globalThis;
  const hidden = (target, name, value) =>
    Object.defineProperty(target, name, {
      value,
      writable: true,
      configurable: true,
      enumerable: false,
    });
  const isObject = (value) =>
    value !== null && (typeof value === 'object' || typeof value === 'function');
  const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);

  if (node) hidden(global, 'global', global);

  // The helpers that compiled TypeScript decorators (experimentalDecorators)
  // call, as TypeScript's own __decorate, __param and __metadata.
  hidden(global, 'babelHelpers', {
    decorate(decorators, target, key, descriptor) {
      const count = arguments.length;
      let result = count < 3 ? target : descriptor === null ? (descriptor = Object.getOwnPropertyDescriptor(target, key)) : descriptor;
      if (typeof Reflect === 'object' && typeof Reflect.decorate === 'function') {
        result = Reflect.decorate(decorators, target, key, descriptor);
      } else {
        for (let index = decorators.length - 1; index >= 0; index--) {
          const decorator = decorators[index];
          if (decorator) result = (count < 3 ? decorator(result) : count > 3 ? decorator(target, key, result) : decorator(target, key)) || result;
        }
      }
      if (count > 3 && result) Object.defineProperty(target, key, result);
      return result;
    },
    decorateParam: (index, decorator) => (target, key) => decorator(target, key, index),
    decorateMetadata(key, value) {
      if (typeof Reflect === 'object' && typeof Reflect.metadata === 'function') return Reflect.metadata(key, value);
      return undefined;
    },
  });

  // Module interop for the bundler's module format. -----------------------
  const namespaces = new WeakMap();
  hidden(global, '__nui_interop', (value) => {
    if (isObject(value) && value.__esModule) return value;
    if (isObject(value) && namespaces.has(value)) return namespaces.get(value);
    const namespace = Object.create(null);
    Object.defineProperty(namespace, 'default', { value, enumerable: true });
    if (isObject(value)) {
      for (const key of Object.keys(value)) {
        if (key !== 'default') {
          Object.defineProperty(namespace, key, { get: () => value[key], enumerable: true });
        }
      }
      namespaces.set(value, namespace);
    }
    return namespace;
  });
  hidden(global, '__nui_export', (exports, getters) => {
    if (!own(exports, '__esModule')) {
      Object.defineProperty(exports, '__esModule', { value: true });
    }
    for (const name of Object.keys(getters)) {
      Object.defineProperty(exports, name, {
        get: getters[name],
        enumerable: true,
        configurable: true,
      });
    }
  });
  hidden(global, '__nui_export_star', (exports, namespace) => {
    for (const name of Object.keys(namespace)) {
      if (name === 'default' || name === '__esModule' || own(exports, name)) continue;
      Object.defineProperty(exports, name, {
        get: () => namespace[name],
        enumerable: true,
        configurable: true,
      });
    }
  });

  // Module loading. --------------------------------------------------------
  const builtins = Object.create(null);
  hidden(host, 'builtins', builtins);
  hidden(host, 'packages', Object.create(null));
  const notFound = (name, reason) => {
    const error = new Error(`Cannot find module '${name}'${reason}`);
    error.code = 'MODULE_NOT_FOUND';
    return error;
  };
  const builtin = (name) => {
    if (own(host.packages, name)) return host.packages[name];
    const key = name.startsWith('node:') ? name.slice(5) : name;
    if (key in builtins) return builtins[key];
    throw notFound(name, ': it is not provided by nativeboat');
  };
  hidden(host, 'require', builtin);
  const dirname = (path) => {
    const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return index < 0 ? '.' : path.slice(0, index) || '/';
  };
  const pathKey = (path) => {
    const text = path.replace(/\\/g, '/');
    const parts = [];
    const rooted = /^[A-Za-z]:\//.test(text) ? 1 : text.startsWith('//') ? 4 : text.startsWith('/') ? 1 : 0;
    for (const part of text.split('/')) {
      if (part === '.') continue;
      if (part === '..' && parts.length > rooted && parts[parts.length - 1] !== '..') parts.pop();
      else if (part === '..' && rooted) continue;
      else parts.push(part);
    }
    const normalized = parts.join('/');
    return /^[A-Za-z]:/.test(normalized) || normalized.startsWith('//') ? normalized.toLowerCase() : normalized;
  };
  const contexts = new Map();
  hidden(host, 'createRequire', (filename) => {
    const key = pathKey(filename);
    if (contexts.has(key)) {
      const context = contexts.get(key);
      return context.getRequire(context.index);
    }
    const directory = dirname(key);
    for (const [path, context] of contexts) {
      if (dirname(path) === directory) return context.getRequire(context.index);
    }
    const require = (name) => builtin(name);
    require.resolve = (name) => { builtin(name); return name; };
    require.cache = Object.create(null);
    return require;
  });
  hidden(host, 'load', (bundle, scope) => {
    // Names a module sees besides its own, such as `process` in preloads.
    const names = scope ? Object.keys(scope) : [];
    const values = names.map((name) => scope[name]);
    const records = bundle.modules;
    const cache = [];
    const sharedCache = Object.create(null);
    const registered = new Set();
    const filenameOf = (index) => {
      const record = records[index];
      return record.runtimePath ? bundle.root + record.runtimePath : bundle.filename || bundle.root + record.path;
    };
    const requires = [];
    // Register paths eagerly, but build closures only for modules whose
    // require function is used. Lazy dependency graphs need no JS functions.
    const getRequire = (index) => {
      if (requires[index]) return requires[index];
      const record = records[index];
      const targetOf = (specifier) => {
        if (typeof specifier !== 'string') {
          const error = new TypeError('The "id" argument must be of type string');
          error.code = 'ERR_INVALID_ARG_TYPE';
          throw error;
        }
        if (own(record.dependencies, specifier)) return record.dependencies[specifier];
        // Absolute and computed relative filenames can name modules already
        // present in this graph without evaluating them during resolution.
        if (/^(?:\.{1,2}[\\/]|[\\/]|[A-Za-z]:[\\/])/.test(specifier)) {
          const path = /^[\\/]|^[A-Za-z]:/.test(specifier) ? specifier : dirname(filenameOf(index)) + '/' + specifier;
          for (const suffix of ['', '.js', '.json', '/index.js', '/index.json']) {
            const key = pathKey(path + suffix);
            const found = records.findIndex((_, candidate) => pathKey(filenameOf(candidate)) === key);
            if (found >= 0) return found;
            for (const candidate of records) {
              for (const target of Object.values(candidate.dependencies)) {
                if (typeof target.file === 'string' && pathKey(bundle.root + target.file) === key) return target;
              }
            }
          }
        }
        builtin(specifier);
        return { builtin: specifier };
      };
      const require = (specifier) => {
        const target = targetOf(specifier);
        if (typeof target === 'number') return instantiate(target, cache[index] || null);
        if (target.empty === true) return {};
        if (typeof target.builtin === 'string') return builtin(target.builtin);
        if (typeof target.file === 'string') throw notFound(specifier, ': this file has no compiled module');
        throw notFound(target.missing, '');
      };
      require.resolve = (specifier) => {
        const target = targetOf(specifier);
        if (typeof target === 'number') return filenameOf(target);
        if (target.empty === true) return specifier;
        if (typeof target.builtin === 'string') return target.builtin;
        if (typeof target.file === 'string') return bundle.root + target.file;
        throw notFound(target.missing, '');
      };
      require.cache = sharedCache;
      requires[index] = require;
      return require;
    };
    for (let index = 0; index < records.length; index++) {
      const key = pathKey(filenameOf(index));
      // Flattened application modules may share an emitted filename; keep
      // its entry context. External packages have their own runtime paths.
      if (!registered.has(key)) {
        contexts.set(key, { getRequire, index });
        registered.add(key);
      }
    }
    const instantiate = (index, parent) => {
      if (cache[index]) return cache[index].exports;
      // Bundled entries run from their output file, like after a build.
      const filename = filenameOf(index);
      const module = {
        id: filename,
        filename,
        path: dirname(filename),
        exports: {},
        loaded: false,
        parent,
        children: [],
      };
      if (parent) parent.children.push(module);
      const require = getRequire(index);
      module.require = require;
      const filePath = filename.replace(/\\/g, '/');
      const encoded = filePath.split('/').map((part) => encodeURIComponent(part).replace(/%3A/i, ':')).join('/');
      const meta = {
        url: filePath.startsWith('//') ? `file:${encoded}` : `file:///${encoded.replace(/^\//, '')}`,
        env: bundle.env,
        filename,
        dirname: module.path,
      };
      cache[index] = module;
      sharedCache[filename] = module;
      const factory = bundle.compile(index, filename, names.join(', '));
      try {
        factory.call(module.exports, module.exports, require, module, filename, module.path, meta, require, ...values);
      } catch (error) {
        cache[index] = undefined;
        delete sharedCache[filename];
        throw error;
      }
      module.loaded = true;
      return module.exports;
    };
    for (const entry of bundle.entries) {
      const record = records[entry.index];
      if (entry.classic) bundle.evaluate(entry.index, record.path);
      else instantiate(entry.index, null);
    }
  });

  // Console. ---------------------------------------------------------------
  const identifier = /^[A-Za-z_$][\w$]*$/;
  const quote = (text) => `'${text.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/\n/g, '\\n')}'`;
  const constructorName = (value) => {
    const prototype = Object.getPrototypeOf(value);
    if (prototype === null) return '[Object: null prototype] ';
    const name = prototype.constructor && prototype.constructor.name;
    return name && name !== 'Object' ? `${name} ` : '';
  };
  const inspect = (value, depth, seen, top) => {
    switch (typeof value) {
      case 'string':
        return top ? value : quote(value);
      case 'number':
        return Object.is(value, -0) ? '-0' : String(value);
      case 'bigint':
        return `${value}n`;
      case 'symbol':
        return value.toString();
      case 'function':
        return value.name ? `[Function: ${value.name}]` : '[Function (anonymous)]';
      case 'undefined':
      case 'boolean':
        return String(value);
    }
    if (value === null) return 'null';
    if (value instanceof Error) return host.describe(value);
    if (value instanceof Date) return Number.isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
    if (value instanceof RegExp) return String(value);
    if (value instanceof Promise) return 'Promise { <pending> }';
    if (seen.has(value)) return '[Circular]';
    if (depth < 0) return Array.isArray(value) ? '[Array]' : '[Object]';
    seen.add(value);
    const nested = (item) => inspect(item, depth - 1, seen, false);
    let items;
    let open;
    if (Array.isArray(value)) {
      items = value.slice(0, 100).map(nested);
      if (value.length > 100) items.push(`... ${value.length - 100} more items`);
      open = '';
    } else if (value instanceof Map) {
      items = [...value].map(([key, item]) => `${nested(key)} => ${nested(item)}`);
      open = `Map(${value.size}) `;
    } else if (value instanceof Set) {
      items = [...value].map(nested);
      open = `Set(${value.size}) `;
    } else if (ArrayBuffer.isView(value)) {
      items = Array.from(value, (item) => String(item));
      open = `${value.constructor.name}(${value.length}) `;
    } else {
      items = Object.keys(value).map(
        (key) => `${identifier.test(key) ? key : quote(key)}: ${nested(value[key])}`,
      );
      open = constructorName(value);
    }
    seen.delete(value);
    const [left, right] = Array.isArray(value) || ArrayBuffer.isView(value) ? ['[', ']'] : ['{', '}'];
    return items.length ? `${open}${left} ${items.join(', ')} ${right}` : `${open}${left}${right}`;
  };
  const show = (value) => inspect(value, 2, new Set(), true);
  const format = (args) => {
    if (typeof args[0] !== 'string') return args.map(show).join(' ');
    let index = 1;
    const text = args[0].replace(/%[sdifjoOc%]/g, (token) => {
      if (token === '%%') return '%';
      if (index >= args.length) return token;
      const argument = args[index++];
      switch (token) {
        case '%s':
          return typeof argument === 'string' ? argument : inspect(argument, 1, new Set(), false);
        case '%d':
        case '%i': {
          const number = Number(argument);
          return String(token === '%i' ? Math.trunc(number) : number);
        }
        case '%f':
          return String(Number(argument));
        case '%j':
          return JSON.stringify(argument);
        case '%c':
          return '';
        default:
          return inspect(argument, 4, new Set(), false);
      }
    });
    return [text, ...args.slice(index).map(show)].join(' ');
  };
  let indent = '';
  const counts = new Map();
  const started = new Map();
  const print = (level, args) => {
    const text = format(args);
    host.print(level, indent ? text.split('\n').map((line) => indent + line).join('\n') : text);
  };
  const console = {};
  for (const level of ['log', 'info', 'debug', 'warn', 'error']) {
    console[level] = (...args) => print(level, args);
  }
  console.trace = (...args) => print('error', ['Trace:', ...args]);
  console.dir = (value) => print('log', [value]);
  console.table = (value) => print('log', [value]);
  console.assert = (condition, ...args) => {
    if (!condition) print('error', ['Assertion failed', ...args]);
  };
  console.group = (...args) => {
    if (args.length) print('log', args);
    indent += '  ';
  };
  console.groupCollapsed = console.group;
  console.groupEnd = () => {
    indent = indent.slice(2);
  };
  console.count = (label = 'default') => {
    const count = (counts.get(label) || 0) + 1;
    counts.set(label, count);
    print('log', [`${label}: ${count}`]);
  };
  console.countReset = (label = 'default') => counts.delete(label);
  console.time = (label = 'default') => started.set(label, host.now());
  console.timeLog = (label = 'default', ...args) => {
    if (started.has(label)) print('log', [`${label}: ${host.now() - started.get(label)}ms`, ...args]);
  };
  console.timeEnd = (label = 'default') => {
    console.timeLog(label);
    started.delete(label);
  };
  hidden(global, 'console', console);
  hidden(host, 'inspect', show);
  hidden(host, 'format', format);

  // Errors. ----------------------------------------------------------------
  hidden(host, 'describe', (error) => {
    if (!(error instanceof Error)) return inspect(error, 2, new Set(), false);
    const head = `${error.name}: ${error.message}`;
    const stack = typeof error.stack === 'string' ? error.stack.trimEnd() : '';
    if (stack.startsWith(head)) return stack;
    return stack ? `${head}\n${stack}` : head;
  });
  hidden(host, 'toJSON', (value) => {
    const text = JSON.stringify(value);
    return text === undefined ? 'null' : text;
  });
  hidden(host, 'report', (error, promise) => {
    if (typeof host.onerror === 'function') {
      try {
        if (host.onerror(error, promise) === true) return;
      } catch (inner) {
        host.print('error', `Uncaught ${host.describe(inner)}`);
      }
    }
    host.print('error', `Uncaught ${promise ? '(in promise) ' : ''}${host.describe(error)}`);
  });

  // Timers and microtasks. -------------------------------------------------
  const handleOf = (id) => {
    if (!node) return id;
    const handle = {
      ref: () => handle,
      unref: () => handle,
      hasRef: () => true,
      refresh: () => {
        host.timerRefresh(id);
        return handle;
      },
      [Symbol.toPrimitive]: () => id,
    };
    return handle;
  };
  const schedule = (callback, delay, args, repeat) => {
    if (typeof callback !== 'function') {
      throw new TypeError('The "callback" argument must be of type function');
    }
    const wait = Math.max(0, Number(delay) || 0);
    const id = host.timerSchedule(callback, wait, args, repeat);
    return handleOf(id);
  };
  const cancel = (handle) => {
    host.timerCancel(isObject(handle) ? Number(handle) : handle);
  };
  hidden(global, 'setTimeout', (callback, delay, ...args) => schedule(callback, delay, args, false));
  hidden(global, 'setInterval', (callback, delay, ...args) => schedule(callback, delay, args, true));
  hidden(global, 'setImmediate', (callback, ...args) => schedule(callback, 0, args, false));
  hidden(global, 'clearTimeout', cancel);
  hidden(global, 'clearInterval', cancel);
  hidden(global, 'clearImmediate', cancel);
  hidden(global, 'queueMicrotask', (callback) => {
    if (typeof callback !== 'function') {
      throw new TypeError('The "callback" argument must be of type function');
    }
    Promise.resolve().then(() => callback());
  });
  hidden(host, 'timers', {
    next: host.timerNext,
    run: host.timerRun,
  });

  // Background work: natives started with a task id send events back to the
  // task's handler on this thread. Ids are unique in the process, so natives
  // can key shared state by them.
  const tasks = new Map();
  hidden(host, 'task', (handler) => {
    const id = host.taskId();
    tasks.set(id, handler);
    return id;
  });
  hidden(host, 'endTask', (id) => tasks.delete(id));
  hidden(host, 'complete', (id, event, value) => {
    const handler = tasks.get(id);
    if (!handler) return;
    try {
      handler(event, value);
    } catch (error) {
      host.report(error, false);
    }
  });

  // Web platform globals shared by every context. --------------------------

  const exceptionCodes = {
    IndexSizeError: 1,
    HierarchyRequestError: 3,
    WrongDocumentError: 4,
    InvalidCharacterError: 5,
    NoModificationAllowedError: 7,
    NotFoundError: 8,
    NotSupportedError: 9,
    InvalidStateError: 11,
    SyntaxError: 12,
    InvalidModificationError: 13,
    NamespaceError: 14,
    InvalidAccessError: 15,
    TypeMismatchError: 17,
    SecurityError: 18,
    NetworkError: 19,
    AbortError: 20,
    URLMismatchError: 21,
    QuotaExceededError: 22,
    TimeoutError: 23,
    InvalidNodeTypeError: 24,
    DataCloneError: 25,
  };

  class DOMException extends Error {
    constructor(message = '', name = 'Error') {
      super(message);
      Object.defineProperty(this, 'name', { value: String(name), writable: true, configurable: true });
    }

    get code() {
      return exceptionCodes[this.name] || 0;
    }
  }

  // URLs. --------------------------------------------------------------------------

  const PARTS = Symbol('nativeboat.url');
  const specialPorts = { 'http:': '80', 'https:': '443', 'ws:': '80', 'wss:': '443', 'ftp:': '21', 'file:': '' };
  const encodePath = (path) => path.replace(/[\s"<>`{}]/g, (char) => encodeURIComponent(char));

  const normalizePath = (path) => {
    const output = [];
    const segments = path.split('/');
    for (let index = 1; index < segments.length; index++) {
      const segment = segments[index];
      const last = index === segments.length - 1;
      if (segment === '..' || segment.toLowerCase() === '%2e%2e') {
        output.pop();
        if (last) output.push('');
      } else if (segment === '.' || segment.toLowerCase() === '%2e') {
        if (last) output.push('');
      } else output.push(segment);
    }
    return `/${output.join('/')}`;
  };

  const splitTail = (text) => {
    const hashAt = text.indexOf('#');
    const hash = hashAt < 0 ? '' : text.slice(hashAt);
    const beforeHash = hashAt < 0 ? text : text.slice(0, hashAt);
    const queryAt = beforeHash.indexOf('?');
    return {
      path: queryAt < 0 ? beforeHash : beforeHash.slice(0, queryAt),
      query: queryAt < 0 ? '' : beforeHash.slice(queryAt),
      hash,
    };
  };

  const parseURL = (input, base) => {
    const text = input.replace(/^[\u0000-\u0020]+|[\u0000-\u0020]+$/g, '').replace(/[\t\n\r]/g, '');
    const scheme = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(text);
    if (!scheme) {
      if (!base) return null;
      if (base.opaque) return text.startsWith('#') ? { ...base, hash: text === '#' ? '' : text } : null;
      if (text.startsWith('//') || (base.protocol in specialPorts && text.startsWith('\\\\'))) {
        return parseURL(base.protocol + text, null);
      }
      const { path, query, hash } = splitTail(text);
      const special = base.protocol in specialPorts;
      let pathname;
      if (path.startsWith('/') || (special && path.startsWith('\\'))) pathname = path;
      else if (path === '') pathname = base.pathname;
      else pathname = base.pathname.slice(0, base.pathname.lastIndexOf('/') + 1) + path;
      const keepsQuery = path === '' && query === '';
      return {
        ...base,
        pathname: encodePath(normalizePath(pathname.replace(/\\/g, '/'))),
        search: keepsQuery ? base.search : query === '?' ? '' : query,
        hash: hash === '#' ? '' : hash,
      };
    }
    const protocol = scheme[1].toLowerCase() + ':';
    let rest = text.slice(scheme[0].length);
    const special = protocol in specialPorts;
    if (!special && !rest.startsWith('//')) {
      const { path, query, hash } = splitTail(rest);
      return { protocol, username: '', password: '', hostname: '', port: '', pathname: path, search: query === '?' ? '' : query, hash: hash === '#' ? '' : hash, opaque: true };
    }
    if (protocol === 'file:') {
      let after = rest.replace(/\\/g, '/');
      let hostname = '';
      if (after.startsWith('//')) {
        after = after.slice(2);
        const end = after.search(/[/?#]/);
        hostname = end < 0 ? after : after.slice(0, end);
        after = end < 0 ? '' : after.slice(end);
      }
      // A drive letter is part of the path, with its case kept.
      if (/^[a-zA-Z][:|]$/.test(hostname)) {
        after = `/${hostname}${after}`;
        hostname = '';
      }
      hostname = hostname.toLowerCase();
      const { path, query, hash } = splitTail(after);
      return {
        protocol,
        username: '',
        password: '',
        hostname,
        port: '',
        pathname: encodePath(normalizePath(path.startsWith('/') ? path : `/${path}`)),
        search: query === '?' ? '' : query,
        hash: hash === '#' ? '' : hash,
        opaque: false,
      };
    }
    rest = special ? rest.replace(/^[/\\]*/, '') : rest.slice(2);
    const authorityEnd = rest.search(/[/\\?#]/);
    const authority = authorityEnd < 0 ? rest : rest.slice(0, authorityEnd);
    const { path, query, hash } = splitTail(authorityEnd < 0 ? '' : rest.slice(authorityEnd));
    const at = authority.lastIndexOf('@');
    const credentials = at < 0 ? '' : authority.slice(0, at);
    const hostPort = at < 0 ? authority : authority.slice(at + 1);
    const portMatch = /:(\d*)$/.exec(hostPort);
    const hostname = (portMatch ? hostPort.slice(0, portMatch.index) : hostPort).toLowerCase();
    if (special && hostname === '') return null;
    const port = portMatch && portMatch[1] !== specialPorts[protocol] ? portMatch[1] : '';
    const [username = '', password = ''] = credentials ? credentials.split(':') : [];
    return {
      protocol,
      username,
      password,
      hostname,
      port,
      pathname: encodePath(normalizePath(((special ? path || '/' : path) || '').replace(/\\/g, '/'))),
      search: query === '?' ? '' : query,
      hash: hash === '#' ? '' : hash,
      opaque: false,
    };
  };

  const serializeURL = (parts) => {
    if (parts.opaque) return `${parts.protocol}${parts.pathname}${parts.search}${parts.hash}`;
    const credentials = parts.username ? `${parts.username}${parts.password ? `:${parts.password}` : ''}@` : '';
    const host = parts.port ? `${parts.hostname}:${parts.port}` : parts.hostname;
    return `${parts.protocol}//${credentials}${host}${parts.pathname}${parts.search}${parts.hash}`;
  };

  const formDecode = (text) => {
    try {
      return decodeURIComponent(text.replace(/\+/g, ' '));
    } catch (error) {
      return text;
    }
  };
  const formEncode = (text) =>
    encodeURIComponent(text).replace(/%20/g, '+').replace(/[!'()~]/g, (char) => `%${char.charCodeAt(0).toString(16).toUpperCase()}`);

  const LISTENER = Symbol('nativeboat.params');
  class URLSearchParams {
    constructor(init = '') {
      hidden(this, PARTS, []);
      hidden(this, LISTENER, null);
      if (init instanceof URLSearchParams) this[PARTS].push(...init[PARTS].map((pair) => [...pair]));
      else if (typeof init === 'object' && init !== null) {
        const pairs = typeof init[Symbol.iterator] === 'function' ? [...init] : Object.entries(init);
        for (const pair of pairs) {
          const [name, value] = [...pair];
          this[PARTS].push([String(name), String(value)]);
        }
      } else this.parse(String(init));
    }

    parse(text) {
      this[PARTS].length = 0;
      for (const part of text.replace(/^\?/, '').split('&')) {
        if (!part) continue;
        const equals = part.indexOf('=');
        const name = equals < 0 ? part : part.slice(0, equals);
        const value = equals < 0 ? '' : part.slice(equals + 1);
        this[PARTS].push([formDecode(name), formDecode(value)]);
      }
    }

    update() {
      if (this[LISTENER]) this[LISTENER](this.toString());
    }

    get size() {
      return this[PARTS].length;
    }

    append(name, value) {
      this[PARTS].push([String(name), String(value)]);
      this.update();
    }

    delete(name, value) {
      const remaining = this[PARTS].filter(([key, item]) => key !== String(name) || (value !== undefined && item !== String(value)));
      this[PARTS].splice(0, this[PARTS].length, ...remaining);
      this.update();
    }

    get(name) {
      const pair = this[PARTS].find(([key]) => key === String(name));
      return pair ? pair[1] : null;
    }

    getAll(name) {
      return this[PARTS].filter(([key]) => key === String(name)).map(([, value]) => value);
    }

    has(name, value) {
      return this[PARTS].some(([key, item]) => key === String(name) && (value === undefined || item === String(value)));
    }

    set(name, value) {
      const index = this[PARTS].findIndex(([key]) => key === String(name));
      if (index < 0) this[PARTS].push([String(name), String(value)]);
      else {
        this[PARTS][index][1] = String(value);
        const kept = this[PARTS].filter(([key], position) => key !== String(name) || position <= index);
        this[PARTS].splice(0, this[PARTS].length, ...kept);
      }
      this.update();
    }

    sort() {
      this[PARTS].sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
      this.update();
    }

    forEach(callback, thisArg) {
      for (const [name, value] of this[PARTS]) callback.call(thisArg, value, name, this);
    }

    keys() {
      return this[PARTS].map(([name]) => name)[Symbol.iterator]();
    }

    values() {
      return this[PARTS].map(([, value]) => value)[Symbol.iterator]();
    }

    entries() {
      return this[PARTS].map((pair) => [...pair])[Symbol.iterator]();
    }

    [Symbol.iterator]() {
      return this.entries();
    }

    toString() {
      return this[PARTS].map(([name, value]) => `${formEncode(name)}=${formEncode(value)}`).join('&');
    }
  }

  class URL {
    constructor(url, base) {
      const baseParts = base === undefined ? null : parseURL(String(base), null);
      if (base !== undefined && !baseParts) throw new TypeError(`Invalid base URL: ${base}`);
      const parts = parseURL(String(url), baseParts);
      if (!parts) throw new TypeError(`Invalid URL: ${url}`);
      hidden(this, PARTS, parts);
      const params = new URLSearchParams(parts.search);
      params[LISTENER] = (query) => {
        this[PARTS].search = query ? `?${query}` : '';
      };
      hidden(this, '_params', params);
    }

    static canParse(url, base) {
      try {
        new URL(url, base);
        return true;
      } catch (error) {
        return false;
      }
    }

    static createObjectURL() {
      return `blob:nodedata:${Math.random().toString(16).slice(2)}`;
    }

    static revokeObjectURL() {}

    get href() {
      return serializeURL(this[PARTS]);
    }

    set href(value) {
      const parts = parseURL(String(value), null);
      if (!parts) throw new TypeError(`Invalid URL: ${value}`);
      this[PARTS] = parts;
      this._params.parse(parts.search);
    }

    get origin() {
      const parts = this[PARTS];
      if (parts.opaque || parts.protocol === 'file:' || !(parts.protocol in specialPorts)) return 'null';
      return `${parts.protocol}//${this.host}`;
    }

    get protocol() {
      return this[PARTS].protocol;
    }

    set protocol(value) {
      const protocol = String(value).replace(/:?$/, ':').toLowerCase();
      if (/^[a-z][a-z0-9+.-]*:$/.test(protocol)) this[PARTS].protocol = protocol;
    }

    get username() {
      return this[PARTS].username;
    }

    set username(value) {
      this[PARTS].username = String(value);
    }

    get password() {
      return this[PARTS].password;
    }

    set password(value) {
      this[PARTS].password = String(value);
    }

    get host() {
      const parts = this[PARTS];
      return parts.port ? `${parts.hostname}:${parts.port}` : parts.hostname;
    }

    set host(value) {
      const [hostname, port = ''] = String(value).split(':');
      Object.assign(this[PARTS], { hostname: hostname.toLowerCase(), port });
    }

    get hostname() {
      return this[PARTS].hostname;
    }

    set hostname(value) {
      this[PARTS].hostname = String(value).toLowerCase();
    }

    get port() {
      return this[PARTS].port;
    }

    set port(value) {
      const port = String(value);
      this[PARTS].port = port === specialPorts[this[PARTS].protocol] ? '' : port;
    }

    get pathname() {
      return this[PARTS].pathname;
    }

    set pathname(value) {
      const path = String(value);
      this[PARTS].pathname = encodePath(normalizePath(path.startsWith('/') ? path : `/${path}`));
    }

    get search() {
      return this[PARTS].search;
    }

    set search(value) {
      const query = String(value).replace(/^\?/, '');
      this[PARTS].search = query ? `?${query}` : '';
      this._params.parse(query);
    }

    get searchParams() {
      return this._params;
    }

    get hash() {
      return this[PARTS].hash;
    }

    set hash(value) {
      const hash = String(value).replace(/^#/, '');
      this[PARTS].hash = hash ? `#${hash}` : '';
    }

    toString() {
      return this.href;
    }

    toJSON() {
      return this.href;
    }
  }

  // Encoding and cloning. ----------------------------------------------------------

  class TextEncoder {
    get encoding() {
      return 'utf-8';
    }

    encode(input = '') {
      const text = String(input);
      const bytes = [];
      for (const char of text) {
        let code = char.codePointAt(0);
        if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
        if (code < 0x80) bytes.push(code);
        else if (code < 0x800) bytes.push(0xc0 | (code >> 6), 0x80 | (code & 63));
        else if (code < 0x10000) bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63));
        else bytes.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 63), 0x80 | ((code >> 6) & 63), 0x80 | (code & 63));
      }
      return new Uint8Array(bytes);
    }

    encodeInto(input, destination) {
      const bytes = this.encode(input);
      const written = Math.min(bytes.length, destination.length);
      destination.set(bytes.subarray(0, written));
      return { read: String(input).length, written };
    }
  }

  const decodeUTF8 = (bytes, fatal) => {
    let text = '';
    for (let index = 0; index < bytes.length; ) {
      const first = bytes[index];
      const width = first < 0x80 ? 1 : first >= 0xf0 && first < 0xf8 ? 4 : first >= 0xe0 ? 3 : first >= 0xc0 ? 2 : 0;
      let code = width === 1 ? first : width ? first & (0xff >> (width + 1)) : -1;
      for (let offset = 1; offset < width && code >= 0; offset++) {
        const next = bytes[index + offset];
        code = next !== undefined && (next & 0xc0) === 0x80 ? (code << 6) | (next & 63) : -1;
      }
      if (code < 0 || code > 0x10ffff) {
        if (fatal) throw new TypeError('The encoded data was not valid UTF-8.');
        text += '\ufffd';
        index += 1;
        continue;
      }
      text += String.fromCodePoint(code);
      index += width;
    }
    return text;
  };
  class TextDecoder {
    constructor(label = 'utf-8', options = {}) {
      const encoding = String(label).toLowerCase();
      if (!['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(encoding)) {
        throw new RangeError(`The encoding '${label}' is not supported.`);
      }
      hidden(this, '_fatal', !!options.fatal);
      hidden(this, '_ignoreBOM', !!options.ignoreBOM);
    }

    get encoding() {
      return 'utf-8';
    }

    get fatal() {
      return this._fatal;
    }

    get ignoreBOM() {
      return this._ignoreBOM;
    }

    decode(input = new Uint8Array(0)) {
      const bytes = input instanceof ArrayBuffer ? new Uint8Array(input) : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      const text = decodeUTF8(bytes, this._fatal);
      return !this._ignoreBOM && text.startsWith('\ufeff') ? text.slice(1) : text;
    }
  }

  const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const btoa = (input) => {
    const text = String(input);
    if (/[^\u0000-\u00ff]/.test(text)) {
      throw new DOMException('The string to be encoded contains characters outside of the Latin1 range.', 'InvalidCharacterError');
    }
    let output = '';
    for (let index = 0; index < text.length; index += 3) {
      const [a, b, c] = [text.charCodeAt(index), text.charCodeAt(index + 1), text.charCodeAt(index + 2)];
      const triple = (a << 16) | ((b || 0) << 8) | (c || 0);
      output += BASE64[triple >> 18] + BASE64[(triple >> 12) & 63];
      output += Number.isNaN(b) ? '==' : BASE64[(triple >> 6) & 63] + (Number.isNaN(c) ? '=' : BASE64[triple & 63]);
    }
    return output;
  };
  const atob = (input) => {
    const text = String(input).replace(/[\t\n\f\r ]/g, '');
    const clean = text.length % 4 === 0 ? text.replace(/==?$/, '') : text;
    if (clean.length % 4 === 1 || /[^A-Za-z0-9+/]/.test(clean)) {
      throw new DOMException('The string to be decoded is not correctly encoded.', 'InvalidCharacterError');
    }
    let output = '';
    let buffer = 0;
    let bits = 0;
    for (const char of clean) {
      buffer = (buffer << 6) | BASE64.indexOf(char);
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        output += String.fromCharCode((buffer >> bits) & 0xff);
      }
    }
    return output;
  };

  const structuredClone = (value, seen = new Map()) => {
    if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
      if (typeof value === 'symbol') throw new DOMException('A symbol could not be cloned.', 'DataCloneError');
      return value;
    }
    if (typeof value === 'function') throw new DOMException(`${value.name || 'function'} could not be cloned.`, 'DataCloneError');
    if (seen.has(value)) return seen.get(value);
    let copy;
    if (value instanceof Date) copy = new Date(value.getTime());
    else if (value instanceof RegExp) copy = new RegExp(value.source, value.flags);
    else if (value instanceof ArrayBuffer) copy = value.slice(0);
    else if (ArrayBuffer.isView(value)) copy = new value.constructor(value);
    else if (value instanceof Error) {
      copy = new (global[value.name] || Error)(value.message);
      copy.stack = value.stack;
    } else if (value instanceof Map) {
      copy = new Map();
      seen.set(value, copy);
      for (const [key, item] of value) copy.set(structuredClone(key, seen), structuredClone(item, seen));
      return copy;
    } else if (value instanceof Set) {
      copy = new Set();
      seen.set(value, copy);
      for (const item of value) copy.add(structuredClone(item, seen));
      return copy;
    } else {
      copy = Array.isArray(value) ? [] : {};
      seen.set(value, copy);
      for (const key of Object.keys(value)) copy[key] = structuredClone(value[key], seen);
      return copy;
    }
    seen.set(value, copy);
    return copy;
  };

  const crypto = {
    getRandomValues(array) {
      if (!ArrayBuffer.isView(array) || array instanceof DataView) throw new TypeError('The value must be a typed array');
      if (array.byteLength > 65536) {
        throw new DOMException(`The ArrayBufferView's byte length (${array.byteLength}) exceeds 65536`, 'QuotaExceededError');
      }
      new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(host.random(array.byteLength));
      return array;
    },
    randomUUID() {
      const bytes = host.random(16);
      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;
      const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    },
    subtle: undefined,
  };

  for (const [name, value] of Object.entries({
      DOMException,
      URL,
      URLSearchParams,
      TextEncoder,
      TextDecoder,
      atob,
      btoa,
      structuredClone: (value) => structuredClone(value),
      crypto,
    })) {
      hidden(global, name, value);
    }

  // Values crossing between contexts (IPC) as text. Bytes, dates, maps, sets,
  // errors and undefined survive the trip.
  const encodeBytes = (view) => {
    const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    let text = '';
    for (const byte of bytes) text += String.fromCharCode(byte);
    return btoa(text);
  };
  hidden(host, 'serialize', (value) =>
    JSON.stringify(value === undefined ? { $nui: 'undefined' } : value, function (key, item) {
      const raw = this[key];
      if (raw instanceof Date) return { $nui: 'date', value: raw.getTime() };
      if (ArrayBuffer.isView(raw)) return { $nui: 'bytes', value: encodeBytes(raw) };
      if (raw instanceof ArrayBuffer) return { $nui: 'bytes', value: encodeBytes(new Uint8Array(raw)) };
      if (raw instanceof Map) return { $nui: 'map', value: [...raw] };
      if (raw instanceof Set) return { $nui: 'set', value: [...raw] };
      if (raw instanceof Error) return { $nui: 'error', name: raw.name, message: raw.message, stack: raw.stack };
      if (raw === undefined && Array.isArray(this)) return { $nui: 'undefined' };
      if (typeof raw === 'function' || typeof raw === 'symbol') return undefined;
      return item;
    }),
  );
  const revive = (item) => {
    switch (item.$nui) {
      case 'date':
        return new Date(item.value);
      case 'bytes': {
        const text = atob(item.value);
        const bytes = Uint8Array.from(text, (char) => char.charCodeAt(0));
        return typeof global.Buffer === 'function' ? global.Buffer.from(bytes) : bytes;
      }
      case 'map':
        return new Map(item.value);
      case 'set':
        return new Set(item.value);
      case 'error': {
        const error = new (global[item.name] || Error)(item.message);
        if (!(global[item.name] instanceof Function)) error.name = item.name;
        error.stack = item.stack;
        return error;
      }
      default:
        return undefined;
    }
  };
  hidden(host, 'deserialize', (text) =>
    JSON.parse(text, (key, item) => (item !== null && typeof item === 'object' && typeof item.$nui === 'string' ? revive(item) : item)),
  );
});
