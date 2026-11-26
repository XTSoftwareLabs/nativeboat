// The global scope of a dedicated worker, which a worker's realm runs in
// place of the window's prelude: events, the messages a worker exchanges
// with its page, `importScripts`, and the script the worker was started
// with. That is a compiled module of the program, which `host.import` loads
// by its URL, or a script the program made while it ran, as a blob: a
// program cannot be compiled once it runs, so `evaluate` runs what such a
// script does when it only declares, assigns, calls and imports, as the
// scripts that start workers do.
//
// `host` is the page's host with the worker's additions, `port` the handle
// of the worker's end of its message channel, `errors` that of the channel
// that reports its errors to the page, `url` the script's URL, `name` the
// worker's name and `script` the text of a blob script, or null. A service
// worker has no ports; `service` is its scope, and null for other workers.

const global = globalThis;
const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
const hidden = (target, key, value) =>
  Object.defineProperty(target, key, { value, writable: true, configurable: true, enumerable: false });
const illegal = () => {
  throw new TypeError('Illegal constructor');
};

// Events. ------------------------------------------------------------------------

const EVENT = Symbol('nativeboat.event');
class Event {
  constructor(type, init = {}) {
    if (arguments.length === 0) throw new TypeError("Failed to construct 'Event': 1 argument required, but only 0 present.");
    const options = init === null || init === undefined ? {} : init;
    hidden(this, EVENT, {
      type: String(type),
      bubbles: !!options.bubbles,
      cancelable: !!options.cancelable,
      composed: !!options.composed,
      canceled: false,
      stopped: false,
      trusted: false,
      target: null,
      currentTarget: null,
      phase: 0,
      timeStamp: performance.now(),
    });
  }
  get type() { return this[EVENT].type; }
  get bubbles() { return this[EVENT].bubbles; }
  get cancelable() { return this[EVENT].cancelable; }
  get composed() { return this[EVENT].composed; }
  get defaultPrevented() { return this[EVENT].canceled; }
  get returnValue() { return !this[EVENT].canceled; }
  get target() { return this[EVENT].target; }
  get srcElement() { return this[EVENT].target; }
  get currentTarget() { return this[EVENT].currentTarget; }
  get eventPhase() { return this[EVENT].phase; }
  get isTrusted() { return this[EVENT].trusted; }
  get timeStamp() { return this[EVENT].timeStamp; }
  composedPath() {
    const target = this[EVENT].currentTarget;
    return target ? [target] : [];
  }
  preventDefault() {
    if (this[EVENT].cancelable) this[EVENT].canceled = true;
  }
  stopPropagation() {}
  stopImmediatePropagation() {
    this[EVENT].stopped = true;
  }
}
for (const [name, value] of [['NONE', 0], ['CAPTURING_PHASE', 1], ['AT_TARGET', 2], ['BUBBLING_PHASE', 3]]) {
  Object.defineProperty(Event, name, { value, enumerable: true });
  Object.defineProperty(Event.prototype, name, { value, enumerable: true });
}

// An event class whose instances carry the fields of their init dictionary.
const eventClass = (name, fields) => {
  const constructor = class extends Event {
    constructor(type, init = {}) {
      super(type, init);
      const options = init === null || init === undefined ? {} : init;
      const values = {};
      for (const [key, fallback] of Object.entries(fields)) values[key] = options[key] === undefined ? fallback : options[key];
      hidden(this, FIELDS, values);
    }
  };
  for (const key of Object.keys(fields)) {
    Object.defineProperty(constructor.prototype, key, {
      get() { return this[FIELDS][key]; },
      configurable: true,
      enumerable: true,
    });
  }
  Object.defineProperty(constructor, 'name', { value: name });
  return constructor;
};
const FIELDS = Symbol('nativeboat.fields');
const MessageEvent = eventClass('MessageEvent', { data: null, origin: '', lastEventId: '', source: null, ports: [] });
const ErrorEvent = eventClass('ErrorEvent', { message: '', filename: '', lineno: 0, colno: 0, error: undefined });
const PromiseRejectionEvent = eventClass('PromiseRejectionEvent', { promise: undefined, reason: undefined });
const CustomEvent = eventClass('CustomEvent', { detail: null });

const LISTENERS = Symbol('nativeboat.listeners');
const listenersOf = (target) => {
  if (!own(target, LISTENERS)) hidden(target, LISTENERS, new Map());
  return target[LISTENERS];
};
const captureOf = (options) => (typeof options === 'boolean' ? options : !!(options && options.capture));

class EventTarget {
  addEventListener(type, callback, options) {
    if (callback === null || callback === undefined) return;
    const capture = captureOf(options);
    const once = !!(options && typeof options === 'object' && options.once);
    const signal = options && typeof options === 'object' ? options.signal : undefined;
    if (signal && signal.aborted) return;
    const listeners = listenersOf(this);
    const key = String(type);
    const list = listeners.get(key) || [];
    if (list.some((entry) => entry.callback === callback && entry.capture === capture)) return;
    list.push({ callback, capture, once, removed: false });
    listeners.set(key, list);
    if (signal && typeof signal.addEventListener === 'function') {
      signal.addEventListener('abort', () => this.removeEventListener(key, callback, { capture }));
    }
  }

  removeEventListener(type, callback, options) {
    const capture = captureOf(options);
    const list = listenersOf(this).get(String(type));
    if (!list) return;
    const index = list.findIndex((entry) => entry.callback === callback && entry.capture === capture);
    if (index < 0) return;
    list[index].removed = true;
    list.splice(index, 1);
  }

  dispatchEvent(event) {
    if (!(event instanceof Event)) {
      throw new TypeError("Failed to execute 'dispatchEvent' on 'EventTarget': parameter 1 is not of type 'Event'.");
    }
    const state = event[EVENT];
    state.target = this;
    state.currentTarget = this;
    state.phase = 2;
    for (const entry of [...(listenersOf(this).get(state.type) || [])]) {
      if (entry.removed) continue;
      if (entry.once) this.removeEventListener(state.type, entry.callback, { capture: entry.capture });
      try {
        if (typeof entry.callback === 'function') entry.callback.call(this, event);
        else if (entry.callback && typeof entry.callback.handleEvent === 'function') entry.callback.handleEvent(event);
      } catch (error) {
        reportError(error);
      }
      if (state.stopped) break;
    }
    state.phase = 0;
    state.currentTarget = null;
    return !state.canceled;
  }
}

