// Nodes: Node, CharacterData, Element, Document. -----------------------------

const nodeTypes = {
  ELEMENT_NODE: 1,
  ATTRIBUTE_NODE: 2,
  TEXT_NODE: 3,
  CDATA_SECTION_NODE: 4,
  PROCESSING_INSTRUCTION_NODE: 7,
  COMMENT_NODE: 8,
  DOCUMENT_NODE: 9,
  DOCUMENT_TYPE_NODE: 10,
  DOCUMENT_FRAGMENT_NODE: 11,
};
const positions = {
  DOCUMENT_POSITION_DISCONNECTED: 1,
  DOCUMENT_POSITION_PRECEDING: 2,
  DOCUMENT_POSITION_FOLLOWING: 4,
  DOCUMENT_POSITION_CONTAINS: 8,
  DOCUMENT_POSITION_CONTAINED_BY: 16,
  DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: 32,
};
const documentTypes = new WeakMap(), implementations = new WeakMap();

// Script elements a page inserts load their source once they are in the
// document, as browsers run them: the page's own compiled scripts and the
// package scripts it may load are found by URL, and the element gets `load`
// when the script has run, or `error` when it is not compiled or throws.
// Scripts parsed from markup do not run, as in browsers.
const startedScripts = new WeakSet();
const scriptTypes = /^(?:(?:text|application)\/(?:java|ecma)script|module)$/i;
const startScripts = (node) => {
  if (!node || !node.isConnected) return;
  const scripts = node.localName === 'script' ? [node] : typeof node.querySelectorAll === 'function' ? Array.from(node.querySelectorAll('script')) : [];
  for (const script of scripts) {
    const src = script.getAttribute('src');
    const type = script.getAttribute('type');
    if (startedScripts.has(script) || src === null || (type && type.trim() && !scriptTypes.test(type.trim()))) continue;
    startedScripts.add(script);
    let loading;
    try {
      loading = page.loadScript(new URL(src, document.baseURI).href);
    } catch (error) {
      loading = Promise.reject(error);
    }
    Promise.resolve(loading).then(
      () => script.dispatchEvent(new Event('load')),
      () => script.dispatchEvent(new Event('error')),
    );
  }
};
const CHILD_NODES = Symbol('nativeboat.childNodes'), ELEMENT_CHILDREN = Symbol('nativeboat.children');
const ownTree = (node, owner) => {
  native.setOwnerDocument(node[ID], owner[ID]);
  return node;
};
const detachedHTMLDocument = (source) => {
  const doc = wrap(native.parseDocument(String(source)));
  documentTypes.set(doc, 'text/html');
  return doc;
};

// Converts the arguments of append(), before() and friends into one node.
const nodesToNode = (nodes) => {
  if (nodes.length === 1 && isNode(nodes[0])) return nodes[0];
  const fragment = wrap(native.createFragment());
  for (const node of nodes) {
    fragment.appendChild(isNode(node) ? node : wrap(native.createText(String(node))));
  }
  return fragment;
};

const ancestors = (node) => {
  const chain = [];
  for (let current = node; current; current = current.parentNode) chain.unshift(current);
  return chain;
};

class Node extends EventTarget {
  constructor() {
    illegal();
  }

  get nodeType() {
    return native.kind(this[ID]);
  }

  get nodeName() {
    return native.name(this[ID]);
  }

  get nodeValue() {
    return native.data(this[ID]);
  }

  set nodeValue(value) {
    if (native.data(this[ID]) !== null) native.setData(this[ID], value === null ? '' : String(value));
  }

  get textContent() {
    return this[ID] === 0 ? null : native.text(this[ID]);
  }

  set textContent(value) {
    if (this[ID] !== 0) native.setText(this[ID], value === null || value === undefined ? '' : String(value));
  }

  get parentNode() {
    return wrap(native.parent(this[ID]));
  }

  get parentElement() {
    return wrap(native.parentElement(this[ID]));
  }

  get childNodes() {
    if (!this[CHILD_NODES]) hidden(this, CHILD_NODES, nodeList(
      () => wrapAll(native.children(this[ID])),
      () => native.childCount(this[ID]),
      (index) => wrap(native.childAt(this[ID], index)),
    ));
    return this[CHILD_NODES];
  }

  get firstChild() {
    return wrap(native.firstChild(this[ID]));
  }

  get lastChild() {
    return wrap(native.lastChild(this[ID]));
  }

