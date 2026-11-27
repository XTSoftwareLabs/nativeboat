'use strict';

// nativeboat: the Fetch API of Node's globals, written for the native
// runtime: fetch, Headers, Request, Response and FormData, with bodies as
// web streams. Requests go through the http and https modules; gzip and
// deflate responses are decoded, and redirects are followed as the Fetch
// standard says. Node's undici is not bundled. In renderer pages, which
// have a location, URLs resolve against the page and file: URLs read the
// application's files, as Chromium's fetch does.

const { Buffer } = require('buffer');
const { Blob } = require('internal/blob');
const { File } = require('internal/file');
const { ReadableStream } = require('internal/webstreams/readablestream');
const { URL, URLSearchParams } = require('internal/url');
const { DOMException } = require('internal/per_context/domexception');

const kInspect = Symbol.for('nodejs.util.inspect.custom');
const tokenRegExp = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
const kHeaders = Symbol('headers list');
const kGuard = Symbol('guard');
const kBody = Symbol('body');
const kState = Symbol('state');

function invalid(message) {
  return new TypeError(message);
}

// The page's URL in a renderer; undefined in Node contexts.
function pageURL() {
  const location = globalThis.location;
  return location && typeof location.href === 'string' ? location.href : undefined;
}

const fileTypes = {
  css: 'text/css', gif: 'image/gif', htm: 'text/html', html: 'text/html', ico: 'image/x-icon',
  jpeg: 'image/jpeg', jpg: 'image/jpeg', js: 'text/javascript', json: 'application/json',
  mjs: 'text/javascript', mp3: 'audio/mpeg', mp4: 'video/mp4', png: 'image/png',
  svg: 'image/svg+xml', txt: 'text/plain', wasm: 'application/wasm', webp: 'image/webp',
  woff: 'font/woff', woff2: 'font/woff2', xml: 'text/xml',
};

// --- Headers ---

const normalizeValue = (value) => String(value).replace(/^[\t\n\r ]+|[\t\n\r ]+$/g, '');

function validateName(name, method) {
  if (!tokenRegExp.test(name)) throw invalid(`Headers.${method}: "${name}" is an invalid header name.`);
}

function validateValue(name, value, method) {
  if (/[\0\r\n]/.test(value)) {
    throw invalid(`Headers.${method}: "${value}" is an invalid header value.`);
  }
}

class Headers {
  constructor(init = undefined) {
    // Lower-case name -> [name as given, values].
    this[kHeaders] = new Map();
    this[kGuard] = 'none';
    if (init === undefined || init === null) return;
    if (typeof init !== 'object' && typeof init !== 'function') {
      throw invalid("Headers constructor: Expected init to be a sequence or record.");
    }
    if (init instanceof Headers) {
      for (const [name, entry] of init[kHeaders]) this[kHeaders].set(name, [entry[0], [...entry[1]]]);
    } else if (typeof init[Symbol.iterator] === 'function') {
      for (const pair of init) {
        const entry = [...pair];
        if (entry.length !== 2) throw invalid('Headers constructor: expected name/value pair to be length 2, found ' + entry.length + '.');
        this.append(entry[0], entry[1]);
      }
    } else {
      for (const name of Object.keys(init)) this.append(name, init[name]);
    }
  }

  _check() {
    if (this[kGuard] === 'immutable') throw invalid('immutable');
  }

  append(name, value) {
    name = String(name);
    value = normalizeValue(value);
    validateName(name, 'append');
    validateValue(name, value, 'append');
    this._check();
    const key = name.toLowerCase();
    const entry = this[kHeaders].get(key);
    if (entry) entry[1].push(value);
    else this[kHeaders].set(key, [name, [value]]);
  }

  delete(name) {
    name = String(name);
    validateName(name, 'delete');
    this._check();
    this[kHeaders].delete(name.toLowerCase());
  }

  get(name) {
    name = String(name);
    validateName(name, 'get');
    const entry = this[kHeaders].get(name.toLowerCase());
    return entry ? entry[1].join(', ') : null;
  }

