// HTML and SVG elements, inline styles, style sheets and form controls. -----

const htmlClasses = Object.create(null);
const defineElement = (name, tags, parent = HTMLElement) => {
  const constructor = {
    [name]: class extends parent {},
  }[name];
  for (const tag of tags) htmlClasses[tag] = constructor;
  return constructor;
};
const register = (constructor, tags) => {
  for (const tag of tags) htmlClasses[tag] = constructor;
  return constructor;
};

// Reflects content attributes as properties.
const resolveURL = (value) => {
  if (value === null) return '';
  try {
    return new URL(value, location.href).href;
  } catch (error) {
    return value;
  }
};
const reflect = (prototype, specs) => {
  for (const [property, name, type, fallback] of specs) {
    const attribute = name || property.toLowerCase();
    const accessors = {
      string: [(element) => element.getAttribute(attribute) ?? (fallback ?? ''), (element, value) => element.setAttribute(attribute, value)],
      boolean: [(element) => element.hasAttribute(attribute), (element, value) => element.toggleAttribute(attribute, !!value)],
      number: [
        (element) => {
          const value = parseInt(element.getAttribute(attribute), 10);
          return Number.isNaN(value) ? fallback ?? 0 : value;
        },
        (element, value) => element.setAttribute(attribute, String(Math.trunc(Number(value)))),
      ],
      url: [(element) => resolveURL(element.getAttribute(attribute)), (element, value) => element.setAttribute(attribute, value)],
    }[type || 'string'];
    Object.defineProperty(prototype, property, {
      get() {
        return accessors[0](this);
      },
      set(value) {
        accessors[1](this, value);
      },
      configurable: true,
      enumerable: true,
    });
  }
};

// Inline and computed styles. ----------------------------------------------

const DECLARATION = Symbol('nativeboat.declaration');
const cssName = (property) => {
  if (property === 'cssFloat') return 'float';
  if (property.startsWith('--')) return property;
  const kebab = property.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  return /^(webkit|moz|ms|o)-/.test(kebab) ? `-${kebab}` : kebab;
};
const cssProperty = /^-?-?[a-zA-Z][a-zA-Z0-9-]*$/;

class CSSStyleDeclaration {
  constructor() {
    illegal();
  }

  get cssText() {
    return this[DECLARATION].text();
  }

  set cssText(value) {
    this[DECLARATION].setText(String(value));
  }

  get length() {
    return this[DECLARATION].names().length;
  }

  get parentRule() {
    return null;
  }

  get cssFloat() {
    return this.getPropertyValue('float');
  }

  set cssFloat(value) {
    this.setProperty('float', value);
  }

  item(index) {
    return this[DECLARATION].names()[index] ?? '';
  }

  getPropertyValue(name) {
    return this[DECLARATION].get(String(name).trim());
  }

  getPropertyPriority(name) {
    return this[DECLARATION].priority(String(name).trim());
  }

  setProperty(name, value, priority = '') {
    const property = String(name).trim();
    if (value === null || value === undefined || String(value) === '') {
      this.removeProperty(property);
      return;
    }
    this[DECLARATION].set(property, String(value), String(priority).toLowerCase() === 'important');
  }

  removeProperty(name) {
    return this[DECLARATION].remove(String(name).trim());
  }

  [Symbol.iterator]() {
    return this[DECLARATION].names()[Symbol.iterator]();
  }
}

const declaration = (source) => {
  const target = Object.create(CSSStyleDeclaration.prototype);
  hidden(target, DECLARATION, source);
  return new Proxy(target, {
    get(object, key, receiver) {
      if (typeof key === 'string' && !(key in object)) {
        const index = collectionIndex(key);
        if (index !== null) return object.item(index);
        if (cssProperty.test(key)) return object.getPropertyValue(cssName(key));
      }
      return Reflect.get(object, key, receiver);
    },
    set(object, key, value, receiver) {
      if (typeof key === 'string' && !(key in object) && cssProperty.test(key)) {
        object.setProperty(cssName(key), value);
        return true;
      }
      return Reflect.set(object, key, value, receiver);
    },
    has(object, key) {
      return (typeof key === 'string' && cssProperty.test(key)) || Reflect.has(object, key);
    },
  });
};

const styles = new WeakMap();
const inlineStyle = (element) => {
  let style = styles.get(element);
  if (!style) {
    const id = element[ID];
    style = declaration({
      text: () => native.style(id),
      setText: (text) => element.setAttribute('style', text),
      names: () => native.styleNames(id),
      get: (name) => native.styleGet(id, name),
      priority: (name) => native.stylePriority(id, name),
      set: (name, value, important) => native.styleSet(id, name, value, important),
      remove: (name) => native.styleRemove(id, name),
    });
    styles.set(element, style);
  }
  return style;
};

const readOnly = () => {
  throw new DOMException('The style declaration is read-only.', 'NoModificationAllowedError');
};
const computedStyle = (element) => {
  // Computed declarations are live: each read queries the current style.
  const id = isNode(element) && element.nodeType === 1 ? element[ID] : null;
  const values = () => id === null ? [] : page.computedStyle(id);
  return declaration({
    text: () => values().map(([name, value]) => `${name}: ${value};`).join(' '),
    setText: readOnly,
    names: () => values().map(([name]) => name),
    get: (name) => id === null ? '' : page.computedStyleValue(id, name),
    priority: () => '',
    set: readOnly,
    remove: readOnly,
  });
};

const datasets = new WeakMap();
class DOMStringMap {
  constructor() {
    illegal();
  }
}
const datasetOf = (element) => {
  let dataset = datasets.get(element);
  if (!dataset) {
    const attributeOf = (key) => `data-${key.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`;
    const keyOf = (name) => name.slice(5).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    dataset = new Proxy(Object.create(DOMStringMap.prototype), {
      get(target, key) {
        if (typeof key !== 'string') return target[key];
        return element.getAttribute(attributeOf(key)) ?? undefined;
      },
      set(target, key, value) {
        element.setAttribute(attributeOf(String(key)), String(value));
        return true;
      },
      deleteProperty(target, key) {
        element.removeAttribute(attributeOf(String(key)));
        return true;
      },
      has(target, key) {
        return typeof key === 'string' && element.hasAttribute(attributeOf(key));
      },
      ownKeys() {
        return element
          .getAttributeNames()
          .filter((name) => name.startsWith('data-'))
          .map(keyOf);
      },
      getOwnPropertyDescriptor(target, key) {
        const value = typeof key === 'string' ? element.getAttribute(attributeOf(key)) : null;
        return value === null ? undefined : { value, enumerable: true, configurable: true, writable: true };
      },
    });
    datasets.set(element, dataset);
  }
  return dataset;
};

// Style sheets of <style> elements. Rules added with insertRule() are kept
// beside the element's text and reported to the renderer with it.

// Splits style sheet text into top-level rules.
const splitRules = (text) => {
  const rules = [];
  let depth = 0;
  let start = 0;
  let quote = null;
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (quote) {
      if (char === '\\') index++;
      else if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === '/' && text[index + 1] === '*') {
      const end = text.indexOf('*/', index + 2);
      index = end < 0 ? text.length : end + 1;
    } else if (char === '{') depth++;
    else if (char === '}' && depth > 0 && --depth === 0) {
      rules.push(text.slice(start, index + 1).trim().replace(/^(\/\*[\s\S]*?\*\/\s*)+/, ''));
      start = index + 1;
    } else if (char === ';' && depth === 0) {
      rules.push(text.slice(start, index + 1).trim());
      start = index + 1;
    }
  }
  return rules.filter(Boolean);
};

