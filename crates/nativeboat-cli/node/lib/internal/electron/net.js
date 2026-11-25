'use strict';

// Electron's `net.request` over the http and https modules: the request is
// a Writable whose body is sent once it ends, redirects follow Electron's
// `follow`, `error` and `manual` modes, and the response is a Readable with
// Node's lower-cased headers, as Electron's network service reports them.
// Every session shares the one native network stack.

const { Buffer } = require('buffer');
const { Readable, Writable } = require('stream');
const http = require('http');
const https = require('https');
const { URL } = require('internal/url');

const REDIRECTS = new Set([301, 302, 303, 307, 308]);
const MODES = new Set(['follow', 'error', 'manual']);
// Chromium's names for the connection failures Electron reports.
const NET_ERRORS = {
  ECONNREFUSED: 'net::ERR_CONNECTION_REFUSED',
  ECONNRESET: 'net::ERR_CONNECTION_RESET',
  ECONNABORTED: 'net::ERR_CONNECTION_ABORTED',
  ETIMEDOUT: 'net::ERR_CONNECTION_TIMED_OUT',
  ENOTFOUND: 'net::ERR_NAME_NOT_RESOLVED',
  EAI_AGAIN: 'net::ERR_NAME_NOT_RESOLVED',
  ENETUNREACH: 'net::ERR_ADDRESS_UNREACHABLE',
  EHOSTUNREACH: 'net::ERR_ADDRESS_UNREACHABLE',
};

function networkError(error) {
  const name = NET_ERRORS[error && error.code];
  if (!name) return error;
  const reported = new Error(name);
  reported.code = error.code;
  return reported;
}

class IncomingMessage extends Readable {
  constructor(response) {
    super();
    this._response = response;
    this.statusCode = response.statusCode;
    this.statusMessage = response.statusMessage;
    this.headers = response.headers;
    this.rawHeaders = response.rawHeaders;
    this.httpVersion = response.httpVersion;
    this.httpVersionMajor = response.httpVersionMajor;
    this.httpVersionMinor = response.httpVersionMinor;
    response.on('data', (chunk) => {
      if (!this.push(chunk)) response.pause();
    });
    response.on('end', () => this.push(null));
    response.on('error', (error) => this.destroy(error));
  }

  _read() {
    this._response.resume();
  }
}

// The URL a request goes to: `url`, or Electron's protocol, host, hostname,
// port and path options.
function target(options) {
  if (options.url !== undefined) return new URL(options.url);
  const protocol = options.protocol || 'http:';
  const authority = options.hostname !== undefined ?
    `${options.hostname}${options.port ? `:${options.port}` : ''}` :
    (options.host || 'localhost');
  return new URL(`${protocol}//${authority}${options.path || '/'}`);
}

class ClientRequest extends Writable {
  constructor(options, callback, userAgent) {
    // `close` is the last event, after the response ends.
    super({ autoDestroy: false });
    options = typeof options === 'string' ? { url: options } : { ...options };
    const url = target(options);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      throw new Error(`Protocol "${url.protocol}" not supported`);
    }
    const redirect = options.redirect || 'follow';
    if (!MODES.has(redirect)) {
      throw new Error('redirect mode should be one of follow, error or manual');
    }
    this.method = (options.method || 'GET').toUpperCase();
    this._url = url;
    this._redirect = redirect;
    this._headers = new Map();
    this._body = [];
    this._sent = 0;
    this._started = false;
    this._aborted = false;
    this._request = null;
    this._response = null;
    this._followRedirect = null;
    this.chunkedEncoding = false;
    for (const [name, value] of Object.entries(options.headers || {})) {
      this.setHeader(name, value);
    }
    if (userAgent && !this._headers.has('user-agent')) this.setHeader('User-Agent', userAgent);
    if (callback) this.once('response', callback);
  }

  setHeader(name, value) {
    if (typeof name !== 'string') throw new TypeError('`name` should be a string in setHeader(name, value)');
    if (value == null) throw new Error('`value` required in setHeader("' + name + '", value)');
    if (this._started) throw new Error('Can\'t set headers after they are sent');
    this._headers.set(name.toLowerCase(), [name, String(value)]);
  }

  getHeader(name) {
    if (name == null) throw new Error('`name` is required for getHeader(name)');
    const header = this._headers.get(String(name).toLowerCase());
    return header && header[1];
  }

  removeHeader(name) {
    if (name == null) throw new Error('`name` is required for removeHeader(name)');
    if (this._started) throw new Error('Can\'t remove headers after they are sent');
    this._headers.delete(String(name).toLowerCase());
  }

  _write(chunk, encoding, callback) {
    this._started = true;
    this._body.push(chunk);
    this._sent += chunk.length;
    callback();
  }

  _final(callback) {
    this._started = true;
    this._send(this._url);
    callback();
  }

  _send(url) {
    const headers = {};
    for (const [name, value] of this._headers.values()) headers[name] = value;
    const transport = url.protocol === 'https:' ? https : http;
    const request = transport.request({
      protocol: url.protocol,
      hostname: url.hostname.replace(/^\[|\]$/g, ''),
      port: url.port || undefined,
      path: `${url.pathname}${url.search}`,
      method: this.method,
      headers,
      // Each request connects afresh, so no pooled connection a server has
      // since closed is reused.
      agent: false,
    });
    this._request = request;
    request.on('response', (response) => this._respond(response, url));
    request.on('error', (error) => {
      if (!this._aborted) this._die(networkError(error));
    });
    request.end(Buffer.concat(this._body));
  }

  _respond(response, url) {
    if (this._aborted) {
      response.resume();
      return;
    }
    const location = response.headers.location;
    if (REDIRECTS.has(response.statusCode) && location !== undefined) {
      response.resume();
      this._redirected(response, new URL(location, url));
      return;
    }
    this._response = new IncomingMessage(response);
    this._response.on('end', () => this.destroy());
    this.emit('response', this._response);
  }

  _redirected(response, url) {
    const status = response.statusCode;
    // 303, and 301 and 302 after a POST, continue as GET, as Chromium does.
    const method = status === 303 || ((status === 301 || status === 302) && this.method === 'POST') ?
      'GET' : this.method;
    if (this._redirect === 'error') {
      this._die(new Error('Attempted to redirect, but redirect policy was \'error\''));
      return;
    }
    let followed = this._redirect === 'follow';
    this._followRedirect = () => {
      followed = true;
    };
    try {
      this.emit('redirect', status, method, url.href, response.headers);
    } finally {
      this._followRedirect = null;
    }
    if (this._aborted) return;
    if (!followed) {
      this._die(new Error('Redirect was cancelled'));
      return;
    }
    if (method !== this.method) {
      this.method = method;
      this._body = [];
    }
    this._url = url;
    this._send(url);
  }

  followRedirect() {
    if (!this._followRedirect) throw new Error('followRedirect() called, but was not waiting for a redirect');
    this._followRedirect();
  }

  abort() {
    if (this._aborted) return;
    this._aborted = true;
    process.nextTick(() => this.emit('abort'));
    if (this._response) this._response.emit('aborted');
    this._die();
  }

  _die(error) {
    if (this._request) this._request.destroy();
    if (this._response) this._response.destroy(error);
    if (this.destroyed) {
      if (error) this.emit('error', error);
    } else {
      this.destroy(error);
    }
  }

  getUploadProgress() {
    return {
      active: this._started && !this._response,
      started: this._started,
      current: this._sent,
      total: this._sent,
    };
  }
}

module.exports = {
  request(options, callback, userAgent) {
    return new ClientRequest(options, callback, userAgent);
  },
};