  getSetCookie() {
    const entry = this[kHeaders].get('set-cookie');
    return entry ? [...entry[1]] : [];
  }

  has(name) {
    name = String(name);
    validateName(name, 'has');
    return this[kHeaders].has(name.toLowerCase());
  }

  set(name, value) {
    name = String(name);
    value = normalizeValue(value);
    validateName(name, 'set');
    validateValue(name, value, 'set');
    this._check();
    this[kHeaders].set(name.toLowerCase(), [name, [value]]);
  }

  // Sorted and combined, as the standard iterates headers; set-cookie
  // values stay apart.
  _entries() {
    const entries = [];
    for (const name of [...this[kHeaders].keys()].sort()) {
      const values = this[kHeaders].get(name)[1];
      if (name === 'set-cookie') {
        for (const value of values) entries.push([name, value]);
      } else {
        entries.push([name, values.join(', ')]);
      }
    }
    return entries;
  }

  forEach(callback, thisArg = undefined) {
    if (typeof callback !== 'function') throw invalid("Headers.forEach: Argument 1 could not be converted to: Function.");
    for (const [name, value] of this._entries()) callback.call(thisArg, value, name, this);
  }

  *keys() { for (const [name] of this._entries()) yield name; }
  *values() { for (const [, value] of this._entries()) yield value; }
  *entries() { yield* this._entries(); }
  [Symbol.iterator]() { return this.entries(); }

  get [Symbol.toStringTag]() { return 'Headers'; }

  [kInspect]() {
    return Object.fromEntries(this._entries());
  }
}

// The headers as a flat name/value array for http.request.
function headerArray(headers) {
  const list = [];
  for (const [, [name, values]] of headers[kHeaders]) {
    for (const value of values) list.push(name, value);
  }
  return list;
}

// --- Bodies ---

function bytesOf(view) {
  return new Uint8Array(view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength));
}

function randomBoundary() {
  let digits = '';
  for (let index = 0; index < 11; index++) digits += Math.floor(Math.random() * 10);
  return `----formdata-undici-0${digits}`;
}