class CSSRule {
  constructor() {
    illegal();
  }
}
const ruleTypes = { '@media': 4, '@font-face': 5, '@import': 3, '@keyframes': 7, '@supports': 12 };
const cssRule = (text, sheet) => {
  const rule = Object.create(CSSRule.prototype);
  const brace = text.indexOf('{');
  const prelude = (brace < 0 ? text : text.slice(0, brace)).trim();
  const at = /^@[\w-]+/.exec(prelude);
  Object.assign(rule, {
    cssText: text,
    parentStyleSheet: sheet,
    parentRule: null,
    type: at ? ruleTypes[at[0].toLowerCase()] || 0 : 1,
  });
  if (!at) {
    rule.selectorText = prelude;
    const body = text.slice(brace + 1, text.lastIndexOf('}'));
    rule.style = declaration({
      text: () => body.trim(),
      setText: readOnly,
      names: () => body.split(';').map((part) => part.split(':')[0].trim()).filter(Boolean),
      get: (name) => {
        const entry = body.split(';').find((part) => part.split(':')[0].trim() === name);
        return entry ? entry.slice(entry.indexOf(':') + 1).trim() : '';
      },
      priority: () => '',
      set: readOnly,
      remove: readOnly,
    });
  }
  return rule;
};

const SHEET = Symbol('nativeboat.sheet');
// Rules inserted into a <style> element's sheet go to the renderer.
const publishRules = (state) => {
  if (state.owner) page.sheetRules(state.owner[ID], state.rules.join('\n'));
};
class StyleSheet {
  constructor() {
    illegal();
  }
}
class CSSStyleSheet extends StyleSheet {
  constructor() {
    const sheet = Object.create(CSSStyleSheet.prototype);
    hidden(sheet, SHEET, { owner: null, rules: [] });
    return sheet;
  }

  get ownerNode() {
    return this[SHEET].owner;
  }

  get type() {
    return 'text/css';
  }

  get href() {
    return null;
  }

  get disabled() {
    return false;
  }

  get media() {
    return { mediaText: '', length: 0 };
  }

  get cssRules() {
    const owner = this[SHEET].owner;
    const text = owner ? splitRules(owner.textContent) : [];
    return [...text, ...this[SHEET].rules].map((rule) => cssRule(rule, this));
  }

  get rules() {
    return this.cssRules;
  }

  insertRule(rule, index = 0) {
    const state = this[SHEET];
    const ownText = state.owner ? splitRules(state.owner.textContent).length : 0;
    const position = Math.max(0, Number(index) - ownText);
    if (position > state.rules.length) throw new DOMException('The index is out of range.', 'IndexSizeError');
    state.rules.splice(position, 0, String(rule));
    publishRules(state);
    return Number(index);
  }

  deleteRule(index) {
    const state = this[SHEET];
    const ownText = state.owner ? splitRules(state.owner.textContent).length : 0;
    const position = Number(index) - ownText;
    if (position < 0 || position >= state.rules.length) throw new DOMException('The index is out of range.', 'IndexSizeError');
    state.rules.splice(position, 1);
    publishRules(state);
  }

  addRule(selector, style, index = this.cssRules.length) {
    return this.insertRule(`${selector} { ${style} }`, index);
  }

  removeRule(index) {
    this.deleteRule(index);
  }

  replaceSync(text) {
    this[SHEET].rules = splitRules(String(text));
    publishRules(this[SHEET]);
  }

  replace(text) {
    this.replaceSync(text);
    return Promise.resolve(this);
  }
}
const sheets = new WeakMap();
const sheetOf = (element) => {
  let sheet = sheets.get(element);
  if (!sheet) {
    sheet = new CSSStyleSheet();
    sheet[SHEET].owner = element;
    sheets.set(element, sheet);
  }
  return sheet;
};
const styleSheetsOf = () => {
  const list = [...document.querySelectorAll('style')].map(sheetOf);
  return Object.assign(list, { item: (index) => list[index] ?? null });
};

// HTMLElement and SVGElement. ------------------------------------------------

const offsetParentOf = (element) => {
  if (element.localName === 'body' || element.localName === 'html') return null;
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (parent.localName === 'body') return parent;
    const position = page.computedStyleValue(parent[ID], 'position');
    if (position && position !== 'static') return parent;
  }
  return null;
};

// offsetLeft and offsetTop: the element's border edge from its offset
// parent's padding edge, in layout positions, which scrolling the boxes in
// between, or the offset parent itself, does not change. The document's
// own scrolling moves both edges alike.
const offsetFrom = (element, axis) => {
  const parent = offsetParentOf(element);
  const position = rectOf(element)[axis];
  if (!parent) return position;
  let scrolled = 0;
  for (let node = element.parentElement; node; node = node.parentElement) {
    if (node.localName === 'body' || node.localName === 'html') break;
    scrolled += axis === 'x' ? node.scrollLeft : node.scrollTop;
    if (node === parent) break;
  }
  const border = (page.client(parent[ID]) || [0, 0, 0, 0])[axis === 'x' ? 0 : 1];
  return position - rectOf(parent)[axis] - border + scrolled;
};

// innerText: the text as rendered, following the HTML "rendered text
// collection steps": hidden boxes give nothing, blocks start lines,
// paragraphs leave a blank line, cells and rows are separated by tabs and
// newlines, and collapsible white space collapses as layout collapses it.
const SPACE = Symbol('space');
const blockDisplays = new Set([
  'block', 'flex', 'grid', 'list-item', 'table', 'flow-root', 'table-caption',
  'table-row-group', 'table-header-group', 'table-footer-group',
]);
const opaqueText = new Set(['input', 'textarea', 'img', 'canvas', 'iframe', 'video', 'audio', 'object', 'embed']);
const preserved = new Set(['pre', 'pre-wrap', 'break-spaces']);
const renderedText = (element) => {
  if (!element.isConnected || !page.rect(element[ID])) return element.textContent;
  const style = (node, name) => page.computedStyleValue(node[ID], name);
  const items = [];
  const followedBy = (node, display) => {
    for (let next = node.nextElementSibling; next; next = next.nextElementSibling) {
      if (style(next, 'display') === display) return true;
    }
    return false;
  };
  const lastRow = (row) => {
    const table = row.closest('table');
    if (!table || row.localName !== 'tr') return !followedBy(row, 'table-row');
    const rows = [...table.querySelectorAll('tr')].filter((tr) => tr.closest('table') === table);
    return rows[rows.length - 1] === row;
  };
  const collect = (node) => {
    if (node.nodeType === 3) {
      const parent = node.parentNode;
      if (!parent || parent.nodeType !== 1 || style(parent, 'visibility') !== 'visible') return;
      const whiteSpace = style(parent, 'white-space');
      if (preserved.has(whiteSpace)) {
        if (node.data) items.push(node.data);
        return;
      }
      const data = whiteSpace === 'pre-line'
        ? node.data.replace(/[ \t\r\f]*\n[ \t\r\f]*/g, '\n')
        : node.data.replace(/\n/g, ' ');
      for (const part of data.split(/([ \t\r\f]+|\n)/)) {
        if (part === '') continue;
        items.push(part === '\n' ? '\n' : /^[ \t\r\f]+$/.test(part) ? SPACE : part);
      }
      return;
    }
    if (node.nodeType !== 1) return;
    const display = style(node, 'display');
    if (display === 'none') return;
    if (node.localName === 'br') {
      items.push('\n');
      return;
    }
    const breaks = node.localName === 'p' ? 2 : blockDisplays.has(display) ? 1 : 0;
    if (breaks) items.push(breaks);
    if (!opaqueText.has(node.localName)) {
      for (const child of node.childNodes) collect(child);
    }
    if (display === 'table-cell' && followedBy(node, 'table-cell')) items.push('\t');
    if (display === 'table-row' && !lastRow(node)) items.push('\n');
    if (breaks) items.push(breaks);
  };
  for (const child of element.childNodes) collect(child);
  let text = '';
  let breaks = 0;
  let space = false;
  for (const item of items) {
    if (typeof item === 'number') {
      breaks = Math.max(breaks, item);
    } else if (item === SPACE) {
      space = true;
    } else {
      if (breaks) {
        if (text) text += '\n'.repeat(breaks);
      } else if (space && text && !/\s$/.test(text) && !/^\s/.test(item)) {
        text += ' ';
      }
      breaks = 0;
      space = false;
      text += item;
    }
  }
  return text;
};