  get previousSibling() {
    return wrap(native.previous(this[ID]));
  }

  get nextSibling() {
    return wrap(native.next(this[ID]));
  }

  get ownerDocument() {
    return wrap(native.ownerDocument(this[ID]));
  }

  get isConnected() {
    return native.connected(this[ID]);
  }

  get baseURI() {
    return location.href;
  }

  hasChildNodes() {
    return native.firstChild(this[ID]) !== null;
  }

  appendChild(child) {
    const inserted = child.nodeType === 11 ? Array.from(child.childNodes) : [child];
    call('insertBefore', this[ID], idOf(child), null);
    for (const node of inserted) startScripts(node);
    return child;
  }

  insertBefore(child, reference) {
    const inserted = child.nodeType === 11 ? Array.from(child.childNodes) : [child];
    call('insertBefore', this[ID], idOf(child), reference === null || reference === undefined ? null : idOf(reference));
    for (const node of inserted) startScripts(node);
    return child;
  }

  removeChild(child) {
    call('removeChild', this[ID], idOf(child));
    return child;
  }

  replaceChild(replacement, child) {
    call('replaceChild', this[ID], idOf(replacement), idOf(child));
    return child;
  }

  cloneNode(deep = false) {
    const clone = wrap(native.clone(this[ID], !!deep));
    if (this.nodeType === 9) documentTypes.set(clone, this.contentType);
    return clone;
  }

  contains(other) {
    return isNode(other) && native.contains(this[ID], other[ID]);
  }

  getRootNode(options) {
    return wrap(options && options.composed ? native.rootNode(this[ID], true) : native.root(this[ID]));
  }

  isSameNode(other) {
    return this === other;
  }

  isEqualNode(other) {
    return isNode(other) && this.nodeType === other.nodeType && (this.outerHTML ?? this.textContent) === (other.outerHTML ?? other.textContent);
  }

  compareDocumentPosition(other) {
    if (other === this) return 0;
    const mine = ancestors(this);
    const theirs = ancestors(other);
    if (mine[0] !== theirs[0]) {
      return positions.DOCUMENT_POSITION_DISCONNECTED | positions.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC | positions.DOCUMENT_POSITION_PRECEDING;
    }
    if (mine.includes(other)) return positions.DOCUMENT_POSITION_CONTAINS | positions.DOCUMENT_POSITION_PRECEDING;
    if (theirs.includes(this)) return positions.DOCUMENT_POSITION_CONTAINED_BY | positions.DOCUMENT_POSITION_FOLLOWING;
    let depth = 0;
    while (mine[depth] === theirs[depth]) depth++;
    const siblings = native.children(mine[depth - 1][ID]);
    return siblings.indexOf(mine[depth][ID]) < siblings.indexOf(theirs[depth][ID])
      ? positions.DOCUMENT_POSITION_FOLLOWING
      : positions.DOCUMENT_POSITION_PRECEDING;
  }

  normalize() {
    native.normalize(this[ID]);
  }

  lookupNamespaceURI() {
    return null;
  }
}
for (const [name, value] of Object.entries({ ...nodeTypes, ...positions })) {
  Object.defineProperty(Node, name, { value });
  Object.defineProperty(Node.prototype, name, { value });
}