const escapeName = (name) => name.replace(/\n/g, '%0A').replace(/\r/g, '%0D').replace(/"/g, '%22');
const normalizeLines = (text) => text.replace(/\r\n|\r|\n/g, '\r\n');

// A body from a value: { stream, source, length } and its content type.
function extractBody(object) {
  let source = null;
  let stream = null;
  let type = null;
  let length = null;
  if (typeof object === 'string') {
    source = Buffer.from(object, 'utf8');
    type = 'text/plain;charset=UTF-8';
  } else if (object instanceof URLSearchParams) {
    source = Buffer.from(object.toString(), 'utf8');
    type = 'application/x-www-form-urlencoded;charset=UTF-8';
  } else if (object instanceof ArrayBuffer) {
    source = new Uint8Array(object.slice(0));
  } else if (ArrayBuffer.isView(object)) {
    source = bytesOf(object);
  } else if (object instanceof Blob) {
    source = object;
    length = object.size;
    if (object.type) type = object.type;
  } else if (object instanceof FormData) {
    const boundary = randomBoundary();
    type = `multipart/form-data; boundary=${boundary}`;
    const parts = [];
    for (const [name, value] of object) {
      let head = `--${boundary}\r\nContent-Disposition: form-data; name="${escapeName(normalizeLines(name))}"`;
      if (typeof value === 'string') {
        parts.push(Buffer.from(`${head}\r\n\r\n${normalizeLines(value)}\r\n`));
      } else {
        head += `; filename="${escapeName(value.name)}"\r\nContent-Type: ${value.type || 'application/octet-stream'}\r\n\r\n`;
        parts.push(Buffer.from(head), value, Buffer.from('\r\n'));
      }
    }
    parts.push(Buffer.from(`--${boundary}--\r\n`));
    source = new Blob(parts);
    length = source.size;
  } else if (object instanceof ReadableStream) {
    if (object.locked) throw invalid('Response body object should not be disturbed or locked');
    stream = object;
  } else if (object != null && typeof object[Symbol.asyncIterator] === 'function') {
    stream = ReadableStream.from(object);
  } else {
    source = Buffer.from(String(object), 'utf8');
    type = 'text/plain;charset=UTF-8';
  }
  if (source && !(source instanceof Blob)) length = source.byteLength;
  return { body: { stream, source, length }, type };
}

function streamOf(body) {
  if (!body.stream) {
    const { source } = body;
    if (source instanceof Blob) {
      body.stream = source.stream();
    } else {
      body.stream = new ReadableStream({
        type: 'bytes',
        start(controller) {
          if (source.byteLength) controller.enqueue(new Uint8Array(source));
          controller.close();
        },
      });
    }
  }
  return body.stream;
}

async function readAll(stream) {
  const reader = stream.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (!(value instanceof Uint8Array)) {
      throw invalid('Received non-Uint8Array chunk');
    }
    chunks.push(value);
    total += value.byteLength;
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

function isUnusable(object) {
  const body = object[kState].body;
  return body !== null && (object[kState].used || (body.stream !== null && (body.stream.locked || isDisturbed(body.stream))));
}

function isDisturbed(stream) {
  return Boolean(stream[Symbol.for('nodejs.stream.disturbed')]);
}

async function consume(object) {
  if (isUnusable(object)) throw invalid('Body is unusable: Body has already been read');
  const body = object[kState].body;
  object[kState].used = true;
  if (body === null) return new Uint8Array(0);
  if (!body.stream && body.source instanceof Blob) return new Uint8Array(await body.source.arrayBuffer());
  if (!body.stream) return new Uint8Array(body.source);
  return readAll(body.stream);
}

function mimeType(object) {
  return object[kState].headers.get('content-type') || '';
}

function parseMultipart(bytes, contentType) {
  const match = /boundary=(?:"([^"]+)"|([^;]+))/i.exec(contentType);
  if (!match) throw invalid('Failed to parse body as FormData.');
  const boundary = Buffer.from(`--${match[1] || match[2]}`);
  const data = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const form = new FormData();
  let position = data.indexOf(boundary);
  if (position < 0) throw invalid('Failed to parse body as FormData.');
  for (;;) {
    position += boundary.length;
    if (data[position] === 45 && data[position + 1] === 45) break;
    position += 2;
    const headEnd = data.indexOf('\r\n\r\n', position);
    if (headEnd < 0) throw invalid('Failed to parse body as FormData.');
    const head = data.subarray(position, headEnd).toString('utf8');
    const next = data.indexOf(boundary, headEnd + 4);
    if (next < 0) throw invalid('Failed to parse body as FormData.');
    const content = data.subarray(headEnd + 4, next - 2);
    const disposition = /content-disposition:([^\r\n]*)/i.exec(head)?.[1] || '';
    const name = /\bname="([^"]*)"/i.exec(disposition)?.[1];
    if (name === undefined) throw invalid('Failed to parse body as FormData.');
    const filename = /\bfilename="([^"]*)"/i.exec(disposition)?.[1];
    if (filename !== undefined) {
      const type = /content-type:\s*([^\r\n]*)/i.exec(head)?.[1] || '';
      form.append(name, new File([content], filename, { type }));
    } else {
      form.append(name, content.toString('utf8'));
    }
    position = next;
  }
  return form;
}

const bodyMethods = {
  get body() {
    const body = this[kState].body;
    return body === null ? null : streamOf(body);
  },
  get bodyUsed() {
    const body = this[kState].body;
    return body !== null && (this[kState].used || (body.stream !== null && isDisturbed(body.stream)));
  },
  async arrayBuffer() {
    const bytes = await consume(this);
    return bytes.buffer.byteLength === bytes.byteLength ? bytes.buffer : bytes.slice().buffer;
  },
  async bytes() {
    return consume(this);
  },
  async blob() {
    const bytes = await consume(this);
    return new Blob([bytes], { type: mimeType(this) });
  },
  async text() {
    const bytes = await consume(this);
    return new TextDecoder().decode(bytes);
  },
  async json() {
    return JSON.parse(await this.text());
  },
  async formData() {
    const type = mimeType(this);
    const bytes = await consume(this);
    if (/^multipart\/form-data/i.test(type)) return parseMultipart(bytes, type);
    if (/^application\/x-www-form-urlencoded/i.test(type)) {
      const form = new FormData();
      for (const [name, value] of new URLSearchParams(new TextDecoder().decode(bytes))) form.append(name, value);
      return form;
    }
    throw invalid('Content-Type was not one of "multipart/form-data" or "application/x-www-form-urlencoded".');
  },
};