// Setting innerText replaces the children with text and a <br> per line
// break.
const setRenderedText = (element, value) => {
  const lines = String(value ?? '').split(/\r\n|\r|\n/);
  element.textContent = '';
  lines.forEach((line, index) => {
    if (index) element.appendChild(element.ownerDocument.createElement('br'));
    if (line) element.appendChild(element.ownerDocument.createTextNode(line));
  });
};

const elementMethods = {
  focus(options = {}) {
    focusNode(this, options, false);
    placeEditingCaret(this);
  },
  blur() {
    if (native.focused() === this[ID]) focusNode(null);
  },
};
const elementAccessors = {
  style() {
    return inlineStyle(this);
  },
  dataset() {
    return datasetOf(this);
  },
};

class HTMLElement extends Element {
  // Only custom element definitions construct HTML elements.
  constructor() {
    return constructHTMLElement(new.target);
  }

  set style(value) {
    this.setAttribute('style', value);
  }

  get tabIndex() {
    const value = parseInt(this.getAttribute('tabindex'), 10);
    if (!Number.isNaN(value)) return value;
    return native.focusable(this[ID]) ? 0 : -1;
  }

  set tabIndex(value) {
    this.setAttribute('tabindex', String(Math.trunc(Number(value))));
  }

  get innerText() {
    return renderedText(this);
  }

  set innerText(value) {
    setRenderedText(this, value);
  }

  get outerText() {
    return renderedText(this);
  }

  get contentEditable() {
    const value = this.getAttribute('contenteditable')?.toLowerCase();
    if (value === '' || value === 'true') return 'true';
    return value === 'false' || value === 'plaintext-only' ? value : 'inherit';
  }

  set contentEditable(value) {
    const state = String(value).toLowerCase();
    if (state === 'inherit') this.removeAttribute('contenteditable');
    else if (['true', 'false', 'plaintext-only'].includes(state)) this.setAttribute('contenteditable', state);
    else throw new DOMException(`'${value}' is not a contenteditable state.`, 'SyntaxError');
  }

  get isContentEditable() {
    return native.editingHost(this[ID]) !== null;
  }

  get offsetParent() {
    return offsetParentOf(this);
  }

  // Whole pixels, as Chromium gives them.
  get offsetWidth() {
    return Math.round(rectOf(this).width);
  }

  get offsetHeight() {
    return Math.round(rectOf(this).height);
  }

  get offsetLeft() {
    return Math.round(offsetFrom(this, 'x'));
  }

  get offsetTop() {
    return Math.round(offsetFrom(this, 'y'));
  }

  click() {
    activate(this, {}, false);
  }
}
mixin(HTMLElement.prototype, elementMethods, elementAccessors);
reflect(HTMLElement.prototype, [
  ['title'],
  ['lang'],
  ['dir'],
  ['accessKey', 'accesskey'],
  ['hidden', null, 'boolean'],
  ['autofocus', null, 'boolean'],
  ['inert', null, 'boolean'],
  ['spellcheck', null, 'boolean'],
  ['translate', null, 'boolean'],
  ['nonce'],
  ['enterKeyHint', 'enterkeyhint'],
  ['inputMode', 'inputmode'],
]);
Object.defineProperty(HTMLElement.prototype, 'draggable', {
  get() {
    const value = this.getAttribute('draggable');
    if (value !== null) return value.toLowerCase() === 'true';
    return this.localName === 'img' || (this.localName === 'a' && this.hasAttribute('href'));
  },
  set(value) { this.setAttribute('draggable', value ? 'true' : 'false'); },
});
defineHandlers(HTMLElement.prototype, handlerNames);

// SVG's animated string attributes, as `className` on SVG elements.
class SVGAnimatedString {
  constructor() {
    illegal();
  }
}

class SVGElement extends Element {
  constructor() {
    illegal();
  }

  set style(value) {
    this.setAttribute('style', value);
  }

  // SVG elements' `className` is an animated string: its `baseVal` is the
  // `class` attribute.
  get className() {
    const element = this;
    return Object.create(SVGAnimatedString.prototype, {
      baseVal: {
        get: () => element.getAttribute('class') ?? '',
        set: (value) => element.setAttribute('class', String(value)),
        enumerable: true,
      },
      animVal: { get: () => element.getAttribute('class') ?? '', enumerable: true },
    });
  }

  // The nearest ancestor `<svg>`: null for the outermost one.
  get ownerSVGElement() {
    for (let node = this.parentNode; node; node = node.parentNode) {
      if (node instanceof SVGSVGElement) return node;
    }
    return null;
  }

  getBBox() {
    const box = page.bbox(this[ID]);
    return box ? new DOMRect(box[0], box[1], box[2], box[3]) : new DOMRect();
  }

  getComputedTextLength() {
    return this.getBBox().width;
  }

  // The map from user units to the viewport: its SVG's position, without
  // the transforms in between.
  getScreenCTM() {
    const svg = this.localName === 'svg' ? this : this.ownerSVGElement;
    const rect = (svg ?? this).getBoundingClientRect();
    return new DOMMatrix([1, 0, 0, 1, rect.left, rect.top]);
  }

  getCTM() {
    return new DOMMatrix();
  }
}
mixin(SVGElement.prototype, elementMethods, elementAccessors);
defineHandlers(SVGElement.prototype, handlerNames);
class SVGSVGElement extends SVGElement {
  constructor() {
    illegal();
  }

  createSVGMatrix() {
    return new DOMMatrix();
  }

  createSVGPoint() {
    const point = { x: 0, y: 0 };
    point.matrixTransform = (matrix) => {
      const transformed = new DOMMatrix(matrixOf(matrix)).transformPoint(point);
      return Object.assign(this.createSVGPoint(), { x: transformed.x, y: transformed.y });
    };
    return point;
  }

  createSVGRect() {
    return new DOMRect();
  }

  // The `viewBox` attribute as an animated rectangle: four numbers, or all
  // zeros when it is missing or malformed.
  get viewBox() {
    const numbers = String(this.getAttribute('viewBox') ?? '').trim().split(/[\s,]+/).map(Number);
    const valid = numbers.length === 4 && numbers.every(Number.isFinite) && numbers[2] >= 0 && numbers[3] >= 0;
    const rect = () => new DOMRect(...(valid ? numbers : [0, 0, 0, 0]));
    return { baseVal: rect(), animVal: rect() };
  }
}

// Form controls. --------------------------------------------------------------

const selections = new WeakMap();
const textTypes = ['text', 'search', 'url', 'tel', 'password', 'email', ''];
const inputTypes = [
  'hidden', 'text', 'search', 'tel', 'url', 'email', 'password', 'date', 'month', 'week', 'time',
  'datetime-local', 'number', 'range', 'color', 'checkbox', 'radio', 'file', 'submit', 'image',
  'reset', 'button',
];