const childNodeMethods = {
  before(...nodes) {
    const parent = this.parentNode;
    if (parent) parent.insertBefore(nodesToNode(nodes), this);
  },
  after(...nodes) {
    const parent = this.parentNode;
    if (parent) parent.insertBefore(nodesToNode(nodes), this.nextSibling);
  },
  replaceWith(...nodes) {
    const parent = this.parentNode;
    if (parent) parent.replaceChild(nodesToNode(nodes), this);
  },
  remove() {
    native.remove(this[ID]);
  },
};
const parentNodeMethods = {
  append(...nodes) {
    this.appendChild(nodesToNode(nodes));
  },
  prepend(...nodes) {
    this.insertBefore(nodesToNode(nodes), this.firstChild);
  },
  replaceChildren(...nodes) {
    native.setText(this[ID], '');
    if (nodes.length) this.appendChild(nodesToNode(nodes));
  },
  querySelector(selectors) {
    return wrap(call('querySelector', this[ID], String(selectors)));
  },
  querySelectorAll(selectors) {
    return staticNodeList(wrapAll(call('querySelectorAll', this[ID], String(selectors))));
  },
  getElementsByTagName(name) {
    return htmlCollection(() => wrapAll(native.byTag(this[ID], String(name))));
  },
  getElementsByClassName(names) {
    return htmlCollection(() => wrapAll(native.byClass(this[ID], String(names))));
  },
};
const parentNodeAccessors = {
  children() {
    if (!this[ELEMENT_CHILDREN]) hidden(this, ELEMENT_CHILDREN, htmlCollection(
      () => wrapAll(native.elementChildren(this[ID])),
      () => native.elementChildCount(this[ID]),
      (index) => wrap(native.elementChildAt(this[ID], index)),
    ));
    return this[ELEMENT_CHILDREN];
  },
  childElementCount() {
    return native.elementChildCount(this[ID]);
  },
  firstElementChild() {
    return wrap(native.elementChildAt(this[ID], 0));
  },
  lastElementChild() {
    return wrap(native.elementChildAt(this[ID], native.elementChildCount(this[ID]) - 1));
  },
};
const mixin = (prototype, methods, accessors = {}) => {
  for (const [name, method] of Object.entries(methods)) hidden(prototype, name, method);
  for (const [name, get] of Object.entries(accessors)) {
    const existing = Object.getOwnPropertyDescriptor(prototype, name);
    Object.defineProperty(prototype, name, { get, set: existing && existing.set, configurable: true });
  }
};
const siblingElement = (node, step) => {
  return wrap(native[step === 'next' ? 'nextElement' : 'previousElement'](node[ID]));
};

class CharacterData extends Node {
  constructor() {
    illegal();
  }

  get data() {
    return native.data(this[ID]);
  }

  set data(value) {
    native.setData(this[ID], String(value));
  }

  get length() {
    return this.data.length;
  }

  get nextElementSibling() {
    return siblingElement(this, 'next');
  }

  get previousElementSibling() {
    return siblingElement(this, 'previous');
  }

  appendData(text) {
    this.replaceData(this.length, 0, text);
  }

  substringData(offset, count) {
    const data = this.data;
    offset >>>= 0;
    if (offset > data.length) throw new DOMException('The offset exceeds the length of the data.', 'IndexSizeError');
    return data.substr(offset, count >>> 0);
  }

  insertData(offset, text) {
    this.replaceData(offset, 0, text);
  }

  deleteData(offset, count) {
    this.replaceData(offset, count, '');
  }

  replaceData(offset, count, text) {
    call('replaceData', this[ID], offset >>> 0, count >>> 0, String(text));
  }
}
mixin(CharacterData.prototype, childNodeMethods);

class Text extends CharacterData {
  constructor(data = '') {
    return wrap(native.createText(String(data)));
  }

  get wholeText() {
    let first = this;
    while (first.previousSibling && first.previousSibling.nodeType === 3) first = first.previousSibling;
    let text = '';
    for (let node = first; node && node.nodeType === 3; node = node.nextSibling) text += node.data;
    return text;
  }

  splitText(offset) {
    return wrap(call('splitText', this[ID], offset >>> 0));
  }
}

class Comment extends CharacterData {
  constructor(data = '') {
    return wrap(native.createComment(String(data)));
  }
}

class DocumentFragment extends Node {
  constructor() {
    return wrap(native.createFragment());
  }

  getElementById(name) {
    return wrap(native.byId(this[ID], String(name)));
  }
}
mixin(DocumentFragment.prototype, parentNodeMethods, parentNodeAccessors);

// Element. -------------------------------------------------------------------

const insertAdjacent = (element, position, node) => {
  switch (String(position).toLowerCase()) {
    case 'beforebegin':
      if (!element.parentNode) return null;
      element.parentNode.insertBefore(node, element);
      return node;
    case 'afterbegin':
      element.insertBefore(node, element.firstChild);
      return node;
    case 'beforeend':
      element.appendChild(node);
      return node;
    case 'afterend':
      if (!element.parentNode) return null;
      element.parentNode.insertBefore(node, element.nextSibling);
      return node;
    default:
      throw new DOMException(`'${position}' is not a valid position.`, 'SyntaxError');
  }
};

const rectOf = (element) => {
  const rect = page.rect(element[ID]);
  return rect ? new DOMRect(rect[0], rect[1], rect[2], rect[3]) : new DOMRect();
};

