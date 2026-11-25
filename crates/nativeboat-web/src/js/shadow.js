// Shadow trees. ------------------------------------------------------------------
//
// A shadow root is a document fragment that the DOM attaches to a host
// element. Events that leave a shadow tree are retargeted to its host, focus
// inside it is reported as the host's to the trees outside, and the host's
// children are assigned to the tree's slots by name.

class ShadowRoot extends DocumentFragment {
  constructor() {
    illegal();
  }

  get host() {
    return wrap(native.shadowHost(this[ID]));
  }

  get mode() {
    return native.shadowMode(this[ID]);
  }

  get delegatesFocus() {
    return native.shadowDelegatesFocus(this[ID]);
  }

  get slotAssignment() {
    return 'named';
  }

  get clonable() {
    return false;
  }

  get serializable() {
    return false;
  }

  get innerHTML() {
    return native.innerHTML(this[ID]);
  }

  set innerHTML(html) {
    const markup = html === null ? '' : String(html);
    native.setInnerHTML(this[ID], markup);
    parsedMarkup(markup, this);
  }

  // The focused element as this tree sees it, if focus is in it.
  get activeElement() {
    const focused = native.focused();
    if (focused === null || !shadowIncludingContains(this[ID], focused)) return null;
    const retargeted = native.retarget(focused, this[ID]);
    return retargeted === this[ID] ? null : wrap(retargeted);
  }

  get styleSheets() {
    return Array.from(this.querySelectorAll('style'), (element) => element.sheet).filter(Boolean);
  }

  elementFromPoint(x, y) {
    const id = page.elementFromPoint(Number(x), Number(y));
    if (id === null || !shadowIncludingContains(this[ID], id)) return null;
    return wrap(native.retarget(id, this[ID]));
  }

  cloneNode() {
    throw new DOMException('A shadow root cannot be cloned.', 'NotSupportedError');
  }
}

const shadowIncludingContains = (ancestor, node) => {
  for (let current = node; current !== null; ) {
    if (current === ancestor) return true;
    const parent = native.parent(current);
    current = parent !== null ? parent : native.shadowHost(current);
  }
  return false;
};

const attachShadow = (host, init) => {
  if (init === null || typeof init !== 'object') throw new TypeError("attachShadow needs an init dictionary with a 'mode'");
  const mode = String(init.mode);
  if (mode !== 'open' && mode !== 'closed') {
    throw new TypeError(`The mode of a shadow root must be 'open' or 'closed', not '${mode}'`);
  }
  const id = call('attachShadow', host[ID], mode, !!init.delegatesFocus);
  shadowRootIds.push(id);
  // Custom element reactions follow the shadow tree's mutations too.
  if (connectionObserver !== null) observeConnections(id);
  return wrap(id);
};

// The slot a node is assigned to, unless it is in a closed shadow tree.
const openSlot = (node) => {
  const slot = native.assignedSlot(node[ID]);
  if (slot === null) return null;
  return native.shadowMode(native.root(slot)) === 'open' ? wrap(slot) : null;
};

for (const prototype of [Element.prototype, Text.prototype]) {
  Object.defineProperty(prototype, 'assignedSlot', {
    get() {
      return openSlot(this);
    },
    configurable: true,
    enumerable: true,
  });
}

Object.defineProperties(Element.prototype, {
  shadowRoot: {
    get() {
      const root = native.shadowRoot(this[ID]);
      return root !== null && native.shadowMode(root) === 'open' ? wrap(root) : null;
    },
    configurable: true,
    enumerable: true,
  },
  slot: {
    get() {
      return this.getAttribute('slot') ?? '';
    },
    set(value) {
      this.setAttribute('slot', value);
    },
    configurable: true,
    enumerable: true,
  },
});

const slotElements = (nodes) => nodes.filter((node) => node.nodeType === 1);
Object.defineProperties(elementClasses.HTMLSlotElement.prototype, {
  name: {
    get() {
      return this.getAttribute('name') ?? '';
    },
    set(value) {
      this.setAttribute('name', value);
    },
    configurable: true,
    enumerable: true,
  },
  assignedNodes: {
    value(options) {
      return wrapAll(native.assignedNodes(this[ID], !!(options && options.flatten)));
    },
    writable: true,
    configurable: true,
  },
  assignedElements: {
    value(options) {
      return slotElements(this.assignedNodes(options));
    },
    writable: true,
    configurable: true,
  },
  // Shadow roots assign by name; manual assignment changes nothing.
  assign: {
    value() {},
    writable: true,
    configurable: true,
  },
});

// Retargets a node against a node or the window, as event listeners and
// focus see it from outside the shadow trees it is in.
const retargetFor = (node, against) => {
  if (shadowRootIds.length === 0 || !isNode(node)) return node;
  const id = against === global || !isNode(against) ? 0 : against[ID];
  return wrap(native.retarget(node[ID], id));
};
