// Events: Event classes, listeners and the dispatch algorithm. -------------

const STATE = Symbol('nativeboat.event');
const started = host.now();

class Event {
  constructor(type, init = {}) {
    if (arguments.length === 0) throw new TypeError("Failed to construct 'Event': 1 argument required");
    hidden(this, STATE, {
      type: String(type),
      bubbles: !!init.bubbles,
      cancelable: !!init.cancelable,
      composed: !!init.composed,
      target: null,
      currentTarget: null,
      phase: 0,
      stop: false,
      stopImmediate: false,
      canceled: false,
      passive: false,
      dispatching: false,
      trusted: false,
      timeStamp: host.now() - started,
      path: [],
    });
  }

  get type() {
    return this[STATE].type;
  }

  get bubbles() {
    return this[STATE].bubbles;
  }

  get cancelable() {
    return this[STATE].cancelable;
  }

  get composed() {
    return this[STATE].composed;
  }

  get target() {
    return this[STATE].target;
  }

  get srcElement() {
    return this[STATE].target;
  }

  get currentTarget() {
    return this[STATE].currentTarget;
  }

  get eventPhase() {
    return this[STATE].phase;
  }

  get defaultPrevented() {
    return this[STATE].canceled;
  }

  get returnValue() {
    return !this[STATE].canceled;
  }

  set returnValue(value) {
    if (!value) this.preventDefault();
  }

  get cancelBubble() {
    return this[STATE].stop;
  }

  set cancelBubble(value) {
    if (value) this[STATE].stop = true;
  }

  get timeStamp() {
    return this[STATE].timeStamp;
  }

  get isTrusted() {
    return this[STATE].trusted;
  }

  preventDefault() {
    const state = this[STATE];
    if (state.cancelable && !state.passive) state.canceled = true;
  }

  stopPropagation() {
    this[STATE].stop = true;
  }

  stopImmediatePropagation() {
    this[STATE].stop = true;
    this[STATE].stopImmediate = true;
  }

  // The path, without the nodes of closed shadow trees the current target
  // is outside of.
  composedPath() {
    const state = this[STATE];
    if (!state.dispatching) return [];
    if (!shadowRootIds.length || !isNode(state.currentTarget)) return state.path.slice();
    return state.path.filter((node) => !isNode(node) || retargetFor(node, state.currentTarget) === node || native.shadowMode(native.root(idOf(node))) === 'open');
  }

  initEvent(type, bubbles = false, cancelable = false) {
    const state = this[STATE];
    if (state.dispatching) return;
    Object.assign(state, {
      type: String(type),
      bubbles: !!bubbles,
      cancelable: !!cancelable,
      stop: false,
      stopImmediate: false,
      canceled: false,
    });
  }
}
for (const [name, value] of [['NONE', 0], ['CAPTURING_PHASE', 1], ['AT_TARGET', 2], ['BUBBLING_PHASE', 3]]) {
  Object.defineProperty(Event, name, { value });
  Object.defineProperty(Event.prototype, name, { value });
}

// Defines event subclasses whose init dictionary members become read-only
// properties with defaults.
const eventClass = (name, parent, defaults) => {
  const constructor = {
    [name]: class extends parent {
      constructor(type, init = {}) {
        super(type, init);
        for (const key of Object.keys(defaults)) {
          hidden(this, `_${key}`, init[key] === undefined ? defaults[key] : init[key]);
        }
      }
    },
  }[name];
  for (const key of Object.keys(defaults)) {
    Object.defineProperty(constructor.prototype, key, {
      get() {
        return this[`_${key}`];
      },
      configurable: true,
      enumerable: true,
    });
  }
  return constructor;
};