// Scripts scroll within the scroll range of a scroll container or the
// viewport, as Chromium clamps them; other elements do not scroll.
const applyScroll = (element, left, top) => {
  const id = element[ID];
  const size = page.scrollSize(id);
  const client = page.client(id);
  const scrolls = id === native.documentElement() ||
    ['overflow-x', 'overflow-y'].some((name) => ['auto', 'scroll', 'hidden'].includes(styleValue(element, name)));
  const range = (extent, shown) => (scrolls && extent !== undefined && shown !== undefined ? Math.max(0, extent - shown) : 0);
  const within = (value, most) => Math.min(Math.max(0, Number.isFinite(value) ? value : 0), most);
  const x = within(left, range(size?.[0], client?.[2]));
  const y = within(top, range(size?.[1], client?.[3]));
  // Input tracing names the scripted scrolls too, which move what the user
  // sees as input does.
  if (native.tracing()) {
    const [fromX, fromY] = native.scroll(id);
    if (x !== fromX || y !== fromY) native.trace(`script scrolls <${element.localName} class="${element.getAttribute('class') || ''}"> from ${fromX},${fromY} to ${x},${y}`);
  }
  native.setScroll(id, x, y);
};

const scrollOptions = (element, args, relative) => {
  const [first, second] = args;
  const [left, top] = native.scroll(element[ID]);
  let x = left;
  let y = top;
  if (typeof first === 'object' && first !== null) {
    if (first.left !== undefined) x = Number(first.left) + (relative ? left : 0);
    if (first.top !== undefined) y = Number(first.top) + (relative ? top : 0);
  } else if (args.length) {
    x = Number(first) + (relative ? left : 0);
    y = Number(second) + (relative ? top : 0);
  }
  return [x, y];
};

class Element extends Node {
  constructor() {
    illegal();
  }

  get tagName() {
    return native.name(this[ID]);
  }

  get localName() {
    return native.localName(this[ID]);
  }

  get namespaceURI() {
    return native.namespace(this[ID]);
  }

  get prefix() {
    return null;
  }

  get id() {
    return this.getAttribute('id') ?? '';
  }

  set id(value) {
    this.setAttribute('id', value);
  }

  get className() {
    return this.getAttribute('class') ?? '';
  }

  set className(value) {
    this.setAttribute('class', value);
  }

  get classList() {
    return tokenList(this, 'class');
  }

  get slot() {
    return this.getAttribute('slot') ?? '';
  }

  get attributes() {
    return namedNodeMap(this);
  }

  getAttribute(name) {
    return native.getAttribute(this[ID], String(name));
  }

  getAttributeNS(namespace, name) {
    return this.getAttribute(name);
  }

  getAttributeNode(name) {
    const value = this.getAttribute(name);
    return value === null ? null : attribute(this, String(name).toLowerCase(), value);
  }

  setAttribute(name, value) {
    call('setAttribute', this[ID], String(name), String(value));
    // Handler attributes the compiler found in the page's HTML: the same
    // text set later gets the same compiled handler.
    const key = String(name).toLowerCase();
    const handler = key.startsWith('on') && globalThis.__nativeboatHandlers?.get(`${key}\u0000${value}`);
    if (handler && key in this) this[key] = handler;
  }

  setAttributeNS(namespace, name, value) {
    this.setAttribute(name, value);
  }

  removeAttribute(name) {
    native.removeAttribute(this[ID], String(name));
    const key = String(name).toLowerCase();
    if (key.startsWith('on') && globalThis.__nativeboatHandlers && key in this) this[key] = null;
  }

  removeAttributeNS(namespace, name) {
    this.removeAttribute(name);
  }

  hasAttribute(name) {
    return this.getAttribute(name) !== null;
  }

  hasAttributeNS(namespace, name) {
    return this.hasAttribute(name);
  }

  hasAttributes() {
    return native.attributes(this[ID]).length > 0;
  }

  getAttributeNames() {
    return native.attributes(this[ID]).map(([name]) => name);
  }

  toggleAttribute(name, force) {
    return call('toggleAttribute', this[ID], String(name), force);
  }

  get innerHTML() {
    return native.innerHTML(this[ID]);
  }

  set innerHTML(html) {
    const markup = html === null ? '' : String(html);
    native.setInnerHTML(this[ID], markup);
    parsedMarkup(markup, this);
  }

  get outerHTML() {
    return native.outerHTML(this[ID]);
  }

  set outerHTML(html) {
    const parent = this.parentNode;
    if (!parent) return;
    const context = parent.nodeType === 1 ? parent[ID] : native.createElement('body');
    const fragment = wrap(native.parseFragment(String(html), context));
    parsedMarkup(String(html), fragment);
    parent.replaceChild(fragment, this);
  }

