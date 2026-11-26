// Frames: the documents `<iframe>` elements show. ---------------------------
//
// A frame's document runs in a realm of its own. A frame and its page talk
// over a channel of the process's message ports; each side wraps its end as
// the other's window, whose `postMessage` sends the data with the sender's
// origin and the ports it transfers. The frame tells its page when its
// document has loaded, and the page then fires the element's `load`.

// The window of a frame's or a page's document, as the other side sees it:
// cross-origin, so only messaging and the window's own references.
const remoteWindow = (port) => {
  const state = { closed: false };
  const proxy = {
    postMessage(data, targetOrigin, transfer) {
      const options = typeof targetOrigin === 'object' && targetOrigin !== null ? targetOrigin : { targetOrigin, transfer };
      const target = options.targetOrigin === undefined ? '/' : String(options.targetOrigin);
      if (target !== '*' && target !== '/') {
        try {
          new URL(target);
        } catch {
          throw new DOMException(`Failed to execute 'postMessage' on 'Window': Invalid target origin '${target}' in a call to 'postMessage'.`, 'SyntaxError');
        }
      }
      const ports = transferList(options.transfer, null);
      if (state.closed) return;
      port.postMessage({ kind: 'message', data, origin: location.origin, target }, ports);
    },
    focus() {},
    blur() {},
    close() {},
    get closed() {
      return state.closed;
    },
    get window() {
      return proxy;
    },
    get self() {
      return proxy;
    },
    get frames() {
      return proxy;
    },
    length: 0,
  };
  return { proxy, state };
};

// Delivers a channel's envelopes: messages as `message` events of this
// window from `source`, unless their target origin is another, and the
// frame's load to `loaded`.
const receiveWindowMessages = (port, source, loaded) => {
  port.onmessage = (event) => {
    const envelope = event.data;
    if (!envelope || typeof envelope !== 'object') return;
    if (envelope.kind === 'load') {
      if (loaded) loaded();
      return;
    }
    if (envelope.kind !== 'message') return;
    const { target, origin } = envelope;
    if (target !== '*' && !(target === '/' ? origin === location.origin : target === location.origin)) return;
    fire(global, MessageEvent, 'message', { data: envelope.data, origin, source, ports: Object.freeze([...event.ports]) });
  };
};

// In a frame's realm: the page's window is the frame's `parent` and `top`,
// itself for a page of the frame's origin.
{
  const frame = globalThis.__nativeboat_frame;
  if (frame) {
    delete globalThis.__nativeboat_frame;
    const [handle, parentWindow] = frame;
    const port = nativePort(handle);
    const { proxy } = remoteWindow(port);
    const parent = parentWindow || proxy;
    hidden(global, 'parent', parent);
    hidden(global, 'top', parentWindow ? parentWindow.top : proxy);
    hidden(global, 'frameElement', null);
    receiveWindowMessages(port, proxy, null);
    global.addEventListener('load', () => port.postMessage({ kind: 'load' }), { once: true });
  }
}

// In a page: each connected `<iframe>` shows the document its `src` names,
// started once the element is in the document and again when its `src`
// changes, and ended when it leaves.
const frameStates = new Map();
const frameURL = (element) => {
  const src = element.getAttribute('src');
  if (src === null || src.trim() === '') return 'about:blank';
  try {
    return new URL(src, document.baseURI).href;
  } catch {
    return 'about:blank';
  }
};
const endFrame = (element) => {
  const state = frameStates.get(element);
  if (!state) return;
  frameStates.delete(element);
  state.window.state.closed = true;
  state.port.close();
  if (state.id) host.endFrame(state.id);
};
const startFrame = (element) => {
  // Hosts without frames, as a renderer without the runtime's, show none.
  if (typeof host.startFrame !== 'function') return;
  const url = frameURL(element);
  const current = frameStates.get(element);
  if (current && current.url === url) return;
  endFrame(element);
  const [near, far] = host.messageChannel();
  const port = nativePort(near);
  const remote = remoteWindow(port);
  const state = { port, window: remote, url, id: 0, direct: null };
  frameStates.set(element, state);
  receiveWindowMessages(port, remote.proxy, () => fire(element, Event, 'load'));
  const width = parseFloat(element.getAttribute('width')) || element.clientWidth || 300;
  const height = parseFloat(element.getAttribute('height')) || element.clientHeight || 150;
  // A document of this page's origin is reached directly.
  let sameOrigin = false;
  try {
    sameOrigin = location.origin !== 'null' && new URL(url).origin === location.origin;
  } catch {}
  const [id, frameWindow] = host.startFrame(far, url, width, height, idOf(element), sameOrigin);
  state.id = id;
  if (sameOrigin && frameWindow) state.direct = frameWindow;
  // A frame with no compiled document stays empty, and loads as one does.
  if (!state.id) setTimeout(() => frameStates.get(element) === state && fire(element, Event, 'load'), 0);
};
// An element in the document has its frame at once, as browsers give it
// one when it is inserted.
const frameOf = (element) => {
  if (!element.isConnected) return null;
  if (!frameStates.has(element)) startFrame(element);
  return frameStates.get(element) || null;
};
const frameContentWindow = (element) => {
  const state = frameOf(element);
  return state ? state.direct || state.window.proxy : null;
};
const frameContentDocument = (element) => {
  const state = frameOf(element);
  return state && state.direct ? state.direct.document : null;
};
const syncFrames = () => {
  for (const element of [...frameStates.keys()]) {
    if (!element.isConnected) endFrame(element);
  }
  for (const element of document.querySelectorAll('iframe')) startFrame(element);
};
// Only mutations that may add, move or remove an `<iframe>`, or change its
// `src`, look for frames again.
const touchesFrames = (records) =>
  records.some((record) => {
    if (record.type === 'attributes') return record.target.localName === 'iframe';
    if (frameStates.size && record.removedNodes.length) return true;
    for (const node of record.addedNodes) {
      if (node.nodeType === 1 && (node.localName === 'iframe' || node.querySelector('iframe'))) return true;
    }
    return false;
  });
new MutationObserver((records) => {
  if (touchesFrames(records)) syncFrames();
}).observe(document, { childList: true, subtree: true, attributes: true, attributeFilter: ['src'] });
syncFrames();