const CustomEvent = eventClass('CustomEvent', Event, { detail: null });
class ToggleEvent extends Event {
  constructor(type, init = {}) {
    if (arguments.length === 0) throw new TypeError("Failed to construct 'ToggleEvent': 1 argument required");
    init ??= {};
    super(type, init);
    const source = init.source ?? null;
    if (source !== null && !(source instanceof Element)) throw new TypeError('ToggleEvent source must be an Element');
    hidden(this, '_oldState', init.oldState === undefined ? '' : String(init.oldState));
    hidden(this, '_newState', init.newState === undefined ? '' : String(init.newState));
    hidden(this, '_source', source);
  }
  get oldState() { return this._oldState; }
  get newState() { return this._newState; }
  get source() { return this._source; }
}
Object.defineProperty(ToggleEvent.prototype, Symbol.toStringTag, { value: 'ToggleEvent' });
CustomEvent.prototype.initCustomEvent = function (type, bubbles, cancelable, detail) {
  this.initEvent(type, bubbles, cancelable);
  hidden(this, '_detail', detail);
};
const UIEvent = eventClass('UIEvent', Event, { view: null, detail: 0, which: 0 });
const modifiers = { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false };
const MouseEvent = eventClass('MouseEvent', UIEvent, {
  screenX: 0,
  screenY: 0,
  clientX: 0,
  clientY: 0,
  ...modifiers,
  button: 0,
  buttons: 0,
  relatedTarget: null,
  movementX: 0,
  movementY: 0,
});
for (const [name, source] of [
  ['pageX', 'clientX'],
  ['pageY', 'clientY'],
  ['x', 'clientX'],
  ['y', 'clientY'],
  ['offsetX', 'clientX'],
  ['offsetY', 'clientY'],
]) {
  Object.defineProperty(MouseEvent.prototype, name, {
    get() {
      return this[source];
    },
    configurable: true,
  });
}
// The legacy button number, one more than `button`, which editors such as
// CodeMirror read before `button`.
Object.defineProperty(MouseEvent.prototype, 'which', {
  get() {
    return this.button + 1;
  },
  configurable: true,
  enumerable: true,
});
MouseEvent.prototype.getModifierState = function (key) {
  return { Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[key] || false;
};
const PointerEvent = eventClass('PointerEvent', MouseEvent, {
  pointerId: 1,
  width: 1,
  height: 1,
  pressure: 0,
  tangentialPressure: 0,
  tiltX: 0,
  tiltY: 0,
  twist: 0,
  pointerType: 'mouse',
  isPrimary: true,
});
const WheelEvent = eventClass('WheelEvent', MouseEvent, { deltaX: 0, deltaY: 0, deltaZ: 0, deltaMode: 0 });
const KeyboardEvent = eventClass('KeyboardEvent', UIEvent, {
  key: '',
  code: '',
  location: 0,
  ...modifiers,
  repeat: false,
  isComposing: false,
  charCode: 0,
  keyCode: 0,
});
KeyboardEvent.prototype.getModifierState = MouseEvent.prototype.getModifierState;
const FocusEvent = eventClass('FocusEvent', UIEvent, { relatedTarget: null });
const InputEvent = eventClass('InputEvent', UIEvent, {
  data: null,
  inputType: '',
  isComposing: false,
  dataTransfer: null,
});
const CompositionEvent = eventClass('CompositionEvent', UIEvent, { data: '' });
const ErrorEvent = eventClass('ErrorEvent', Event, { message: '', filename: '', lineno: 0, colno: 0, error: undefined });
const PromiseRejectionEvent = eventClass('PromiseRejectionEvent', Event, { promise: undefined, reason: undefined });
const SubmitEvent = eventClass('SubmitEvent', Event, { submitter: null });
const ProgressEvent = eventClass('ProgressEvent', Event, { lengthComputable: false, loaded: 0, total: 0 });
const MessageEvent = eventClass('MessageEvent', Event, { data: null, origin: '', lastEventId: '', source: null, ports: [] });
const StorageEvent = eventClass('StorageEvent', Event, { key: null, oldValue: null, newValue: null, url: '', storageArea: null });
const PopStateEvent = eventClass('PopStateEvent', Event, { state: null });
const HashChangeEvent = eventClass('HashChangeEvent', Event, { oldURL: '', newURL: '' });
const AnimationEvent = eventClass('AnimationEvent', Event, { animationName: '', elapsedTime: 0, pseudoElement: '' });
const TransitionEvent = eventClass('TransitionEvent', Event, { propertyName: '', elapsedTime: 0, pseudoElement: '' });
const DragEvent = eventClass('DragEvent', MouseEvent, { dataTransfer: null });
const ClipboardEvent = eventClass('ClipboardEvent', Event, { clipboardData: null });

// Each trusted drag event exposes a fresh view of a shared store. Views
// disconnect after dispatch; application-created transfers stay writable.
const transfers = new WeakMap();
const fileLists = new WeakMap();
const dataFormat = (format) => {
  const name = String(format).toLowerCase();
  if (name === 'text') return 'text/plain';
  return name === 'url' ? 'text/uri-list' : name;
};
const transferEntries = (data) => {
  const state = transfers.get(data);
  return state.mode === 'disconnected' ? [] : state.store;
};
const transferReadable = (data) => ['readwrite', 'readonly'].includes(transfers.get(data).mode);
const fileEntries = (data) => transferReadable(data) ? transferEntries(data).filter((entry) => entry.kind === 'file') : [];
const itemActive = (data, entry) => transferEntries(data).includes(entry);
const transferItem = (data, entry) => {
  return {
    get kind() { return itemActive(data, entry) ? entry.kind : ''; },
    get type() { return itemActive(data, entry) ? entry.type : ''; },
    getAsFile() { return transferReadable(data) && itemActive(data, entry) && entry.kind === 'file' ? entry.value : null; },
    getAsString(callback) {
      if (!transferReadable(data) || !itemActive(data, entry) || entry.kind !== 'string' || typeof callback !== 'function') return;
      const value = entry.value;
      setTimeout(() => callback(value), 0);
    },
  };
};

class FileList {
  constructor() { throw new TypeError('Illegal constructor'); }
  get length() { return fileEntries(fileLists.get(this)).length; }
  item(index) { return fileEntries(fileLists.get(this))[Number(index) >>> 0]?.value ?? null; }
  *[Symbol.iterator]() { for (let index = 0; index < this.length; index++) yield this.item(index); }
}
Object.defineProperty(FileList.prototype, Symbol.toStringTag, { value: 'FileList' });
const transferFileList = (data) => {
  const list = Object.create(FileList.prototype);
  const proxy = new Proxy(list, {
    get(target, key, receiver) {
      return typeof key === 'string' && /^(0|[1-9]\d*)$/.test(key) ? receiver.item(Number(key)) ?? undefined : Reflect.get(target, key, receiver);
    },
  });
  fileLists.set(list, data);
  fileLists.set(proxy, data);
  return proxy;
};

class DataTransfer {
  constructor() {
    transfers.set(this, { store: [], mode: 'readwrite', drag: false, dropEffect: 'none', effectAllowed: 'none', items: null, files: null });
  }

  get dropEffect() { return transfers.get(this).dropEffect; }
  set dropEffect(value) { value = String(value); if (transfers.get(this).drag && ['none', 'copy', 'link', 'move'].includes(value)) transfers.get(this).dropEffect = value; }
  get effectAllowed() { return transfers.get(this).effectAllowed; }
  set effectAllowed(value) {
    value = String(value);
    if (transfers.get(this).drag && transfers.get(this).mode === 'readwrite' && ['none', 'copy', 'copyLink', 'copyMove', 'link', 'linkMove', 'move', 'all', 'uninitialized'].includes(value)) transfers.get(this).effectAllowed = value;
  }
  get types() {
    const entries = transferEntries(this);
    return Object.freeze([...entries.filter((entry) => entry.kind === 'string').map((entry) => entry.type), ...(entries.some((entry) => entry.kind === 'file') ? ['Files'] : [])]);
  }

  get files() {
    return transfers.get(this).files ??= transferFileList(this);
  }

  get items() {
    const data = this;
    const entries = () => transferEntries(data);
    return transfers.get(this).items ??= new Proxy({
      get length() { return entries().length; },
      [Symbol.iterator]() { return entries().map((entry) => transferItem(data, entry))[Symbol.iterator](); },
      add(value, type) {
        if (transfers.get(data).mode !== 'readwrite') return null;
        let entry;
        if (value instanceof File) entry = { kind: 'file', type: value.type, value };
        else {
          type = dataFormat(type);
          if (entries().some((entry) => entry.kind === 'string' && entry.type === type)) throw new DOMException('The format already exists.', 'NotSupportedError');
          entry = { kind: 'string', type, value: String(value) };
        }
        entries().push(entry);
        return transferItem(data, entry);
      },
      remove(index) {
        if (transfers.get(data).mode !== 'readwrite') throw new DOMException('The data store is not writable.', 'InvalidStateError');
        entries().splice(Number(index) >>> 0, 1);
      },
      clear() { if (transfers.get(data).mode === 'readwrite') entries().splice(0); },
    }, { get(target, key) { return typeof key === 'string' && /^(0|[1-9]\d*)$/.test(key) ? (entries()[Number(key)] ? transferItem(data, entries()[Number(key)]) : undefined) : Reflect.get(target, key); } });
  }

  getData(format) {
    format = String(format).toLowerCase();
    if (!transferReadable(this)) return '';
    const type = dataFormat(format);
    const value = transferEntries(this).find((entry) => entry.kind === 'string' && entry.type === type)?.value ?? '';
    return format === 'url' ? value.split(/\r?\n/).find((line) => line.trim() && !line.startsWith('#')) ?? '' : value;
  }

  setData(format, data) {
    if (transfers.get(this).mode !== 'readwrite') return;
    const type = dataFormat(format);
    this.clearData(type);
    transferEntries(this).push({ kind: 'string', type, value: String(data) });
  }

  clearData(format) {
    if (transfers.get(this).mode !== 'readwrite') return;
    const type = format === undefined ? null : dataFormat(format);
    const entries = transferEntries(this);
    for (let index = entries.length - 1; index >= 0; index--) {
      if (entries[index].kind === 'string' && (type === null || entries[index].type === type)) entries.splice(index, 1);
    }
  }

  setDragImage() {}
}

// Listener lists by target id and event type.
const listeners = new Map();

const flatten = (options) =>
  typeof options === 'object' && options !== null
    ? { capture: !!options.capture, once: !!options.once, passive: !!options.passive, signal: options.signal }
    : { capture: !!options, once: false, passive: false, signal: undefined };

const listenersOf = (id, type) => {
  let types = listeners.get(id);
  if (!types) listeners.set(id, (types = new Map()));
  let list = types.get(type);
  if (!list) types.set(type, (list = []));
  return list;
};

const addListener = (target, type, callback, options) => {
  if (callback === null || callback === undefined) return;
  const { capture, once, passive, signal } = flatten(options);
  if (signal && signal.aborted) return;
  const list = listenersOf(idOf(target), String(type));
  if (list.some((entry) => entry.callback === callback && entry.capture === capture)) return;
  const entry = { callback, capture, once, passive, removed: false };
  list.push(entry);
  if (signal) signal.addEventListener('abort', () => removeListener(target, type, callback, { capture }));
};

const removeListener = (target, type, callback, options) => {
  const { capture } = flatten(options);
  const types = listeners.get(idOf(target));
  const list = types && types.get(String(type));
  if (!list) return;
  const index = list.findIndex((entry) => entry.callback === callback && entry.capture === capture);
  if (index >= 0) {
    list[index].removed = true;
    list.splice(index, 1);
  }
};

// The next target in the event path: the parent node, the document for the
// root element, and the window for the document (except for `load`).
// The next target of an event's path: a node's assigned slot or parent, a
// shadow root's host unless the event is not composed and started in that
// shadow tree, and the window after the document.
const parentOf = (target, event, origin) => {
  if (target === global) return null;
  if (target === document) return event.type === 'load' ? null : global;
  if (!isNode(target)) return null;
  const id = idOf(target);
  if (shadowRootIds.length) {
    const slot = native.assignedSlot(id);
    if (slot !== null) return wrap(slot);
    const host = native.shadowHost(id);
    if (host !== null) {
      return !event[STATE].composed && isNode(origin) && native.root(idOf(origin)) === id ? null : wrap(host);
    }
  }
  return wrap(native.parent(id));
};

const invoke = (target, event, phase, capture) => {
  const state = event[STATE];
  const types = listeners.get(idOf(target));
  const list = types && types.get(state.type);
  if (!list || list.length === 0) return;
  state.currentTarget = target;
  state.phase = phase;
  for (const entry of list.slice()) {
    if (entry.removed || entry.capture !== capture) continue;
    if (entry.once) removeListener(target, state.type, entry.callback, { capture });
    state.passive = entry.passive;
    try {
      if (typeof entry.callback === 'function') entry.callback.call(target, event);
      else if (entry.callback && typeof entry.callback.handleEvent === 'function') entry.callback.handleEvent(event);
    } catch (error) {
      host.report(error, false);
    }
    state.passive = false;
    if (state.stopImmediate) break;
  }
};

const dispatch = (target, event) => {
  if (!(event instanceof Event)) throw new TypeError('The value is not an Event');
  const state = event[STATE];
  if (state.dispatching) throw new DOMException('The event is already being dispatched.', 'InvalidStateError');
  state.dispatching = true;
  state.target = target;
  const path = [];
  for (let current = target; current; current = parentOf(current, event, target)) path.push(current);
  state.path = path;
  // Listeners outside a shadow tree see its host as the target.
  const targets = shadowRootIds.length ? path.map((node) => retargetFor(target, node)) : null;
  // Where the retargeted target is the listener's own node, the listener
  // runs at the target rather than in capture or bubbling.
  const phaseAt = (index, other) => (index === 0 || (targets && targets[index] === path[index]) ? 2 : other);
  for (let index = path.length - 1; index >= 0 && !state.stop; index--) {
    if (targets) state.target = targets[index];
    invoke(path[index], event, phaseAt(index, 1), true);
  }
  for (let index = 0; index < path.length && !state.stop; index++) {
    if (index > 0 && !state.bubbles && phaseAt(index, 3) !== 2) continue;
    if (targets) state.target = targets[index];
    invoke(path[index], event, phaseAt(index, 3), false);
  }
  // A target left inside a shadow tree is cleared, as browsers do.
  if (targets) {
    const last = targets[targets.length - 1];
    state.target = isNode(last) && native.shadowHost(native.root(idOf(last))) !== null ? null : last;
  }
  state.dispatching = false;
  state.phase = 0;
  state.currentTarget = null;
  state.stop = false;
  state.stopImmediate = false;
  return !state.canceled;
};

// Creates and dispatches an event raised by the platform or a default action.
// Events of user input and focus leave shadow trees, as Chromium's do.
const composedEvents = new Set([
  'auxclick', 'beforeinput', 'blur', 'click', 'compositionend', 'compositionstart', 'compositionupdate',
  'contextmenu', 'copy', 'cut', 'dblclick', 'drag', 'dragend', 'dragenter', 'dragleave', 'dragover',
  'dragstart', 'drop', 'focus', 'focusin', 'focusout', 'input', 'keydown', 'keypress', 'keyup',
  'mousedown', 'mousemove', 'mouseout', 'mouseover', 'mouseup', 'paste', 'pointercancel', 'pointerdown',
  'pointermove', 'pointerout', 'pointerover', 'pointerrawupdate', 'pointerup', 'select', 'selectionchange',
  'touchcancel', 'touchend', 'touchmove', 'touchstart', 'wheel',
]);
const fire = (target, constructor, type, init = {}) => {
  const event = new constructor(type, init);
  event[STATE].trusted = true;
  if (init.composed === undefined && composedEvents.has(type)) event[STATE].composed = true;
  return { event, allowed: dispatch(target, event) };
};

class EventTarget {
  constructor() {
    adopt(native.createTarget(), this);
  }

  addEventListener(type, callback, options) {
    addListener(this, type, callback, options);
  }

  removeEventListener(type, callback, options) {
    removeListener(this, type, callback, options);
  }

  dispatchEvent(event) {
    return dispatch(this, event);
  }
}

// `on<event>` handler properties.
const HANDLERS = Symbol('nativeboat.handlers');
const handlerNames = [
  'abort', 'animationcancel', 'animationend', 'animationiteration', 'animationstart', 'beforeinput', 'blur', 'change', 'click', 'contextmenu',
  'compositionstart', 'compositionupdate', 'compositionend',
  'copy', 'cut', 'dblclick', 'drag', 'dragend', 'dragenter', 'dragleave', 'dragover', 'dragstart',
  'drop', 'error', 'focus', 'focusin', 'focusout', 'input', 'invalid', 'keydown', 'keypress', 'keyup',
  'load', 'mousedown', 'mouseenter', 'mouseleave', 'mousemove', 'mouseout', 'mouseover', 'mouseup',
  'paste', 'pointercancel', 'pointerdown', 'pointerenter', 'pointerleave', 'pointermove', 'pointerout',
  'pointerover', 'pointerup', 'reset', 'resize', 'scroll', 'select', 'submit', 'toggle',
  'transitionrun', 'transitionstart', 'transitioncancel', 'transitionend', 'wheel',
];
const defineHandlers = (prototype, names) => {
  for (const name of names) {
    Object.defineProperty(prototype, `on${name}`, {
      get() {
        return (this[HANDLERS] && this[HANDLERS][name]) || null;
      },
      set(value) {
        if (!own(this, HANDLERS)) hidden(this, HANDLERS, {});
        const handlers = this[HANDLERS];
        if (handlers[`${name}:listener`]) removeListener(this, name, handlers[`${name}:listener`]);
        handlers[name] = typeof value === 'function' ? value : null;
        handlers[`${name}:listener`] = null;
        if (handlers[name]) {
          const listener = (event) => {
            const result = handlers[name].call(this, event);
            if (result === false) event.preventDefault();
          };
          handlers[`${name}:listener`] = listener;
          addListener(this, name, listener);
        }
      },
      configurable: true,
      enumerable: true,
    });
  }
};

class AbortSignal extends EventTarget {
  constructor() {
    illegal();
  }

  get aborted() {
    return this._aborted;
  }

  get reason() {
    return this._reason;
  }

  throwIfAborted() {
    if (this._aborted) throw this._reason;
  }

  static abort(reason) {
    const controller = new AbortController();
    controller.abort(reason);
    return controller.signal;
  }

  static timeout(milliseconds) {
    const controller = new AbortController();
    setTimeout(() => controller.abort(new DOMException('The operation timed out.', 'TimeoutError')), milliseconds);
    return controller.signal;
  }
}
defineHandlers(AbortSignal.prototype, ['abort']);

class AbortController {
  constructor() {
    const signal = Object.create(AbortSignal.prototype);
    adopt(native.createTarget(), signal);
    hidden(signal, '_aborted', false);
    hidden(signal, '_reason', undefined);
    hidden(this, '_signal', signal);
  }

  get signal() {
    return this._signal;
  }

  abort(reason = new DOMException('signal is aborted without reason', 'AbortError')) {
    const signal = this._signal;
    if (signal._aborted) return;
    signal._aborted = true;
    signal._reason = reason;
    fire(signal, Event, 'abort');
  }
}
