// Core of the web prelude: node wrappers, DOMException and collections.
//
// The document lives in Rust; `native` operates on it by node id. Every node
// has one wrapper object, created on first use and cached, so properties that
// libraries attach to nodes (React's fiber links, for example) persist.

const ID = Symbol('nativeboat.node');
const HTML_NAMESPACE = 'http://www.w3.org/1999/xhtml';
const SVG_NAMESPACE = 'http://www.w3.org/2000/svg';
const global = globalThis;
const wrappers = new Map();
// The shadow roots attached so far, by id; until there is one, events and
// focus need no retargeting.
const shadowRootIds = [];
const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
const hidden = (target, name, value) =>
  Object.defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });
const illegal = () => {
  throw new TypeError('Illegal constructor');
};

// Native DOM errors carry the exception name in front of the message.
const call = (operation, ...args) => {
  try {
    return native[operation](...args);
  } catch (error) {
    const match = /^(\w+Error): ([\s\S]*)$/.exec(error && error.message);
    if (match && new DOMException('', match[1]).code !== 0) throw new DOMException(match[2], match[1]);
    throw error;
  }
};

const idOf = (node) => {
  if (node === null || typeof node !== 'object' || !(ID in node)) {
    throw new TypeError('The value is not a node');
  }
  return node[ID];
};

const isNode = (value) => value !== null && typeof value === 'object' && ID in value;

let prototypeOf = () => null;
const wrap = (id) => {
  if (id === null || id === undefined) return null;
  let node = wrappers.get(id);
  if (node) return node;
  node = Object.create(prototypeOf(id));
  Object.defineProperty(node, ID, { value: id });
  wrappers.set(id, node);
  return node;
};
const adopt = (id, object) => {
  Object.defineProperty(object, ID, { value: id });
  wrappers.set(id, object);
  return object;
};

// Collections. ---------------------------------------------------------------

// Collection traps run for every child access. Recognize digit keys without
// allocating and compiling a RegExp for each lookup (including named keys).
const collectionIndex = (key) => {
  if (typeof key !== 'string' || key.length === 0) return null;
  for (let index = 0; index < key.length; index++) {
    const code = key.charCodeAt(index);
    if (code < 48 || code > 57) return null;
  }
  return Number(key);
};
const indexed = (target, read, count = () => read().length, at = (index) => read()[index]) =>
  new Proxy(target, {
    get(object, key, receiver) {
      const index = collectionIndex(key);
      if (index !== null) return at(index) ?? undefined;
      return Reflect.get(object, key, receiver);
    },
    has(object, key) {
      const index = collectionIndex(key);
      if (index !== null) return index < count();
      return Reflect.has(object, key);
    },
    ownKeys(object) {
      return Array.from({ length: count() }, (_, index) => String(index)).concat(Reflect.ownKeys(object));
    },
    getOwnPropertyDescriptor(object, key) {
      const index = collectionIndex(key);
      if (index !== null && index < count()) {
        return { value: at(index), enumerable: true, configurable: true, writable: false };
      }
      return Reflect.getOwnPropertyDescriptor(object, key);
    },
  });

class NodeList {
  constructor() {
    illegal();
  }

  get length() {
    return this[LIST_COUNT]();
  }

  item(index) {
    return this[LIST]()[index] ?? null;
  }

  forEach(callback, thisArg) {
    this[LIST]().forEach((node, index) => callback.call(thisArg, node, index, this));
  }

  entries() {
    return this[LIST]().entries();
  }

  keys() {
    return this[LIST]().keys();
  }

  values() {
    return this[LIST]().values();
  }

  [Symbol.iterator]() {
    return this[LIST]()[Symbol.iterator]();
  }
}
const LIST = Symbol('nativeboat.list');
const LIST_COUNT = Symbol('nativeboat.listCount');

class HTMLCollection {
  constructor() {
    illegal();
  }

  get length() {
    return this[LIST_COUNT]();
  }

  item(index) {
    return this[LIST]()[index] ?? null;
  }

  namedItem(name) {
    return this[LIST]().find((node) => node.id === name || node.getAttribute('name') === name) ?? null;
  }

  [Symbol.iterator]() {
    return this[LIST]()[Symbol.iterator]();
  }
}

// A live (or, for a fixed array, static) list of nodes.
const collection = (constructor, read, count = () => read().length, at = (index) => read()[index]) => {
  const list = Object.create(constructor.prototype);
  hidden(list, LIST, read);
  hidden(list, LIST_COUNT, count);
  return indexed(list, read, count, at);
};
const nodeList = (read, count, at) => collection(NodeList, read, count, at);
const staticNodeList = (nodes) => nodeList(() => nodes);
const htmlCollection = (read, count, at) => collection(HTMLCollection, read, count, at);
const wrapAll = (ids) => ids.map(wrap);

class DOMTokenList {
  constructor() {
    illegal();
  }

  get [Symbol.toStringTag]() {
    return 'DOMTokenList';
  }