  insertAdjacentElement(position, element) {
    return insertAdjacent(this, position, element);
  }

  insertAdjacentText(position, text) {
    insertAdjacent(this, position, wrap(native.createText(String(text))));
  }

  insertAdjacentHTML(position, html) {
    const where = String(position).toLowerCase();
    const parent = where === 'beforebegin' || where === 'afterend' ? this.parentNode : this;
    const context = parent && parent.nodeType === 1 ? parent[ID] : this[ID];
    const fragment = wrap(native.parseFragment(String(html), context));
    parsedMarkup(String(html), fragment);
    insertAdjacent(this, position, fragment);
  }

  get nextElementSibling() {
    return siblingElement(this, 'next');
  }

  get previousElementSibling() {
    return siblingElement(this, 'previous');
  }

  matches(selectors) {
    return call('matches', this[ID], String(selectors));
  }

  webkitMatchesSelector(selectors) {
    return this.matches(selectors);
  }

  closest(selectors) {
    return wrap(call('closest', this[ID], String(selectors)));
  }

  getBoundingClientRect() {
    return rectOf(this);
  }

  getClientRects() {
    const rect = page.rect(this[ID]);
    if (native.tracing()) {
      const shown = rect ? `${Math.round(rect[0])},${Math.round(rect[1])},${Math.round(rect[2])}x${Math.round(rect[3])}` : '';
      native.trace(`element rects <${this.localName} class="${this.getAttribute('class') || ''}"> [${shown}]`);
    }
    return collection(DOMRectList, () => (rect ? [rectOf(this)] : []));
  }

  // Boxes lie at fractions of a pixel; these sizes, as Chromium gives
  // them, are whole pixels.
  get clientWidth() {
    return Math.round((page.client(this[ID]) || [0, 0, 0, 0])[2]);
  }

  get clientHeight() {
    return Math.round((page.client(this[ID]) || [0, 0, 0, 0])[3]);
  }

  get clientLeft() {
    return Math.round((page.client(this[ID]) || [0, 0, 0, 0])[0]);
  }

  get clientTop() {
    return Math.round((page.client(this[ID]) || [0, 0, 0, 0])[1]);
  }

  get scrollWidth() {
    return Math.round((page.scrollSize(this[ID]) || [0, 0])[0]);
  }

  get scrollHeight() {
    return Math.round((page.scrollSize(this[ID]) || [0, 0])[1]);
  }

  get scrollLeft() {
    return native.scroll(this[ID])[0];
  }

  set scrollLeft(value) {
    applyScroll(this, Number(value), native.scroll(this[ID])[1]);
  }

  get scrollTop() {
    return native.scroll(this[ID])[1];
  }

  set scrollTop(value) {
    applyScroll(this, native.scroll(this[ID])[0], Number(value));
  }

  scrollTo(...args) {
    applyScroll(this, ...scrollOptions(this, args, false));
  }

  scroll(...args) {
    this.scrollTo(...args);
  }

  scrollBy(...args) {
    applyScroll(this, ...scrollOptions(this, args, true));
  }

  scrollIntoView(options = true) {
    const [block, inline] =
      options === false ? ['end', 'nearest'] : options !== null && typeof options === 'object' ? [options.block ?? 'start', options.inline ?? 'nearest'] : ['start', 'nearest'];
    page.scrollIntoView(this[ID], String(block), String(inline));
  }

  scrollIntoViewIfNeeded() {
    page.scrollIntoView(this[ID], 'nearest', 'nearest');
  }

  setPointerCapture() {}

  releasePointerCapture() {}

  hasPointerCapture() {
    return false;
  }

  attachShadow(init) {
    return attachShadow(this, init);
  }

  requestFullscreen() {
    return Promise.reject(new DOMException('Fullscreen is not supported.', 'NotSupportedError'));
  }

  animate() {
    const animation = new EventTarget();
    Object.assign(animation, {
      finished: Promise.resolve(animation),
      ready: Promise.resolve(animation),
      playState: 'finished',
      cancel() {},
      finish() {},
      pause() {},
      play() {},
      reverse() {},
    });
    return animation;
  }

  getAnimations() {
    return [];
  }
}
mixin(Element.prototype, { ...childNodeMethods, ...parentNodeMethods }, parentNodeAccessors);

// Document. -------------------------------------------------------------------

class Document extends Node {
  constructor() {
    illegal();
  }