// Creates and dispatches an event the platform raises: whether no listener
// canceled it.
const fire = (target, constructor, type, init = {}) => {
  const event = new constructor(type, init);
  event[EVENT].trusted = true;
  return target.dispatchEvent(event);
};

// `on<event>` handler properties.
const HANDLERS = Symbol('nativeboat.handlers');
const defineHandlers = (prototype, names) => {
  for (const name of names) {
    Object.defineProperty(prototype, `on${name}`, {
      get() {
        return (own(this, HANDLERS) && this[HANDLERS][name]) || null;
      },
      set(value) {
        if (!own(this, HANDLERS)) hidden(this, HANDLERS, {});
        const handlers = this[HANDLERS];
        if (handlers[`${name}:listener`]) this.removeEventListener(name, handlers[`${name}:listener`]);
        handlers[name] = typeof value === 'function' ? value : null;
        handlers[`${name}:listener`] = null;
        if (!handlers[name]) return;
        const listener = (event) => {
          const result = handlers[name].call(this, event);
          if (result === false) event.preventDefault();
        };
        handlers[`${name}:listener`] = listener;
        this.addEventListener(name, listener);
      },
      configurable: true,
      enumerable: true,
    });
  }
};

// The global scope. -----------------------------------------------------------------

class WorkerGlobalScope extends EventTarget {
  constructor() {
    super();
    illegal();
  }

  importScripts(...urls) {
    for (const each of urls) host.importScript(new URL(String(each), base).href);
  }

  get origin() {
    return originOf(scriptURL);
  }
}
class DedicatedWorkerGlobalScope extends WorkerGlobalScope {
  constructor() {
    super();
    illegal();
  }

  get name() {
    return name;
  }

  postMessage(data, transfer) {
    port.postMessage(data, movable(transfer));
  }

  close() {
    host.close();
  }
}
defineHandlers(WorkerGlobalScope.prototype, ['error', 'languagechange', 'offline', 'online', 'rejectionhandled', 'unhandledrejection']);
defineHandlers(DedicatedWorkerGlobalScope.prototype, ['message', 'messageerror']);
Object.setPrototypeOf(global, DedicatedWorkerGlobalScope.prototype);

// Buffers in a transfer list are cloned rather than moved; ports move.
const movable = (transfer) => {
  if (transfer === undefined || transfer === null) return transfer;
  const list = Array.isArray(transfer) ? transfer : transfer.transfer;
  if (list === undefined || list === null) return [];
  return [...list].filter((item) => !(item instanceof ArrayBuffer));
};

// The URL relative URLs resolve against: the script's, or for a blob
// script the URL its `blob:` URL names after the scheme, in the page's
// origin.
const base = (() => {
  if (url.startsWith('blob:')) {
    try {
      return new URL(url.slice(5)).href;
    } catch {
      return url;
    }
  }
  return url;
})();
const scriptURL = (() => {
  try {
    return new URL(url);
  } catch {
    return null;
  }
})();
class WorkerLocation {
  constructor() {
    illegal();
  }

  toString() {
    return url;
  }
}
for (const key of ['origin', 'protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']) {
  Object.defineProperty(WorkerLocation.prototype, key, {
    get() { return scriptURL ? scriptURL[key] : ''; },
    configurable: true,
    enumerable: true,
  });
}
Object.defineProperty(WorkerLocation.prototype, 'href', { get() { return url; }, configurable: true, enumerable: true });
const location = Object.create(WorkerLocation.prototype);
// A URL's origin, with the scheme and host of a custom scheme's URL, as
// Chromium gives those an application registers as standard.
const originOf = (parsed) => {
  if (!parsed) return 'null';
  if (parsed.origin !== 'null') return parsed.origin;
  return parsed.host ? `${parsed.protocol}//${parsed.host}` : 'null';
};