function mixBody(Class) {
  for (const name of Object.getOwnPropertyNames(bodyMethods)) {
    Object.defineProperty(Class.prototype, name, Object.getOwnPropertyDescriptor(bodyMethods, name));
  }
}

function cloneBody(body) {
  if (body === null) return null;
  if (body.stream === null) return { stream: null, source: body.source, length: body.length };
  const [left, right] = body.stream.tee();
  body.stream = left;
  return { stream: right, source: null, length: body.length };
}

// --- Request ---

const methods = new Set(['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT']);
const forbiddenMethods = new Set(['CONNECT', 'TRACE', 'TRACK']);
const redirects = new Set(['follow', 'error', 'manual']);

class Request {
  constructor(input, init = {}) {
    if (init === null || (typeof init !== 'object' && typeof init !== 'function')) init = {};
    let state;
    if (input instanceof Request) {
      const other = input[kState];
      state = {
        url: other.url,
        method: other.method,
        headers: new Headers(other.headers),
        body: null,
        used: false,
        redirect: other.redirect,
        signal: other.signal,
        keepalive: other.keepalive,
        mode: other.mode,
        credentials: other.credentials,
        cache: other.cache,
        referrer: other.referrer,
        referrerPolicy: other.referrerPolicy,
        integrity: other.integrity,
      };
      if (init.body === undefined && other.body !== null) {
        if (isUnusable(input)) throw invalid('Cannot construct a Request with a Request object that has already been used.');
        state.body = other.body;
        other.used = true;
      }
    } else {
      let url;
      try {
        url = new URL(String(input), pageURL());
      } catch (cause) {
        throw new TypeError(`Failed to parse URL from ${input}`, { cause });
      }
      if (url.username || url.password) {
        throw invalid(`Request cannot be constructed from a URL that includes credentials: ${input}`);
      }
      state = {
        url,
        method: 'GET',
        headers: new Headers(),
        body: null,
        used: false,
        redirect: 'follow',
        signal: null,
        keepalive: false,
        mode: 'cors',
        credentials: 'same-origin',
        cache: 'default',
        referrer: 'about:client',
        referrerPolicy: '',
        integrity: '',
      };
    }
    if (init.method !== undefined) {
      let method = String(init.method);
      if (!tokenRegExp.test(method)) throw invalid(`'${method}' is not a valid HTTP method.`);
      if (forbiddenMethods.has(method.toUpperCase())) throw invalid(`'${method}' HTTP method is unsupported.`);
      if (methods.has(method.toUpperCase())) method = method.toUpperCase();
      state.method = method;
    }
    if (init.redirect !== undefined) {
      if (!redirects.has(init.redirect)) {
        throw invalid(`Request constructor: ${init.redirect} is not an accepted type. Expected one of follow, error, manual.`);
      }
      state.redirect = init.redirect;
    }
    for (const name of ['mode', 'credentials', 'cache', 'referrerPolicy', 'integrity']) {
      if (init[name] !== undefined) state[name] = String(init[name]);
    }
    if (init.keepalive !== undefined) state.keepalive = Boolean(init.keepalive);
    if (init.headers !== undefined) state.headers = new Headers(init.headers);
    const controller = new AbortController();
    const parent = init.signal !== undefined ? init.signal : state.signal;
    if (parent != null) {
      if (typeof parent.aborted !== 'boolean' || typeof parent.addEventListener !== 'function') {
        throw invalid("Failed to construct 'Request': member signal is not of type AbortSignal.");
      }
      if (parent.aborted) controller.abort(parent.reason);
      else parent.addEventListener('abort', () => controller.abort(parent.reason), { once: true });
    }
    state.signal = controller.signal;
    if (init.body !== undefined && init.body !== null) {
      if (state.method === 'GET' || state.method === 'HEAD') {
        throw invalid('Request with GET/HEAD method cannot have body.');
      }
      const { body, type } = extractBody(init.body);
      if (body.stream && init.duplex !== 'half') {
        throw invalid('RequestInit: duplex option is required when sending a body.');
      }
      state.body = body;
      if (type && !state.headers.has('content-type')) state.headers.append('content-type', type);
    }
    this[kState] = state;
  }