const labelsOf = (control) => {
  const labels = [...document.querySelectorAll('label')].filter((label) => label.control === control);
  return staticNodeList(labels);
};

// The constraints the document computes for a control, by the bit each has
// in its validity flags.
const validityNames = [
  'valueMissing', 'typeMismatch', 'patternMismatch', 'tooLong', 'tooShort', 'rangeUnderflow',
  'rangeOverflow', 'stepMismatch', 'badInput', 'customError',
];
const validityOf = (control) => {
  const flags = native.validity(control[ID]);
  const validity = {};
  validityNames.forEach((name, bit) => {
    validity[name] = (flags & (1 << bit)) !== 0;
  });
  validity.valid = flags === 0;
  return validity;
};

const formControlMethods = {
  checkValidity() {
    const valid = validityOf(this).valid;
    if (!valid) fire(this, Event, 'invalid', { cancelable: true });
    return valid;
  },
  reportValidity() {
    return this.checkValidity();
  },
  setCustomValidity(message) {
    native.setCustomValidity(this[ID], String(message));
  },
};
const formControlAccessors = {
  form() {
    return wrap(native.formOwner(this[ID]));
  },
  labels() {
    return labelsOf(this);
  },
  validity() {
    return validityOf(this);
  },
  validationMessage() {
    return native.validationMessage(this[ID]);
  },
  willValidate() {
    return native.willValidate(this[ID]);
  },
};

// Text selection inside text controls: what the native text control shows,
// or what scripts set while the element is not shown as one.
const selectionOf = (element) => {
  const length = element.value.length;
  const stored = selections.get(element) || [length, length, 'none'];
  const shown = page.textSelection(element[ID]);
  return shown ? [Math.min(shown[0], length), Math.min(shown[1], length), stored[2]] : stored;
};
const textSelection = {
  selectionStart() {
    return selectionOf(this)[0];
  },
  selectionEnd() {
    return selectionOf(this)[1];
  },
  selectionDirection() {
    return selectionOf(this)[2];
  },
};
const textSelectionMethods = {
  setSelectionRange(start, end, direction = 'none') {
    const length = this.value.length;
    const from = Math.min(Math.max(0, Number(start) || 0), length);
    const to = Math.min(Math.max(from, Number(end) || 0), length);
    selections.set(this, [from, to, direction]);
    page.selectText(this[ID], from, to);
  },
  select() {
    this.setSelectionRange(0, this.value.length);
    fire(this, Event, 'select', { bubbles: true });
  },
  setRangeText(text, start = this.selectionStart, end = this.selectionEnd) {
    const value = this.value;
    this.value = value.slice(0, start) + text + value.slice(end);
    this.setSelectionRange(start + String(text).length, start + String(text).length);
  },
};
const defineSelection = (prototype) => {
  mixin(prototype, textSelectionMethods);
  for (const [name, get] of Object.entries(textSelection)) {
    Object.defineProperty(prototype, name, {
      get,
      set(value) {
        const [start, end] = [this.selectionStart, this.selectionEnd];
        if (name === 'selectionStart') this.setSelectionRange(value, Math.max(value, end));
        else if (name === 'selectionEnd') this.setSelectionRange(start, value);
        else this.setSelectionRange(start, end, value);
      },
      configurable: true,
    });
  }
};

class HTMLInputElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get type() {
    const type = (this.getAttribute('type') || '').toLowerCase();
    return inputTypes.includes(type) ? type : 'text';
  }

  set type(value) {
    this.setAttribute('type', value);
  }

  get value() {
    return native.value(this[ID]);
  }

  set value(value) {
    native.setValue(this[ID], value === null ? '' : String(value));
    selections.delete(this);
  }

  get defaultValue() {
    return this.getAttribute('value') ?? '';
  }

  set defaultValue(value) {
    this.setAttribute('value', value);
  }

  get checked() {
    return native.checked(this[ID]);
  }

  set checked(value) {
    native.setChecked(this[ID], !!value);
  }

  get defaultChecked() {
    return this.hasAttribute('checked');
  }

  set defaultChecked(value) {
    this.toggleAttribute('checked', !!value);
  }

  get indeterminate() {
    return native.indeterminate(this[ID]);
  }

  set indeterminate(value) {
    native.setIndeterminate(this[ID], !!value);
  }

  get valueAsNumber() {
    const number = parseFloat(this.value);
    return Number.isFinite(number) ? number : NaN;
  }

  set valueAsNumber(value) {
    this.value = String(value);
  }

  get valueAsDate() {
    return null;
  }

  get files() {
    return this.type === 'file' ? Object.assign([], { item: () => null }) : null;
  }

  get list() {
    return document.getElementById(this.getAttribute('list') || '');
  }

  stepUp(count = 1) {
    this.value = String(this.valueAsNumber + Number(this.step || 1) * count);
  }

  stepDown(count = 1) {
    this.stepUp(-count);
  }

  showPicker() {}
}
mixin(HTMLInputElement.prototype, formControlMethods, formControlAccessors);
defineSelection(HTMLInputElement.prototype);
reflect(HTMLInputElement.prototype, [
  ['name'],
  ['placeholder'],
  ['accept'],
  ['alt'],
  ['autocomplete'],
  ['min'],
  ['max'],
  ['step'],
  ['pattern'],
  ['src', null, 'url'],
  ['disabled', null, 'boolean'],
  ['readOnly', 'readonly', 'boolean'],
  ['required', null, 'boolean'],
  ['multiple', null, 'boolean'],
  ['formNoValidate', 'formnovalidate', 'boolean'],
  ['maxLength', 'maxlength', 'number', -1],
  ['minLength', 'minlength', 'number', -1],
  ['size', null, 'number', 20],
]);
register(HTMLInputElement, ['input']);

class HTMLTextAreaElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get type() {
    return 'textarea';
  }

  get value() {
    return native.value(this[ID]);
  }

  set value(value) {
    native.setValue(this[ID], value === null ? '' : String(value));
    selections.delete(this);
  }

  get defaultValue() {
    return this.textContent;
  }

  set defaultValue(value) {
    this.textContent = value;
  }

  get textLength() {
    return this.value.length;
  }
}
mixin(HTMLTextAreaElement.prototype, formControlMethods, formControlAccessors);
defineSelection(HTMLTextAreaElement.prototype);
reflect(HTMLTextAreaElement.prototype, [
  ['name'],
  ['placeholder'],
  ['wrap'],
  ['autocomplete'],
  ['disabled', null, 'boolean'],
  ['readOnly', 'readonly', 'boolean'],
  ['required', null, 'boolean'],
  ['rows', null, 'number', 2],
  ['cols', null, 'number', 20],
  ['maxLength', 'maxlength', 'number', -1],
  ['minLength', 'minlength', 'number', -1],
]);
register(HTMLTextAreaElement, ['textarea']);

const optionsOf = (select) => wrapAll(native.options(select[ID]));

class HTMLSelectElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get type() {
    return this.multiple ? 'select-multiple' : 'select-one';
  }

  get value() {
    return native.value(this[ID]);
  }

  set value(value) {
    native.setValue(this[ID], value === null ? '' : String(value));
  }

  get selectedIndex() {
    return native.selectedIndex(this[ID]);
  }

  set selectedIndex(value) {
    native.setSelectedIndex(this[ID], Number(value));
  }

  get options() {
    const list = htmlCollection(() => optionsOf(this));
    const select = this;
    return Object.defineProperties(list, {
      selectedIndex: {
        get: () => select.selectedIndex,
        set: (value) => {
          select.selectedIndex = value;
        },
      },
      add: { value: (option, before) => select.add(option, before) },
      remove: { value: (index) => select.remove(index) },
    });
  }

  get selectedOptions() {
    return htmlCollection(() => optionsOf(this).filter((option) => option.selected));
  }

  get length() {
    return optionsOf(this).length;
  }

  item(index) {
    return optionsOf(this)[index] ?? null;
  }

  namedItem(name) {
    return optionsOf(this).find((option) => option.id === name || option.getAttribute('name') === name) ?? null;
  }

  add(option, before = null) {
    const reference = typeof before === 'number' ? optionsOf(this)[before] ?? null : before;
    if (reference) reference.parentNode.insertBefore(option, reference);
    else this.appendChild(option);
  }

  remove(index) {
    if (index === undefined) {
      native.remove(this[ID]);
      return;
    }
    const option = optionsOf(this)[index];
    if (option) option.remove();
  }
}
mixin(HTMLSelectElement.prototype, formControlMethods, formControlAccessors);
reflect(HTMLSelectElement.prototype, [
  ['name'],
  ['autocomplete'],
  ['disabled', null, 'boolean'],
  ['multiple', null, 'boolean'],
  ['required', null, 'boolean'],
  ['size', null, 'number', 0],
]);
register(HTMLSelectElement, ['select']);

class HTMLOptionElement extends HTMLElement {
  constructor(text = '', value, defaultSelected = false, selected = false) {
    const option = wrap(native.createElement('option'));
    if (text !== '') option.textContent = String(text);
    if (value !== undefined) option.setAttribute('value', value);
    if (defaultSelected) option.setAttribute('selected', '');
    if (selected) option.selected = true;
    return option;
  }

  get value() {
    return native.value(this[ID]);
  }

  set value(value) {
    this.setAttribute('value', value);
  }

  get text() {
    return this.textContent.replace(/\s+/g, ' ').trim();
  }

  set text(value) {
    this.textContent = value;
  }

  get label() {
    return this.getAttribute('label') ?? this.text;
  }

  set label(value) {
    this.setAttribute('label', value);
  }

  get selected() {
    return native.optionSelected(this[ID]);
  }

  set selected(value) {
    native.setOptionSelected(this[ID], !!value);
  }

  get defaultSelected() {
    return this.hasAttribute('selected');
  }

  set defaultSelected(value) {
    this.toggleAttribute('selected', !!value);
  }

  get index() {
    const select = this.closest('select');
    return select ? optionsOf(select).indexOf(this) : 0;
  }

  get form() {
    const select = this.closest('select');
    return select ? select.form : null;
  }
}
reflect(HTMLOptionElement.prototype, [['disabled', null, 'boolean']]);
register(HTMLOptionElement, ['option']);

class HTMLButtonElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get type() {
    const type = (this.getAttribute('type') || '').toLowerCase();
    return type === 'button' || type === 'reset' ? type : 'submit';
  }

  set type(value) {
    this.setAttribute('type', value);
  }
}
mixin(HTMLButtonElement.prototype, formControlMethods, formControlAccessors);
reflect(HTMLButtonElement.prototype, [
  ['name'],
  ['value'],
  ['disabled', null, 'boolean'],
  ['formNoValidate', 'formnovalidate', 'boolean'],
]);
register(HTMLButtonElement, ['button']);

const listed = 'button, fieldset, input, object, output, select, textarea';
class HTMLFormElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get elements() {
    // Boa loses `this` in arrows nested in arrows, so the id is captured.
    const id = this[ID];
    return htmlCollection(() =>
      [...document.querySelectorAll(listed)].filter(
        (control) => (control.localName !== 'input' || control.type !== 'image') && native.formOwner(control[ID]) === id,
      ),
    );
  }

  get length() {
    return this.elements.length;
  }

  get method() {
    return (this.getAttribute('method') || '').toLowerCase() === 'post' ? 'post' : 'get';
  }

  set method(value) {
    this.setAttribute('method', value);
  }

  submit() {
    navigateForm(this);
  }

  requestSubmit(submitter = null) {
    submitForm(this, submitter);
  }

  reset() {
    if (!fire(this, Event, 'reset', { bubbles: true, cancelable: true }).allowed) return;
    for (const control of this.elements) {
      if (control.localName === 'input') {
        if (control.type === 'checkbox' || control.type === 'radio') control.checked = control.defaultChecked;
        else control.value = control.defaultValue;
      } else if (control.localName === 'textarea') control.value = control.defaultValue;
      else if (control.localName === 'select') {
        for (const option of optionsOf(control)) option.selected = option.defaultSelected;
      }
    }
  }

  checkValidity() {
    return [...this.elements].every((control) => !control.checkValidity || control.checkValidity());
  }

  reportValidity() {
    return this.checkValidity();
  }
}
reflect(HTMLFormElement.prototype, [
  ['action', null, 'url'],
  ['name'],
  ['target'],
  ['enctype'],
  ['acceptCharset', 'accept-charset'],
  ['autocomplete'],
  ['noValidate', 'novalidate', 'boolean'],
]);
register(HTMLFormElement, ['form']);

class HTMLLabelElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get control() {
    const target = this.getAttribute('for');
    if (target !== null) {
      const control = document.getElementById(target);
      return control && labelable(control) ? control : null;
    }
    return [...this.querySelectorAll('button, input, meter, output, progress, select, textarea')].find(labelable) ?? null;
  }

  get form() {
    const control = this.control;
    return control ? control.form : null;
  }
}
const labelable = (element) =>
  ['button', 'meter', 'output', 'progress', 'select', 'textarea'].includes(element.localName) ||
  (element.localName === 'input' && element.type !== 'hidden');
reflect(HTMLLabelElement.prototype, [['htmlFor', 'for']]);
register(HTMLLabelElement, ['label']);

const urlParts = ['protocol', 'host', 'hostname', 'port', 'pathname', 'search', 'hash', 'origin', 'username', 'password'];
const defineURLParts = (prototype) => {
  for (const part of urlParts) {
    Object.defineProperty(prototype, part, {
      get() {
        try {
          return new URL(this.href)[part];
        } catch (error) {
          return '';
        }
      },
      set(value) {
        const url = new URL(this.href);
        url[part] = value;
        this.href = url.href;
      },
      configurable: true,
    });
  }
};
class HTMLAnchorElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get text() {
    return this.textContent;
  }

  get relList() {
    return tokenList(this, 'rel');
  }

  toString() {
    return this.href;
  }
}
reflect(HTMLAnchorElement.prototype, [['href', null, 'url'], ['target'], ['rel'], ['download'], ['type'], ['hreflang'], ['referrerPolicy', 'referrerpolicy']]);
defineURLParts(HTMLAnchorElement.prototype);
register(HTMLAnchorElement, ['a']);
const HTMLAreaElement = defineElement('HTMLAreaElement', ['area']);
reflect(HTMLAreaElement.prototype, [['href', null, 'url'], ['target'], ['alt'], ['shape'], ['coords']]);