class WorkerNavigator {
  constructor() {
    illegal();
  }
}
for (const [key, read] of Object.entries({
  userAgent: () => host.userAgent(),
  appVersion: () => host.userAgent().replace(/^Mozilla\//, ''),
  language: () => host.languages()[0] || host.locale(),
  languages: () => Object.freeze([...host.languages()]),
  hardwareConcurrency: () => host.processors(),
  onLine: () => true,
  platform: () => 'Win32',
})) {
  Object.defineProperty(WorkerNavigator.prototype, key, { get: read, configurable: true, enumerable: true });
}
const navigator = Object.create(WorkerNavigator.prototype);

for (const [key, value] of Object.entries({
  self: global,
  location,
  navigator,
  Event,
  EventTarget,
  CustomEvent,
  MessageEvent,
  ErrorEvent,
  PromiseRejectionEvent,
  WorkerGlobalScope,
  DedicatedWorkerGlobalScope,
  WorkerLocation,
  WorkerNavigator,
})) {
  hidden(global, key, value);
}

// Messages from the page, and errors to it. ------------------------------------------

// Messages wait on the port until the worker's script ran, as they wait for
// a worker's script to load.
const listen = () =>
  port.bind({
    _deliver(data, ports) {
      fire(global, MessageEvent, 'message', { data, ports: Object.freeze([...ports]) });
    },
    _peerClosed() {},
  });

// Reports an exception no script caught: the worker's `error` event, then
// the page's, which reports it on the console unless a handler cancels it.
const reportError = (error) => {
  const message = error instanceof Error ? `Uncaught ${error.name}: ${error.message}` : `Uncaught ${String(error)}`;
  if (fire(global, ErrorEvent, 'error', { cancelable: true, message, error, filename: url })) {
    if (errors) errors.postMessage({ message, filename: url, lineno: 0, colno: 0 });
    else host.report(error);
  }
};
hidden(global, 'reportError', reportError);
host.onerror = (error, promise) => {
  if (promise) {
    if (fire(global, PromiseRejectionEvent, 'unhandledrejection', { cancelable: true, reason: error, promise })) host.report(error);
    return true;
  }
  reportError(error);
  return true;
};

// Scripts made at run time. ---------------------------------------------------------

// What a script uses that `evaluate` does not run, and where.
const OFFSET = Symbol('nativeboat.offset');
const unsupported = (what, at) => {
  const error = new SyntaxError(`a worker runs the scripts a program makes while it runs only when they declare, assign, call and import; this one uses ${what} at offset ${at}`);
  hidden(error, OFFSET, at);
  return error;
};

const PUNCTUATORS = ['===', '!==', '...', '?.', '??', '==', '!=', '<=', '>=', '&&', '||', '=>', '{', '}', '(', ')', '[', ']', ';', ',', '.', ':', '?', '=', '!', '+', '-', '<', '>'];
const isDigit = (code) => code >= 48 && code <= 57;
const isNameStart = (code) => (code >= 65 && code <= 90) || (code >= 97 && code <= 122) || code === 36 || code === 95;
const isNamePart = (code) => isNameStart(code) || isDigit(code);
const isLineTerminator = (code) => code === 10 || code === 13 || code === 0x2028 || code === 0x2029;
const isSpace = (code) => code === 32 || code === 9 || code === 11 || code === 12 || code === 0xa0 || code === 0xfeff;
const hexValue = (text, at, length) => {
  const digits = text.slice(at, at + length);
  if (digits.length !== length || !/^[0-9a-fA-F]+$/.test(digits)) throw unsupported('an invalid escape', at);
  return parseInt(digits, 16);
};

// The text of a string or template literal from its opening quote at
// `start`, and the offset after it.
const readQuoted = (source, start) => {
  const quote = source[start];
  let at = start + 1;
  let text = '';
  for (;;) {
    if (at >= source.length) throw unsupported('an unterminated string', start);
    const char = source[at];
    if (char === quote) return [text, at + 1];
    if (quote === '`' && char === '$' && source[at + 1] === '{') throw unsupported('a template substitution', at);
    if (quote !== '`' && isLineTerminator(char.charCodeAt(0))) throw unsupported('an unterminated string', start);
    if (char !== '\\') {
      text += char;
      at += 1;
      continue;
    }
    const escape = source[at + 1];
    at += 2;
    switch (escape) {
      case 'n': text += '\n'; break;
      case 'r': text += '\r'; break;
      case 't': text += '\t'; break;
      case 'b': text += '\b'; break;
      case 'f': text += '\f'; break;
      case 'v': text += '\v'; break;
      case '0':
        if (isDigit(source.charCodeAt(at))) throw unsupported('an octal escape', at - 2);
        text += '\0';
        break;
      case 'x':
        text += String.fromCharCode(hexValue(source, at, 2));
        at += 2;
        break;
      case 'u':
        if (source[at] === '{') {
          const end = source.indexOf('}', at);
          if (end < 0) throw unsupported('an invalid escape', at);
          text += String.fromCodePoint(hexValue(source, at + 1, end - at - 1));
          at = end + 1;
        } else {
          text += String.fromCharCode(hexValue(source, at, 4));
          at += 4;
        }
        break;
      case '\r':
        if (source[at] === '\n') at += 1;
        break;
      case '\n':
      case '\u2028':
      case '\u2029':
        break;
      case undefined:
        throw unsupported('an unterminated string', start);
      default:
        text += escape;
    }
  }
};

const tokenize = (source) => {
  const tokens = [];
  let at = 0;
  let newline = false;
  const push = (type, value, start) => {
    tokens.push({ type, value, at: start, newline });
    newline = false;
  };
  while (at < source.length) {
    const code = source.charCodeAt(at);
    if (isSpace(code)) {
      at += 1;
      continue;
    }
    if (isLineTerminator(code)) {
      newline = true;
      at += 1;
      continue;
    }
    if (code === 47 && source[at + 1] === '*') {
      const end = source.indexOf('*/', at + 2);
      if (end < 0) throw unsupported('an unterminated comment', at);
      if (/[\n\r\u2028\u2029]/.test(source.slice(at, end))) newline = true;
      at = end + 2;
      continue;
    }
    if (code === 47 && source[at + 1] === '/') {
      while (at < source.length && !isLineTerminator(source.charCodeAt(at))) at += 1;
      continue;
    }
    if (code === 34 || code === 39 || code === 96) {
      const [text, end] = readQuoted(source, at);
      push('string', text, at);
      at = end;
      continue;
    }
    if (isDigit(code) || (code === 46 && isDigit(source.charCodeAt(at + 1)))) {
      const match = /^(?:0[xX][0-9a-fA-F]+|0[bB][01]+|0[oO][0-7]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)/.exec(source.slice(at, at + 64));
      if (!match || isNamePart(source.charCodeAt(at + match[0].length))) throw unsupported('a number', at);
      push('number', Number(match[0]), at);
      at += match[0].length;
      continue;
    }
    if (isNameStart(code)) {
      let end = at + 1;
      while (end < source.length && isNamePart(source.charCodeAt(end))) end += 1;
      push('name', source.slice(at, end), at);
      at = end;
      continue;
    }
    const punctuator = PUNCTUATORS.find((candidate) => source.startsWith(candidate, at));
    if (punctuator === undefined) throw unsupported(`'${source[at]}'`, at);
    // `a?.5:b` is a conditional, not a chain.
    const value = punctuator === '?.' && isDigit(source.charCodeAt(at + 2)) ? '?' : punctuator;
    push('punctuator', value, at);
    at += value.length;
  }
  push('end', '', source.length);
  return tokens;
};

// Words that start what `evaluate` does not run.
const UNSUPPORTED_WORDS = new Set([
  'function', 'class', 'async', 'for', 'while', 'do', 'switch', 'return', 'export', 'with', 'yield',
  'break', 'continue', 'delete', 'in', 'instanceof', 'debugger', 'super', 'arguments', 'eval',
]);

// Parses a script into statements over a small tree of nodes.
const parse = (source) => {
  const tokens = tokenize(source);
  let index = 0;
  const peek = (offset = 0) => tokens[Math.min(index + offset, tokens.length - 1)];
  const next = () => tokens[index++];
  const is = (value, token = peek()) => (token.type === 'punctuator' || token.type === 'name') && token.value === value;
  const describe = (token) => (token.type === 'end' ? 'the end of the script' : `'${token.value}'`);
  const fail = (token = peek()) => unsupported(describe(token), token.at);
  const expect = (value) => {
    if (!is(value)) throw fail();
    return next();
  };
  const semicolon = () => {
    if (is(';')) {
      next();
      return;
    }
    if (is('}') || peek().type === 'end' || peek().newline) return;
    throw fail();
  };
  const unsupportedWord = (token) => token.type === 'name' && UNSUPPORTED_WORDS.has(token.value);

  const block = () => {
    expect('{');
    const body = [];
    while (!is('}')) {
      if (peek().type === 'end') throw fail();
      const statement = parseStatement();
      if (statement) body.push(statement);
    }
    next();
    return { kind: 'block', body };
  };

  const parseStatement = () => {
    const token = peek();
    if (is(';')) {
      next();
      return null;
    }
    if (is('{')) return block();
    if (is('const') || is('let') || is('var')) {
      const kind = next().value;
      const declarations = [];
      do {
        const name = next();
        if (name.type !== 'name' || unsupportedWord(name)) throw fail(name);
        let init = null;
        if (is('=')) {
          next();
          init = parseAssign();
        } else if (kind === 'const') {
          throw fail();
        }
        declarations.push({ name: name.value, init });
      } while (is(',') && next());
      semicolon();
      return { kind: 'declare', constant: kind === 'const', hoisted: kind === 'var', declarations };
    }
    if (is('try')) {
      next();
      const body = block();
      let parameter = null;
      let handler = null;
      let finalizer = null;
      if (is('catch')) {
        next();
        if (is('(')) {
          next();
          const name = next();
          if (name.type !== 'name' || unsupportedWord(name)) throw fail(name);
          parameter = name.value;
          expect(')');
        }
        handler = block();
      }
      if (is('finally')) {
        next();
        finalizer = block();
      }
      if (!handler && !finalizer) throw fail();
      return { kind: 'try', body, parameter, handler, finalizer };
    }
    if (is('throw')) {
      next();
      if (peek().newline) throw fail();
      const argument = parseExpression();
      semicolon();
      return { kind: 'throw', argument };
    }
    if (is('if')) {
      next();
      expect('(');
      const test = parseExpression();
      expect(')');
      const consequent = parseStatement() || { kind: 'block', body: [] };
      let alternate = null;
      if (is('else')) {
        next();
        alternate = parseStatement() || { kind: 'block', body: [] };
      }
      return { kind: 'if', test, consequent, alternate };
    }
    if (unsupportedWord(token) || (is('import') && !is('(', peek(1)))) throw fail(token);
    const expression = parseExpression();
    semicolon();
    return { kind: 'expression', expression };
  };

  const parseExpression = () => {
    const expressions = [parseAssign()];
    while (is(',')) {
      next();
      expressions.push(parseAssign());
    }
    return expressions.length === 1 ? expressions[0] : { kind: 'sequence', expressions };
  };

  // An arrow function whose body is an expression; `() =>` and `(a, b) =>`
  // are recognized before their parameters parse as an expression.
  const arrowParameters = () => {
    if (peek().type === 'name' && !unsupportedWord(peek()) && is('=>', peek(1))) {
      const parameter = next().value;
      return [parameter];
    }
    if (!is('(')) return null;
    let offset = 1;
    const parameters = [];
    while (!is(')', peek(offset))) {
      const token = peek(offset);
      if (token.type !== 'name' || unsupportedWord(token)) return null;
      parameters.push(token.value);
      offset += 1;
      if (is(',', peek(offset))) offset += 1;
      else if (!is(')', peek(offset))) return null;
    }
    if (!is('=>', peek(offset + 1))) return null;
    index += offset + 1;
    return parameters;
  };
  const parseAssign = () => {
    const parameters = arrowParameters();
    if (parameters) {
      expect('=>');
      if (is('{')) throw fail();
      return { kind: 'arrow', parameters, body: parseAssign() };
    }
    const start = peek();
    const target = parseConditional();
    if (!is('=')) return target;
    if (!(target.kind === 'name' || (target.kind === 'member' && !target.optional)) || target.parenthesized) throw fail(start);
    next();
    return { kind: 'assign', target, value: parseAssign() };
  };

  const parseConditional = () => {
    const test = parseLogical();
    if (!is('?')) return test;
    next();
    const consequent = parseAssign();
    expect(':');
    return { kind: 'conditional', test, consequent, alternate: parseAssign() };
  };

  const binary = (operators, operand, kind) => () => {
    let left = operand();
    while (operators.some((operator) => is(operator)) && peek().type === 'punctuator') {
      const operator = next().value;
      left = { kind, operator, left, right: operand() };
    }
    return left;
  };
  const parseUnary = () => {
    if (is('!') || is('-') || is('+') || is('typeof') || is('void')) {
      const operator = next().value;
      return { kind: 'unary', operator, argument: parseUnary() };
    }
    if (is('await')) {
      const { at } = next();
      return { kind: 'await', argument: parseUnary(), at };
    }
    return parsePostfix();
  };
  const parseAdditive = binary(['+', '-'], parseUnary, 'binary');
  const parseRelational = binary(['<', '>', '<=', '>='], parseAdditive, 'binary');
  const parseEquality = binary(['===', '!==', '==', '!='], parseRelational, 'binary');
  const parseAnd = binary(['&&'], parseEquality, 'logical');
  const parseLogical = binary(['||', '??'], parseAnd, 'logical');

  const parseArguments = () => {
    expect('(');
    const list = [];
    while (!is(')')) {
      if (is('...')) {
        next();
        list.push({ kind: 'spread', argument: parseAssign() });
      } else {
        list.push(parseAssign());
      }
      if (!is(')')) expect(',');
    }
    next();
    return list;
  };

  const propertyName = () => {
    const token = next();
    if (token.type !== 'name') throw fail(token);
    return token.value;
  };

  // A member, call or chain of them; with `calls` false, as `new` takes its
  // constructor, member accesses only.
  const parsePostfix = (calls = true) => {
    let expression = is('new') ? parseNew() : parsePrimary();
    let optional = false;
    for (;;) {
      if (is('.')) {
        next();
        expression = { kind: 'member', object: expression, property: propertyName(), computed: false, optional: false };
      } else if (is('?.')) {
        if (!calls) throw fail();
        next();
        optional = true;
        if (is('(')) {
          expression = { kind: 'call', callee: expression, arguments: parseArguments(), optional: true };
        } else if (is('[')) {
          next();
          const property = parseExpression();
          expect(']');
          expression = { kind: 'member', object: expression, property, computed: true, optional: true };
        } else {
          expression = { kind: 'member', object: expression, property: propertyName(), computed: false, optional: true };
        }
      } else if (is('[')) {
        next();
        const property = parseExpression();
        expect(']');
        expression = { kind: 'member', object: expression, property, computed: true, optional: false };
      } else if (is('(') && calls) {
        expression = { kind: 'call', callee: expression, arguments: parseArguments(), optional: false };
      } else if (peek().type === 'string' && !peek().newline) {
        throw fail();
      } else {
        break;
      }
    }
    return optional ? { kind: 'chain', expression } : expression;
  };

  const parseNew = () => {
    expect('new');
    if (is('.')) throw fail();
    const callee = parsePostfix(false);
    const list = is('(') ? parseArguments() : [];
    return { kind: 'new', callee, arguments: list };
  };

  const parsePrimary = () => {
    const token = next();
    if (token.type === 'string' || token.type === 'number') return { kind: 'literal', value: token.value };
    if (token.type === 'name') {
      switch (token.value) {
        case 'true': return { kind: 'literal', value: true };
        case 'false': return { kind: 'literal', value: false };
        case 'null': return { kind: 'literal', value: null };
        case 'this': return { kind: 'this' };
        case 'import': {
          const list = parseArguments();
          if (list.length < 1 || list.length > 2 || list[0].kind === 'spread') throw fail(token);
          return { kind: 'import', source: list[0] };
        }
        default:
          if (unsupportedWord(token) || ['const', 'let', 'var', 'try', 'catch', 'finally', 'throw', 'if', 'else', 'new'].includes(token.value)) throw fail(token);
          return { kind: 'name', name: token.value };
      }
    }
    if (is('(', token)) {
      const expression = parseExpression();
      expect(')');
      if (is('=>')) throw fail();
      return { ...expression, parenthesized: true };
    }
    if (is('[', token)) {
      const elements = [];
      while (!is(']')) {
        if (is('...')) {
          next();
          elements.push({ kind: 'spread', argument: parseAssign() });
        } else {
          elements.push(parseAssign());
        }
        if (!is(']')) expect(',');
      }
      next();
      return { kind: 'array', elements };
    }
    if (is('{', token)) {
      const properties = [];
      while (!is('}')) {
        if (is('...')) {
          next();
          properties.push({ spread: parseAssign() });
        } else {
          const key = next();
          let name;
          if (key.type === 'string') name = { kind: 'literal', value: key.value };
          else if (key.type === 'number') name = { kind: 'literal', value: String(key.value) };
          else if (key.type === 'name') name = { kind: 'literal', value: key.value };
          else if (is('[', key)) {
            name = parseAssign();
            expect(']');
          } else throw fail(key);
          if (is(':')) {
            next();
            properties.push({ key: name, value: parseAssign() });
          } else if (key.type === 'name' && (is(',') || is('}')) && !unsupportedWord(key)) {
            properties.push({ key: name, value: { kind: 'name', name: key.value } });
          } else {
            throw fail();
          }
        }
        if (!is('}')) expect(',');
      }
      next();
      return { kind: 'object', properties };
    }
    throw fail(token);
  };

  const body = [];
  while (peek().type !== 'end') {
    const statement = parseStatement();
    if (statement) body.push(statement);
  }
  return body;
};

// A chain whose optional part met null or undefined.
const SHORT = Symbol('nativeboat.short');

// A scope of declared names, inside `parent`.
const scope = (parent) => ({ names: new Map(), parent });
const lookup = (scope, name) => {
  for (let current = scope; current; current = current.parent) {
    if (current.names.has(name)) return current.names.get(name);
  }
  return null;
};
const describeNode = (node) => {
  if (node.kind === 'name') return node.name;
  if (node.kind === 'member' && !node.computed) return `${describeNode(node.object)}.${node.property}`;
  return 'expression';
};

// Runs a parsed script. Expressions evaluate at once, so the arrow
// functions a script makes return their values to native callers; its
// statements may await, at their top: `await import(url);`,
// `const module = await import(url);` or `x = await promise;`.
const run = async (statements, functions = []) => {
  const top = scope(null);
  for (const [name, value] of functions) top.names.set(name, { value, constant: true });

  const read = (name, current) => {
    const binding = lookup(current, name);
    if (binding) return binding.value;
    if (name in global) return global[name];
    if (name === 'undefined') return undefined;
    throw new ReferenceError(`${name} is not defined`);
  };
  const write = (name, value, current) => {
    const binding = lookup(current, name);
    if (!binding) {
      global[name] = value;
      return;
    }
    if (binding.constant) throw new TypeError('Assignment to constant variable.');
    binding.value = value;
  };
  const key = (node, current) => (node.computed ? valueOf(node.property, current) : node.property);
  const list = (nodes, current) => {
    const values = [];
    for (const node of nodes) {
      if (node.kind === 'spread') values.push(...valueOf(node.argument, current));
      else values.push(valueOf(node, current));
    }
    return values;
  };

  // A node's value, or SHORT inside a chain that stopped.
  const evaluate = (node, current) => {
    switch (node.kind) {
      case 'literal':
        return node.value;
      case 'name':
        return read(node.name, current);
      case 'this':
        return undefined;
      case 'array':
        return list(node.elements, current);
      case 'object': {
        const object = {};
        for (const property of node.properties) {
          if (property.spread) Object.assign(object, valueOf(property.spread, current));
          else object[valueOf(property.key, current)] = valueOf(property.value, current);
        }
        return object;
      }
      case 'arrow': {
        const { parameters, body } = node;
        return (...args) => {
          const inner = scope(current);
          parameters.forEach((parameter, index) => inner.names.set(parameter, { value: args[index], constant: false }));
          return valueOf(body, inner);
        };
      }
      case 'member': {
        const object = evaluate(node.object, current);
        if (object === SHORT) return SHORT;
        if (node.optional && (object === null || object === undefined)) return SHORT;
        const property = key(node, current);
        if (object === null || object === undefined) {
          throw new TypeError(`Cannot read properties of ${object} (reading '${String(property)}')`);
        }
        return object[property];
      }
      case 'call': {
        let receiver;
        let callee;
        if (node.callee.kind === 'member') {
          receiver = evaluate(node.callee.object, current);
          if (receiver === SHORT) return SHORT;
          if (node.callee.optional && (receiver === null || receiver === undefined)) return SHORT;
          const property = key(node.callee, current);
          if (receiver === null || receiver === undefined) {
            throw new TypeError(`Cannot read properties of ${receiver} (reading '${String(property)}')`);
          }
          callee = receiver[property];
        } else {
          callee = evaluate(node.callee, current);
          if (callee === SHORT) return SHORT;
        }
        if (node.optional && (callee === null || callee === undefined)) return SHORT;
        if (typeof callee !== 'function') throw new TypeError(`${describeNode(node.callee)} is not a function`);
        return Reflect.apply(callee, receiver, list(node.arguments, current));
      }
      case 'chain': {
        const value = evaluate(node.expression, current);
        return value === SHORT ? undefined : value;
      }
      case 'new': {
        const constructor = valueOf(node.callee, current);
        if (typeof constructor !== 'function') throw new TypeError(`${describeNode(node.callee)} is not a constructor`);
        return Reflect.construct(constructor, list(node.arguments, current));
      }
      case 'import':
        return host.import(String(valueOf(node.source, current)));
      case 'await':
        throw unsupported("'await' inside an expression", node.at);
      case 'unary': {
        if (node.operator === 'typeof' && node.argument.kind === 'name' && !lookup(current, node.argument.name) && !(node.argument.name in global)) {
          return 'undefined';
        }
        const value = valueOf(node.argument, current);
        switch (node.operator) {
          case '!': return !value;
          case '-': return -value;
          case '+': return +value;
          case 'typeof': return typeof value;
          default: return undefined;
        }
      }
      case 'binary': {
        const left = valueOf(node.left, current);
        const right = valueOf(node.right, current);
        switch (node.operator) {
          case '===': return left === right;
          case '!==': return left !== right;
          // eslint-disable-next-line eqeqeq
          case '==': return left == right;
          // eslint-disable-next-line eqeqeq
          case '!=': return left != right;
          case '<': return left < right;
          case '>': return left > right;
          case '<=': return left <= right;
          case '>=': return left >= right;
          case '+': return left + right;
          default: return left - right;
        }
      }
      case 'logical': {
        const left = valueOf(node.left, current);
        if (node.operator === '&&') return left ? valueOf(node.right, current) : left;
        if (node.operator === '||') return left ? left : valueOf(node.right, current);
        return left === null || left === undefined ? valueOf(node.right, current) : left;
      }
      case 'conditional':
        return valueOf(node.test, current) ? valueOf(node.consequent, current) : valueOf(node.alternate, current);
      case 'sequence': {
        let value;
        for (const expression of node.expressions) value = valueOf(expression, current);
        return value;
      }
      case 'assign': {
        if (node.target.kind === 'name') {
          const value = valueOf(node.value, current);
          write(node.target.name, value, current);
          return value;
        }
        const object = valueOf(node.target.object, current);
        const property = key(node.target, current);
        const value = valueOf(node.value, current);
        if (object === null || object === undefined) {
          throw new TypeError(`Cannot set properties of ${object} (setting '${String(property)}')`);
        }
        object[property] = value;
        return value;
      }
      default:
        throw unsupported(node.kind, 0);
    }
  };
  const valueOf = (node, current) => {
    const value = evaluate(node, current);
    return value === SHORT ? undefined : value;
  };
  // The value of an expression at the top of a statement, which may await.
  const settle = async (node, current) => {
    if (node.kind === 'await') return await settle(node.argument, current);
    if (node.kind === 'assign' && node.value.kind === 'await') {
      const value = await settle(node.value, current);
      return valueOf({ ...node, value: { kind: 'literal', value } }, current);
    }
    return valueOf(node, current);
  };

  const execute = async (statement, current) => {
    switch (statement.kind) {
      case 'expression':
        await settle(statement.expression, current);
        return;
      case 'declare':
        for (const { name, init } of statement.declarations) {
          const value = init ? await settle(init, current) : undefined;
          const target = statement.hoisted ? top : current;
          if (!statement.hoisted && target.names.has(name)) throw new SyntaxError(`Identifier '${name}' has already been declared`);
          target.names.set(name, { value, constant: statement.constant });
        }
        return;
      case 'block': {
        const inner = scope(current);
        for (const each of statement.body) await execute(each, inner);
        return;
      }
      case 'if':
        if (valueOf(statement.test, current)) await execute(statement.consequent, current);
        else if (statement.alternate) await execute(statement.alternate, current);
        return;
      case 'throw':
        throw await settle(statement.argument, current);
      case 'try':
        try {
          await execute(statement.body, current);
        } catch (error) {
          if (!statement.handler) throw error;
          const inner = scope(current);
          if (statement.parameter !== null) inner.names.set(statement.parameter, { value: error, constant: false });
          await execute(statement.handler, inner);
        } finally {
          if (statement.finalizer) await execute(statement.finalizer, current);
        }
        return;
      default:
        throw unsupported(statement.kind, 0);
    }
  };

  for (const statement of statements) await execute(statement, top);
};

// The text of the functions whose text the program read, as
// `(${f})()` puts it in a script, runs as the compiled functions they are:
// each such text becomes a name bound to its function. Longer texts go
// first, so a function's text is replaced before those inside it.
const compiledFunctions = (source) => {
  const functions = [];
  let text = source;
  const known = typeof host.sourceFunctions === 'function' ? host.sourceFunctions() : [];
  known.sort((a, b) => b[0].length - a[0].length);
  for (const [functionText, value] of known) {
    if (!functionText || !text.includes(functionText)) continue;
    const name = `__nativeboat_function_${functions.length}`;
    text = text.split(functionText).join(name);
    functions.push([name, value]);
  }
  return [text, functions];
};

// Runs a script; what it cannot run is reported with the script's text
// around it.
const evaluate = (script) => {
  const [source, functions] = compiledFunctions(script);
  let statements;
  try {
    statements = parse(source);
  } catch (error) {
    const at = error?.[OFFSET];
    if (typeof at === 'number') {
      const near = source.slice(Math.max(0, at - 40), at + 40);
      error.message += `: ${JSON.stringify(near)}`;
    }
    throw error;
  }
  return run(statements, functions);
};

// Service workers. -----------------------------------------------------------------

// A service worker's realm runs this scope instead of a dedicated worker's:
// pages and frames in its scope connect to it with ports, as `host`'s
// `connectServiceWorker` connects theirs, and it answers their messages and
// the requests of the pages it controls with its events.
const WAITS = Symbol('nativeboat.waits');
const RESPONSE = Symbol('nativeboat.response');
const CLIENT = Symbol('nativeboat.client');
const CACHE = Symbol('nativeboat.cache');
const invalidState = (message) => {
  const error = new Error(message);
  error.name = 'InvalidStateError';
  return error;
};
class ExtendableEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    hidden(this, WAITS, []);
  }

  waitUntil(promise) {
    this[WAITS].push(Promise.resolve(promise));
  }
}
// An extendable event class whose instances carry their init's fields.
const extendableClass = (name, fields) => {
  const constructor = class extends ExtendableEvent {
    constructor(type, init = {}) {
      super(type, init);
      const options = init === null || init === undefined ? {} : init;
      const values = {};
      for (const [key, fallback] of Object.entries(fields)) values[key] = options[key] === undefined ? fallback : options[key];
      hidden(this, FIELDS, values);
    }
  };
  for (const key of Object.keys(fields)) {
    Object.defineProperty(constructor.prototype, key, {
      get() { return this[FIELDS][key]; },
      configurable: true,
      enumerable: true,
    });
  }
  Object.defineProperty(constructor, 'name', { value: name });
  return constructor;
};
const ExtendableMessageEvent = extendableClass('ExtendableMessageEvent', { data: null, origin: '', lastEventId: '', source: null, ports: [] });
const FetchEventBase = extendableClass('FetchEvent', {
  request: null,
  clientId: '',
  resultingClientId: '',
  replacesClientId: '',
  handled: Promise.resolve(),
  preloadResponse: Promise.resolve(undefined),
});
class FetchEvent extends FetchEventBase {
  respondWith(response) {
    if (this[RESPONSE]) throw invalidState("Failed to execute 'respondWith' on 'FetchEvent': The event has already been responded to.");
    hidden(this, RESPONSE, { value: Promise.resolve(response) });
    this.stopImmediatePropagation();
  }
}
Object.defineProperty(FetchEvent, 'name', { value: 'FetchEvent' });