  get url() { return this[kState].url.href; }
  get method() { return this[kState].method; }
  get headers() { return this[kState].headers; }
  get signal() { return this[kState].signal; }
  get redirect() { return this[kState].redirect; }
  get keepalive() { return this[kState].keepalive; }
  get mode() { return this[kState].mode; }
  get credentials() { return this[kState].credentials; }
  get cache() { return this[kState].cache; }
  get referrer() { return this[kState].referrer; }
  get referrerPolicy() { return this[kState].referrerPolicy; }
  get integrity() { return this[kState].integrity; }
  get destination() { return ''; }
  get isReloadNavigation() { return false; }
  get isHistoryNavigation() { return false; }
  get duplex() { return 'half'; }

  clone() {
    if (isUnusable(this)) throw invalid('Request.clone: Body has already been consumed.');
    const clone = Object.create(Object.getPrototypeOf(this));
    const state = this[kState];
    clone[kState] = { ...state, headers: new Headers(state.headers), body: cloneBody(state.body), used: false };
    return clone;
  }

  get [Symbol.toStringTag]() { return 'Request'; }
}
mixBody(Request);

// --- Response ---

const nullBodyStatus = new Set([101, 103, 204, 205, 304]);
const redirectStatus = new Set([301, 302, 303, 307, 308]);

function makeResponse(fields) {
  const response = Object.create(Response.prototype);
  response[kState] = {
    type: 'default',
    url: null,
    urlList: [],
    status: 200,
    statusText: '',
    headers: new Headers(),
    body: null,
    used: false,
    ...fields,
  };
  return response;
}

class Response {
  constructor(body = null, init = {}) {
    if (init === null || (typeof init !== 'object' && typeof init !== 'function')) init = {};
    const status = init.status !== undefined ? Number(init.status) : 200;
    if (!Number.isInteger(status) || status < 200 || status > 599) {
      throw new RangeError(`init["status"] must be in the range of 200 to 599, inclusive.`);
    }
    const statusText = init.statusText !== undefined ? String(init.statusText) : '';
    if (/[^\t\x20-\x7e\x80-\xff]/.test(statusText)) throw invalid('Invalid statusText');
    const headers = new Headers(init.headers);
    let state = null;
    if (body !== null && body !== undefined) {
      if (nullBodyStatus.has(status)) {
        throw invalid(`Response constructor: Invalid response status code ${status}`);
      }
      const extracted = extractBody(body);
      state = extracted.body;
      if (extracted.type && !headers.has('content-type')) headers.append('content-type', extracted.type);
    }
    this[kState] = {
      type: 'default',
      url: null,
      urlList: [],
      status,
      statusText,
      headers,
      body: state,
      used: false,
    };
  }

  static error() {
    const response = makeResponse({ type: 'error', status: 0 });
    response[kState].headers[kGuard] = 'immutable';
    return response;
  }

  static json(data, init = {}) {
    const text = JSON.stringify(data);
    if (text === undefined) throw invalid('Value is not JSON serializable');
    const headers = new Headers(init?.headers);
    if (!headers.has('content-type')) headers.set('content-type', 'application/json');
    return new Response(text, { ...init, headers });
  }