// Downloads of http(s) images under way, with the callbacks waiting on each.
const imageDownloads = new Map();
const downloadImage = (url, done) => {
  const waiting = imageDownloads.get(url);
  if (waiting) {
    waiting.push(done);
    return;
  }
  imageDownloads.set(url, [done]);
  page.fetchImage(url, (bytes) => {
    const loaded = bytes !== null && page.addImage(url, bytes);
    const callbacks = imageDownloads.get(url);
    imageDownloads.delete(url);
    for (const callback of callbacks) callback(loaded);
  });
};
const imageEvents = (image) => {
  const source = image.getAttribute('src');
  if (source === null) return;
  const url = resolveURL(source);
  // A network image fires its event once downloaded, unless the source
  // changed meanwhile.
  if (/^https?:/i.test(url) && !page.imageSize(url)) {
    downloadImage(url, (loaded) => {
      if (image.getAttribute('src') === source) fire(image, Event, loaded ? 'load' : 'error');
    });
    return;
  }
  setTimeout(() => {
    fire(image, Event, page.imageSize(url) || page.imageSize(source) ? 'load' : 'error');
  }, 0);
};
// Markup parsed into the document, in `root`: its handler attributes get the
// handlers the compiler made of their text, and its images load, as the
// parser starts them. Markup without either costs one test.
const parsedMarkup = (html, root) => {
  if (!/<img|<video|<audio|\son[a-z]+\s*=/i.test(html)) return;
  const handlers = globalThis.__nativeboatHandlers;
  for (const element of root.querySelectorAll('*')) {
    // Template contents are inert.
    if (element.closest('template')) continue;
    if (handlers) {
      for (const { name, value } of [...element.attributes]) {
        const handler = name.startsWith('on') && handlers.get(`${name}\u0000${value}`);
        if (handler && name in element) element[name] = handler;
      }
    }
    if (element.localName === 'img' && element.hasAttribute('src')) imageEvents(element);
    else if (element.localName === 'video' || element.localName === 'audio') mediaLoad(element);
  }
};
class HTMLImageElement extends HTMLElement {
  constructor(width, height) {
    const image = wrap(native.createElement('img'));
    if (width !== undefined) image.width = width;
    if (height !== undefined) image.height = height;
    return image;
  }

  setAttribute(name, value) {
    super.setAttribute(name, value);
    if (String(name).toLowerCase() === 'src') imageEvents(this);
  }

  get naturalWidth() {
    return (page.imageSize(this.currentSrc) || page.imageSize(this.getAttribute('src') || '') || [0, 0])[0];
  }

  get naturalHeight() {
    return (page.imageSize(this.currentSrc) || page.imageSize(this.getAttribute('src') || '') || [0, 0])[1];
  }

  get width() {
    const value = parseInt(this.getAttribute('width'), 10);
    return Number.isNaN(value) ? Math.round(rectOf(this).width) || this.naturalWidth : value;
  }

  set width(value) {
    this.setAttribute('width', String(Math.trunc(Number(value))));
  }

  get height() {
    const value = parseInt(this.getAttribute('height'), 10);
    return Number.isNaN(value) ? Math.round(rectOf(this).height) || this.naturalHeight : value;
  }

  set height(value) {
    this.setAttribute('height', String(Math.trunc(Number(value))));
  }

  get complete() {
    return true;
  }

  get currentSrc() {
    return this.src;
  }

  decode() {
    return Promise.resolve();
  }
}
reflect(HTMLImageElement.prototype, [
  ['src', null, 'url'],
  ['alt'],
  ['srcset'],
  ['sizes'],
  ['loading'],
  ['decoding'],
  ['crossOrigin', 'crossorigin'],
  ['useMap', 'usemap'],
  ['isMap', 'ismap', 'boolean'],
]);
register(HTMLImageElement, ['img']);

class HTMLStyleElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get sheet() {
    return this.isConnected ? sheetOf(this) : null;
  }
}
reflect(HTMLStyleElement.prototype, [['media'], ['type'], ['disabled', null, 'boolean']]);
register(HTMLStyleElement, ['style']);

// A style sheet link reports `load` once its `href` is set, as the page
// applies the sheet when it next styles; links in the page's markup do when
// the page is ready.
const linkEvents = (link) => {
  const rel = (link.getAttribute('rel') || '').toLowerCase().split(/\s+/);
  if (!rel.includes('stylesheet') || link.getAttribute('href') === null) return;
  setTimeout(() => fire(link, Event, 'load'), 0);
};
class HTMLLinkElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  setAttribute(name, value) {
    super.setAttribute(name, value);
    if (String(name).toLowerCase() === 'href') linkEvents(this);
  }

  get sheet() {
    return null;
  }

  get relList() {
    return tokenList(this, 'rel');
  }
}
reflect(HTMLLinkElement.prototype, [['href', null, 'url'], ['rel'], ['as'], ['media'], ['type'], ['crossOrigin', 'crossorigin'], ['disabled', null, 'boolean']]);
register(HTMLLinkElement, ['link']);

class HTMLCanvasElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  getContext(type) {
    return String(type) === '2d' ? getContext2d(this) : null;
  }

  toDataURL(type) {
    return canvasDataURL(this, type);
  }

  toBlob(callback, type) {
    const url = canvasDataURL(this, type);
    const comma = url.indexOf(',');
    const blob =
      comma === url.length - 1
        ? null
        : new Blob([Uint8Array.from(atob(url.slice(comma + 1)), (character) => character.charCodeAt(0))], { type: url.slice(5, url.indexOf(';')) });
    setTimeout(() => callback(blob));
  }

  transferControlToOffscreen() {
    throw new DOMException('OffscreenCanvas is not supported.', 'NotSupportedError');
  }
}
reflect(HTMLCanvasElement.prototype, [['width', null, 'number', 300], ['height', null, 'number', 150]]);
// Setting a size, even the same one, starts the bitmap over.
for (const name of ['width', 'height']) {
  const { get, set } = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, name);
  Object.defineProperty(HTMLCanvasElement.prototype, name, {
    get,
    set(value) {
      set.call(this, value);
      resizeCanvas(this);
    },
    configurable: true,
    enumerable: true,
  });
}
register(HTMLCanvasElement, ['canvas']);

// Media elements load their media's metadata and first frame, as Chromium
// does before playback; they do not play.
const MEDIA = Symbol('nativeboat.media');
const VIDEO_FRAME = 'nativeboat-video-frame:';
const mediaState = (media) => {
  if (!media[MEDIA]) {
    hidden(media, MEDIA, { readyState: 0, networkState: 0, duration: NaN, width: 0, height: 0, error: null, currentSrc: '', load: 0 });
  }
  return media[MEDIA];
};
// The media an element plays: its `src`, else its first `<source>`'s.
const mediaURL = (media) => {
  const src = media.getAttribute('src');
  if (src !== null) return src;
  for (const child of media.children) {
    if (child.localName === 'source' && child.hasAttribute('src')) return child.getAttribute('src');
  }
  return null;
};
const mediaPreload = (media) => {
  const value = (media.getAttribute('preload') ?? 'metadata').toLowerCase();
  if (value === 'none' || value === 'metadata') return value;
  return 'auto';
};
// The media element load algorithm, as far as loading goes.
const mediaLoad = (media) => {
  const state = mediaState(media);
  const load = ++state.load;
  if (state.networkState === 1 || state.networkState === 2) fire(media, Event, 'abort');
  if (state.networkState !== 0) {
    const loaded = state.readyState > 0;
    Object.assign(state, { readyState: 0, networkState: 0, duration: NaN, width: 0, height: 0, error: null, currentSrc: '' });
    fire(media, Event, 'emptied');
    if (loaded) fire(media, Event, 'durationchange');
  }
  const source = mediaURL(media);
  if (source === null) return;
  state.networkState = 3;
  const url = resolveURL(source);
  const later = (step) => setTimeout(() => {
    if (state.load === load) step();
  }, 0);
  const fail = () => {
    state.error = new MediaError(4);
    state.networkState = 3;
    fire(media, Event, 'error');
  };
  later(() => {
    // Media not to preload waits, idle, for playback.
    const waits = mediaPreload(media) === 'none' && !media.hasAttribute('autoplay');
    state.currentSrc = url;
    state.networkState = waits ? 1 : 2;
    fire(media, Event, 'loadstart');
    if (waits) {
      fire(media, Event, 'suspend');
      return;
    }
    if (typeof page.loadMedia !== 'function') {
      fail();
      return;
    }
    page.loadMedia(url, (info) => {
      if (state.load !== load) return;
      if (info === null) {
        fail();
        return;
      }
      const video = media.localName === 'video';
      if (video && info.length > 1) {
        page.addFrame(VIDEO_FRAME + source, info[1], info[2], info[3]);
        state.width = info[1];
        state.height = info[2];
      }
      // Chromium queues these events, so by the time they run the media
      // has loaded: each sees the final states, as in Electron.
      state.duration = info[0];
      state.readyState = 4;
      state.networkState = 1;
      fire(media, Event, 'durationchange');
      if (video) fire(media, Event, 'resize');
      for (const type of ['loadedmetadata', 'progress', 'suspend', 'loadeddata', 'canplay', 'canplaythrough']) fire(media, Event, type);
    });
  });
};

