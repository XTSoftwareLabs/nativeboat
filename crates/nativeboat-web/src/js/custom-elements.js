// Custom elements. ----------------------------------------------------------------
//
// A definition upgrades the elements with its name: the element's wrapper
// takes the definition's prototype and the definition's constructor runs on
// it, its `super()` returning the element instead of making one. Reactions
// run when the DOM operation that causes them returns, as at the end of a
// browser's [CEReactions] method: `connectedCallback` and
// `disconnectedCallback` as elements enter and leave the document, which an
// internal mutation observer of the document reports, and
// `attributeChangedCallback` for the attributes a definition observes.

const customDefinitions = new Map();
const customConstructors = new Map();
const customWaiting = new Map();
// The definition of each upgraded element, or `false` when its constructor
// failed.
const customStates = new WeakMap();
// Upgraded elements whose `connectedCallback` ran and whose
// `disconnectedCallback` has not run since.
const customConnected = new WeakSet();
const ALREADY_CONSTRUCTED = Symbol('already constructed');
const RESERVED_CUSTOM_NAMES = new Set([
  'annotation-xml', 'color-profile', 'font-face', 'font-face-src', 'font-face-uri',
  'font-face-format', 'font-face-name', 'missing-glyph',
]);
let connectionObserver = null;

const isCustomElementName = (name) =>
  /^[a-z][-.0-9_a-z·À-￿]*$/.test(name) && name.includes('-') && !RESERVED_CUSTOM_NAMES.has(name);

// The definition that applies to an element of the document: by its name,
// or by its `is` attribute for a customized built-in element.
const customDefinitionOf = (id) => {
  if (native.namespace(id) !== HTML_NAMESPACE) return null;
  const local = native.localName(id);
  const autonomous = customDefinitions.get(local);
  if (autonomous && autonomous.localName === local) return autonomous;
  const is = native.getAttribute(id, 'is');
  const customized = is === null ? null : customDefinitions.get(is);
  return customized && customized.localName === local ? customized : null;
};

const customCallback = (element, name, args) => {
  const definition = customStates.get(element);
  const callback = definition && definition.callbacks[name];
  if (!callback) return;
  try {
    callback.apply(element, args);
  } catch (error) {
    host.report(error, false);
  }
};

const connectCustom = (element) => {
  if (customConnected.has(element) || !customStates.get(element) || !native.connected(element[ID])) return;
  customConnected.add(element);
  customCallback(element, 'connectedCallback', []);
};

const disconnectCustom = (element) => {
  if (!customConnected.has(element)) return;
  customConnected.delete(element);
  customCallback(element, 'disconnectedCallback', []);
};

// Upgrades an element its definition applies to, unless it was upgraded or
// failed before. The observed attributes it has and its connection are
// reported after its constructor runs.
const upgradeCustom = (element, definition) => {
  if (customStates.has(element)) return;
  customStates.set(element, false);
  definition.stack.push(element);
  try {
    const constructed = Reflect.construct(definition.constructor, []);
    if (constructed !== element) {
      throw new DOMException('The custom element constructor did not produce the element being upgraded.', 'InvalidStateError');
    }
  } catch (error) {
    host.report(error, false);
    return;
  } finally {
    definition.stack.pop();
  }
  customStates.set(element, definition);
  native.defineCustom(element[ID]);
  for (const name of definition.observed) {
    const value = native.getAttribute(element[ID], name);
    if (value !== null) customCallback(element, 'attributeChangedCallback', [name, null, value, null]);
  }
  connectCustom(element);
};

const upgradeCustomTree = (root, connectedOnly) => {
  for (const id of native.customCandidates(root)) {
    const element = wrap(id);
    if (customStates.has(element)) continue;
    if (connectedOnly && !native.connected(id)) continue;
    const definition = customDefinitionOf(id);
    if (definition) upgradeCustom(element, definition);
  }
};