  static redirect(url, status = 302) {
    let parsed;
    try {
      parsed = new URL(String(url), pageURL());
    } catch (cause) {
      throw new TypeError(`Failed to parse URL from ${url}`, { cause });
    }
    if (!redirectStatus.has(status)) throw new RangeError(`Invalid status code ${status}`);
    const response = makeResponse({ status });
    response[kState].headers.set('location', parsed.href);
    response[kState].headers[kGuard] = 'immutable';
    return response;
  }

  get type() { return this[kState].type; }
  get url() {
    const url = this[kState].url;
    if (!url) return '';
    const href = url.href;
    const hash = href.indexOf('#');
    return hash === -1 ? href : href.slice(0, hash);
  }
  get redirected() { return this[kState].urlList.length > 1; }
  get status() { return this[kState].status; }
  get ok() { return this[kState].status >= 200 && this[kState].status <= 299; }
  get statusText() { return this[kState].statusText; }
  get headers() { return this[kState].headers; }

  clone() {
    if (isUnusable(this)) throw invalid('Response.clone: Body has already been consumed.');
    const state = this[kState];
    const clone = Object.create(Object.getPrototypeOf(this));
    const headers = new Headers(state.headers);
    headers[kGuard] = state.headers[kGuard];
    clone[kState] = { ...state, headers, body: cloneBody(state.body), used: false };
    return clone;
  }

  get [Symbol.toStringTag]() { return 'Response'; }
}
mixBody(Response);

// --- FormData ---

function formEntry(name, value, filename) {
  name = String(name);
  if (value instanceof Blob) {
    if (!(value instanceof File) || filename !== undefined) {
      value = new File([value], filename !== undefined ? String(filename) : (value.name ?? 'blob'), {
        type: value.type,
        lastModified: value.lastModified,
      });
    }
    return [name, value];
  }
  return [name, String(value)];
}

class FormData {
  constructor(form = undefined) {
    if (form !== undefined) {
      throw invalid("FormData constructor: Argument 1 could not be converted to: undefined.");
    }
    this[kHeaders] = [];
  }

  append(name, value, filename = undefined) {
    this[kHeaders].push(formEntry(name, value, filename));
  }

  delete(name) {
    name = String(name);
    this[kHeaders] = this[kHeaders].filter(([entry]) => entry !== name);
  }

  get(name) {
    name = String(name);
    const entry = this[kHeaders].find(([key]) => key === name);
    return entry ? entry[1] : null;
  }

  getAll(name) {
    name = String(name);
    return this[kHeaders].filter(([key]) => key === name).map(([, value]) => value);
  }

  has(name) {
    name = String(name);
    return this[kHeaders].some(([key]) => key === name);
  }

  set(name, value, filename = undefined) {
    const entry = formEntry(name, value, filename);
    const index = this[kHeaders].findIndex(([key]) => key === entry[0]);
    if (index === -1) {
      this[kHeaders].push(entry);
      return;
    }
    this[kHeaders][index] = entry;
    this[kHeaders] = this[kHeaders].filter(([key], position) => position <= index || key !== entry[0]);
  }

  forEach(callback, thisArg = undefined) {
    for (const [name, value] of this) callback.call(thisArg, value, name, this);
  }

  *entries() { for (const [name, value] of this[kHeaders]) yield [name, value]; }
  *keys() { for (const [name] of this[kHeaders]) yield name; }
  *values() { for (const [, value] of this[kHeaders]) yield value; }
  [Symbol.iterator]() { return this.entries(); }

  get [Symbol.toStringTag]() { return 'FormData'; }
}

// --- fetch ---

function abortReason(signal) {
  return signal.reason !== undefined ? signal.reason : new DOMException('This operation was aborted', 'AbortError');
}

function fetchFailed(cause) {
  return new TypeError('fetch failed', { cause });
}

