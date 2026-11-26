// Window: focus and activation behaviour, URLs, location and history,
// storage, frames, observers, encoding and the global object. ----------------

// The page's language: the application's locale, as Chromium's --lang sets it.
const languageTag = () => {
  try {
    return Intl.getCanonicalLocales(String(host.locale()))[0];
  } catch {
    return 'en-US';
  }
};

// Focus. -----------------------------------------------------------------------

const edited = new WeakSet();
const focusNode = (element, options = {}, visible = false) => {
  const target = element ? element[ID] : null;
  if (compositionSession && compositionSession.host !== element) interruptComposition();
  const changed = native.setFocus(target, visible);
  if (changed === null) return;
  // Tracing names the script that moved the focus.
  if (native.tracing()) native.trace(`focus moved to ${element ? element.localName : 'none'} by ${new Error().stack}`);
  const previous = wrap(changed[0]);
  if (previous) {
    fire(previous, FocusEvent, 'blur', { relatedTarget: element, view: global });
    fire(previous, FocusEvent, 'focusout', { bubbles: true, relatedTarget: element, view: global });
    if (edited.has(previous)) {
      edited.delete(previous);
      fire(previous, Event, 'change', { bubbles: true });
    }
  }
  if (element) {
    fire(element, FocusEvent, 'focus', { relatedTarget: previous, view: global });
    fire(element, FocusEvent, 'focusin', { bubbles: true, relatedTarget: previous, view: global });
    if (!options.preventScroll) page.scrollIntoView(target, 'nearest', 'nearest');
    // A focused text control holds the frame's selection in Chromium, so
    // the document's collapses beside it.
    const parent = element.parentNode;
    if (isTextControl(element) && parent) associateRange(createRange([parent, childIndex(element)]), 'none', false);
  }
  page.focus(target);
};

// Activation behaviour of clicks. ---------------------------------------------

const activationTarget = (target) => {
  for (let element = target; element && element.nodeType === 1; element = element.parentElement) {
    const name = element.localName;
    if (name === 'input' || name === 'button' || name === 'label' || name === 'summary' || name === 'select') return element;
    if ((name === 'a' || name === 'area') && element.hasAttribute('href')) return element;
  }
  return null;
};

const radioGroup = (radio) => {
  const name = radio.getAttribute('name');
  if (!name) return [radio];
  const owner = native.formOwner(radio[ID]);
  return [...document.querySelectorAll('input[type=radio]')].filter(
    (other) => other.getAttribute('name') === name && native.formOwner(other[ID]) === owner,
  );
};

const submitForm = (form, submitter) => {
  if (!form) return;
  const event = fire(form, SubmitEvent, 'submit', { bubbles: true, cancelable: true, submitter });
  if (event.allowed) navigateForm(form);
};

const navigateForm = (form) => {
  host.print('warn', `Form submission to '${form.action}' is not supported; call preventDefault() in the submit handler.`);
};

const navigate = (url, target = '') => {
  const next = new URL(url, location.href);
  if (target && target !== '_self') {
    if (typeof host.web.onopen === 'function') host.web.onopen(next.href, target);
    return;
  }
  const current = new URL(location.href);
  if (next.href.split('#')[0] === current.href.split('#')[0]) {
    if (next.hash !== current.hash) {
      history.pushState(history.state, '', next.href);
      changeHash(current.href);
    }
    scrollToFragment(next.hash);
    return;
  }
  if (typeof host.web.onnavigate === 'function') host.web.onnavigate(next.href);
};

const activate = (target, init, trusted = true) => {
  // Disabled form controls receive no clicks.
  const control = target && target.nodeType === 1 ? target.closest('button, input, select, textarea') : null;
  if (control && control.disabled) return true;
  const activated = activationTarget(target);
  const input = activated && activated.localName === 'input' && !activated.disabled ? activated : null;
  let restore = null;
  if (input && input.type === 'checkbox') {
    const id = input[ID];
    const before = { checked: native.checked(id), indeterminate: native.indeterminate(id) };
    native.setChecked(id, !before.checked);
    native.setIndeterminate(id, false);
    restore = () => {
      native.setChecked(id, before.checked);
      native.setIndeterminate(id, before.indeterminate);
    };
  } else if (input && input.type === 'radio') {
    const previous = radioGroup(input).find((radio) => native.checked(radio[ID])) || null;
    if (previous !== input) {
      native.setChecked(input[ID], true);
      restore = () => {
        native.setChecked(input[ID], false);
        if (previous) native.setChecked(previous[ID], true);
      };
    }
  }
  const event = new MouseEvent('click', {
    bubbles: true,
    cancelable: true,
    composed: true,
    view: global,
    detail: 1,
    ...init,
  });
  event[STATE].trusted = trusted;
  const allowed = dispatch(target, event);
  if (!allowed) {
    if (restore) restore();
    return false;
  }
  if (!activated || activated.disabled) return true;
  if (activated.localName === 'select') {
    if (trusted && isDropDown(activated)) page.openSelect(activated[ID]);
    return true;
  }
  if (restore) {
    fire(input, Event, 'input', { bubbles: true, composed: true });
    fire(input, Event, 'change', { bubbles: true });
    return true;
  }
  const name = activated.localName;
  const type = activated.type;
  if ((name === 'button' || name === 'input') && (type === 'submit' || type === 'image')) {
    submitForm(activated.form, activated);
  } else if ((name === 'button' || name === 'input') && type === 'reset') {
    if (activated.form) activated.form.reset();
  } else if (name === 'label') {
    const control = activated.control;
    if (control && control !== target && !control.contains(target)) activate(control, init, trusted);
  } else if (name === 'summary') {
    const details = activated.parentElement;
    if (details && details.localName === 'details' && [...details.children].find(child=>child.localName==='summary') === activated) {
      details.open = !details.open;
    }
  } else if (name === 'a' || name === 'area') {
    navigate(activated.href, activated.target);
  }
  return true;
};

// Object URLs name the objects they were made for until revoked; a worker
// started from one runs the script of its blob.
const objectURLs = new Map();
URL.createObjectURL = (object) => {
  const url = `blob:${location.origin}/${crypto.randomUUID()}`;
  objectURLs.set(url, object);
  return url;
};
URL.revokeObjectURL = (url) => {
  objectURLs.delete(String(url));
};

// Location and history. -----------------------------------------------------------

let currentURL = new URL(page.url());
const entries = [{ url: currentURL.href, state: null }];
let entryIndex = 0;