// The pages connected to this worker, by client id.
const clientsById = new Map();
class Client {
  constructor() {
    illegal();
  }

  get id() {
    return this[CLIENT].id;
  }

  get url() {
    return this[CLIENT].url;
  }

  get type() {
    return this[CLIENT].type;
  }

  get frameType() {
    return 'nested';
  }

  postMessage(data, transfer) {
    this[CLIENT].port.postMessage({ kind: 'message', data }, movable(transfer));
  }
}
class WindowClient extends Client {
  get focused() {
    return false;
  }

  get visibilityState() {
    return 'visible';
  }

  focus() {
    return Promise.resolve(this);
  }

  navigate() {
    return Promise.resolve(null);
  }
}
class Clients {
  constructor() {
    illegal();
  }

  get(id) {
    return Promise.resolve(clientsById.get(String(id)));
  }

  matchAll(options = {}) {
    const type = (options && options.type) || 'window';
    const uncontrolled = !!(options && options.includeUncontrolled);
    return Promise.resolve([...clientsById.values()].filter((client) =>
      (type === 'all' || client[CLIENT].type === type) && (uncontrolled || client[CLIENT].controlled)));
  }

  openWindow() {
    return Promise.resolve(null);
  }

  claim() {
    if (serviceState !== 'activating' && serviceState !== 'activated') {
      return Promise.reject(invalidState("Failed to execute 'claim' on 'Clients': Only the active worker can claim clients."));
    }
    for (const client of clientsById.values()) {
      if (client[CLIENT].controlled) continue;
      client[CLIENT].controlled = true;
      client[CLIENT].port.postMessage({ kind: 'claim' });
    }
    return Promise.resolve();
  }
}
const clients = Object.create(Clients.prototype);