class MediaError {
  constructor(code) {
    if (code === undefined) illegal();
    this.code = code;
    this.message = code === 4 ? 'MEDIA_ELEMENT_ERROR: Format error' : '';
  }
}
Object.assign(MediaError, { MEDIA_ERR_ABORTED: 1, MEDIA_ERR_NETWORK: 2, MEDIA_ERR_DECODE: 3, MEDIA_ERR_SRC_NOT_SUPPORTED: 4 });

// The time ranges of loaded media: all of it, or none.
class TimeRanges {
  constructor(end) {
    if (end === undefined) illegal();
    this[MEDIA] = end;
  }

  get length() {
    return this[MEDIA] > 0 ? 1 : 0;
  }

  start(index) {
    if (index !== 0 || !(this[MEDIA] > 0)) throw new DOMException('The index provided is out of range.', 'IndexSizeError');
    return 0;
  }

  end(index) {
    if (index !== 0 || !(this[MEDIA] > 0)) throw new DOMException('The index provided is out of range.', 'IndexSizeError');
    return this[MEDIA];
  }
}

// Chromium's answers for the containers and codecs Windows reads.
const playableTypes = {
  'video/mp4': ['avc1', 'mp4a'],
  'audio/mp4': ['mp4a'],
  'audio/x-m4a': ['mp4a'],
  'video/webm': ['vp8', 'vp9', 'vorbis', 'opus'],
  'audio/webm': ['vorbis', 'opus'],
  'audio/mpeg': [],
  'audio/mp3': [],
  'audio/wav': ['1'],
  'audio/x-wav': ['1'],
  'audio/aac': [],
};

class HTMLMediaElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  setAttribute(name, value) {
    super.setAttribute(name, value);
    if (String(name).toLowerCase() === 'src') mediaLoad(this);
  }

  play() {
    return Promise.resolve();
  }

  pause() {}

  load() {
    mediaLoad(this);
  }

  canPlayType(type) {
    const [container, ...parameters] = String(type).toLowerCase().split(';').map((part) => part.trim());
    const codecs = playableTypes[container];
    if (!codecs) return '';
    const listed = parameters.find((parameter) => parameter.startsWith('codecs='));
    if (!listed) return codecs.length ? 'maybe' : 'probably';
    const names = listed.slice(7).replace(/^"|"$/g, '').split(',').map((codec) => codec.trim().split('.')[0]);
    return names.every((name) => codecs.includes(name)) ? 'probably' : '';
  }

  get paused() {
    return true;
  }

  get ended() {
    return false;
  }

  get seeking() {
    return false;
  }

  get currentTime() {
    return 0;
  }

  set currentTime(value) {}

  get volume() {
    return 1;
  }

  set volume(value) {}

  get playbackRate() {
    return 1;
  }

  set playbackRate(value) {}

  get defaultPlaybackRate() {
    return 1;
  }

  set defaultPlaybackRate(value) {}

  get duration() {
    return mediaState(this).duration;
  }

  get readyState() {
    return mediaState(this).readyState;
  }

  get networkState() {
    return mediaState(this).networkState;
  }

  get error() {
    return mediaState(this).error;
  }

  get currentSrc() {
    return mediaState(this).currentSrc;
  }

  get buffered() {
    return new TimeRanges(mediaState(this).readyState > 0 ? mediaState(this).duration : 0);
  }

  get seekable() {
    return this.buffered;
  }

  get played() {
    return new TimeRanges(0);
  }

  get preload() {
    return mediaPreload(this);
  }

  set preload(value) {
    this.setAttribute('preload', value);
  }
}
reflect(HTMLMediaElement.prototype, [
  ['src', null, 'url'],
  ['autoplay', null, 'boolean'],
  ['controls', null, 'boolean'],
  ['loop', null, 'boolean'],
  ['muted', null, 'boolean'],
  ['defaultMuted', 'muted', 'boolean'],
  ['crossOrigin', 'crossorigin'],
]);
const mediaConstants = {
  NETWORK_EMPTY: 0, NETWORK_IDLE: 1, NETWORK_LOADING: 2, NETWORK_NO_SOURCE: 3,
  HAVE_NOTHING: 0, HAVE_METADATA: 1, HAVE_CURRENT_DATA: 2, HAVE_FUTURE_DATA: 3, HAVE_ENOUGH_DATA: 4,
};
Object.assign(HTMLMediaElement, mediaConstants);
for (const [name, value] of Object.entries(mediaConstants)) {
  Object.defineProperty(HTMLMediaElement.prototype, name, { value, enumerable: true });
}
class HTMLVideoElement extends HTMLMediaElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get videoWidth() {
    return mediaState(this).width;
  }

  get videoHeight() {
    return mediaState(this).height;
  }
}
reflect(HTMLVideoElement.prototype, [
  ['poster', null, 'url'],
  ['width', null, 'number'],
  ['height', null, 'number'],
  ['playsInline', 'playsinline', 'boolean'],
]);
register(HTMLVideoElement, ['video']);
const HTMLAudioElement = defineElement('HTMLAudioElement', ['audio'], HTMLMediaElement);
// `new Audio(src)`, which loads all its media; HTMLAudioElement itself
// cannot be constructed.
const Audio = function Audio(src) {
  if (!new.target) throw new TypeError("Failed to construct 'Audio': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
  const audio = wrap(native.createElement('audio'));
  audio.setAttribute('preload', 'auto');
  if (src !== undefined) audio.setAttribute('src', src);
  return audio;
};
Audio.prototype = HTMLAudioElement.prototype;

class HTMLDialogElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  show() {
    this.open = true;
  }

  showModal() {
    this.open = true;
  }

  close(value) {
    if (!this.open) return;
    if (value !== undefined) this.returnValue = String(value);
    this.open = false;
    fire(this, Event, 'close');
  }
}
reflect(HTMLDialogElement.prototype, [['open', null, 'boolean']]);
register(HTMLDialogElement, ['dialog']);

class HTMLDetailsElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }
}
reflect(HTMLDetailsElement.prototype, [['open', null, 'boolean'], ['name']]);
register(HTMLDetailsElement, ['details']);

class HTMLProgressElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get value() {
    const value = parseFloat(this.getAttribute('value'));
    return Number.isFinite(value) ? value : 0;
  }

  set value(value) {
    this.setAttribute('value', String(value));
  }

  get max() {
    const value = parseFloat(this.getAttribute('max'));
    return Number.isFinite(value) && value > 0 ? value : 1;
  }

  set max(value) {
    this.setAttribute('max', String(value));
  }

  get position() {
    return this.hasAttribute('value') ? this.value / this.max : -1;
  }
}
register(HTMLProgressElement, ['progress']);

// A template keeps its content in a fragment of its own, not as children:
// the children the parser gave it move there when the content is first used.
const templateContents = new WeakMap();

class HTMLTemplateElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get content() {
    let content = templateContents.get(this);
    if (!content) {
      content = wrap(native.createFragment());
      templateContents.set(this, content);
      while (this.firstChild) content.appendChild(this.firstChild);
    }
    return content;
  }

  get innerHTML() {
    return native.innerHTML(this.content[ID]);
  }

  set innerHTML(html) {
    const parsed = wrap(native.parseFragment(html === null ? '' : String(html), this[ID]));
    this.content.replaceChildren(parsed);
  }
}
register(HTMLTemplateElement, ['template']);

class HTMLIFrameElement extends HTMLElement {
  constructor() {
    return constructHTMLElement(new.target);
  }

  get contentWindow() {
    return null;
  }

  get contentDocument() {
    return null;
  }

  get sandbox() {
    return tokenList(this, 'sandbox');
  }

  set sandbox(value) {
    this.setAttribute('sandbox', value);
  }
}
reflect(HTMLIFrameElement.prototype, [
  ['src', null, 'url'],
  ['srcdoc'],
  ['name'],
  ['allow'],
  ['allowFullscreen', 'allowfullscreen', 'boolean'],
  ['width'],
  ['height'],
  ['referrerPolicy', 'referrerpolicy'],
  ['loading', null, 'string', 'eager'],
]);
register(HTMLIFrameElement, ['iframe']);

const HTMLScriptElement = defineElement('HTMLScriptElement', ['script']);
reflect(HTMLScriptElement.prototype, [['src', null, 'url'], ['type'], ['async', null, 'boolean'], ['defer', null, 'boolean'], ['crossOrigin', 'crossorigin']]);
Object.defineProperty(HTMLScriptElement.prototype, 'text', {
  get() {
    return this.textContent;
  },
  set(value) {
    this.textContent = value;
  },
});
const HTMLMetaElement = defineElement('HTMLMetaElement', ['meta']);
reflect(HTMLMetaElement.prototype, [['name'], ['content'], ['httpEquiv', 'http-equiv'], ['charset']]);
const HTMLTitleElement = defineElement('HTMLTitleElement', ['title']);
Object.defineProperty(HTMLTitleElement.prototype, 'text', {
  get() {
    return this.textContent;
  },
  set(value) {
    this.textContent = value;
  },
});
const HTMLOListElement = defineElement('HTMLOListElement', ['ol']);
reflect(HTMLOListElement.prototype, [['start', null, 'number', 1], ['reversed', null, 'boolean'], ['type']]);
const HTMLLIElement = defineElement('HTMLLIElement', ['li']);
reflect(HTMLLIElement.prototype, [['value', null, 'number', 0]]);
const HTMLFieldSetElement = defineElement('HTMLFieldSetElement', ['fieldset']);
reflect(HTMLFieldSetElement.prototype, [['disabled', null, 'boolean'], ['name']]);
const HTMLOutputElement = defineElement('HTMLOutputElement', ['output']);
Object.defineProperty(HTMLOutputElement.prototype, 'value', {
  get() {
    return this.textContent;
  },
  set(value) {
    this.textContent = value;
  },
});
const HTMLTableCellElement = defineElement('HTMLTableCellElement', ['td', 'th']);
reflect(HTMLTableCellElement.prototype, [['colSpan', 'colspan', 'number', 1], ['rowSpan', 'rowspan', 'number', 1], ['headers'], ['scope']]);
const simpleElements = {
  HTMLHtmlElement: ['html'],
  HTMLHeadElement: ['head'],
  HTMLBodyElement: ['body'],
  HTMLDivElement: ['div'],
  HTMLSpanElement: ['span'],
  HTMLParagraphElement: ['p'],
  HTMLHeadingElement: ['h1', 'h2', 'h3', 'h4', 'h5', 'h6'],
  HTMLUListElement: ['ul'],
  HTMLDListElement: ['dl'],
  HTMLPreElement: ['pre', 'listing', 'xmp'],
  HTMLBRElement: ['br'],
  HTMLHRElement: ['hr'],
  HTMLQuoteElement: ['blockquote', 'q'],
  HTMLModElement: ['ins', 'del'],
  HTMLTableElement: ['table'],
  HTMLTableRowElement: ['tr'],
  HTMLTableSectionElement: ['thead', 'tbody', 'tfoot'],
  HTMLTableCaptionElement: ['caption'],
  HTMLTableColElement: ['col', 'colgroup'],
  HTMLLegendElement: ['legend'],
  HTMLOptGroupElement: ['optgroup'],
  HTMLDataListElement: ['datalist'],
  HTMLMeterElement: ['meter'],
  HTMLTimeElement: ['time'],
  HTMLDataElement: ['data'],
  HTMLSourceElement: ['source'],
  HTMLTrackElement: ['track'],
  HTMLPictureElement: ['picture'],
  HTMLMapElement: ['map'],
  HTMLEmbedElement: ['embed'],
  HTMLObjectElement: ['object'],
  HTMLSlotElement: ['slot'],
  HTMLBaseElement: ['base'],
  HTMLMenuElement: ['menu'],
  HTMLUnknownElement: [],
};
const elementClasses = {
  HTMLElement,
  HTMLInputElement,
  HTMLTextAreaElement,
  HTMLSelectElement,
  HTMLOptionElement,
  HTMLButtonElement,
  HTMLFormElement,
  HTMLLabelElement,
  HTMLAnchorElement,
  HTMLAreaElement,
  HTMLImageElement,
  HTMLStyleElement,
  HTMLLinkElement,
  HTMLCanvasElement,
  HTMLMediaElement,
  HTMLVideoElement,
  HTMLAudioElement,
  MediaError,
  TimeRanges,
  HTMLDialogElement,
  HTMLDetailsElement,
  HTMLProgressElement,
  HTMLTemplateElement,
  HTMLIFrameElement,
  HTMLScriptElement,
  HTMLMetaElement,
  HTMLTitleElement,
  HTMLOListElement,
  HTMLLIElement,
  HTMLFieldSetElement,
  HTMLOutputElement,
  HTMLTableCellElement,
};
for (const [name, tags] of Object.entries(simpleElements)) {
  elementClasses[name] = defineElement(name, tags);
}

prototypeOf = (id) => {
  switch (native.kind(id)) {
    case 1: {
      const namespace = native.namespace(id);
      const name = native.localName(id);
      if (namespace === SVG_NAMESPACE) return (name === 'svg' ? SVGSVGElement : SVGElement).prototype;
      if (namespace !== HTML_NAMESPACE) return Element.prototype;
      return (htmlClasses[name] || HTMLElement).prototype;
    }
    case 3:
      return Text.prototype;
    case 8:
      return Comment.prototype;
    case 9:
      return Document.prototype;
    case 11:
      return (native.shadowHost(id) === null ? DocumentFragment : ShadowRoot).prototype;
    default:
      return EventTarget.prototype;
  }
};