function dataResponse(url) {
  const href = url.href;
  const comma = href.indexOf(',');
  if (comma === -1) throw fetchFailed(new Error('invalid data: URL'));
  let meta = href.slice(5, comma).trim();
  const base64 = /;\s*base64$/i.test(meta);
  if (base64) meta = meta.replace(/;\s*base64$/i, '');
  const payload = decodeURIComponent(href.slice(comma + 1).replace(/#.*$/, ''));
  const bytes = base64 ? Buffer.from(payload.replace(/\s/g, ''), 'base64') : Buffer.from(payload, 'latin1');
  const type = meta && !meta.startsWith(';') ? meta : `text/plain${meta || ';charset=US-ASCII'}`;
  const response = makeResponse({
    type: 'basic',
    url,
    urlList: [url],
    statusText: 'OK',
    body: { stream: null, source: new Uint8Array(bytes), length: bytes.length },
  });
  response[kState].headers.set('content-type', type);
  response[kState].headers[kGuard] = 'immutable';
  return response;
}

// A page's file: URL: the file's bytes, typed by its extension.
async function fileResponse(url) {
  const { readFile } = require('fs/promises');
  const { fileURLToPath } = require('internal/url');
  let bytes;
  try {
    bytes = await readFile(fileURLToPath(url));
  } catch (cause) {
    throw fetchFailed(cause);
  }
  const response = makeResponse({
    type: 'basic',
    url,
    urlList: [url],
    statusText: 'OK',
    body: { stream: null, source: new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength), length: bytes.length },
  });
  const extension = /\.([^./\\]+)$/.exec(url.pathname)?.[1]?.toLowerCase();
  if (fileTypes[extension]) response[kState].headers.set('content-type', fileTypes[extension]);
  response[kState].headers[kGuard] = 'immutable';
  return response;
}

// URLs whose scheme the main process serves through Electron's protocol
// module, such as VS Code's vscode-file:, and file: when it intercepts it.
const protocolBinding = internalBinding('protocol');

// The main process's answer for a URL: its status and bytes, typed as the
// handler says or else by the URL's extension. A refused request fails.
function servedResponse(url) {
  const served = protocolBinding.serve(url.href);
  if (served === undefined) throw fetchFailed(new Error(`${url.href} was not served`));
  const { status, body, mimeType } = served;
  const response = makeResponse({
    type: 'basic',
    url,
    urlList: [url],
    status,
    statusText: status >= 200 && status < 300 ? 'OK' : '',
    body: { stream: null, source: body, length: body.length },
  });
  const extension = /\.([^./\\]+)$/.exec(url.pathname)?.[1]?.toLowerCase();
  const type = mimeType ?? fileTypes[extension];
  if (type) response[kState].headers.set('content-type', type);
  response[kState].headers[kGuard] = 'immutable';
  return response;
}

// A web stream over a Node readable, read as the consumer asks.
function webStream(source) {
  return new ReadableStream({
    start(controller) {
      source.on('data', (chunk) => {
        controller.enqueue(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength));
        if (controller.desiredSize <= 0) source.pause();
      });
      source.once('end', () => controller.close());
      source.once('error', (error) => controller.error(fetchFailed(error)));
      source.pause();
    },
    pull() {
      source.resume();
    },
    cancel(reason) {
      source.destroy(reason);
    },
  });
}

function send(url, method, headers, body, signal) {
  const module = url.protocol === 'https:' ? require('https') : require('http');
  return new Promise((resolve, reject) => {
    const request = module.request(url, { method, headers: headerArray(headers) }, resolve);
    request.on('error', reject);
    const onAbort = () => request.destroy(abortReason(signal));
    signal.addEventListener('abort', onAbort, { once: true });
    request.once('close', () => signal.removeEventListener('abort', onAbort));
    request.end(body ? Buffer.from(body.buffer, body.byteOffset, body.byteLength) : undefined);
  });
}