// The worker's caches, in memory for as long as it runs.
const cacheKey = (request) => (typeof request === 'string' ? new URL(request, url).href : request.url);
const cachedResponse = (entry) => new Response([101, 103, 204, 205, 304].includes(entry.status) ? null : entry.body.slice(0), {
  status: entry.status,
  statusText: entry.statusText,
  headers: entry.headers,
});
class Cache {
  constructor() {
    illegal();
  }

  async match(request) {
    const entry = this[CACHE].get(cacheKey(request));
    return entry ? cachedResponse(entry) : undefined;
  }

  async matchAll(request) {
    if (request === undefined) return [...this[CACHE].values()].map(cachedResponse);
    const entry = this[CACHE].get(cacheKey(request));
    return entry ? [cachedResponse(entry)] : [];
  }

  async put(request, response) {
    const body = await response.arrayBuffer();
    this[CACHE].set(cacheKey(request), { status: response.status, statusText: response.statusText, headers: [...response.headers], body });
  }

  async add(request) {
    await this.put(request, await fetch(request));
  }

  async addAll(requests) {
    for (const request of requests) await this.add(request);
  }

  async delete(request) {
    return this[CACHE].delete(cacheKey(request));
  }

  async keys() {
    return [...this[CACHE].keys()].map((key) => new Request(key));
  }
}
const cacheStores = new Map();
class CacheStorage {
  constructor() {
    illegal();
  }