  get documentElement() {
    return this[ID] === 0 ? wrap(native.documentElement()) : this.firstElementChild;
  }

  get head() {
    if (this[ID] === 0) return wrap(native.htmlChild('head'));
    return [...(this.documentElement?.children || [])].find(node => node.localName === 'head') || null;
  }

  get body() {
    if (this[ID] === 0) return wrap(native.htmlChild('body'));
    return [...(this.documentElement?.children || [])].find(node => node.localName === 'body' || node.localName === 'frameset') || null;
  }

  get title() {
    const title = this.querySelector('title');
    return title ? title.textContent.replace(/\s+/g, ' ').trim() : '';
  }

  set title(value) {
    let title = this.querySelector('title');
    if (!title) {
      title = this.createElement('title');
      if (this.head) this.head.appendChild(title);
    }
    title.textContent = String(value);
    if (this[ID] === 0) host.title(this.title);
  }

  get defaultView() {
    return this[ID] === 0 ? global : null;
  }

  get readyState() {
    return readyState;
  }

  get visibilityState() {
    return 'visible';
  }

  get hidden() {
    return false;
  }

  get characterSet() {
    return 'UTF-8';
  }

  get charset() {
    return 'UTF-8';
  }

  get contentType() {
    return documentTypes.get(this) || 'text/html';
  }

  get compatMode() {
    return 'CSS1Compat';
  }

  get documentURI() {
    return location.href;
  }

  get URL() {
    return location.href;
  }

  get location() {
    return location;
  }

  get referrer() {
    return '';
  }

  get domain() {
    return location.hostname;
  }

  get cookie() {
    return cookies;
  }

  set cookie(value) {
    const [pair] = String(value).split(';');
    const [name] = pair.split('=');
    const kept = cookies
      .split('; ')
      .filter((entry) => entry && entry.split('=')[0] !== name.trim());
    if (!/max-age=0|expires=thu, 01 jan 1970/i.test(String(value))) kept.push(pair.trim());
    cookies = kept.join('; ');
  }

  // Focus in a shadow tree is its host's, as the document sees it.
  get activeElement() {
    return this[ID] === 0 ? retargetFor(wrap(native.focused()), this) || this.body : null;
  }

  get scrollingElement() {
    return this.documentElement;
  }

  get forms() {
    return this.getElementsByTagName('form');
  }

  get images() {
    return this.getElementsByTagName('img');
  }

  get links() {
    return htmlCollection(() => [...this.querySelectorAll('a[href], area[href]')]);
  }

  get scripts() {
    return this.getElementsByTagName('script');
  }

  get styleSheets() {
    return styleSheetsOf();
  }

  get fonts() {
    return fonts;
  }

  get implementation() {
    let implementation = implementations.get(this);
    if (implementation) return implementation;
    implementation = {
      hasFeature: () => true,
      createHTMLDocument(title) {
        const doc = detachedHTMLDocument('');
        if (title !== undefined) doc.title = String(title);
        return doc;
      },
      createDocument(namespace, name, doctype = null) {
        if (doctype !== null) throw new DOMException('DocumentType nodes are not supported', 'NotSupportedError');
        const doc = wrap(native.clone(0, false));
        documentTypes.set(doc, 'application/xml');
        if (name != null && String(name) !== '') doc.appendChild(doc.createElementNS(namespace, String(name)));
        return doc;
      },
    };
    implementations.set(this, implementation);
    return implementation;
  }

  hasFocus() {
    return true;
  }

  createElement(name, options) {
    const is = options && typeof options === 'object' ? options.is : undefined;
    const custom = this === document ? createCustomElement(name, is) : null;
    if (custom) return custom;
    const element = ownTree(wrap(call('createElement', String(name))), this);
    if (is) element.setAttribute('is', is);
    return element;
  }

  createElementNS(namespace, name) {
    return ownTree(wrap(call('createElementNS', namespace === null ? '' : String(namespace), String(name))), this);
  }

  createTextNode(text) {
    return ownTree(wrap(native.createText(String(text))), this);
  }

  createComment(text) {
    return ownTree(wrap(native.createComment(String(text))), this);
  }

  createDocumentFragment() {
    return ownTree(wrap(native.createFragment()), this);
  }