async function run(request) {
  const state = request[kState];
  const signal = state.signal;
  let url = state.url;
  const urlList = [url];
  let method = state.method;
  const headers = new Headers(state.headers);
  let body = state.body !== null ? await consume(request) : null;
  for (let followed = 0; ; followed++) {
    if (signal.aborted) throw abortReason(signal);
    if (url.protocol === 'data:') return dataResponse(url);
    if (pageURL() !== undefined && protocolBinding.serves(url.href)) return servedResponse(url);
    if (url.protocol === 'file:' && pageURL() !== undefined) return fileResponse(url);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      throw fetchFailed(new Error(`URL scheme "${url.protocol.slice(0, -1)}" is not supported.`));
    }
    if (!headers.has('accept')) headers.set('accept', '*/*');
    if (!headers.has('accept-language')) headers.set('accept-language', '*');
    if (!headers.has('sec-fetch-mode')) headers.set('sec-fetch-mode', 'cors');
    if (!headers.has('user-agent')) headers.set('user-agent', 'node');
    if (!headers.has('accept-encoding')) headers.set('accept-encoding', 'gzip, deflate');
    if (body !== null) headers.set('content-length', String(body.byteLength));
    let incoming;
    try {
      incoming = await send(url, method, headers, body, signal);
    } catch (error) {
      if (signal.aborted) throw abortReason(signal);
      throw fetchFailed(error);
    }
    const status = incoming.statusCode;
    const location = incoming.headers.location;
    if (redirectStatus.has(status) && location !== undefined && state.redirect !== 'manual') {
      incoming.resume();
      if (state.redirect === 'error') throw fetchFailed(new Error('unexpected redirect'));
      if (followed >= 20) throw fetchFailed(new Error('redirect count exceeded'));
      const next = new URL(location, url);
      if ((status === 303 && method !== 'GET' && method !== 'HEAD') ||
          ((status === 301 || status === 302) && method === 'POST')) {
        method = 'GET';
        body = null;
        for (const name of ['content-encoding', 'content-language', 'content-location', 'content-type', 'content-length']) {
          headers.delete(name);
        }
      }
      if (next.origin !== url.origin) {
        headers.delete('authorization');
        headers.delete('proxy-authorization');
        headers.delete('cookie');
        headers.delete('host');
      }
      url = next;
      urlList.push(url);
      continue;
    }
    const responseHeaders = new Headers();
    for (let index = 0; index < incoming.rawHeaders.length; index += 2) {
      responseHeaders.append(incoming.rawHeaders[index], incoming.rawHeaders[index + 1]);
    }
    responseHeaders[kGuard] = 'immutable';
    let responseBody = null;
    if (method !== 'HEAD' && !nullBodyStatus.has(status)) {
      let source = incoming;
      const coding = (incoming.headers['content-encoding'] || '').trim().toLowerCase();
      if (coding === 'gzip' || coding === 'x-gzip' || coding === 'deflate') {
        const zlib = require('zlib');
        const options = { flush: zlib.constants.Z_SYNC_FLUSH, finishFlush: zlib.constants.Z_SYNC_FLUSH };
        source = coding === 'deflate' ? zlib.createUnzip(options) : zlib.createGunzip(options);
        incoming.on('error', (error) => source.destroy(error));
        incoming.pipe(source);
      }
      responseBody = { stream: webStream(source), source: null, length: null };
      signal.addEventListener('abort', () => incoming.destroy(abortReason(signal)), { once: true });
    } else {
      incoming.resume();
    }
    return makeResponse({
      type: 'basic',
      url,
      urlList,
      status,
      statusText: incoming.statusMessage || '',
      headers: responseHeaders,
      body: responseBody,
    });
  }
}

function fetch(input, init = undefined) {
  let request;
  try {
    request = new Request(input, init);
  } catch (error) {
    return Promise.reject(error);
  }
  if (request.signal.aborted) return Promise.reject(abortReason(request.signal));
  // A page a service worker controls asks the worker first, as its `fetch`
  // event (the Web API prelude's hook); without an answer the request goes
  // on as usual.
  const serviceWorker = globalThis.__nativeboatServiceWorkerFetch;
  if (typeof serviceWorker === 'function') {
    return serviceWorker(request).then((response) => response ?? run(request));
  }
  return run(request);
}

module.exports = {
  fetch,
  FormData,
  Headers,
  Request,
  Response,
};