// What the HTML element constructors do: `new` of a definition's
// constructor makes an element of its name, and a constructor that upgrades
// an element gets that element from `super()`.
const constructHTMLElement = (target) => {
  const definition = customConstructors.get(target);
  if (!definition) illegal();
  const { stack } = definition;
  if (stack.length === 0) {
    const element = ownTree(wrap(call('createElement', definition.localName)), document);
    if (definition.localName !== definition.name) native.setAttribute(element[ID], 'is', definition.name);
    Object.setPrototypeOf(element, target.prototype);
    customStates.set(element, definition);
    native.defineCustom(element[ID]);
    return element;
  }
  const element = stack[stack.length - 1];
  if (element === ALREADY_CONSTRUCTED) {
    throw new DOMException('The custom element was already constructed.', 'InvalidStateError');
  }
  Object.setPrototypeOf(element, target.prototype);
  stack[stack.length - 1] = ALREADY_CONSTRUCTED;
  return element;
};

// `document.createElement` of a defined name: an autonomous element is made
// by its constructor; a customized built-in element is made and upgraded.
// Returns null when no definition applies.
const createCustomElement = (name, is) => {
  if (customDefinitions.size === 0) return null;
  const local = String(name).toLowerCase();
  const autonomous = customDefinitions.get(local);
  if (autonomous && autonomous.localName === local) {
    try {
      const element = Reflect.construct(autonomous.constructor, []);
      if (!isNode(element) || native.kind(element[ID]) !== 1) {
        throw new TypeError('The custom element constructor did not produce an element.');
      }
      if (native.attributes(element[ID]).length || native.firstChild(element[ID]) !== null || native.parent(element[ID]) !== null) {
        throw new DOMException('A custom element constructor must not give the element attributes, children or a parent.', 'NotSupportedError');
      }
      if (native.localName(element[ID]) !== local) {
        throw new DOMException('The custom element constructor produced an element of another name.', 'NotSupportedError');
      }
      return element;
    } catch (error) {
      host.report(error, false);
      const element = ownTree(wrap(call('createElement', local)), document);
      Object.setPrototypeOf(element, elementClasses.HTMLUnknownElement.prototype);
      customStates.set(element, false);
      return element;
    }
  }
  const customized = is === undefined ? null : customDefinitions.get(String(is));
  if (!customized || customized.localName !== local) return null;
  const element = ownTree(wrap(call('createElement', local)), document);
  native.setAttribute(element[ID], 'is', customized.name);
  upgradeCustom(element, customized);
  return element;
};

// Reactions to a DOM operation: what `before` saw of an observed attribute
// it changes, then what the connection observer recorded.
const customElementsBefore = (operation, args) => {
  let name;
  switch (operation) {
    case 'setAttribute':
    case 'removeAttribute':
    case 'toggleAttribute':
      name = String(args[1]);
      break;
    case 'styleSet':
    case 'styleRemove':
      name = 'style';
      break;
    default:
      return null;
  }
  const element = wrappers.get(args[0]);
  const definition = element && customStates.get(element);
  if (!definition) return null;
  if (native.namespace(args[0]) === HTML_NAMESPACE) name = name.toLowerCase();
  if (!definition.observed.has(name)) return null;
  return { element, name, old: native.getAttribute(args[0], name) };
};

// The connection observer watches the document and every shadow tree, whose
// mutations the document's observation does not see.
const observeConnections = (root) => {
  native.observe(connectionObserver, root, true, false, false, true, false, false, null);
};

const runCustomReactions = () => {
  if (connectionObserver === null) return;
  for (const [, , added, removed] of native.takeRecords(connectionObserver)) {
    for (const id of removed) {
      for (const candidate of native.customCandidates(id)) {
        const element = wrappers.get(candidate);
        if (element) disconnectCustom(element);
      }
    }
    for (const id of added) {
      if (!native.connected(id)) continue;
      for (const candidate of native.customCandidates(id)) {
        const element = wrap(candidate);
        if (customStates.has(element)) connectCustom(element);
        else {
          const definition = customDefinitionOf(candidate);
          if (definition) upgradeCustom(element, definition);
        }
      }
    }
  }
};