  async open(name) {
    const key = String(name);
    let cache = cacheStores.get(key);
    if (!cache) {
      cache = Object.create(Cache.prototype);
      hidden(cache, CACHE, new Map());
      cacheStores.set(key, cache);
    }
    return cache;
  }

  async has(name) {
    return cacheStores.has(String(name));
  }

  async delete(name) {
    return cacheStores.delete(String(name));
  }

  async keys() {
    return [...cacheStores.keys()];
  }

  async match(request) {
    for (const cache of cacheStores.values()) {
      const found = await cache.match(request);
      if (found) return found;
    }
    return undefined;
  }
}
const caches = Object.create(CacheStorage.prototype);

class ServiceWorkerGlobalScope extends WorkerGlobalScope {
  constructor() {
    super();
    illegal();
  }

  get clients() {
    return clients;
  }

  get registration() {
    return serviceRegistration;
  }

  get serviceWorker() {
    return serviceRegistration.active ?? serviceRegistration.installing;
  }

  skipWaiting() {
    return Promise.resolve();
  }
}
defineHandlers(ServiceWorkerGlobalScope.prototype, ['install', 'activate', 'fetch', 'message', 'messageerror']);

// This worker as its own scope sees it, and its registration.
let serviceState = 'parsed';
const serviceWorkerSelf = {
  get scriptURL() { return url; },
  get state() { return serviceState; },
  postMessage() {},
  addEventListener() {},
  removeEventListener() {},
};
const serviceRegistration = {
  get scope() { return service; },
  get installing() { return serviceState === 'parsed' || serviceState === 'installing' ? serviceWorkerSelf : null; },
  get waiting() { return serviceState === 'installed' ? serviceWorkerSelf : null; },
  get active() { return serviceState === 'activating' || serviceState === 'activated' ? serviceWorkerSelf : null; },
  update: () => Promise.resolve(serviceRegistration),
  unregister: () => Promise.resolve(false),
  addEventListener() {},
  removeEventListener() {},
};