// Scrolls to what a fragment names, as following a link does: the element
// with that id, else an `<a name>`, else the top for an empty fragment or
// `#top`. It runs after `hashchange` handlers, which often show the target.
const scrollToFragment = (hash) => {
  const name = decodeURIComponent(hash.replace(/^#/, ''));
  const target = name && (document.getElementById(name) || [...document.getElementsByTagName('a')].find((anchor) => anchor.getAttribute('name') === name));
  if (target) page.scrollIntoView(target[ID], 'start', 'nearest');
  else if (name === '' || name.toLowerCase() === 'top') document.documentElement.scrollTo(0, 0);
};

const changeHash = (oldURL) => {
  fire(global, PopStateEvent, 'popstate', { state: history.state });
  fire(global, HashChangeEvent, 'hashchange', { oldURL, newURL: currentURL.href });
};

class Location {
  constructor() {
    illegal();
  }

  get href() {
    return currentURL.href;
  }

  set href(value) {
    navigate(String(value));
  }

  get origin() {
    return currentURL.protocol === 'file:' ? 'file://' : currentURL.origin;
  }

  assign(url) {
    navigate(String(url));
  }

  replace(url) {
    const next = new URL(String(url), currentURL.href);
    if (next.href.split('#')[0] === currentURL.href.split('#')[0]) {
      const old = currentURL.href;
      history.replaceState(history.state, '', next.href);
      if (old !== next.href) changeHash(old);
      scrollToFragment(next.hash);
      return;
    }
    navigate(next.href);
  }

  reload() {
    if (typeof host.web.onreload === 'function') host.web.onreload();
  }

  toString() {
    return currentURL.href;
  }
}
for (const part of ['protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash']) {
  Object.defineProperty(Location.prototype, part, {
    get() {
      return currentURL[part];
    },
    set(value) {
      const next = new URL(currentURL.href);
      next[part] = value;
      navigate(next.href);
    },
    configurable: true,
    enumerable: true,
  });
}
const location = Object.create(Location.prototype);

class History {
  constructor() {
    illegal();
  }

  get length() {
    return entries.length;
  }

  get state() {
    return entries[entryIndex].state;
  }

  get scrollRestoration() {
    return 'auto';
  }

  set scrollRestoration(value) {}

  pushState(state, title, url) {
    const next = url === undefined || url === null ? currentURL : new URL(String(url), currentURL.href);
    entries.splice(entryIndex + 1, entries.length, { url: next.href, state: structuredClone(state) });
    entryIndex = entries.length - 1;
    currentURL = new URL(next.href);
  }

  replaceState(state, title, url) {
    const next = url === undefined || url === null ? currentURL : new URL(String(url), currentURL.href);
    entries[entryIndex] = { url: next.href, state: structuredClone(state) };
    currentURL = new URL(next.href);
  }

  go(delta = 0) {
    const target = entryIndex + Number(delta);
    if (delta === 0 || target < 0 || target >= entries.length) return;
    setTimeout(() => {
      const old = currentURL.href;
      entryIndex = target;
      currentURL = new URL(entries[target].url);
      fire(global, PopStateEvent, 'popstate', { state: entries[target].state });
      if (old.split('#')[1] !== currentURL.href.split('#')[1]) {
        fire(global, HashChangeEvent, 'hashchange', { oldURL: old, newURL: currentURL.href });
      }
    }, 0);
  }

  back() {
    this.go(-1);
  }

  forward() {
    this.go(1);
  }
}
const history = Object.create(History.prototype);

// Navigator, screen and performance. ---------------------------------------------

class Navigator {
  constructor() {
    illegal();
  }
}
let acceptLanguages = null;
const clipboardDenied = () => Promise.reject(new DOMException('The clipboard is not available.', 'NotAllowedError'));
Object.defineProperties(Navigator.prototype, {
  language: { get: () => languageTag(), configurable: true, enumerable: true },
  languages: {
    // As Electron lists them: the application's locale, then the user's
    // other preferred languages; one frozen array, as Chromium keeps.
    get() {
      if (!acceptLanguages) {
        const tag = languageTag();
        const base = tag.split('-')[0];
        const listed = host.languages ? Array.from(host.languages(), String) : [];
        acceptLanguages = Object.freeze(listed.length ? listed : base === tag ? [tag] : [tag, base]);
      }
      return acceptLanguages;
    },
    configurable: true,
    enumerable: true,
  },
});
const navigator = Object.create(Navigator.prototype);
Object.assign(navigator, {
  userAgent: host.userAgent(),
  appName: 'Netscape',
  appVersion: host.userAgent().replace(/^Mozilla\//, ''),
  appCodeName: 'Mozilla',
  product: 'Gecko',
  vendor: 'Google Inc.',
  platform: 'Win32',
  onLine: true,
  cookieEnabled: true,
  hardwareConcurrency: host.processors(),
  maxTouchPoints: 0,
  doNotTrack: null,
  webdriver: false,
  pdfViewerEnabled: false,
  userAgentData: {
    // Chromium's brands, its grease brand first, as Electron reports them.
    brands: [{ brand: 'Not/A)Brand', version: '99' }, { brand: 'Chromium', version: /Chrome\/(\d+)/.exec(host.userAgent())[1] }],
    mobile: false,
    platform: 'Windows',
  },
  mediaDevices: undefined,
  clipboard: {
    writeText: (text) => (page.writeClipboard(String(text)) ? Promise.resolve() : clipboardDenied()),
    readText: () => {
      const text = page.readClipboard();
      return text === null ? clipboardDenied() : Promise.resolve(text);
    },
  },
  sendBeacon: () => false,
  vibrate: () => false,
  javaEnabled: () => false,
});

const viewport = () => page.viewport();
const screen = {
  get width() {
    return viewport()[0];
  },
  get height() {
    return viewport()[1];
  },
  get availWidth() {
    return viewport()[0];
  },
  get availHeight() {
    return viewport()[1];
  },
  colorDepth: 24,
  pixelDepth: 24,
  orientation: { type: 'landscape-primary', angle: 0, addEventListener() {}, removeEventListener() {} },
};

const timeOrigin = Date.now() - (host.now() - started);
const marks = [];
const performance = {
  timeOrigin,
  now: () => host.now() - started,
  mark(name, options = {}) {
    const entry = { name, entryType: 'mark', startTime: options.startTime ?? performance.now(), duration: 0, detail: options.detail ?? null };
    marks.push(entry);
    observeEntry(entry);
    return entry;
  },
  measure(name, start, end) {
    const time = (mark) => (typeof mark === 'string' ? (marks.find((entry) => entry.name === mark) || { startTime: 0 }).startTime : performance.now());
    const startTime = start === undefined ? 0 : time(start);
    const entry = { name, entryType: 'measure', startTime, duration: (end === undefined ? performance.now() : time(end)) - startTime, detail: null };
    marks.push(entry);
    observeEntry(entry);
    return entry;
  },
  getEntries: () => marks.slice(),
  getEntriesByName: (name, type) => marks.filter((entry) => entry.name === name && (!type || entry.entryType === type)),
  getEntriesByType: (type) => marks.filter((entry) => entry.entryType === type),
  clearMarks(name) {
    const kept = marks.filter((entry) => entry.entryType !== 'mark' || (name !== undefined && entry.name !== name));
    marks.splice(0, marks.length, ...kept);
  },
  clearMeasures(name) {
    const kept = marks.filter((entry) => entry.entryType !== 'measure' || (name !== undefined && entry.name !== name));
    marks.splice(0, marks.length, ...kept);
  },
  toJSON: () => ({ timeOrigin }),
};

// Performance observers get the marks and measures the page makes, in a
// task after them, as Chromium delivers them. Other entry types, such as
// long tasks, are never reported here.
const performanceObservers = new Set();
const supportedEntryTypes = Object.freeze(['mark', 'measure']);
class PerformanceObserverEntryList {
  constructor(entries) {
    hidden(this, '_entries', entries);
  }

  getEntries() {
    return this._entries.slice();
  }

  getEntriesByType(type) {
    return this._entries.filter((entry) => entry.entryType === type);
  }

  getEntriesByName(name, type) {
    return this._entries.filter((entry) => entry.name === name && (type === undefined || entry.entryType === type));
  }
}
class PerformanceObserver {
  constructor(callback) {
    if (typeof callback !== 'function') {
      throw new TypeError("Failed to construct 'PerformanceObserver': The callback provided as parameter 1 is not a function.");
    }
    hidden(this, '_callback', callback);
    hidden(this, '_types', new Set());
    hidden(this, '_buffer', []);
    hidden(this, '_scheduled', false);
  }

  static get supportedEntryTypes() {
    return supportedEntryTypes;
  }

  observe(options = {}) {
    const types = options.entryTypes !== undefined ? [...options.entryTypes] : options.type !== undefined ? [options.type] : [];
    if (types.length === 0) {
      throw new TypeError("Failed to execute 'observe' on 'PerformanceObserver': An observe() call must include either entryTypes or type arguments.");
    }
    for (const type of types) this._types.add(String(type));
    performanceObservers.add(this);
    if (options.buffered && options.type !== undefined) {
      for (const entry of marks) if (entry.entryType === options.type) this._queue(entry);
    }
  }

  disconnect() {
    performanceObservers.delete(this);
    this._types.clear();
    this._buffer.length = 0;
  }

  takeRecords() {
    return this._buffer.splice(0, this._buffer.length);
  }

  _queue(entry) {
    this._buffer.push(entry);
    if (this._scheduled) return;
    this._scheduled = true;
    setTimeout(() => {
      this._scheduled = false;
      const entries = this.takeRecords();
      if (entries.length === 0) return;
      try {
        this._callback.call(this, new PerformanceObserverEntryList(entries), this);
      } catch (error) {
        host.report(error, false);
      }
    }, 0);
  }
}
const observeEntry = (entry) => {
  for (const observer of performanceObservers) {
    if (observer._types.has(entry.entryType)) observer._queue(entry);
  }
};

// Storage. ---------------------------------------------------------------------

const ITEMS = Symbol('nativeboat.items');
class Storage {
  constructor() {
    illegal();
  }

  get length() {
    return this[ITEMS].map.size;
  }

  key(index) {
    return [...this[ITEMS].map.keys()][index] ?? null;
  }

  getItem(key) {
    return this[ITEMS].map.get(String(key)) ?? null;
  }

  setItem(key, value) {
    this[ITEMS].map.set(String(key), String(value));
    this[ITEMS].save();
  }

  removeItem(key) {
    this[ITEMS].map.delete(String(key));
    this[ITEMS].save();
  }

  clear() {
    this[ITEMS].map.clear();
    this[ITEMS].save();
  }
}
const storage = (entries, save) => {
  const target = Object.create(Storage.prototype);
  hidden(target, ITEMS, { map: new Map(entries), save: () => save([...target[ITEMS].map]) });
  return new Proxy(target, {
    get(object, key, receiver) {
      if (typeof key === 'string' && !(key in object)) return object.getItem(key) ?? undefined;
      return Reflect.get(object, key, receiver);
    },
    set(object, key, value) {
      if (typeof key === 'string' && !(key in object)) object.setItem(key, value);
      else Reflect.set(object, key, value);
      return true;
    },
    deleteProperty(object, key) {
      if (typeof key === 'string') object.removeItem(key);
      return true;
    },
    has(object, key) {
      return (typeof key === 'string' && object[ITEMS].map.has(key)) || Reflect.has(object, key);
    },
    ownKeys(object) {
      return [...object[ITEMS].map.keys()];
    },
    getOwnPropertyDescriptor(object, key) {
      return object[ITEMS].map.has(key) ? { value: object[ITEMS].map.get(key), enumerable: true, configurable: true, writable: true } : undefined;
    },
  });
};
const savedStorage = (() => {
  try {
    const saved = JSON.parse(page.loadStorage() || '[]');
    return Array.isArray(saved) ? saved : [];
  } catch (error) {
    return [];
  }
})();
const localStorage = storage(savedStorage, (items) => page.saveStorage(JSON.stringify(items)));
const sessionStorage = storage([], () => {});

// Animation frames and idle callbacks. --------------------------------------------

const frames = new Map();
let nextFrame = 1;
const requestAnimationFrame = (callback) => {
  if (typeof callback !== 'function') throw new TypeError('The callback must be a function');
  frames.set(nextFrame, callback);
  return nextFrame++;
};
const cancelAnimationFrame = (handle) => {
  frames.delete(handle);
};
const requestIdleCallback = (callback, options = {}) => {
  const start = performance.now();
  return setTimeout(() => {
    callback({ didTimeout: false, timeRemaining: () => Math.max(0, 50 - (performance.now() - start)) });
  }, Math.min(1, Number(options.timeout) || 1));
};
const cancelIdleCallback = (handle) => clearTimeout(handle);

// Media queries and computed styles. --------------------------------------------

const mediaLists = new Set();
class MediaQueryList extends EventTarget {
  constructor() {
    illegal();
  }

  get matches() {
    return page.matchMedia(this.media);
  }

  addListener(listener) {
    this.addEventListener('change', listener);
  }

  removeListener(listener) {
    this.removeEventListener('change', listener);
  }
}
defineHandlers(MediaQueryList.prototype, ['change']);
const matchMedia = (query) => {
  const list = Object.create(MediaQueryList.prototype);
  adopt(native.createTarget(), list);
  hidden(list, 'media', String(query));
  hidden(list, '_last', list.matches);
  mediaLists.add(list);
  return list;
};
const getComputedStyle = (element) => computedStyle(element);

// Observers. ---------------------------------------------------------------------

const observers = new Map();
let deliveryQueued = false;
const deliverMutations = () => {
  deliveryQueued = false;
  for (const id of native.pendingObservers()) {
    // Custom element reactions take their records as operations return.
    if (id === connectionObserver) {
      runCustomReactions();
      continue;
    }
    const observer = observers.get(id);
    const records = observer.takeRecords();
    if (records.length === 0) continue;
    try {
      observer._callback.call(observer, records, observer);
    } catch (error) {
      host.report(error, false);
    }
  }
};
const queueMutations = () => {
  if (deliveryQueued || observers.size === 0) return;
  deliveryQueued = true;
  Promise.resolve().then(deliverMutations);
};
// Elements whose scroll offsets changed get `scroll` events: the document
// for the root's, bubbling to the window.
const fireScrolls = () => {
  const ids = native.takeScrolled();
  if (ids === null) return;
  const root = native.documentElement();
  for (const id of ids) {
    if (id === root) fire(document, Event, 'scroll', { bubbles: true });
    else if (wrap(id)) fire(wrap(id), Event, 'scroll');
  }
  scheduleIntersectionCheck();
  movePointerInPlace();
};
// The pointer's last position over the page. Content that scrolls under a
// still pointer moves the hover to what is now there, with the boundary
// events Chromium sends then but no move, unless a button is held.
let lastPointer = null;
const movePointerInPlace = () => {
  if (!lastPointer || lastPointer.buttons) return;
  const id = page.elementFromPoint(lastPointer.clientX, lastPointer.clientY);
  updateHover(id == null ? null : targetOf(id), { view: global, ...lastPointer, movementX: 0, movementY: 0 });
};
const detailsToggles = new Map();
const queueDetailsToggles = () => {
  const changes = native.takeDetailsToggles();
  if (changes === null) return;
  for (const [id, old, next] of changes) {
    const pending = detailsToggles.get(id);
    if (pending) clearTimeout(pending.timer);
    const oldState = pending ? pending.oldState : old ? 'open' : 'closed';
    const newState = next ? 'open' : 'closed';
    const record = { timer: null, oldState };
    record.timer = setTimeout(() => {
      // The host may already have collected due timers before another
      // element's listener reschedules this notification.
      if (detailsToggles.get(id) !== record) return;
      // Clear before dispatch, as Chromium does, so a listener's next
      // change starts a fresh notification rather than retaining oldState.
      detailsToggles.delete(id);
      fire(wrap(id), ToggleEvent, 'toggle', { oldState, newState });
    }, 0);
    detailsToggles.set(id, record);
  }
};
const resizeObservers = new Set();
let resizeCheckQueued = false;
const observingResizes = () => {
  for (const observer of resizeObservers) if (observer._targets.size) return true;
  return false;
};
for (const operation of [
  'insertBefore', 'removeChild', 'replaceChild', 'remove', 'setText', 'setData', 'setAttribute',
  'removeAttribute', 'toggleAttribute', 'styleSet', 'styleRemove', 'setInnerHTML', 'clone',
]) {
  const original = native[operation];
  native[operation] = (...args) => {
    const attribute = customDefinitions.size ? customElementsBefore(operation, args) : null;
    const result = original(...args);
    if (customDefinitions.size) customElementsAfter(operation, result, attribute);
    queueMutations();
    queueDetailsToggles();
    // A change can resize observed elements: the next frame checks them,
    // as Chromium's rendering steps gather resize observations every frame.
    if (!resizeCheckQueued && observingResizes()) scheduleResizeCheck();
    // It can move them too, for intersection observers.
    scheduleIntersectionCheck();
    return result;
  };
}

class MutationObserver {
  constructor(callback) {
    if (typeof callback !== 'function') throw new TypeError('The callback must be a function');
    hidden(this, '_callback', callback);
    hidden(this, '_id', native.createObserver());
    observers.set(this._id, this);
  }

  observe(target, options = {}) {
    const attributes = options.attributes ?? (options.attributeOldValue !== undefined || options.attributeFilter !== undefined);
    const characterData = options.characterData ?? options.characterDataOldValue !== undefined;
    if (!options.childList && !attributes && !characterData) {
      throw new TypeError("The options must set at least one of 'childList', 'attributes' or 'characterData'");
    }
    native.observe(
      this._id,
      idOf(target),
      !!options.childList,
      !!attributes,
      !!characterData,
      !!options.subtree,
      !!options.attributeOldValue,
      !!options.characterDataOldValue,
      options.attributeFilter ? [...options.attributeFilter].map(String) : null,
    );
  }

  disconnect() {
    native.disconnect(this._id);
  }

  takeRecords() {
    return native.takeRecords(this._id).map(([type, target, added, removed, previous, next, name, oldValue]) => ({
      type,
      target: wrap(target),
      addedNodes: staticNodeList(wrapAll(added)),
      removedNodes: staticNodeList(wrapAll(removed)),
      previousSibling: wrap(previous),
      nextSibling: wrap(next),
      attributeName: name,
      attributeNamespace: null,
      oldValue,
    }));
  }
}

class ResizeObserver {
  constructor(callback) {
    hidden(this, '_callback', callback);
    hidden(this, '_targets', new Map());
    resizeObservers.add(this);
  }

  observe(target) {
    this._targets.set(target, null);
    scheduleResizeCheck();
  }

  unobserve(target) {
    this._targets.delete(target);
  }

  disconnect() {
    this._targets.clear();
  }
}
const scheduleResizeCheck = () => {
  if (resizeCheckQueued) return;
  resizeCheckQueued = true;
  requestAnimationFrame(checkResizes);
};
const checkResizes = () => {
  resizeCheckQueued = false;
  for (const observer of resizeObservers) {
    const entries = [];
    for (const [target, last] of observer._targets) {
      const rect = target.getBoundingClientRect();
      const size = `${rect.width}x${rect.height}`;
      if (size === last) continue;
      observer._targets.set(target, size);
      const box = { inlineSize: rect.width, blockSize: rect.height };
      entries.push({
        target,
        contentRect: new DOMRectReadOnly(0, 0, rect.width, rect.height),
        borderBoxSize: [box],
        contentBoxSize: [box],
        devicePixelContentBoxSize: [box],
      });
    }
    if (entries.length) {
      try {
        observer._callback.call(observer, entries, observer);
      } catch (error) {
        host.report(error, false);
      }
    }
  }
};

class IntersectionObserverEntry {
  constructor(init = {}) {
    Object.assign(this, {
      time: init.time ?? 0,
      rootBounds: init.rootBounds ?? null,
      boundingClientRect: init.boundingClientRect,
      intersectionRect: init.intersectionRect,
      isIntersecting: !!init.isIntersecting,
      isVisible: !!init.isVisible,
      intersectionRatio: init.intersectionRatio ?? 0,
      target: init.target,
    });
  }
}

// Root margins as Chromium serializes them: four lengths, top first.
const rootMargins = (text) => {
  const parts = String(text).trim().split(/\s+/).filter(Boolean);
  if (parts.length > 4) throw new DOMException('Extra text found at the end of rootMargin.', 'SyntaxError');
  const margins = (parts.length ? parts : ['0px']).map((part) => {
    const match = /^([+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?)(px|%)?$/i.exec(part);
    if (!match || (!match[2] && +match[1] !== 0)) {
      throw new DOMException('rootMargin must be specified in pixels or percent.', 'SyntaxError');
    }
    return { value: +match[1], percent: match[2] === '%' };
  });
  const [top, right = top, bottom = top, left = right] = margins;
  return [top, right, bottom, left];
};
const marginText = ({ value, percent }) => `${value}${percent ? '%' : 'px'}`;

const intersectionObservers = new Set();
let intersectionCheckQueued = false;
// As in Chromium, intersections are computed once the frame has rendered,
// in a task after it: layout is then fresh and reused, where a check before
// the frame's layout would lay the page out once more.
const scheduleIntersectionCheck = () => {
  if (intersectionCheckQueued || intersectionObservers.size === 0) return;
  intersectionCheckQueued = true;
  requestAnimationFrame(() => setTimeout(checkIntersections, 0));
};

class IntersectionObserver {
  constructor(callback, options = {}) {
    if (typeof callback !== 'function') throw new TypeError('IntersectionObserver needs a callback.');
    const root = options.root ?? null;
    if (root !== null && !(isNode(root) && (root.nodeType === 1 || root.nodeType === 9))) {
      throw new TypeError('The root must be an Element or a Document.');
    }
    const thresholds = [].concat(options.threshold ?? 0).map(Number);
    if (thresholds.some((value) => !(value >= 0 && value <= 1))) {
      throw new RangeError('Threshold values must be numbers between 0 and 1.');
    }
    thresholds.sort((a, b) => a - b);
    const margins = rootMargins(options.rootMargin ?? '0px');
    hidden(this, '_callback', callback);
    hidden(this, '_margins', margins);
    // Targets with the threshold index and intersecting state last reported.
    hidden(this, '_targets', new Map());
    hidden(this, '_records', []);
    Object.assign(this, {
      root,
      rootMargin: margins.map(marginText).join(' '),
      thresholds: thresholds.length ? thresholds : [0],
    });
  }

  observe(target) {
    if (!isNode(target) || target.nodeType !== 1) throw new TypeError('The target must be an Element.');
    if (this._targets.has(target)) return;
    this._targets.set(target, { index: -1, intersecting: false });
    intersectionObservers.add(this);
    scheduleIntersectionCheck();
  }

  unobserve(target) {
    this._targets.delete(target);
    if (this._targets.size === 0) intersectionObservers.delete(this);
  }

  disconnect() {
    this._targets.clear();
    intersectionObservers.delete(this);
  }

  takeRecords() {
    return this._records.splice(0);
  }
}

const clips = (element) =>
  styleValue(element, 'overflow-x') !== 'visible' || styleValue(element, 'overflow-y') !== 'visible';
// The padding box of a clipping element, in viewport coordinates.
const paddingBox = (element) => {
  const rect = element.getBoundingClientRect();
  const left = rect.left + element.clientLeft;
  const top = rect.top + element.clientTop;
  return { left, top, right: left + element.clientWidth, bottom: top + element.clientHeight };
};
const intersect = (a, b) => ({
  left: Math.max(a.left, b.left),
  top: Math.max(a.top, b.top),
  right: Math.min(a.right, b.right),
  bottom: Math.min(a.bottom, b.bottom),
});
// Rectangles that overlap or only touch intersect, as in the specification.
const meets = (rect) => rect.right >= rect.left && rect.bottom >= rect.top;
const domRect = (rect) => new DOMRectReadOnly(rect.left, rect.top, Math.max(0, rect.right - rect.left), Math.max(0, rect.bottom - rect.top));

// The root intersection rectangle: the viewport for the document, an
// element's padding box when it clips its content, else its border box;
// grown or shrunk by the root margins.
const rootBounds = (observer) => {
  const root = observer.root;
  let rect;
  if (root === null || root.nodeType === 9) {
    const element = document.documentElement;
    rect = { left: 0, top: 0, right: element ? element.clientWidth : innerWidth, bottom: element ? element.clientHeight : innerHeight };
  } else if (clips(root)) rect = paddingBox(root);
  else {
    const { left, top, right, bottom } = root.getBoundingClientRect();
    rect = { left, top, right, bottom };
  }
  const width = rect.right - rect.left;
  const height = rect.bottom - rect.top;
  const [top, right, bottom, left] = observer._margins.map((margin, side) =>
    margin.percent ? (margin.value * (side % 2 ? width : height)) / 100 : margin.value);
  return { left: rect.left - left, top: rect.top - top, right: rect.right + right, bottom: rect.bottom + bottom };
};

// The part of a target that its containing blocks' clips and the root
// leave, or null when the root is not one of those blocks.
const clippedRect = (target, root, bounds) => {
  const { left, top, right, bottom } = target.getBoundingClientRect();
  let rect = { left, top, right, bottom };
  const html = document.documentElement;
  const viewportRoot = root === null || root.nodeType === 9;
  let node = target;
  for (;;) {
    const position = styleValue(node, 'position');
    const block = position === 'fixed' ? null : position === 'absolute' ? node.offsetParent : node.parentElement;
    if (block !== null && block === root) break;
    // The document's own clips are the viewport's.
    if (block === null || block === html || block === document.body) {
      if (!viewportRoot) return null;
      break;
    }
    if (clips(block)) rect = intersect(rect, paddingBox(block));
    node = block;
  }
  return intersect(rect, bounds);
};

// The update intersection observations steps, run in a frame after
// layout, scrolling or the observed elements may have changed.
const checkIntersections = () => {
  intersectionCheckQueued = false;
  const time = performance.now();
  const due = [];
  for (const observer of [...intersectionObservers]) {
    const bounds = rootBounds(observer);
    const sameDocument = observer.root === null || observer.root.isConnected;
    for (const [target, last] of observer._targets) {
      let targetRect = { left: 0, top: 0, right: 0, bottom: 0 };
      let shown = targetRect;
      let intersecting = false;
      if (sameDocument && target.isConnected && target.getClientRects().length) {
        const box = target.getBoundingClientRect();
        targetRect = { left: box.left, top: box.top, right: box.right, bottom: box.bottom };
        const clipped = clippedRect(target, observer.root, bounds);
        if (clipped && meets(clipped)) {
          intersecting = true;
          shown = clipped;
        }
      }
      const area = (targetRect.right - targetRect.left) * (targetRect.bottom - targetRect.top);
      const shownArea = intersecting ? (shown.right - shown.left) * (shown.bottom - shown.top) : 0;
      const ratio = area > 0 ? shownArea / area : intersecting ? 1 : 0;
      let index = 0;
      if (intersecting) {
        index = observer.thresholds.findIndex((threshold) => threshold > ratio);
        if (index < 0) index = observer.thresholds.length;
      }
      if (index === last.index && intersecting === last.intersecting) continue;
      observer._targets.set(target, { index, intersecting });
      observer._records.push(new IntersectionObserverEntry({
        time,
        rootBounds: domRect(bounds),
        boundingClientRect: domRect(targetRect),
        intersectionRect: domRect(intersecting ? shown : { left: 0, top: 0, right: 0, bottom: 0 }),
        isIntersecting: intersecting,
        intersectionRatio: ratio,
        target,
      }));
    }
    if (observer._records.length) due.push(observer);
  }
  for (const observer of due) {
    const records = observer.takeRecords();
    if (!records.length) continue;
    try {
      observer._callback.call(observer, records, observer);
    } catch (error) {
      host.report(error, false);
    }
  }
};

// The prelude provides `crypto` from the system random generator.
const { crypto } = globalThis;

// Message ports. The two ports of a channel made here talk directly. A port
// whose peer lives in another realm of the process, the main process or the
// window's preload, holds the runtime's handle for its end, which clones
// what it posts into that realm. A port's messages wait until it starts, as
// `start()` or setting `onmessage` starts it.
const PORT = Symbol('nativeboat.port');
const makePort = () => {
  const port = adopt(native.createTarget(), Object.create(MessagePort.prototype));
  hidden(port, PORT, { other: null, handle: null, queue: [], started: false, closed: false, draining: false });
  return port;
};
const portState = (port) => {
  if (port === null || typeof port !== 'object' || !port[PORT]) throw new TypeError('Illegal invocation');
  return port[PORT];
};
const dataCloneError = (message) => new DOMException(message, 'DataCloneError');
// Dispatches a port's queued messages, now or in a task.
const drainPort = (port, later) => {
  const state = port[PORT];
  if (!state.started || state.closed || state.queue.length === 0) return;
  if (later) {
    if (state.draining) return;
    state.draining = true;
    setTimeout(() => {
      state.draining = false;
      drainPort(port, false);
    }, 0);
    return;
  }
  while (state.started && !state.closed && state.queue.length > 0) {
    const { data, ports } = state.queue.shift();
    fire(port, MessageEvent, 'message', { data, ports: Object.freeze(ports) });
  }
};
const portClosed = (port) => {
  const state = port[PORT];
  if (state.closed) return;
  state.other = null;
  state.handle = null;
  setTimeout(() => fire(port, Event, 'close'), 0);
};
// What the runtime delivers to a port from its end in another realm, in a
// task of its own.
const portReceiver = (port) => ({
  _deliver(data, ports) {
    const state = port[PORT];
    if (state.closed) return;
    state.queue.push({ data, ports });
    drainPort(port, false);
  },
  _peerClosed() {
    portClosed(port);
  },
});
// A port for an end that moved here from another realm.
const nativePort = (handle) => {
  const port = makePort();
  port[PORT].handle = handle;
  handle.bind(portReceiver(port));
  return port;
};
// The handle of a port that moves to another realm. A port whose peer is
// in this page leaves the peer talking to a new channel's near end.
const exportPort = (port) => {
  const state = port[PORT];
  let { handle } = state;
  if (!handle) {
    const [near, far] = host.messageChannel();
    const { other } = state;
    if (other) {
      other[PORT].other = null;
      other[PORT].handle = near;
      near.bind(portReceiver(other));
    } else {
      near.close();
    }
    handle = far;
  }
  state.handle = null;
  state.other = null;
  state.closed = true;
  state.queue.length = 0;
  return handle;
};
const transferList = (transfer, sender) => {
  if (transfer === undefined || transfer === null) return [];
  const list = Array.isArray(transfer) ? transfer : transfer.transfer;
  if (list === undefined || list === null) return [];
  const ports = [...list];
  ports.forEach((port, index) => {
    if (port === null || typeof port !== 'object' || !port[PORT]) {
      throw dataCloneError(`Value at index ${index} does not have a transferable type.`);
    }
    if (port === sender || ports.indexOf(port) !== index || port[PORT].closed) {
      throw dataCloneError(`Message port at index ${index} is a duplicate of an earlier port, closed, or the port posting the message.`);
    }
  });
  return ports;
};

class MessagePort extends EventTarget {
  constructor() {
    illegal();
  }

  postMessage(data, transfer) {
    const state = portState(this);
    const ports = transferList(transfer, this);
    if (state.closed) return;
    if (state.handle) {
      state.handle.postMessage(data, ports.map(exportPort));
      return;
    }
    const message = structuredClone(data);
    const target = state.other;
    if (!target) return;
    setTimeout(() => {
      const targetState = target[PORT];
      if (targetState.closed) return;
      targetState.queue.push({ data: message, ports });
      drainPort(target, false);
    }, 0);
  }

  start() {
    const state = portState(this);
    if (state.started) return;
    state.started = true;
    drainPort(this, true);
  }

  close() {
    const state = portState(this);
    if (state.closed) return;
    const { other, handle } = state;
    state.closed = true;
    state.queue.length = 0;
    state.other = null;
    state.handle = null;
    if (handle) handle.close();
    if (other) portClosed(other);
  }
}
defineHandlers(MessagePort.prototype, ['message', 'messageerror', 'close']);
{
  const onmessage = Object.getOwnPropertyDescriptor(MessagePort.prototype, 'onmessage');
  Object.defineProperty(MessagePort.prototype, 'onmessage', {
    ...onmessage,
    set(value) {
      onmessage.set.call(this, value);
      if (typeof value === 'function') this.start();
    },
  });
}
class MessageChannel {
  constructor() {
    const [port1, port2] = [makePort(), makePort()];
    port1[PORT].other = port2;
    port2[PORT].other = port1;
    Object.assign(this, { port1, port2 });
  }
}

// Dedicated workers. Each runs in a realm of its own on the page's thread,
// started with the ends of two channels: one carries messages, the other
// the errors the worker reports.
const WORKER = Symbol('nativeboat.worker');
class Worker extends EventTarget {
  constructor(url, options = {}) {
    super();
    if (arguments.length === 0) throw new TypeError("Failed to construct 'Worker': 1 argument required, but only 0 present.");
    const href = String(url);
    let script = null;
    let resolved = href;
    if (href.startsWith('blob:')) {
      const blob = objectURLs.get(href);
      if (!(blob instanceof Blob)) {
        throw new DOMException(`Failed to construct 'Worker': Script at '${href}' cannot be accessed from origin '${location.origin}'.`, 'SecurityError');
      }
      script = new TextDecoder().decode(blob._bytes);
    } else {
      resolved = new URL(href, location.href).href;
    }
    const [near, far] = host.messageChannel();
    const [errorsNear, errorsFar] = host.messageChannel();
    const port = nativePort(near);
    const errors = nativePort(errorsNear);
    hidden(this, WORKER, { port, errors, id: 0, terminated: false });
    port.onmessage = (event) => {
      fire(this, MessageEvent, 'message', { data: event.data, ports: event.ports });
    };
    errors.onmessage = (event) => {
      const { message, filename, lineno, colno } = event.data;
      if (fire(this, ErrorEvent, 'error', { cancelable: true, message, filename, lineno, colno }).allowed) {
        host.report(message.replace(/^Uncaught /, ''));
      }
    };
    const name = options && options.name !== undefined ? String(options.name) : '';
    this[WORKER].id = host.startWorker(far, errorsFar, resolved, name, script);
  }

  postMessage(data, transfer) {
    const state = this[WORKER];
    if (!state || state.terminated) return;
    // Buffers in a transfer list are cloned rather than moved; ports move.
    let ports = transfer;
    if (transfer !== undefined && transfer !== null) {
      const list = Array.isArray(transfer) ? transfer : transfer.transfer;
      ports = list === undefined || list === null ? [] : [...list].filter((item) => !(item instanceof ArrayBuffer));
    }
    state.port.postMessage(data, ports);
  }

  terminate() {
    const state = this[WORKER];
    if (!state || state.terminated) return;
    state.terminated = true;
    state.port.close();
    state.errors.close();
    host.terminateWorker(state.id);
  }
}
defineHandlers(Worker.prototype, ['message', 'messageerror', 'error']);

// Service workers. Registrations are the process's, by scope: a page in a
// registration's scope connects to its worker over a port, which carries the
// worker's messages and state, `clients.claim()`, and the requests of the
// pages the worker controls, which it answers with its `fetch` event.
const SERVICE = Symbol('nativeboat.serviceWorker');
const CONSTRUCTING = Symbol('nativeboat.constructing');
let serviceClientId = null;
const serviceConnections = new Map();
let serviceController = null;
let serviceReady = null;
class ServiceWorker extends EventTarget {
  constructor(token, connection) {
    if (token !== CONSTRUCTING) illegal();
    super();
    hidden(this, SERVICE, connection);
  }

  get scriptURL() {
    return this[SERVICE].script;
  }

  get state() {
    return this[SERVICE].state;
  }

  postMessage(data, transfer) {
    const connection = this[SERVICE];
    let ports = transfer;
    if (transfer !== undefined && transfer !== null) {
      const list = Array.isArray(transfer) ? transfer : transfer.transfer;
      ports = list === undefined || list === null ? [] : [...list].filter((item) => !(item instanceof ArrayBuffer));
    }
    connection.port.postMessage({ kind: 'message', data }, ports);
  }
}
defineHandlers(ServiceWorker.prototype, ['statechange', 'error']);
class ServiceWorkerRegistration extends EventTarget {
  constructor(token, connection) {
    if (token !== CONSTRUCTING) illegal();
    super();
    hidden(this, SERVICE, connection);
  }

  get scope() {
    return this[SERVICE].scope;
  }

  get updateViaCache() {
    return 'imports';
  }

  get installing() {
    const { state, worker } = this[SERVICE];
    return state === 'parsed' || state === 'installing' ? worker : null;
  }

  get waiting() {
    return this[SERVICE].state === 'installed' ? this[SERVICE].worker : null;
  }

  get active() {
    const { state, worker } = this[SERVICE];
    return state === 'activating' || state === 'activated' ? worker : null;
  }

  get navigationPreload() {
    return {
      enable: () => Promise.resolve(),
      disable: () => Promise.resolve(),
      setHeaderValue: () => Promise.resolve(),
      getState: () => Promise.resolve({ enabled: false, headerValue: 'true' }),
    };
  }

  update() {
    return Promise.resolve(this);
  }

  unregister() {
    return Promise.resolve(host.unregisterServiceWorker(this[SERVICE].id));
  }
}
defineHandlers(ServiceWorkerRegistration.prototype, ['updatefound']);
class ServiceWorkerContainer extends EventTarget {
  constructor(token) {
    if (token !== CONSTRUCTING) illegal();
    super();
  }

  get controller() {
    return serviceController;
  }

  get ready() {
    if (!serviceReady) {
      serviceReady = new Promise((resolve) => {
        const check = () => {
          for (const connection of serviceConnections.values()) {
            if (connection.state === 'activated' && location.href.startsWith(connection.scope)) {
              resolve(connection.registration);
              return true;
            }
          }
          return false;
        };
        if (!check()) serviceReadyChecks.push(check);
      });
    }
    return serviceReady;
  }

  register(scriptURL, options = {}) {
    let script;
    let scope;
    try {
      script = new URL(String(scriptURL), document.baseURI);
      scope = new URL(options && options.scope !== undefined ? String(options.scope) : './', script).href;
    } catch (error) {
      return Promise.reject(new TypeError(`Failed to register a ServiceWorker: ${error.message}`));
    }
    host.registerServiceWorker(script.href, scope);
    const found = host.serviceWorkerFor(scope);
    const connection = serviceConnection(found, false);
    return Promise.resolve(connection.registration);
  }

  getRegistration(clientURL) {
    const found = host.serviceWorkerFor(new URL(clientURL === undefined ? '' : String(clientURL), location.href).href);
    return Promise.resolve(found ? serviceConnection(found, false).registration : undefined);
  }

  getRegistrations() {
    const found = host.serviceWorkerFor(location.href);
    return Promise.resolve(found ? [serviceConnection(found, false).registration] : []);
  }

  startMessages() {}
}
defineHandlers(ServiceWorkerContainer.prototype, ['controllerchange', 'message', 'messageerror']);
const serviceReadyChecks = [];
const serviceContainer = new ServiceWorkerContainer(CONSTRUCTING);

// The connection to a registration's worker, made once: `found` is its id,
// scope, script and state; `controlled` whether this page loaded with the
// worker active, so that it controls the page from the start.
const serviceConnection = (found, controlled) => {
  const [id, scope, script, state] = found;
  const known = serviceConnections.get(id);
  if (known) return known;
  serviceClientId ??= crypto.randomUUID();
  const port = nativePort(host.connectServiceWorker(id, serviceClientId, location.href, 'window'));
  const connection = { id, scope, script, state, port, requests: new Map(), nextRequest: 0 };
  connection.worker = new ServiceWorker(CONSTRUCTING, connection);
  connection.registration = new ServiceWorkerRegistration(CONSTRUCTING, connection);
  serviceConnections.set(id, connection);
  const control = () => {
    if (serviceController === connection.worker) return;
    serviceController = connection.worker;
    fire(serviceContainer, Event, 'controllerchange');
  };
  port.onmessage = (event) => {
    const message = event.data;
    switch (message && message.kind) {
      case 'state':
        if (connection.state === message.state) return;
        connection.state = message.state;
        fire(connection.worker, Event, 'statechange');
        if (message.state === 'activated') {
          for (const check of serviceReadyChecks.splice(0)) if (!check()) serviceReadyChecks.push(check);
        }
        return;
      case 'claim':
        control();
        return;
      case 'message':
        fire(serviceContainer, MessageEvent, 'message', {
          data: message.data,
          ports: event.ports,
          source: connection.worker,
          origin: new URL(connection.scope).origin,
        });
        return;
      case 'response': {
        const pending = connection.requests.get(message.id);
        if (!pending) return;
        connection.requests.delete(message.id);
        if (message.fallback) pending.resolve(null);
        else if (message.error !== undefined) pending.reject(new TypeError(message.error));
        else {
          const nullBody = [101, 103, 204, 205, 304].includes(message.status);
          pending.resolve(new Response(nullBody ? null : message.body, {
            status: message.status,
            statusText: message.statusText,
            headers: message.headers,
          }));
        }
        return;
      }
    }
  };
  port.postMessage({ kind: 'hello', controlled });
  if (controlled) serviceController = connection.worker;
  return connection;
};

// A page that loads in an active registration's scope is controlled by its
// worker from the start.
if (typeof host.serviceWorkerFor === 'function') {
  const found = host.serviceWorkerFor(location.href);
  if (found && found[3] === 'activated') serviceConnection(found, true);
}

// The requests of a page a worker controls go to the worker first: its
// response, or null for the network.
const serviceWorkerFetch = (request) => {
  const controller = serviceController;
  if (!controller) return Promise.resolve(null);
  const connection = controller[SERVICE];
  const id = ++connection.nextRequest;
  return new Promise((resolve, reject) => {
    connection.requests.set(id, { resolve, reject });
    connection.port.postMessage({
      kind: 'fetch',
      id,
      url: request.url,
      method: request.method,
      headers: [...request.headers],
      destination: request.destination ?? '',
      mode: request.mode ?? 'cors',
    });
  });
};
hidden(globalThis, '__nativeboatServiceWorkerFetch', serviceWorkerFetch);
navigator.serviceWorker = serviceContainer;

const PARTS = Symbol('nativeboat.form');

class FormData {
  constructor(form) {
    hidden(this, PARTS, []);
    if (!form) return;
    for (const control of form.elements) {
      const name = control.getAttribute('name');
      if (!name || control.disabled) continue;
      const type = control.type;
      if ((type === 'checkbox' || type === 'radio') && !control.checked) continue;
      if (['submit', 'button', 'reset', 'image', 'file'].includes(type) || control.localName === 'fieldset') continue;
      if (control.localName === 'select') {
        for (const option of control.selectedOptions) this.append(name, option.value);
      } else this.append(name, control.value);
    }
  }

  append(name, value) {
    this[PARTS].push([String(name), value instanceof Blob ? value : String(value)]);
  }

  delete(name) {
    const kept = this[PARTS].filter(([key]) => key !== String(name));
    this[PARTS].splice(0, this[PARTS].length, ...kept);
  }

  get(name) {
    const pair = this[PARTS].find(([key]) => key === String(name));
    return pair ? pair[1] : null;
  }

  getAll(name) {
    return this[PARTS].filter(([key]) => key === String(name)).map(([, value]) => value);
  }

  has(name) {
    return this[PARTS].some(([key]) => key === String(name));
  }

  set(name, value) {
    this.delete(name);
    this.append(name, value);
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
}

class Blob {
  constructor(parts = [], options = {}) {
    const encoder = new TextEncoder();
    const chunks = [...parts].map((part) => {
      if (part instanceof Blob) return part._bytes;
      if (part instanceof ArrayBuffer) return new Uint8Array(part);
      if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
      return encoder.encode(String(part));
    });
    const bytes = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0));
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
    hidden(this, '_bytes', bytes);
    hidden(this, '_type', String(options.type || '').toLowerCase());
  }

  get size() {
    return this._bytes.length;
  }

  get type() {
    return this._type;
  }

  slice(start = 0, end = this.size, type = '') {
    return new Blob([this._bytes.slice(start, end)], { type });
  }

  text() {
    return Promise.resolve(new TextDecoder().decode(this._bytes));
  }

  arrayBuffer() {
    return Promise.resolve(this._bytes.slice().buffer);
  }

  bytes() {
    return Promise.resolve(this._bytes.slice());
  }
}

class File extends Blob {
  constructor(parts, name, options = {}) {
    super(parts, options);
    hidden(this, '_name', String(name));
    hidden(this, '_lastModified', options.lastModified ?? Date.now());
  }

  get name() {
    return this._name;
  }

  get lastModified() {
    return this._lastModified;
  }
}

class FileReader extends EventTarget {
  constructor() {
    super();
    Object.assign(this, { result: null, error: null, readyState: 0 });
  }

  read(blob, convert) {
    this.readyState = 1;
    setTimeout(() => {
      this.result = convert(blob._bytes);
      this.readyState = 2;
      fire(this, ProgressEvent, 'load', { loaded: blob.size, total: blob.size, lengthComputable: true });
      fire(this, ProgressEvent, 'loadend', { loaded: blob.size, total: blob.size, lengthComputable: true });
    }, 0);
  }

  readAsText(blob) {
    this.read(blob, (bytes) => new TextDecoder().decode(bytes));
  }

  readAsArrayBuffer(blob) {
    this.read(blob, (bytes) => bytes.slice().buffer);
  }

  readAsDataURL(blob) {
    this.read(blob, (bytes) => `data:${blob.type || 'application/octet-stream'};base64,${btoa(String.fromCharCode(...bytes))}`);
  }

  abort() {}
}
defineHandlers(FileReader.prototype, ['load', 'loadend', 'error', 'abort', 'progress', 'loadstart']);

const CSS = {
  supports: () => true,
  escape: (value) => String(value).replace(/([^\w-])/g, '\\$1').replace(/^(-?\d)/, '\\3$1 '),
};

class FontFace {
  constructor(family, source, descriptors = {}) {
    Object.assign(this, { family, source, status: 'loaded', ...descriptors });
    this.loaded = Promise.resolve(this);
  }

  load() {
    return Promise.resolve(this);
  }
}
const fonts = Object.create(EventTarget.prototype);
adopt(native.createTarget(), fonts);
Object.assign(fonts, {
  status: 'loaded',
  ready: Promise.resolve(fonts),
  check: () => true,
  load: () => Promise.resolve([]),
  add() {},
  delete: () => false,
  clear() {},
  forEach() {},
  has: () => false,
});

// Editing commands. --------------------------------------------------------------

const isTextControl = (element) =>
  !!element && (element.localName === 'textarea' || (element.localName === 'input' && textTypes.includes(element.type)));
const isEditable = (element) => isTextControl(element) && !element.readOnly && !element.disabled;
const selectedText = (element) =>
  isTextControl(element) ? element.value.slice(element.selectionStart, element.selectionEnd) : String(selection);

// Replaces the selected text of a control the way typing does: frameworks
// see an input event for a value they did not set themselves.
const replaceSelection = (target, text, inputType, data = null) => {
  if (userEditing && !fire(target, InputEvent, 'beforeinput', { bubbles: true, composed: true, cancelable: true, inputType, data }).allowed) return;
  const before = historySnapshot(target);
  const { value, selectionStart: start, selectionEnd: end } = target;
  native.setValue(target[ID], value.slice(0, start) + text + value.slice(end));
  edited.add(target);
  target.setSelectionRange(start + text.length, start + text.length);
  fire(target, InputEvent, 'input', { bubbles: true, composed: true, inputType, data });
  rememberEdit(target, before, inputType, data);
};

// Fires a clipboard event at the focused element.
const clipboardEvent = (type, contents = []) => {
  const clipboardData = new DataTransfer();
  for (const [format, data] of contents) clipboardData.setData(format, data);
  if (type === 'paste') transfers.get(clipboardData).mode = 'readonly';
  const target = document.activeElement || document.documentElement;
  const { allowed } = fire(target, ClipboardEvent, type, { bubbles: true, cancelable: true, composed: true, clipboardData });
  return { allowed, clipboardData };
};

// Copies the selection, or what a cancelled copy or cut event provided.
const copySelection = (type) => {
  const target = document.activeElement;
  const { allowed, clipboardData } = clipboardEvent(type);
  try {
    const text = allowed ? selectedText(target) : clipboardData.types.includes('text/plain') ? clipboardData.getData('text/plain') : null;
    let html = allowed ? null : clipboardData.types.includes('text/html') ? clipboardData.getData('text/html') : null;
    if (allowed && !isTextControl(target) && selectedRange && !selectedRange.collapsed) {
      const container = document.createElement('div');
      container.appendChild(selectedRange.cloneContents());
      html = container.innerHTML;
    }
    if (allowed ? text === '' && !html : text === null && html === null) return true;
    if (!page.writeClipboardContents(text, html)) return false;
    if (allowed && type === 'cut') {
      if (isEditable(target)) replaceSelection(target, '', 'deleteByCut');
      else hostCommand('deleteByCut', null, clearPlan);
    }
    return true;
  } finally {
    transfers.get(clipboardData).mode = 'disconnected';
  }
};

// Deletes the selection, or the character before or after the caret.
const deleteText = (inputType, step) => {
  const target = document.activeElement;
  if (!isEditable(target)) return false;
  const { selectionStart: start, selectionEnd: end } = target;
  if (start === end) {
    const [from, to] = step < 0 ? [Math.max(0, start - 1), start] : [start, Math.min(target.value.length, start + 1)];
    if (from === to) return true;
    target.setSelectionRange(from, to);
  }
  replaceSelection(target, '', inputType);
  return true;
};

const editingCommands = {
  undo: () => historyCommand(false),
  redo: () => historyCommand(true),
  copy: () => copySelection('copy'),
  cut: () => copySelection('cut'),
  paste() {
    const contents = page.readClipboardContents();
    if (contents === null) return false;
    const target = document.activeElement;
    const { allowed, clipboardData } = clipboardEvent('paste', contents);
    try {
      if (!allowed) return true;
      const text = clipboardData.getData('text/plain');
      if (isEditable(target)) replaceSelection(target, text, 'insertFromPaste');
      else hostCommand('insertFromPaste', null, insertPlan(text), clipboardData);
      return true;
    } finally {
      transfers.get(clipboardData).mode = 'disconnected';
    }
  },
  selectall() {
    const target = document.activeElement, body = document.body;
    if (isTextControl(target)) target.select();
    else if (!selectHostContents() && body) selectEndpoints([body, 0], [body, body.childNodes.length]);
    return true;
  },
  inserttext(value = '') {
    const target = document.activeElement, text = String(value);
    if (!isEditable(target)) return hostCommand('insertText', text, insertPlan(text));
    replaceSelection(target, text, 'insertText', text);
    return true;
  },
  delete: () =>
    deleteText('deleteContentBackward', -1) || hostCommand('deleteContentBackward', null, deletePlan(false, false)),
  forwarddelete: () =>
    deleteText('deleteContentForward', 1) || hostCommand('deleteContentForward', null, deletePlan(true, false)),
  insertparagraph: () => hostCommand('insertParagraph', null, breakPlan(true)),
  insertlinebreak: () => hostCommand('insertLineBreak', null, breakPlan(false)),
};
// Keys that run edit commands: the clipboard's and select all.
const commandKey = (init) => {
  if (init.altKey || init.metaKey || !init.key) return null;
  const key = init.key.length === 1 ? init.key.toLowerCase() : init.key;
  if (init.ctrlKey && init.shiftKey && key === 'z') return 'redo';
  if (init.ctrlKey && !init.shiftKey) return { z: 'undo', y: 'redo', c: 'copy', x: 'cut', v: 'paste', a: 'selectall', Insert: 'copy' }[key] ?? null;
  if (init.shiftKey && !init.ctrlKey) return { Insert: 'paste', Delete: 'cut' }[key] ?? null;
  return null;
};
const editingCommand = (command) => {
  const name = String(command).toLowerCase();
  return Object.hasOwn(editingCommands, name) ? editingCommands[name] : null;
};

// The global object. -----------------------------------------------------------

defineHandlers(Document.prototype, [...handlerNames, 'readystatechange', 'selectionchange', 'visibilitychange']);

// The visual viewport: the layout viewport, as pages are never pinch-zoomed.
class VisualViewport extends EventTarget {
  get width() {
    return document.documentElement ? document.documentElement.clientWidth : viewport()[0];
  }

  get height() {
    return document.documentElement ? document.documentElement.clientHeight : viewport()[1];
  }

  get offsetLeft() {
    return 0;
  }

  get offsetTop() {
    return 0;
  }

  get pageLeft() {
    return global.scrollX;
  }

  get pageTop() {
    return global.scrollY;
  }

  get scale() {
    return 1;
  }
}
defineHandlers(VisualViewport.prototype, ['resize', 'scroll', 'scrollend']);
const visualViewport = new VisualViewport();

// Notifications, which Electron grants pages and shows as the system's: the
// application's main realm shows them and sends their events back.
const notifications = new Map();
let notificationCount = 0;
const NOTIFICATION = Symbol('notification');
class Notification extends EventTarget {
  constructor(title, options = undefined) {
    super();
    if (arguments.length === 0) {
      throw new TypeError("Failed to construct 'Notification': 1 argument required, but only 0 present.");
    }
    const given = options ?? {};
    const text = (value) => (value === undefined ? '' : String(value));
    const id = ++notificationCount;
    this[NOTIFICATION] = {
      id,
      title: String(title),
      body: text(given.body),
      icon: given.icon === undefined ? '' : resolveURL(String(given.icon)),
      tag: text(given.tag),
      lang: text(given.lang),
      dir: given.dir === undefined ? 'auto' : String(given.dir),
      data: given.data === undefined ? null : given.data,
      silent: given.silent === undefined ? null : !!given.silent,
      requireInteraction: !!given.requireInteraction,
      timestamp: given.timestamp === undefined ? Date.now() : Number(given.timestamp),
    };
    notifications.set(id, this);
    const state = this[NOTIFICATION];
    // The notification shows with its icon's pixels, once a network icon
    // downloaded. Pages without an application to show them get an error
    // event.
    const show = (later) => {
      if (!notifications.has(id) || state.closed) return;
      const pixels = state.icon === '' ? null : page.imagePixels(state.icon);
      if (typeof host.notify === 'function' && host.notify(id, state.title, state.body, !!state.silent, pixels)) return;
      notifications.delete(id);
      const error = () => this.dispatchEvent(new Event('error'));
      if (later) error();
      else setTimeout(error, 0);
    };
    if (/^https?:/i.test(state.icon) && !page.imageSize(state.icon)) downloadImage(state.icon, () => show(true));
    else show(false);
  }

  static get permission() {
    return 'granted';
  }

  static requestPermission(callback) {
    if (typeof callback === 'function') queueMicrotask(() => callback('granted'));
    return Promise.resolve('granted');
  }

  static get maxActions() {
    return 2;
  }

  get title() { return this[NOTIFICATION].title; }
  get body() { return this[NOTIFICATION].body; }
  get icon() { return this[NOTIFICATION].icon; }
  get tag() { return this[NOTIFICATION].tag; }
  get lang() { return this[NOTIFICATION].lang; }
  get dir() { return this[NOTIFICATION].dir; }
  get data() { return this[NOTIFICATION].data; }
  get silent() { return this[NOTIFICATION].silent; }
  get requireInteraction() { return this[NOTIFICATION].requireInteraction; }
  get timestamp() { return this[NOTIFICATION].timestamp; }
  get badge() { return ''; }
  get image() { return ''; }
  get renotify() { return false; }
  get actions() { return []; }
  get vibrate() { return []; }

  close() {
    // One still waiting for its icon does not show.
    this[NOTIFICATION].closed = true;
    if (typeof host.closeNotification === 'function') host.closeNotification(this[NOTIFICATION].id);
  }
}
defineHandlers(Notification.prototype, ['click', 'show', 'error', 'close']);

class Window extends EventTarget {
  constructor() {
    illegal();
  }
}
defineHandlers(Window.prototype, [...handlerNames, 'beforeunload', 'hashchange', 'message', 'offline', 'online', 'pagehide', 'pageshow', 'popstate', 'storage', 'unload', 'unhandledrejection', 'rejectionhandled']);
// Named access: elements with an id are reachable as globals, unless a real
// global has the name.
const namedProperties = new Proxy(Object.create(EventTarget.prototype), {
  has(target, key) {
    return Reflect.has(target, key) || (typeof key === 'string' && native.byId(0, key) !== null);
  },
  get(target, key, receiver) {
    if (typeof key === 'string' && !Reflect.has(target, key)) {
      const element = native.byId(0, key);
      if (element !== null) return wrap(element);
    }
    return Reflect.get(target, key, receiver);
  },
});
Object.setPrototypeOf(Window.prototype, namedProperties);
Object.setPrototypeOf(global, Window.prototype);
adopt(1, global);

const scrollWindow = (args, relative) => {
  const root = document.documentElement;
  if (root) root[relative ? 'scrollBy' : 'scrollTo'](...args);
};

const globals = {
  window: global,
  self: global,
  top: global,
  parent: global,
  frames: global,
  document,
  location,
  history,
  navigator,
  screen,
  performance,
  PerformanceObserver,
  PerformanceObserverEntryList,
  localStorage,
  sessionStorage,
  crypto,
  customElements,
  CustomElementRegistry,
  CSS,
  origin: currentURL.origin,
  isSecureContext: true,
  closed: false,
  length: 0,
  name: '',
  status: '',
  visualViewport,
  requestAnimationFrame,
  cancelAnimationFrame,
  requestIdleCallback,
  cancelIdleCallback,
  matchMedia,
  getComputedStyle,
  getSelection: () => selection,
  reportError: (error) => host.report(error, false),
  alert: (message = '') => {
    if (typeof host.web.onalert === 'function') host.web.onalert(String(message));
  },
  confirm: (message = '') => (typeof host.web.onconfirm === 'function' ? !!host.web.onconfirm(String(message)) : false),
  prompt: (message = '', value = '') => (typeof host.web.onprompt === 'function' ? host.web.onprompt(String(message), String(value)) : null),
  open: (url = '', target = '_blank') => {
    navigate(String(url), String(target) || '_blank');
    return null;
  },
  close: () => {
    if (typeof host.web.onclose === 'function') host.web.onclose();
  },
  print() {},
  focus() {},
  blur() {},
  stop() {},
  moveTo() {},
  resizeTo() {},
  postMessage: (data, targetOrigin, transfer) => {
    const ports = transferList(typeof targetOrigin === 'object' && targetOrigin !== null ? targetOrigin : transfer, null);
    const message = structuredClone(data);
    setTimeout(() => fire(global, MessageEvent, 'message', { data: message, origin: location.origin, source: global, ports: Object.freeze(ports) }), 0);
  },
  scrollTo: (...args) => scrollWindow(args, false),
  scroll: (...args) => scrollWindow(args, false),
  scrollBy: (...args) => scrollWindow(args, true),
  Image: HTMLImageElement,
  Option: HTMLOptionElement,
  Audio,
  Window,
  EventTarget,
  Event,
  CustomEvent,
  ToggleEvent,
  UIEvent,
  MouseEvent,
  PointerEvent,
  WheelEvent,
  KeyboardEvent,
  FocusEvent,
  InputEvent,
  CompositionEvent,
  ErrorEvent,
  PromiseRejectionEvent,
  SubmitEvent,
  ProgressEvent,
  XMLHttpRequest,
  XMLHttpRequestEventTarget,
  XMLHttpRequestUpload,
  MessageEvent,
  StorageEvent,
  PopStateEvent,
  HashChangeEvent,
  AnimationEvent,
  TransitionEvent,
  DragEvent,
  ClipboardEvent,
  DataTransfer,
  FileList,
  CanvasRenderingContext2D,
  CanvasGradient,
  CanvasPattern,
  Path2D,
  ImageData,
  DOMMatrix,
  AbortController,
  AbortSignal,
  DOMRect,
  DOMRectReadOnly,
  DOMRectList,
  DOMTokenList,
  DOMStringMap,
  NamedNodeMap,
  Attr,
  NodeList,
  HTMLCollection,
  Node,
  CharacterData,
  Text,
  Comment,
  DocumentFragment,
  ShadowRoot,
  Document,
  HTMLDocument: Document,
  DOMParser,
  XMLSerializer,
  Element,
  SVGElement,
  SVGSVGElement,
  SVGAnimatedString,
  ...elementClasses,
  CSSStyleDeclaration,
  CSSStyleSheet,
  CSSRule,
  StyleSheet,
  MediaQueryList,
  VisualViewport,
  Location,
  History,
  Navigator,
  Storage,
  TreeWalker,
  NodeIterator,
  Range,
  Selection,
  CaretPosition,
  MutationObserver,
  ResizeObserver,
  IntersectionObserver,
  IntersectionObserverEntry,
  MessageChannel,
  MessagePort,
  Worker,
  ServiceWorker,
  ServiceWorkerContainer,
  ServiceWorkerRegistration,
  Notification,
  FormData,
  Blob,
  File,
  FileReader,
  FontFace,
  NodeFilter: { FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3, SHOW_ALL: 0xffffffff, SHOW_ELEMENT: 1, SHOW_TEXT: 4, SHOW_COMMENT: 128 },
};
for (const [name, value] of Object.entries(globals)) hidden(global, name, value);
for (const [name, read] of Object.entries({
  innerWidth: () => viewport()[0],
  innerHeight: () => viewport()[1],
  outerWidth: () => viewport()[0],
  outerHeight: () => viewport()[1],
  devicePixelRatio: () => viewport()[2],
  scrollX: () => (document.documentElement ? document.documentElement.scrollLeft : 0),
  scrollY: () => (document.documentElement ? document.documentElement.scrollTop : 0),
  pageXOffset: () => global.scrollX,
  pageYOffset: () => global.scrollY,
  screenX: () => 0,
  screenY: () => 0,
  screenLeft: () => 0,
  screenTop: () => 0,
  event: () => undefined,
})) {
  Object.defineProperty(global, name, { get: read, configurable: true, enumerable: true });
}

// Uncaught errors become `error` and `unhandledrejection` events.
host.onerror = (error, promise) => {
  if (promise) {
    return !fire(global, PromiseRejectionEvent, 'unhandledrejection', { cancelable: true, reason: error, promise: Promise.reject(error).catch(() => {}) }).allowed;
  }
  const message = error instanceof Error ? `Uncaught ${error.name}: ${error.message}` : `Uncaught ${String(error)}`;
  return !fire(global, ErrorEvent, 'error', { cancelable: true, message, error }).allowed;
};

// The renderer's interface. ------------------------------------------------------

let hovered = null;
const targetOf = (id) => wrap(id) || document.body || document.documentElement || document;
const keyCodes = {
  Enter: 13, Tab: 9, Escape: 27, Backspace: 8, ' ': 32, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40,
  Delete: 46, Insert: 45, Home: 36, End: 35, PageUp: 33, PageDown: 34, Shift: 16, Control: 17, Alt: 18, Meta: 91,
  CapsLock: 20, ContextMenu: 93,
};
// The legacy key codes of physical keys, Windows' virtual key codes, as
// Chromium reports them there.
const codeKeyCodes = {
  Backspace: 8, Tab: 9, Enter: 13, NumpadEnter: 13, ShiftLeft: 16, ShiftRight: 16, ControlLeft: 17,
  ControlRight: 17, AltLeft: 18, AltRight: 18, Pause: 19, CapsLock: 20, Escape: 27, Space: 32,
  PageUp: 33, PageDown: 34, End: 35, Home: 36, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40,
  PrintScreen: 44, Insert: 45, Delete: 46, MetaLeft: 91, MetaRight: 92, ContextMenu: 93,
  NumpadMultiply: 106, NumpadAdd: 107, NumpadSubtract: 109, NumpadDecimal: 110, NumpadDivide: 111,
  NumLock: 144, ScrollLock: 145, Semicolon: 186, Equal: 187, Comma: 188, Minus: 189, Period: 190,
  Slash: 191, Backquote: 192, BracketLeft: 219, Backslash: 220, BracketRight: 221, Quote: 222,
  IntlBackslash: 226,
};
const codeKeyCode = (code) => {
  if (codeKeyCodes[code] !== undefined) return codeKeyCodes[code];
  const [, kind, rest] = /^(Key|Digit|Numpad|F)(\w+)$/.exec(code || '') || [];
  if (kind === 'Key' && /^[A-Z]$/.test(rest)) return rest.charCodeAt(0);
  if (kind === 'Digit' && /^\d$/.test(rest)) return 48 + Number(rest);
  if (kind === 'Numpad' && /^\d$/.test(rest)) return 96 + Number(rest);
  if (kind === 'F' && /^\d+$/.test(rest) && Number(rest) >= 1 && Number(rest) <= 24) return 111 + Number(rest);
  return undefined;
};
// With NumLock off a numpad key is the navigation key it names, and
// Windows reports that key's virtual key code.
const numpadNavigation = (init) => /^Numpad(\d|Decimal)$/.test(init.code || '') && keyCodes[init.key] !== undefined;
const legacyKey = (init) => {
  const keyCode = (numpadNavigation(init) ? keyCodes[init.key] : codeKeyCode(init.code)) ?? keyCodes[init.key] ??
    (init.key && init.key.length === 1 ? init.key.toUpperCase().charCodeAt(0) : 0);
  return { keyCode, which: keyCode };
};
// The elements from a node up, through the hosts of its shadow trees, which
// pointer boundary events enter and leave.
const ancestorsOf = (node) => {
  const chain = [];
  for (let current = node; current; ) {
    if (current.nodeType === 1) chain.push(current);
    current = current.parentNode || (current instanceof ShadowRoot ? current.host : null);
  }
  return chain;
};

const updateHover = (next, init) => {
  const previous = hovered;
  if (previous === next) return;
  hovered = next;
  const old = ancestorsOf(previous);
  const fresh = ancestorsOf(next);
  if (previous) {
    fire(previous, PointerEvent, 'pointerout', { ...init, bubbles: true, cancelable: true, relatedTarget: next });
    fire(previous, MouseEvent, 'mouseout', { ...init, bubbles: true, cancelable: true, relatedTarget: next });
    for (const element of old.filter((item) => !fresh.includes(item))) {
      fire(element, PointerEvent, 'pointerleave', { ...init, relatedTarget: next });
      fire(element, MouseEvent, 'mouseleave', { ...init, relatedTarget: next });
    }
  }
  if (next) {
    fire(next, PointerEvent, 'pointerover', { ...init, bubbles: true, cancelable: true, relatedTarget: previous });
    fire(next, MouseEvent, 'mouseover', { ...init, bubbles: true, cancelable: true, relatedTarget: previous });
    for (const element of fresh.filter((item) => !old.includes(item)).reverse()) {
      fire(element, PointerEvent, 'pointerenter', { ...init, relatedTarget: previous });
      fire(element, MouseEvent, 'mouseenter', { ...init, relatedTarget: previous });
    }
  }
};

// Sequential focus navigation: elements with a positive tabindex in its
// order, then the others in tree order, skipping those that are not rendered.
const sequentialOrder = () => {
  const rendered = (element) => {
    if (getComputedStyle(element).visibility === 'hidden') return false;
    for (let node = element; node; node = node.parentElement) {
      if (node.hidden || getComputedStyle(node).display === 'none') return false;
      const parent = node.parentElement;
      if (parent?.localName === 'details' && !parent.open && [...parent.children].find(child=>child.localName==='summary') !== node) return false;
    }
    return true;
  };
  const all = [...document.querySelectorAll('*')].filter(
    (element) => native.focusable(element[ID]) && element.tabIndex >= 0 && rendered(element),
  );
  const positive = all.filter((element) => element.tabIndex > 0).sort((a, b) => a.tabIndex - b.tabIndex);
  return [...positive, ...all.filter((element) => element.tabIndex === 0)];
};

const moveFocus = (backwards) => {
  const order = sequentialOrder();
  if (!order.length) return false;
  const current = order.indexOf(wrap(native.focused()));
  const step = backwards ? order.length - 1 : 1;
  const next = current < 0 ? (backwards ? order.length - 1 : 0) : (current + step) % order.length;
  focusNode(order[next], {}, true);
  placeEditingCaret(order[next]);
  return true;
};

// Elements the keyboard activates like a click when they are painted rather
// than native controls: Enter on links and buttons, Space on buttons.
const buttonTypes = ['button', 'submit', 'reset', 'image'];
const isButton = (element) =>
  element.localName === 'button' || (element.localName === 'input' && buttonTypes.includes(element.type)) ||
  (element.localName === 'summary' && element.parentElement?.localName === 'details' && [...element.parentElement.children].find(child=>child.localName==='summary') === element);
const isLink = (element) => (element.localName === 'a' || element.localName === 'area') && element.hasAttribute('href');
// A select showing one option, which opens a list of them.
const isDropDown = (element) => element.localName === 'select' && !element.multiple && element.size <= 1;

// Keys on a select the page draws: arrows choose the neighbouring option,
// Home and End the first and last, Enter, Space, F4 and Alt+arrows open it.
const selectKey = (select, init) => {
  const open = init.key === 'Enter' || init.key === ' ' || init.key === 'F4' || (init.altKey && (init.key === 'ArrowDown' || init.key === 'ArrowUp'));
  if (open) {
    page.openSelect(select[ID]);
    return true;
  }
  const options = [...select.options].filter((option) => !option.disabled);
  if (!options.length) return false;
  const current = options.indexOf(select.options[select.selectedIndex]);
  const target = { ArrowDown: current + 1, ArrowRight: current + 1, ArrowUp: current - 1, ArrowLeft: current - 1, Home: 0, End: options.length - 1 }[init.key];
  if (target === undefined) return false;
  const next = options[Math.max(0, Math.min(options.length - 1, target))];
  if (next !== select.options[select.selectedIndex]) {
    select.selectedIndex = [...select.options].indexOf(next);
    fire(select, Event, 'input', { bubbles: true, composed: true });
    fire(select, Event, 'change', { bubbles: true });
  }
  return true;
};

// Keys the page and its editing left alone scroll, as Chromium's defaults
// do: arrows by a line of 40 pixels, Page Up, Page Down and Space by seven
// eighths of the scroller's height, Home and End (with Control too) to its
// edges. The scroller is the nearest one around the focused element, or
// around the last pressed one when nothing is focused, that can still move
// that way; else the viewport.
let lastPressed = null;
const scrollKeys = {
  ArrowUp: [1, -1, 'line'], ArrowDown: [1, 1, 'line'], ArrowLeft: [0, -1, 'line'], ArrowRight: [0, 1, 'line'],
  PageUp: [1, -1, 'page'], PageDown: [1, 1, 'page'], Home: [1, -1, 'document'], End: [1, 1, 'document'],
};
const userScrollable = (element, axis) => {
  const name = axis ? 'overflow-y' : 'overflow-x';
  let value = styleValue(element, name);
  if (element === document.documentElement) {
    if (value === 'visible' && document.body) value = styleValue(document.body, name);
    return value !== 'hidden' && value !== 'clip';
  }
  return value === 'auto' || value === 'scroll' || value === 'overlay';
};
const scrollStep = (element, axis, sign, granularity) => {
  const id = element[ID];
  const client = page.client(id), size = page.scrollSize(id);
  if (!client || !size) return false;
  const shown = client[2 + axis], extent = size[axis];
  const from = native.scroll(id);
  const step = granularity === 'line' ? 40 : granularity === 'page' ? Math.max(shown * 0.875, 1) : extent;
  const to = Math.min(Math.max(0, from[axis] + sign * step), Math.max(0, extent - shown));
  if (Math.abs(to - from[axis]) < 0.5) return false;
  const next = [from[0], from[1]];
  next[axis] = to;
  applyScroll(element, next[0], next[1]);
  return true;
};
const keyboardScroll = (focused, axis, sign, granularity) => {
  const root = document.documentElement;
  let node = focused === document.body || focused === root || focused.nodeType !== 1 ? lastPressed : focused;
  if (node && !node.isConnected) node = null;
  for (; node; node = node.parentNode && node.parentNode.nodeType === 11 ? node.parentNode.host : node.parentNode) {
    if (node.nodeType === 1 && node !== root && userScrollable(node, axis) && scrollStep(node, axis, sign, granularity)) return true;
  }
  return !!root && userScrollable(root, axis) && scrollStep(root, axis, sign, granularity);
};
// Whether a key's default belongs to the focused element rather than to
// scrolling: form controls keep their keys, buttons Space.
const keepsKeys = (element, space) =>
  element.nodeType === 1 && (['input', 'textarea', 'select'].includes(element.localName) || (space && isButton(element)));

const focusTargetOf = (element) => {
  for (let node = element; node && node.nodeType === 1; node = node.parentElement) {
    if (native.focusable(node[ID])) return node;
  }
  return null;
};

let dragCandidate = null;
let drag = null;
let suppressDragClick = false;
let externalDrag = null;
const dragTransfer = () => {
  const data = new DataTransfer();
  const state = transfers.get(data);
  state.drag = true;
  state.effectAllowed = 'uninitialized';
  return data;
};
const fireDrag = (target, type, source, init = {}) => {
  const data = new DataTransfer();
  const state = transfers.get(data), store = transfers.get(source);
  state.store = store.store;
  state.drag = true;
  state.mode = type === 'dragstart' ? 'readwrite' : type === 'drop' ? 'readonly' : 'protected';
  state.effectAllowed = store.effectAllowed;
  state.dropEffect = store.dropEffect;
  try {
    return fire(target, DragEvent, type, {
      ...init, bubbles: true, cancelable: type !== 'dragleave' && type !== 'dragend',
      view: global, dataTransfer: data,
    });
  } finally {
    store.effectAllowed = state.effectAllowed;
    store.dropEffect = state.dropEffect;
    state.mode = 'disconnected';
  }
};
const dragFire = (target, type, init = {}) => fireDrag(target, type, drag.data, init);
// A drag that leaves the window continues in other applications, which the
// platform runs until the drop; the drop's effect ends the drag here. Text
// moved out of editable content leaves it.
const outsideWindow = (init) => init.clientX < 0 || init.clientY < 0 ||
  init.clientX >= innerWidth || init.clientY >= innerHeight;
const dragOut = (init) => {
  if (typeof page.dragOut !== 'function') return false;
  const items = drag.data.types.filter((type) => type !== 'Files').map((type) => [type, drag.data.getData(type)]);
  if (!items.length) return false;
  if (drag.target) dragFire(drag.target, 'dragleave', init);
  drag.target = null;
  const effect = page.dragOut(items, drag.data.effectAllowed);
  drag.data.dropEffect = effect;
  const selection = drag.selection;
  if (effect === 'move' && selection && editingHostOf(selection.startContainer)) {
    focusNode(editingHostOf(selection.startContainer), {}, false);
    associateRange(selection.cloneRange());
    asUser(() => hostCommand('deleteByDrag', null, clearPlan));
  }
  dragFire(drag.source, 'dragend', init);
  drag = null;
  dragCandidate = null;
  suppressDragClick = true;
  return true;
};
const dragMove = (target, init) => {
  dragFire(drag.source, 'drag', init);
  if (target !== drag.target) {
    const previous = drag.target;
    drag.target = target;
    if (target) dragFire(target, 'dragenter', { ...init, relatedTarget: previous });
    if (previous) dragFire(previous, 'dragleave', { ...init, relatedTarget: target });
  }
  // Editable content takes dragged text unless the page says otherwise.
  drag.accepted = !!target && (!dragFire(target, 'dragover', init).allowed ||
    (!!drag.selection && !!target.isContentEditable && takesText(target, drag.data)));
};
// Selected text dropped in editable content moves there, or with Ctrl
// held is copied, and ends selected, as Chromium drops it; dropped on
// itself it stays.
const dropSelection = (selection, data, init) => {
  const point = caretPoint(init.clientX, init.clientY);
  const host = point && editingHostOf(point[0]);
  const text = droppedText(data);
  if (!host || !text || selection.isPointInRange(point[0], point[1])) return;
  const at = createRange(canonicalPoint(point, host));
  focusNode(host, {}, false);
  asUser(() => {
    if (!init.ctrlKey && editingHostOf(selection.startContainer) === host) {
      associateRange(selection.cloneRange());
      hostCommand('deleteByDrag', null, clearPlan);
    }
    const start = [at.startContainer, at.startOffset];
    selectEndpoints(start, start);
    hostCommand('insertFromDrop', null, insertPlan(text), data);
  });
  selectEndpoints([at.startContainer, at.startOffset], selectionBoundary(false));
};
const finishDrag = (init, drop) => {
  if (drop && drag.accepted) {
    const dropped = dragFire(drag.target, 'drop', init).allowed;
    if (drag.data.dropEffect === 'none') drag.data.dropEffect = drag.data.effectAllowed === 'move' ? 'move' : 'copy';
    if (dropped && drag.selection) dropSelection(drag.selection, drag.data, init);
  } else {
    drag.data.dropEffect = 'none';
    if (drag.target) dragFire(drag.target, 'dragleave', init);
  }
  dragFire(drag.source, 'dragend', init);
  drag = null;
  dragCandidate = null;
  suppressDragClick = true;
};

const fileTransfer = (entries, strings = []) => {
  const data = dragTransfer();
  data.effectAllowed = 'copy';
  data.dropEffect = 'copy';
  for (const [type, value] of strings) data.setData(type, value);
  for (const entry of entries) {
    const path = entry.path, name = path.replace(/\\/g, '/').split('/').at(-1);
    const mime = { pdf: 'application/pdf', txt: 'text/plain', png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml', bmp: 'image/bmp', avif: 'image/avif', html: 'text/html', htm: 'text/html', json: 'application/json', csv: 'text/csv' }[name.split('.').at(-1).toLowerCase()] || '';
    const bytes = Uint8Array.from(atob(entry.bytes ?? ''), character => character.charCodeAt(0));
    const file = new File([bytes], name, { type: mime });
    if (entry.bytes !== undefined) Object.defineProperty(file, 'path', { value: path });
    data.items.add(file);
  }
  return data;
};
const externalMove = (target, init) => {
  if (target !== externalDrag.target) {
    const previous = externalDrag.target;
    externalDrag.target = target;
    if (target) fireDrag(target, 'dragenter', externalDrag.data, { ...init, relatedTarget: previous });
    if (previous) fireDrag(previous, 'dragleave', externalDrag.data, { ...init, relatedTarget: target });
  }
  externalDrag.data.dropEffect = 'copy';
  if (!target) return false;
  if (!fireDrag(target, 'dragover', externalDrag.data, init).allowed) return externalDrag.data.dropEffect === 'copy';
  // Editable content takes dragged text unless the page says otherwise.
  return takesText(target, externalDrag.data);
};
const droppedText = (data) => data.getData('text/plain') || data.getData('text/uri-list');
const takesText = (target, data) => !!target && !!droppedText(data) && (isEditable(target) || !!target.isContentEditable);
// Text dropped from another application goes where it is dropped in
// editable content, as Chromium inserts it.
const dropText = (target, data, init) => {
  const text = droppedText(data);
  if (!text || !takesText(target, data)) return false;
  if (isEditable(target)) {
    target.focus();
    asUser(() => replaceSelection(target, text, 'insertFromDrop'));
    return true;
  }
  const point = caretPoint(init.clientX, init.clientY);
  const host = point && editingHostOf(point[0]);
  if (!host) return false;
  const caret = canonicalPoint(point, host);
  focusNode(host, {}, false);
  selectEndpoints(caret, caret);
  return asUser(() => hostCommand('insertFromDrop', null, insertPlan(text), data));
};
const leaveExternal = (init) => {
  if (externalDrag?.target) fireDrag(externalDrag.target, 'dragleave', externalDrag.data, init);
  externalDrag = null;
};

// What Electron's `context-menu` event reports about the content under a
// context click.
// The misspelled word of the last context menu, which `replaceMisspelling`
// replaces.
let lastMisspelling = null;

const contextMenuParams = (target, init) => {
  const element = target?.nodeType === 1 ? target : target?.parentElement ?? null;
  const field = element?.closest?.('input, textarea') ?? null;
  const textField = field && (field.localName === 'textarea' ||
    !/^(button|checkbox|color|file|hidden|image|radio|range|reset|submit)$/.test(field.type));
  const editable = !!(textField && !field.disabled && !field.readOnly) || !!element?.isContentEditable;
  const selectionText = textField
    ? String(field.value ?? '').slice(field.selectionStart ?? 0, field.selectionEnd ?? 0)
    : String(getSelection() ?? '');
  const selected = selectionText.length > 0;
  const link = element?.closest?.('a[href]') ?? null;
  const image = element?.localName === 'img' ? element : null;
  // The misspelled word the menu was opened on, as spelling marks show it.
  let misspelledWord = '', dictionarySuggestions = [];
  lastMisspelling = null;
  const point = getSelection()?.anchorNode;
  if (editable && !textField && point?.nodeType === 3 && page.misspelling) {
    const offset = getSelection().anchorOffset;
    const found = page.misspelling(point[ID], offset);
    if (found) {
      misspelledWord = point.data.slice(found[0], found[1]);
      dictionarySuggestions = page.spellingSuggestions(misspelledWord);
      lastMisspelling = { node: point, start: found[0], end: found[1], word: misspelledWord };
    }
  }
  const formControlType = !field ? 'none'
    : field.localName === 'textarea' ? 'text-area'
    : `input-${{ tel: 'telephone', 'datetime-local': 'datetime-local' }[field.type] ?? field.type ?? 'text'}`;
  return {
    x: Math.round(init.clientX ?? 0), y: Math.round(init.clientY ?? 0), frame: null,
    linkURL: link?.href ?? '', linkText: link?.textContent ?? '',
    pageURL: location.href, frameURL: '', srcURL: image?.src ?? '',
    mediaType: image ? 'image' : 'none', hasImageContents: !!image?.complete && (image.naturalWidth ?? 0) > 0,
    isEditable: editable, selectionText, titleText: element?.title ?? '', altText: image?.alt ?? '',
    suggestedFilename: '', selectionRect: { x: 0, y: 0, width: 0, height: 0 }, selectionStartOffset: 0,
    referrerPolicy: { url: '', policy: 'strict-origin-when-cross-origin' },
    misspelledWord, dictionarySuggestions, frameCharset: document.characterSet ?? 'UTF-8',
    formControlType, spellcheckEnabled: editable && element?.getAttribute?.('spellcheck') !== 'false', menuSourceType: 'mouse',
    mediaFlags: {
      inError: false, isPaused: false, isMuted: false, canSave: !!image, hasAudio: false, isLooping: false,
      isControlsVisible: false, canToggleControls: false, canPrint: false, canRotate: false,
      canShowPictureInPicture: false, isShowingPictureInPicture: false, canLoop: false,
    },
    editFlags: {
      canUndo: editable, canRedo: editable, canCut: editable && selected, canCopy: selected,
      canPaste: editable, canDelete: editable && selected, canSelectAll: true,
      canEditRichly: editable && !textField,
    },
  };
};

const web = {
  // Fires DOMContentLoaded and load after the page's scripts ran.
  ready() {
    // NATIVEBOAT_DUMP_DOM names a file the page's markup goes to every two
    // seconds, for comparing documents with Chromium's.
    if (native.dumpTarget() !== null) {
      const dump = () => {
        if (document.documentElement) native.writeDump('<!doctype html>' + document.documentElement.outerHTML);
        setTimeout(dump, 2000);
      };
      setTimeout(dump, 2000);
    }
    readyState = 'interactive';
    for (const link of document.querySelectorAll('link')) linkEvents(link);
    // The page's own images load as the parser started them.
    for (const image of document.querySelectorAll('img[src]')) imageEvents(image);
    for (const media of document.querySelectorAll('video, audio')) mediaLoad(media);
    fire(document, Event, 'readystatechange');
    fire(document, Event, 'DOMContentLoaded', { bubbles: true });
    readyState = 'complete';
    fire(document, Event, 'readystatechange');
    fire(global, Event, 'load');
    // A page loaded with a title gives it to its window.
    if (document.title) host.title(document.title);
    const autofocus = document.querySelector('[autofocus]');
    if (autofocus && !native.focused()) autofocus.focus();
  },

  // Mouse input on painted content; returns whether it was cancelled.
  pointer(kind, id, init = {}) {
    let target = id === null ? null : targetOf(id);
    const event = { view: global, ...init };
    lastPointer = kind === 'leave' ? null : { ...init };
    if (kind === 'leave') {
      updateHover(null, event);
      if (drag) dragMove(null, event);
      return false;
    }
    updateHover(target, event);
    const options = { ...event, bubbles: true, cancelable: true, composed: true };
    switch (kind) {
      case 'move': {
        if (!drag && dragCandidate && (init.buttons & 1) &&
            Math.hypot(init.clientX - dragCandidate.x, init.clientY - dragCandidate.y) >= 5) {
          drag = { source: dragCandidate.source, target: null, data: dragTransfer(), accepted: false, selection: dragCandidate.selection };
          if (drag.source.localName === 'a') drag.data.setData('text/uri-list', drag.source.href);
          if (drag.selection) {
            const html = document.createElement('div');
            html.appendChild(drag.selection.cloneContents());
            drag.data.setData('text/plain', String(drag.selection));
            drag.data.setData('text/html', html.innerHTML);
          }
          if (dragFire(drag.source, 'dragstart', event).allowed) {
            suppressDragClick = true;
          } else { drag = null; dragCandidate = null; }
        }
        if (drag && outsideWindow(init) && dragOut(event)) return true;
        if (drag) { dragMove(target, event); return true; }
        const pointer = fire(target, PointerEvent, 'pointermove', options).allowed;
        const allowed = fire(target, MouseEvent, 'mousemove', options).allowed && pointer;
        pointerExtend(init);
        return !allowed;
      }
      case 'down': {
        if ((init.button ?? 0) === 0) interruptComposition();
        // Editors may replace the clicked span when compositionend updates
        // their model. Hit the new DOM before delivering the pointer event.
        if (target && !target.isConnected) target = document.elementFromPoint(init.clientX, init.clientY);
        suppressDragClick = false;
        lastPressed = target;
        const pointer = fire(target, PointerEvent, 'pointerdown', options).allowed;
        const allowed = fire(target, MouseEvent, 'mousedown', { ...options, detail: init.detail ?? 1 }).allowed && pointer;
        if (allowed && (init.button ?? 0) === 0) {
          dragCandidate = null;
          for (let source = target; source && source.nodeType === 1; source = source.parentElement) {
            // `-webkit-user-drag` decides over images, links and `draggable`.
            const userDrag = page.computedStyleValue(source[ID], '-webkit-user-drag');
            if (userDrag === 'element' || (userDrag !== 'none' && source.draggable)) {
              dragCandidate = { source, x: init.clientX, y: init.clientY };
              break;
            }
          }
          // A press on selected text may start dragging it: the selection
          // stays until the button comes up without a drag.
          if (!dragCandidate && (init.detail ?? 1) === 1 && !init.shiftKey && selectedRange &&
              !selectedRange.collapsed && target && !isTextControl(target) && selectableTarget(target)) {
            const point = caretPoint(init.clientX, init.clientY);
            if (point && selectedRange.isPointInRange(point[0], point[1])) {
              dragCandidate = { source: point[0], x: init.clientX, y: init.clientY, selection: selectedRange.cloneRange(), point };
            }
          }
          const focusTarget = focusTargetOf(target);
          if (focusTarget) focusNode(focusTarget, {}, false);
          else if (native.focused() !== null) focusNode(null);
          pointerSelect(target, init);
        } else if (allowed && init.button === 2) {
          contextSelect(target, init);
        }
        return !allowed;
      }
      case 'up': {
        const pressed = dragCandidate;
        dragCandidate = null;
        mouseSelection = null;
        if (drag) { dragMove(target, event); finishDrag(event, true); return true; }
        const pointer = fire(target, PointerEvent, 'pointerup', options).allowed;
        const allowed = fire(target, MouseEvent, 'mouseup', options).allowed && pointer;
        // A press on selected text that did not drag it puts the caret
        // where it was pressed.
        if (pressed?.selection && pressed.point[0].isConnected) {
          const host = editingHostOf(pressed.point[0]);
          associateRange(createRange(host ? canonicalPoint(pressed.point, host) : pressed.point));
        }
        return !allowed;
      }
      case 'click':
        if (suppressDragClick) { suppressDragClick = false; return true; }
        if ((init.button ?? 0) === 2) {
          // An uncancelled context click opens the application's menu.
          const allowed = fire(target, MouseEvent, 'contextmenu', options).allowed;
          if (allowed) host.contextMenu?.(contextMenuParams(target, init));
          return !allowed;
        }
        if ((init.button ?? 0) !== 0) return !fire(target, MouseEvent, 'auxclick', options).allowed;
        return !activate(target, { ...event, detail: init.detail ?? 1 });
      case 'dblclick':
        return !fire(target, MouseEvent, 'dblclick', { ...options, detail: 2 }).allowed;
      default:
        return false;
    }
  },

  wheel(id, init = {}) {
    return !fire(targetOf(id), WheelEvent, 'wheel', { view: global, ...init, bubbles: true, cancelable: true, composed: true }).allowed;
  },

  // OLE hover metadata never contains bytes or application-readable paths.
  dragFiles(kind, id, paths, init = {}, strings = []) {
    if (kind === 'leave') { leaveExternal(init); return false; }
    if (kind === 'enter') {
      leaveExternal(init);
      externalDrag = { target: null, data: fileTransfer(paths, strings) };
    }
    if (!externalDrag) return false;
    return externalMove(targetOf(id), init);
  },

  // Paths and bytes are supplied only by the OS drop operation.
  dropFiles(id, paths, init = {}, strings = []) {
    const target = targetOf(id);
    const data = fileTransfer(paths, strings);
    const drop = () => fireDrag(target, 'drop', data, init).allowed ? dropText(target, data, init) : true;
    if (externalDrag) {
      const accepted = externalMove(target, init);
      externalDrag = null;
      if (accepted) return drop();
      if (target) fireDrag(target, 'dragleave', data, init);
      return false;
    }
    fireDrag(target, 'dragenter', data, init);
    if (!fireDrag(target, 'dragover', data, init).allowed || (target && takesText(target, data))) return drop();
    fireDrag(target, 'dragleave', data, init);
    return false;
  },

  // Key input; the target is the focused element, a native control when
  // `control` is true. Returns whether the default action was cancelled or
  // done here.
  key(down, init = {}, control = false) {
    if (down && init.key === 'Escape' && drag) { finishDrag(init, false); return true; }
    const target = wrap(native.focused()) || document.body || document;
    const composing = !!compositionSession;
    const options = { view: global, ...init, ...legacyKey(init), isComposing: composing, bubbles: true, cancelable: true, composed: true };
    if (composing && down) options.keyCode = options.which = 229;
    const painted = !control && target.nodeType === 1;
    if (!down) {
      if (control) nativeEditBefore.delete(target);
      const allowed = fire(target, KeyboardEvent, 'keyup', options).allowed;
      if (allowed && painted && init.key === ' ' && isButton(target)) activate(target, {});
      return !allowed;
    }
    let allowed = fire(target, KeyboardEvent, 'keydown', options).allowed;
    if (composing) return !allowed;
    const plain = !init.ctrlKey && !init.altKey && !init.metaKey;
    if (allowed && init.key === 'Tab' && plain && moveFocus(!!init.shiftKey)) return true;
    const command = allowed && commandKey(init);
    if (command) {
      if (asUser(() => editingCommands[command]())) return true;
    }
    if (allowed && control && isEditable(target)) nativeEditBefore.set(target, historySnapshot(target));
    if (allowed && painted && editingKey(init)) return true;
    if (allowed && painted && isDropDown(target) && !target.disabled && selectKey(target, init)) return true;
    if (allowed && init.key === 'Enter' && painted && (isButton(target) || isLink(target))) {
      activate(target, {});
      return true;
    }
    const scroll = scrollKeys[init.key];
    if (allowed && painted && scroll && !init.altKey && !init.metaKey && !init.shiftKey &&
        (!init.ctrlKey || scroll[2] === 'document') && !editingSelection() && !keepsKeys(target, false) &&
        keyboardScroll(target, ...scroll)) return true;
    if (allowed && (init.key === 'Enter' || (init.key && init.key.length === 1 && !init.ctrlKey && !init.metaKey))) {
      const charCode = init.key === 'Enter' ? 13 : init.key.charCodeAt(0);
      allowed = fire(target, KeyboardEvent, 'keypress', { ...options, charCode, keyCode: charCode, which: charCode }).allowed;
      if (allowed && painted && init.key === ' ' && !init.ctrlKey && !init.altKey && !init.metaKey &&
          !editingSelection() && !keepsKeys(target, true) && keyboardScroll(target, 1, init.shiftKey ? -1 : 1, 'page')) return true;
    }
    if (allowed && painted && init.key === 'Enter' && !init.ctrlKey && !init.altKey && !init.metaKey && editingSelection()) {
      asUser(() => editingCommands[init.shiftKey ? 'insertlinebreak' : 'insertparagraph']());
      return true;
    }
    if (allowed && init.key === 'Enter' && target.localName === 'input' && textTypes.includes(target.type)) {
      const form = target.form;
      if (form) {
        const submitter = [...form.elements].find((control) => (control.localName === 'button' || control.localName === 'input') && ['submit', 'image'].includes(control.type));
        if (submitter) {
          if (!submitter.disabled) activate(submitter, {});
        } else submitForm(form, null);
      }
    }
    return !allowed;
  },

  // Replaces the misspelled word of the last context menu, as the user
  // would by selecting it and typing.
  replaceMisspelling(text) {
    const found = lastMisspelling;
    lastMisspelling = null;
    if (!found || found.node.data.slice(found.start, found.end) !== found.word) return false;
    const selection = getSelection();
    selection.setBaseAndExtent(found.node, found.start, found.node, found.end);
    return web.text(String(text));
  },

  // Text the user typed while the keyboard focus is on painted content;
  // returns whether an editing host took it.
  text(data) {
    if (compositionSession) return true;
    if (!editingSelection()) return false;
    asUser(() => editHost('insertText', String(data), insertPlan(String(data))));
    return true;
  },

  composition(kind, data = '', cursor = String(data).length) {
    if (kind === 'start') return beginComposition();
    if (kind === 'update') return updateComposition(String(data), cursor);
    if (kind === 'end') return finishComposition(String(data));
    return false;
  },

  command(name) {
    interruptComposition();
    const run = editingCommand(name);
    return !!run && asUser(() => run());
  },

  // The user typed into a native text control.
  input(id, value) {
    const target = wrap(id);
    const before = nativeEditBefore.get(target) || historySnapshot(target);
    nativeEditBefore.delete(target);
    native.setValue(id, String(value));
    selections.delete(target);
    edited.add(target);
    fire(target, InputEvent, 'input', { bubbles: true, composed: true, inputType: 'insertText', data: null });
    rememberEdit(target, before, 'insertText', null, true);
  },

  // The user chose options of a native select.
  select(id, indexes) {
    const target = wrap(id);
    const options = optionsOf(target);
    options.forEach((option, index) => {
      native.setOptionSelected(option[ID], indexes.includes(index));
    });
    fire(target, Event, 'input', { bubbles: true, composed: true });
    fire(target, Event, 'change', { bubbles: true });
  },

  // A native slider moved.
  value(id, value) {
    const target = wrap(id);
    native.setValue(id, String(value));
    fire(target, Event, 'input', { bubbles: true, composed: true });
    fire(target, Event, 'change', { bubbles: true });
  },

  // A native button, checkbox or radio button was activated.
  activate(id) {
    return activate(wrap(id), {});
  },

  // A native control gained or lost the keyboard focus.
  focus(id) {
    const target = id === null ? null : wrap(id);
    if (target === wrap(native.focused())) return;
    focusNode(target, { preventScroll: true }, true);
  },

  resize() {
    fire(visualViewport, Event, 'resize');
    fire(global, Event, 'resize');
    for (const list of mediaLists) {
      const matches = list.matches;
      if (matches !== list._last) {
        list._last = matches;
        fire(list, Event, 'change');
      }
    }
    if (resizeObservers.size) scheduleResizeCheck();
    scheduleIntersectionCheck();
  },

  // The user scrolled: the scroll events are due now.
  scroll() {
    fireScrolls();
  },

  // Runs animation frame callbacks, after the scroll events of offsets
  // that scripts or scrollIntoView changed, as the rendering steps do.
  frame(time = performance.now()) {
    fireScrolls();
    const due = [...frames];
    frames.clear();
    for (const [, callback] of due) {
      try {
        callback(time);
      } catch (error) {
        host.report(error, false);
      }
    }
  },

  // Whether frame callbacks wait; network images the last rendering asked
  // for, such as style sheets' backgrounds, start downloading.
  hasFrames() {
    const wanted = page.takeWantedImages();
    if (wanted !== null) for (const url of wanted) downloadImage(url, () => {});
    return frames.size > 0 || native.hasScrolled();
  },

  // A port for a message port end that moved here from another realm.
  port(handle) {
    return nativePort(handle);
  },

  // A message the window's preload posted with `window.postMessage`, with
  // the ports it moved here.
  message(data, ports) {
    fire(global, MessageEvent, 'message', { data, origin: location.origin, source: global, ports: Object.freeze([...ports]) });
  },

  // An event of a notification the page showed.
  notification(id, type) {
    const notification = notifications.get(id);
    if (!notification) return;
    if (type === 'close' || type === 'error') notifications.delete(id);
    notification.dispatchEvent(new Event(type));
  },

  transition(id, type, propertyName, elapsedTime, pseudoElement = '') {
    fire(wrap(id), TransitionEvent, type, { bubbles: true, propertyName, elapsedTime, pseudoElement });
  },

  animation(id, type, animationName, elapsedTime, pseudoElement = '') {
    fire(wrap(id), AnimationEvent, type, { bubbles: true, animationName, elapsedTime, pseudoElement });
  },

  // Style sheet text of <style> elements, including inserted rules, in
  // document order.
  styleSheets() {
    return [...document.querySelectorAll('style')].map((element) => {
      const sheet = sheets.get(element);
      const inserted = sheet ? sheet[SHEET].rules.join('\n') : '';
      return [element[ID], inserted ? `${element.textContent}\n${inserted}` : element.textContent];
    });
  },

  // Fires beforeunload; returns whether leaving is allowed.
  beforeUnload() {
    return fire(global, Event, 'beforeunload', { cancelable: true }).allowed;
  },

  unload() {
    fire(global, Event, 'pagehide');
    fire(global, Event, 'unload');
  },
};
hidden(host, 'web', web);
// Attribute changes made by the HTML parser precede installation of the
// mutation hooks, but still notify listeners added by the page's scripts.
queueDetailsToggles();