  createEvent(name) {
    const constructors = {
      event: Event,
      events: Event,
      htmlevents: Event,
      customevent: CustomEvent,
      uievent: UIEvent,
      uievents: UIEvent,
      mouseevent: MouseEvent,
      mouseevents: MouseEvent,
      keyboardevent: KeyboardEvent,
      focusevent: FocusEvent,
    };
    const constructor = constructors[String(name).toLowerCase()];
    if (!constructor) throw new DOMException(`The event type '${name}' is not supported.`, 'NotSupportedError');
    return new constructor('');
  }

  createRange() {
    return new Range();
  }

  createTreeWalker(root, whatToShow = 0xffffffff, filter = null) {
    return new TreeWalker(root, whatToShow, filter);
  }

  createNodeIterator(root, whatToShow = 0xffffffff, filter = null) {
    return new NodeIterator(root, whatToShow, filter);
  }

  importNode(node, deep = false) {
    return ownTree(node.cloneNode(deep), this);
  }

  adoptNode(node) {
    if (node.parentNode) node.parentNode.removeChild(node);
    return ownTree(node, this);
  }

  getElementById(name) {
    return wrap(native.byId(this[ID], String(name)));
  }

  getElementsByName(name) {
    return staticNodeList([...this.querySelectorAll('[name]')].filter((element) => element.getAttribute('name') === String(name)));
  }

  elementFromPoint(x, y) {
    return retargetFor(wrap(page.elementFromPoint(Number(x), Number(y))), this);
  }

  elementsFromPoint(x, y) {
    const element = this.elementFromPoint(x, y);
    return element ? ancestors(element).filter((node) => node.nodeType === 1).reverse() : [];
  }

  caretPositionFromPoint(x, y) {
    return caretPosition(x, y);
  }

  caretRangeFromPoint(x, y) {
    const point = caretPoint(x, y);
    return point && createRange(point);
  }

  getSelection() {
    return selection;
  }

  execCommand(command, showUI, value) {
    const run = editingCommand(command);
    return run ? run(value) : false;
  }

  queryCommandSupported(command) {
    return editingCommand(command) !== null;
  }

  queryCommandEnabled(command) {
    return editingCommand(command) !== null;
  }

  queryCommandState() {
    return false;
  }

  queryCommandValue() {
    return '';
  }

  open() {
    return this;
  }

  close() {}

  write() {}

  writeln() {}
}
mixin(Document.prototype, parentNodeMethods, parentNodeAccessors);

let readyState = 'loading';
let cookies = '';
const document = adopt(0, Object.create(Document.prototype));

class TreeWalker {
  constructor(root, whatToShow, filter) {
    Object.assign(this, { root, whatToShow, filter, currentNode: root });
  }

  accepts(node) {
    const shown = (this.whatToShow >> (node.nodeType - 1)) & 1;
    if (!shown) return false;
    if (!this.filter) return true;
    const filter = typeof this.filter === 'function' ? this.filter : this.filter.acceptNode.bind(this.filter);
    return filter(node) === 1;
  }

  nextNode() {
    let node = this.currentNode;
    for (;;) {
      if (node.firstChild) node = node.firstChild;
      else {
        while (node !== this.root && !node.nextSibling) node = node.parentNode;
        if (node === this.root) return null;
        node = node.nextSibling;
      }
      if (this.accepts(node)) return (this.currentNode = node);
    }
  }

  parentNode() {
    let node = this.currentNode;
    while (node !== this.root && (node = node.parentNode)) {
      if (this.accepts(node)) return (this.currentNode = node);
    }
    return null;
  }

  firstChild() {
    for (let node = this.currentNode.firstChild; node; node = node.nextSibling) {
      if (this.accepts(node)) return (this.currentNode = node);
    }
    return null;
  }

  nextSibling() {
    for (let node = this.currentNode.nextSibling; node; node = node.nextSibling) {
      if (this.accepts(node)) return (this.currentNode = node);
    }
    return null;
  }
}