const customElementsAfter = (operation, result, attribute) => {
  if (attribute) {
    const value = native.getAttribute(attribute.element[ID], attribute.name);
    const changed = operation === 'setAttribute' || operation === 'styleSet' || attribute.old !== value;
    if (changed && (attribute.old !== null || value !== null)) {
      customCallback(attribute.element, 'attributeChangedCallback', [attribute.name, attribute.old, value, null]);
    }
  }
  // Clones are made in the document, whose definitions upgrade them.
  if (operation === 'clone' && result !== null) upgradeCustomTree(result, false);
  runCustomReactions();
};

class CustomElementRegistry {
  constructor() {
    illegal();
  }

  define(name, constructor, options) {
    name = String(name);
    if (typeof constructor !== 'function') throw new TypeError('The constructor of a custom element must be a function');
    if (!isCustomElementName(name)) {
      throw new DOMException(`'${name}' is not a valid custom element name`, 'SyntaxError');
    }
    if (customDefinitions.has(name)) {
      throw new DOMException(`'${name}' has already been defined as a custom element.`, 'NotSupportedError');
    }
    if (customConstructors.has(constructor)) {
      throw new DOMException('This constructor has already been used with this registry.', 'NotSupportedError');
    }
    let localName = name;
    const extended = options && options.extends !== undefined ? String(options.extends) : null;
    if (extended !== null) {
      if (isCustomElementName(extended)) {
        throw new DOMException(`'${extended}' is a custom element name, which a definition can't extend.`, 'NotSupportedError');
      }
      localName = extended.toLowerCase();
    }
    const prototype = constructor.prototype;
    if (prototype === null || (typeof prototype !== 'object' && typeof prototype !== 'function')) {
      throw new TypeError("The constructor's prototype must be an object");
    }
    const callbacks = {};
    for (const callback of ['connectedCallback', 'disconnectedCallback', 'adoptedCallback', 'attributeChangedCallback']) {
      const value = prototype[callback];
      if (value !== undefined && typeof value !== 'function') {
        throw new TypeError(`The '${callback}' of a custom element must be a function`);
      }
      callbacks[callback] = value;
    }
    const observed = new Set();
    if (callbacks.attributeChangedCallback) {
      const names = constructor.observedAttributes;
      if (names !== undefined && names !== null) for (const attribute of names) observed.add(String(attribute));
    }
    const definition = { name, localName, constructor, callbacks, observed, stack: [] };
    customDefinitions.set(name, definition);
    customConstructors.set(constructor, definition);
    if (connectionObserver === null) {
      connectionObserver = native.createObserver();
      observeConnections(idOf(document));
      for (const root of shadowRootIds) observeConnections(root);
    }
    upgradeCustomTree(idOf(document), true);
    const waiting = customWaiting.get(name);
    if (waiting) {
      customWaiting.delete(name);
      waiting.resolve(constructor);
    }
  }

  get(name) {
    const definition = customDefinitions.get(String(name));
    return definition ? definition.constructor : undefined;
  }

  getName(constructor) {
    const definition = customConstructors.get(constructor);
    return definition ? definition.name : null;
  }

  whenDefined(name) {
    name = String(name);
    if (!isCustomElementName(name)) {
      return Promise.reject(new DOMException(`'${name}' is not a valid custom element name`, 'SyntaxError'));
    }
    const definition = customDefinitions.get(name);
    if (definition) return Promise.resolve(definition.constructor);
    let waiting = customWaiting.get(name);
    if (!waiting) {
      let resolve;
      const promise = new Promise((settle) => {
        resolve = settle;
      });
      waiting = { promise, resolve };
      customWaiting.set(name, waiting);
    }
    return waiting.promise;
  }

  upgrade(root) {
    upgradeCustomTree(idOf(root), false);
  }
}

const customElements = Object.create(CustomElementRegistry.prototype);