const serviceStateIs = (next) => {
  serviceState = next;
  host.serviceWorkerState(next);
  for (const client of clientsById.values()) client[CLIENT].port.postMessage({ kind: 'state', state: next });
};
// Fires an extendable event and waits for what its listeners extended it
// with; whether all of that fulfilled.
const extended = async (event) => {
  event[EVENT].trusted = true;
  global.dispatchEvent(event);
  try {
    await Promise.all(event[WAITS]);
    return true;
  } catch (error) {
    reportError(error);
    return false;
  }
};
// Installs and activates the worker once its script ran: there is never an
// older worker for it to wait for.
const serviceLifecycle = async () => {
  serviceStateIs('installing');
  if (!(await extended(new ExtendableEvent('install')))) {
    serviceStateIs('redundant');
    return;
  }
  serviceStateIs('installed');
  serviceStateIs('activating');
  await extended(new ExtendableEvent('activate'));
  serviceStateIs('activated');
};

// A request of a page this worker controls: the worker's `fetch` event,
// answered with the response a listener gave, or with none.
const serviceFetch = (client, message) => {
  const reply = (answer, transfer) => client[CLIENT].port.postMessage({ kind: 'response', id: message.id, ...answer }, transfer);
  let event;
  try {
    const request = new Request(message.url, { method: message.method, headers: message.headers });
    event = new FetchEvent('fetch', { request, clientId: client[CLIENT].id, cancelable: true });
  } catch (error) {
    reply({ fallback: true });
    return;
  }
  event[EVENT].trusted = true;
  global.dispatchEvent(event);
  const responded = event[RESPONSE];
  if (!responded) {
    reply({ fallback: true });
    return;
  }
  responded.value
    .then(async (response) => {
      if (!(response instanceof Response)) throw new TypeError('the service worker responded with a value that is not a Response');
      const body = await response.arrayBuffer();
      reply({ status: response.status, statusText: response.statusText, headers: [...response.headers], body });
    })
    .catch((error) => reply({ error: String(error && error.message !== undefined ? error.message : error) }));
};