  get length() {
    return this[TOKENS]().length;
  }

  get value() {
    return this[ELEMENT].getAttribute(this[ATTRIBUTE]) ?? '';
  }

  set value(text) {
    this[ELEMENT].setAttribute(this[ATTRIBUTE], text);
  }

  item(index) {
    return this[TOKENS]()[index] ?? null;
  }

  contains(token) {
    return this[TOKENS]().includes(String(token));
  }

  add(...tokens) {
    const current = this[TOKENS]();
    for (const token of tokens.map(checkToken)) if (!current.includes(token)) current.push(token);
    this[ELEMENT].setAttribute(this[ATTRIBUTE], current.join(' '));
  }

  remove(...tokens) {
    const removed = tokens.map(checkToken);
    const current = this[TOKENS]().filter((token) => !removed.includes(token));
    this[ELEMENT].setAttribute(this[ATTRIBUTE], current.join(' '));
  }

  toggle(token, force) {
    const name = checkToken(token);
    const present = this.contains(name);
    const wanted = force === undefined ? !present : !!force;
    if (wanted && !present) this.add(name);
    if (!wanted && present) this.remove(name);
    return wanted;
  }

  replace(token, replacement) {
    const current = this[TOKENS]();
    const index = current.indexOf(checkToken(token));
    if (index < 0) return false;
    current[index] = checkToken(replacement);
    this[ELEMENT].setAttribute(this[ATTRIBUTE], [...new Set(current)].join(' '));
    return true;
  }

  supports() {
    return true;
  }

  toString() {
    return this.value;
  }

  forEach(callback, thisArg) {
    this[TOKENS]().forEach((token, index) => callback.call(thisArg, token, index, this));
  }

  [Symbol.iterator]() {
    return this[TOKENS]()[Symbol.iterator]();
  }
}
const ELEMENT = Symbol('nativeboat.element');
const ATTRIBUTE = Symbol('nativeboat.attribute');
const TOKENS = Symbol('nativeboat.tokens');
const checkToken = (token) => {
  const text = String(token);
  if (text === '') throw new DOMException('The token must not be empty.', 'SyntaxError');
  if (/\s/.test(text)) throw new DOMException('The token must not contain whitespace.', 'InvalidCharacterError');
  return text;
};
const tokenLists = new WeakMap();
const tokenList = (element, attribute) => {
  const key = `${attribute}`;
  let lists = tokenLists.get(element);
  if (!lists) tokenLists.set(element, (lists = {}));
  if (!lists[key]) {
    const list = Object.create(DOMTokenList.prototype);
    hidden(list, ELEMENT, element);
    hidden(list, ATTRIBUTE, attribute);
    hidden(list, TOKENS, () => (element.getAttribute(attribute) ?? '').split(/\s+/).filter(Boolean));
    lists[key] = indexed(list, list[TOKENS]);
  }
  return lists[key];
};

class Attr {
  constructor() {
    illegal();
  }
}
const attribute = (element, name, value) => {
  const attr = Object.create(Attr.prototype);
  Object.assign(attr, {
    name,
    localName: name,
    value,
    nodeName: name,
    nodeValue: value,
    namespaceURI: null,
    prefix: null,
    ownerElement: element,
    specified: true,
  });
  return attr;
};

class NamedNodeMap {
  constructor() {
    illegal();
  }

  get length() {
    return this[LIST]().length;
  }

  item(index) {
    return this[LIST]()[index] ?? null;
  }

  getNamedItem(name) {
    return this[LIST]().find((attr) => attr.name === String(name).toLowerCase()) ?? null;
  }

  [Symbol.iterator]() {
    return this[LIST]()[Symbol.iterator]();
  }
}
const namedNodeMap = (element) =>
  collection(NamedNodeMap, () =>
    native.attributes(idOf(element)).map(([name, value]) => attribute(element, name, value)),
  );

class DOMRectReadOnly {
  constructor(x = 0, y = 0, width = 0, height = 0) {
    Object.assign(this, { x: +x, y: +y, width: +width, height: +height });
  }

  get top() {
    return Math.min(this.y, this.y + this.height);
  }

  get bottom() {
    return Math.max(this.y, this.y + this.height);
  }

  get left() {
    return Math.min(this.x, this.x + this.width);
  }

  get right() {
    return Math.max(this.x, this.x + this.width);
  }

  toJSON() {
    const { x, y, width, height, top, right, bottom, left } = this;
    return { x, y, width, height, top, right, bottom, left };
  }

  static fromRect(rect = {}) {
    return new this(rect.x, rect.y, rect.width, rect.height);
  }
}
class DOMRect extends DOMRectReadOnly {}

class DOMRectList {
  constructor() {
    illegal();
  }

  get length() {
    return this[LIST]().length;
  }

  item(index) {
    return this[LIST]()[index] ?? null;
  }

  [Symbol.iterator]() {
    return this[LIST]()[Symbol.iterator]();
  }
}