// A node iterator includes its root and treats FILTER_REJECT as FILTER_SKIP.
// Refresh the tree between calls so inserted nodes are visible; when its
// reference was removed, continue after the closest surviving predecessor.
class NodeIterator {
  constructor(root, whatToShow, filter) {
    if (!isNode(root)) throw new TypeError('The root is not a node');
    Object.assign(this, { root, whatToShow: Number(whatToShow) >>> 0, filter,
      referenceNode: root, pointerBeforeReferenceNode: true, previousTree: [root] });
  }
  step(forward) {
    const nodes = [];
    const visit = node => { nodes.push(node); for (const child of node.childNodes) visit(child); };
    visit(this.root);
    let index = nodes.indexOf(this.referenceNode);
    if (index < 0) {
      const before = this.previousTree.slice(0, this.previousTree.indexOf(this.referenceNode));
      this.referenceNode = before.reverse().find(node => nodes.includes(node)) || this.root;
      index = nodes.indexOf(this.referenceNode);
      this.pointerBeforeReferenceNode = false;
    }
    const start = forward ? index + (this.pointerBeforeReferenceNode ? 0 : 1)
      : index - (this.pointerBeforeReferenceNode ? 1 : 0);
    for (let offset = start; offset >= 0 && offset < nodes.length; offset += forward ? 1 : -1) {
      const node = nodes[offset];
      if (!((this.whatToShow >>> (node.nodeType - 1)) & 1)) continue;
      const result = !this.filter ? 1 : typeof this.filter === 'function' ? this.filter(node) : this.filter.acceptNode(node);
      if (result !== 1) continue;
      this.referenceNode = node;
      this.pointerBeforeReferenceNode = !forward;
      this.previousTree = nodes;
      return node;
    }
    return null;
  }
  nextNode() { return this.step(true); }
  previousNode() { return this.step(false); }
  detach() {}
}

// XML serialization (DOM Parsing's algorithm, as Chromium writes it): each
// element declares the default namespace where it changes, `xlink:` and
// `xml:` attributes their prefixes, empty elements close themselves and
// text and attribute values escape what XML reserves.
const XLINK_NAMESPACE = 'http://www.w3.org/1999/xlink';
const escapeXmlText = (text) => String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const escapeXmlAttribute = (text) =>
  escapeXmlText(text).replace(/"/g, '&quot;').replace(/\t/g, '&#9;').replace(/\n/g, '&#10;').replace(/\r/g, '&#13;');
const voidElements = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const serializeXml = (node, inherited, prefixes) => {
  switch (node.nodeType) {
    case 3: return escapeXmlText(node.data);
    case 4: return `<![CDATA[${node.data}]]>`;
    case 7: return `<?${node.target} ${node.data}?>`;
    case 8: return `<!--${node.data}-->`;
    case 9:
    case 11: return [...node.childNodes].map((child) => serializeXml(child, inherited, prefixes)).join('');
    case 10: return `<!DOCTYPE ${node.name}>`;
    case 1: break;
    default: return '';
  }
  const namespace = node.namespaceURI;
  const name = node.prefix ? `${node.prefix}:${node.localName}` : node.localName;
  let markup = `<${name}`;
  const attributes = [...node.attributes];
  const declared = new Set(prefixes);
  if (!node.prefix && namespace !== inherited && !attributes.some((attribute) => attribute.name === 'xmlns')) {
    markup += ` xmlns="${escapeXmlAttribute(namespace ?? '')}"`;
  }
  for (const attribute of attributes) {
    const [prefix] = attribute.name.includes(':') ? attribute.name.split(':') : [];
    if (prefix === 'xlink' && !declared.has('xlink') && !attributes.some((other) => other.name === 'xmlns:xlink')) {
      markup += ` xmlns:xlink="${XLINK_NAMESPACE}"`;
      declared.add('xlink');
    }
    markup += ` ${attribute.name}="${escapeXmlAttribute(attribute.value)}"`;
  }
  const children = node.localName === 'template' && node.content ? node.content.childNodes : node.childNodes;
  if (children.length === 0) {
    if (namespace === 'http://www.w3.org/1999/xhtml') {
      return voidElements.has(node.localName) ? `${markup} />` : `${markup}></${name}>`;
    }
    return `${markup}/>`;
  }
  const inner = [...children].map((child) => serializeXml(child, node.prefix ? inherited : namespace, declared)).join('');
  return `${markup}>${inner}</${name}>`;
};

class XMLSerializer {
  serializeToString(node) {
    if (!(node instanceof Node)) throw new TypeError("Failed to execute 'serializeToString' on 'XMLSerializer': parameter 1 is not of type 'Node'.");
    return serializeXml(node, null, new Set(['xml']));
  }
}

class DOMParser {
  parseFromString(source, type) {
    type = String(type);
    if (type === 'text/html') return detachedHTMLDocument(source);
    if (['application/xml', 'text/xml', 'application/xhtml+xml', 'image/svg+xml'].includes(type))
      throw new DOMException('XML parsing is not supported', 'NotSupportedError');
    throw new TypeError('Unsupported parser media type');
  }
}