// A page that connects: its messages and requests reach this worker.
const serviceClient = (handle, id, clientURL, type) => {
  const client = Object.create((type === 'window' ? WindowClient : Client).prototype);
  hidden(client, CLIENT, { port: handle, id, url: clientURL, type, controlled: false });
  clientsById.set(id, client);
  handle.bind({
    _deliver(data, ports) {
      switch (data && data.kind) {
        case 'hello':
          client[CLIENT].controlled = !!data.controlled;
          handle.postMessage({ kind: 'state', state: serviceState });
          return;
        case 'message': {
          let origin = 'null';
          try {
            origin = originOf(new URL(clientURL));
          } catch {}
          const event = new ExtendableMessageEvent('message', { data: data.data, source: client, ports: Object.freeze([...ports]), origin });
          extended(event);
          return;
        }
        case 'fetch':
          serviceFetch(client, data);
          return;
      }
    },
    _peerClosed() {
      clientsById.delete(id);
    },
  });
};

if (service !== null) {
  Object.setPrototypeOf(global, ServiceWorkerGlobalScope.prototype);
  for (const [key, value] of Object.entries({
    caches,
    clients,
    registration: serviceRegistration,
    ServiceWorkerGlobalScope,
    ExtendableEvent,
    ExtendableMessageEvent,
    FetchEvent,
    Client,
    Clients,
    WindowClient,
    Cache,
    CacheStorage,
  })) {
    hidden(global, key, value);
  }
  hidden(global, '__nativeboat_service_client', serviceClient);
}

// The worker's script. ------------------------------------------------------------

(async () => {
  try {
    if (script === null) await host.import(url);
    else await evaluate(script);
  } finally {
    if (service === null) listen();
  }
  if (service !== null) await serviceLifecycle();
})().catch((error) => {
  if (service !== null) serviceStateIs('redundant');
  reportError(error);
});
