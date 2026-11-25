// XMLHttpRequest, asynchronous only, over the context's `fetch`. -----------------------

class XMLHttpRequestEventTarget extends EventTarget {}
defineHandlers(XMLHttpRequestEventTarget.prototype, ['abort', 'error', 'load', 'loadend', 'loadstart', 'progress', 'timeout']);
class XMLHttpRequestUpload extends XMLHttpRequestEventTarget {}

const XHR = Symbol('nativeboat.xhr');
const xhrStates = { UNSENT: 0, OPENED: 1, HEADERS_RECEIVED: 2, LOADING: 3, DONE: 4 };
const xhrState = (message) => new DOMException(`Failed to execute on 'XMLHttpRequest': ${message}`, 'InvalidStateError');
const progress = (target, type, loaded = 0) => target.dispatchEvent(new ProgressEvent(type, { lengthComputable: loaded > 0, loaded, total: loaded }));
const changeState = (request, state) => {
  request[XHR].state = state;
  request.dispatchEvent(new Event('readystatechange'));
};

class XMLHttpRequest extends XMLHttpRequestEventTarget {
  constructor() {
    super();
    hidden(this, XHR, { state: 0, sent: false, headers: [], responseHeaders: [], bytes: null, status: 0, statusText: '', url: '', responseURL: '' });
    Object.assign(this, { upload: new XMLHttpRequestUpload(), timeout: 0, withCredentials: false, responseType: '' });
  }

  get readyState() {
    return this[XHR].state;
  }

  get status() {
    return this[XHR].status;
  }

  get statusText() {
    return this[XHR].statusText;
  }

  get responseURL() {
    return this[XHR].responseURL;
  }

  get responseText() {
    if (this.responseType !== '' && this.responseType !== 'text') throw xhrState(`the value is only accessible if responseType is '' or 'text' (was '${this.responseType}').`);
    const bytes = this[XHR].bytes;
    return bytes ? new TextDecoder().decode(bytes) : '';
  }

  get response() {
    const { bytes, state } = this[XHR];
    if (this.responseType === '' || this.responseType === 'text') return this.responseText;
    if (state !== 4 || !bytes) return null;
    if (this.responseType === 'json') {
      try {
        return JSON.parse(new TextDecoder().decode(bytes));
      } catch (error) {
        return null;
      }
    }
    if (this.responseType === 'arraybuffer') return bytes.slice().buffer;
    if (this.responseType === 'blob') return new Blob([bytes], { type: this.getResponseHeader('content-type') || '' });
    return null;
  }

  open(method, url, async = true) {
    const name = String(method);
    Object.assign(this[XHR], {
      method: ['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT', 'PATCH'].includes(name.toUpperCase()) ? name.toUpperCase() : name,
      url: new URL(String(url), location.href).href,
      async: async !== false,
      sent: false,
      aborted: false,
      timedOut: false,
      headers: [],
      responseHeaders: [],
      bytes: null,
      status: 0,
      statusText: '',
      responseURL: '',
    });
    changeState(this, 1);
  }

  setRequestHeader(name, value) {
    const state = this[XHR];
    if (state.state !== 1 || state.sent) throw xhrState("The object's state must be OPENED.");
    state.headers.push([String(name), String(value)]);
  }

  getResponseHeader(name) {
    const key = String(name).toLowerCase();
    const values = this[XHR].responseHeaders.filter(([header]) => header === key).map(([, value]) => value);
    return values.length ? values.join(', ') : null;
  }

  getAllResponseHeaders() {
    return this[XHR].responseHeaders.map(([name, value]) => `${name}: ${value}\r\n`).join('');
  }

  overrideMimeType() {}

  send(body = null) {
    const state = this[XHR];
    if (state.state !== 1 || state.sent) throw xhrState("The object's state must be OPENED.");
    if (!state.async) throw new DOMException('Synchronous XMLHttpRequest is not supported.', 'NotSupportedError');
    state.sent = true;
    const controller = new AbortController();
    state.controller = controller;
    progress(this, 'loadstart');
    const timer = this.timeout > 0 ? setTimeout(() => ((state.timedOut = true), controller.abort()), this.timeout) : null;
    const init = { method: state.method, headers: state.headers, signal: controller.signal, credentials: this.withCredentials ? 'include' : 'same-origin' };
    if (body !== null && state.method !== 'GET' && state.method !== 'HEAD') init.body = body;
    global
      .fetch(state.url, init)
      .then(async (response) => {
        Object.assign(state, { status: response.status, statusText: response.statusText, responseURL: response.url });
        response.headers.forEach((value, name) => state.responseHeaders.push([name.toLowerCase(), value]));
        changeState(this, 2);
        changeState(this, 3);
        state.bytes = new Uint8Array(await response.arrayBuffer());
        clearTimeout(timer);
        progress(this, 'progress', state.bytes.length);
        changeState(this, 4);
        progress(this, 'load', state.bytes.length);
        progress(this, 'loadend', state.bytes.length);
      })
      .catch(() => {
        clearTimeout(timer);
        if (state.aborted) return;
        Object.assign(state, { status: 0, statusText: '', responseHeaders: [], bytes: null });
        changeState(this, 4);
        progress(this, state.timedOut ? 'timeout' : 'error');
        progress(this, 'loadend');
      });
  }

  abort() {
    const state = this[XHR];
    if (state.sent && state.state !== 4) {
      state.aborted = true;
      state.controller.abort();
      Object.assign(state, { status: 0, statusText: '', bytes: null });
      changeState(this, 4);
      progress(this, 'abort');
      progress(this, 'loadend');
    }
    if (state.state === 4) state.state = 0;
    state.sent = false;
  }
}
Object.assign(XMLHttpRequest, xhrStates);
Object.assign(XMLHttpRequest.prototype, xhrStates);
defineHandlers(XMLHttpRequest.prototype, ['readystatechange']);
