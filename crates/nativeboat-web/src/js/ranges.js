// Programmatic DOM ranges, their geometry from the painted text, and the
// document's single directional selection. Range boundaries live in the
// document, which moves them as nodes and text change; a range collected by
// the garbage collector releases its boundaries the next time ranges grow.

const rangeIds = new WeakMap();
const liveRanges = new Map();
let pruneAt = 64;
let selectedRange = null;
let selectionDirection = 'none';
let selectionChangePending = false;

const selectionChanged = () => {
  if (selectionChangePending) return;
  selectionChangePending = true;
  setTimeout(() => {
    selectionChangePending = false;
    fire(document, Event, 'selectionchange');
  }, 0);
};
const rangeChanged = (range) => {
  if (range === selectedRange) selectionChanged();
};
const boundaryLength = (node) =>
  [3, 4, 7, 8].includes(node.nodeType) ? node.data.length : node.childNodes.length;
const boundary = (node, offset) => {
  idOf(node);
  if (node.nodeType === 10) throw new DOMException('A document type cannot be a range boundary.', 'InvalidNodeTypeError');
  offset = (+offset) >>> 0;
  if (offset > boundaryLength(node)) throw new DOMException('The offset exceeds the node length.', 'IndexSizeError');
  return [node, offset];
};
const childIndex = (node) => native.index(node[ID]);
const besideBoundary = (node, after) => {
  idOf(node);
  if (!node.parentNode) throw new DOMException('The node has no parent.', 'InvalidNodeTypeError');
  return [node.parentNode, childIndex(node) + (after ? 1 : 0)];
};
const samePoint = (a, b) => a[0] === b[0] && a[1] === b[1];
const rangeState = (range) => {
  const [start, startOffset, end, endOffset] = native.range(rangeIds.get(range));
  return { start: [wrap(start), startOffset], end: [wrap(end), endOffset] };
};
const replaceBoundaries = (range, start, end) => {
  const previous = rangeState(range);
  native.setRange(rangeIds.get(range), start[0][ID], start[1], end[0][ID], end[1]);
  if (!samePoint(previous.start, start) || !samePoint(previous.end, end)) rangeChanged(range);
};

// Orders boundary points, including an ancestor's offset before/after the
// child containing the other point. Different roots have no ordering.
const comparePoints = ([a, offsetA], [b, offsetB]) => {
  if (a === b) return Math.sign(offsetA - offsetB);
  if (a.getRootNode() !== b.getRootNode()) return null;
  if (a.contains(b)) {
    let child = b;
    while (child.parentNode !== a) child = child.parentNode;
    return offsetA <= childIndex(child) ? -1 : 1;
  }
  if (b.contains(a)) return -comparePoints([b, offsetB], [a, offsetA]);
  return a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1;
};
// Gives a range boundaries in the document, first releasing the boundaries of
// collected ranges once the live ones have doubled.
const registerRange = (range, start, end) => {
  if (liveRanges.size >= pruneAt) {
    for (const [id, reference] of liveRanges) {
      if (reference.deref()) continue;
      liveRanges.delete(id);
      native.dropRange(id);
    }
    pruneAt = Math.max(64, liveRanges.size * 2);
  }
  const id = native.createRange(start[0][ID], start[1], end[0][ID], end[1]);
  rangeIds.set(range, id);
  liveRanges.set(id, new WeakRef(range));
  return range;
};
const createRange = (start, end = start) => registerRange(Object.create(Range.prototype), start, end);
const commonAncestor = ({ start, end }) => {
  const first = ancestors(start[0]), last = ancestors(end[0]);
  let i = 0;
  while (i < first.length && i < last.length && first[i] === last[i]) i++;
  return first[i - 1];
};
const rangeText = (state) => {
  const { start, end } = state;
  if (comparePoints(start, end) === 0) return '';
  let text = '';
  const visit = (node) => {
    if (node.nodeType === 3 || node.nodeType === 4) {
      if (comparePoints([node, node.data.length], start) <= 0 || comparePoints([node, 0], end) >= 0) return;
      text += node.data.slice(node === start[0] ? start[1] : 0, node === end[0] ? end[1] : node.data.length);
    } else {
      for (const child of node.childNodes) visit(child);
    }
  };
  visit(commonAncestor(state));
  return text;
};

// The client rectangles of a range: the border boxes of the elements it
// contains whose parents it does not contain, and the part of each text node
// it touches, which for a collapsed range in text is the caret.
const rangeRects = (state) => {
  const { start, end } = state;
  // A collapsed range between an element's children covers no text and
  // has no rectangles, as in Chromium.
  if (start[0] === end[0] && start[1] === end[1] && start[0].nodeType === 1) return [];
  const rects = [];
  const visit = (node, inside) => {
    if (node.nodeType === 3) {
      if (comparePoints([node, node.data.length], start) < 0 || comparePoints([node, 0], end) > 0) return;
      const from = node === start[0] ? start[1] : 0, to = node === end[0] ? end[1] : node.data.length;
      for (const [x, y, width, height] of page.textRects(node[ID], from, to)) rects.push(new DOMRect(x, y, width, height));
      return;
    }
    const contained = node.nodeType === 1 &&
      comparePoints([node, 0], start) > 0 && comparePoints([node, boundaryLength(node)], end) < 0;
    if (contained && !inside) rects.push(...node.getClientRects());
    for (const child of node.childNodes) visit(child, contained);
  };
  visit(commonAncestor(state), false);
  return rects;
};

// The smallest rectangle holding the rectangles that have an area, or the
// first rectangle when none has.
const boundingRect = (rects) => {
  const sized = rects.filter((rect) => rect.width && rect.height);
  if (!sized.length) return rects[0] ?? new DOMRect();
  const left = Math.min(...sized.map((rect) => rect.left)), top = Math.min(...sized.map((rect) => rect.top));
  const right = Math.max(...sized.map((rect) => rect.right)), bottom = Math.max(...sized.map((rect) => rect.bottom));
  return new DOMRect(left, top, right - left, bottom - top);
};

// The caret position nearest a viewport point, as `[node, offset]`.
const caretPoint = (x, y) => {
  const caret = page.caretFromPoint(Number(x), Number(y));
  return caret && [wrap(caret[0]), caret[1]];
};

// Range contents (DOM Standard, "live ranges"). A node is contained when the
// range holds all of it, partially contained when it holds one boundary.
const isCharacterData = (node) => [3, 4, 7, 8].includes(node.nodeType);
const rangeContains = ({ start, end }, node) =>
  comparePoints([node, 0], start) > 0 && comparePoints([node, boundaryLength(node)], end) < 0;
const partiallyContained = ({ start, end }, node) => node.contains(start[0]) !== node.contains(end[0]);
const rangeRoot = (state) => state.start[0].getRootNode();
const wrongDocument = () => new DOMException('The node is in another document.', 'WrongDocumentError');
// Where a range collapses once its contents are gone: its start, or after
// the start's ancestor that is a child of the common ancestor.
const deletionPoint = ({ start: [node, offset], end: [endNode] }) => {
  if (node.contains(endNode)) return [node, offset];
  let reference = node;
  while (!reference.parentNode.contains(endNode)) reference = reference.parentNode;
  return [reference.parentNode, childIndex(reference) + 1];
};
const characterSlice = (node, from, to) => {
  const copy = node.cloneNode(false);
  copy.data = node.data.slice(from, to);
  return copy;
};
// Moves the contents of a range into a fragment, or copies them with
// `copy`; a moved range collapses where its contents were.
const transferContents = (range, state, copy) => {
  const fragment = document.createDocumentFragment();
  const { start: [startNode, startOffset], end: [endNode, endOffset] } = state;
  if (comparePoints(state.start, state.end) === 0) return fragment;
  if (startNode === endNode && isCharacterData(startNode)) {
    fragment.appendChild(characterSlice(startNode, startOffset, endOffset));
    if (!copy) startNode.replaceData(startOffset, endOffset - startOffset, '');
    return fragment;
  }
  let common = startNode;
  while (!common.contains(endNode)) common = common.parentNode;
  const children = [...common.childNodes];
  const partial = children.filter((child) => partiallyContained(state, child));
  const first = startNode.contains(endNode) ? null : partial[0];
  const last = endNode.contains(startNode) ? null : partial[partial.length - 1];
  const contained = children.filter((child) => rangeContains(state, child));
  if (contained.some((child) => child.nodeType === 10)) {
    throw new DOMException('A document type cannot be moved into a fragment.', 'HierarchyRequestError');
  }
  const point = deletionPoint(state);
  if (first && isCharacterData(first)) {
    fragment.appendChild(characterSlice(startNode, startOffset, startNode.data.length));
    if (!copy) startNode.replaceData(startOffset, startNode.data.length - startOffset, '');
  } else if (first) {
    fragment.appendChild(first.cloneNode(false))
      .appendChild(transferContents(null, { start: state.start, end: [first, boundaryLength(first)] }, copy));
  }
  for (const child of contained) fragment.appendChild(copy ? child.cloneNode(true) : child);
  if (last && isCharacterData(last)) {
    fragment.appendChild(characterSlice(endNode, 0, endOffset));
    if (!copy) endNode.replaceData(0, endOffset, '');
  } else if (last) {
    fragment.appendChild(last.cloneNode(false))
      .appendChild(transferContents(null, { start: [last, 0], end: state.end }, copy));
  }
  if (range && !copy) replaceBoundaries(range, point, point);
  return fragment;
};
const deleteRangeContents = (range) => {
  const state = rangeState(range);
  const { start: [startNode, startOffset], end: [endNode, endOffset] } = state;
  if (comparePoints(state.start, state.end) === 0) return;
  if (startNode === endNode && isCharacterData(startNode)) {
    startNode.replaceData(startOffset, endOffset - startOffset, '');
    return;
  }
  const removed = [];
  const visit = (node) => {
    for (const child of node.childNodes) {
      if (rangeContains(state, child)) removed.push(child);
      else if (partiallyContained(state, child)) visit(child);
    }
  };
  visit(commonAncestor(state));
  const point = deletionPoint(state);
  if (isCharacterData(startNode)) startNode.replaceData(startOffset, startNode.data.length - startOffset, '');
  for (const node of removed) node.parentNode.removeChild(node);
  if (isCharacterData(endNode)) endNode.replaceData(0, endOffset, '');
  replaceBoundaries(range, point, point);
};
const insertIntoRange = (range, node) => {
  idOf(node);
  const { start: [startNode, startOffset] } = rangeState(range);
  if ([7, 8].includes(startNode.nodeType) || (startNode.nodeType === 3 && !startNode.parentNode) || startNode === node) {
    throw new DOMException('The node cannot be inserted at the start of the range.', 'HierarchyRequestError');
  }
  let reference = startNode.nodeType === 3 ? startNode : wrap(native.children(startNode[ID])[startOffset] ?? null);
  const parent = reference ? reference.parentNode : startNode;
  if (node.contains(parent) || node.nodeType === 9 || (parent.nodeType === 9 && node.nodeType === 3)) {
    throw new DOMException('The node cannot be inserted there.', 'HierarchyRequestError');
  }
  if (startNode.nodeType === 3) reference = startNode.splitText(startOffset);
  if (node === reference) reference = reference.nextSibling;
  if (node.parentNode) node.parentNode.removeChild(node);
  const offset = (reference ? childIndex(reference) : parent.childNodes.length) +
    (node.nodeType === 11 ? node.childNodes.length : 1);
  parent.insertBefore(node, reference);
  const state = rangeState(range);
  if (comparePoints(state.start, state.end) === 0) replaceBoundaries(range, state.start, [parent, offset]);
};
const rangePoint = (state, node, offset) => {
  idOf(node);
  if (node.getRootNode() !== rangeRoot(state)) return null;
  return boundary(node, offset);
};

const caretStates = new WeakMap();

class CaretPosition {
  constructor() { illegal(); }

  get offsetNode() { return caretStates.get(this)[0]; }
  get offset() { return caretStates.get(this)[1]; }

  getClientRect() {
    const point = caretStates.get(this);
    const rects = rangeRects({ start: point, end: point });
    return rects.length ? rects[0] : null;
  }
}
Object.defineProperty(CaretPosition.prototype, Symbol.toStringTag, { value: 'CaretPosition', configurable: true });

const caretPosition = (x, y) => {
  const point = caretPoint(x, y);
  if (!point) return null;
  const position = Object.create(CaretPosition.prototype);
  caretStates.set(position, point);
  return position;
};

class Range {
  constructor() {
    registerRange(this, [document, 0], [document, 0]);
  }

  // Scalar reads avoid materializing both boundary arrays for each property.
  get startContainer() { return wrap(native.rangeField(rangeIds.get(this), 0)); }
  get startOffset() { return native.rangeField(rangeIds.get(this), 1); }
  get endContainer() { return wrap(native.rangeField(rangeIds.get(this), 2)); }
  get endOffset() { return native.rangeField(rangeIds.get(this), 3); }
  get collapsed() { return native.rangeField(rangeIds.get(this), 4); }
  get commonAncestorContainer() { return wrap(native.rangeField(rangeIds.get(this), 5)); }

  setStart(node, offset) {
    const point = boundary(node, offset), state = rangeState(this);
    const order = comparePoints(point, state.end);
    replaceBoundaries(this, point, order === null || order > 0 ? [...point] : state.end);
  }

  setEnd(node, offset) {
    const point = boundary(node, offset), state = rangeState(this);
    const order = comparePoints(point, state.start);
    replaceBoundaries(this, order === null || order < 0 ? [...point] : state.start, point);
  }

  setStartBefore(node) { this.setStart(...besideBoundary(node, false)); }
  setStartAfter(node) { this.setStart(...besideBoundary(node, true)); }
  setEndBefore(node) { this.setEnd(...besideBoundary(node, false)); }
  setEndAfter(node) { this.setEnd(...besideBoundary(node, true)); }

  selectNode(node) {
    const start = besideBoundary(node, false), end = besideBoundary(node, true);
    replaceBoundaries(this, start, end);
  }

  selectNodeContents(node) {
    const start = boundary(node, 0), end = boundary(node, boundaryLength(node));
    replaceBoundaries(this, start, end);
  }

  collapse(toStart = false) {
    const state = rangeState(this), point = toStart ? state.start : state.end;
    replaceBoundaries(this, [...point], [...point]);
  }

  cloneRange() {
    const { start, end } = rangeState(this);
    return createRange(start, end);
  }

  toString() { return rangeText(rangeState(this)); }

  getBoundingClientRect() { return boundingRect(rangeRects(rangeState(this))); }

  getClientRects() {
    const state = rangeState(this);
    const rects = rangeRects(state);
    // NATIVEBOAT_TRACE_INPUT shows the rectangles scripts read, as editors
    // place carets and scroll by them.
    if (native.tracing()) {
      const at = (point) => {
        const [node, offset] = point;
        const text = node.nodeType === 3 ? JSON.stringify(node.data.slice(0, 16)) : `<${node.localName || node.nodeName}>`;
        return `${text}:${offset}`;
      };
      const list = rects.map((rect) => `${Math.round(rect.x)},${Math.round(rect.y)},${Math.round(rect.width)}x${Math.round(rect.height)}`).join(' ');
      // Where the start is: its first ancestors, and its element's box.
      const chain = [];
      for (let element = state.start[0].parentElement; element && chain.length < 4; element = element.parentElement) {
        chain.push(`${element.localName}.${String(element.getAttribute('class') || '').split(' ')[0]}`);
      }
      const box = state.start[0].parentElement && page.rect(state.start[0].parentElement[ID]);
      const placed = box ? `${Math.round(box[0])},${Math.round(box[1])},${Math.round(box[2])}x${Math.round(box[3])}` : 'none';
      native.trace(`range rects ${at(state.start)}-${at(state.end)} [${list}] in ${chain.join('<')} @${placed}`);
    }
    return collection(DOMRectList, () => rects);
  }

  detach() {}

  deleteContents() { deleteRangeContents(this); }
  extractContents() { return transferContents(this, rangeState(this), false); }
  cloneContents() { return transferContents(null, rangeState(this), true); }
  insertNode(node) { insertIntoRange(this, node); }

  surroundContents(newParent) {
    idOf(newParent);
    const state = rangeState(this);
    const blocking = (node) => node.nodeType !== 3 && partiallyContained(state, node);
    if (ancestors(state.start[0]).some(blocking) || ancestors(state.end[0]).some(blocking)) {
      throw new DOMException('The range partially holds a non-text node.', 'InvalidStateError');
    }
    if ([9, 10, 11].includes(newParent.nodeType)) {
      throw new DOMException('The node cannot surround the range.', 'InvalidNodeTypeError');
    }
    const fragment = this.extractContents();
    while (newParent.firstChild) newParent.removeChild(newParent.firstChild);
    this.insertNode(newParent);
    newParent.appendChild(fragment);
    this.selectNode(newParent);
  }

  compareBoundaryPoints(how, sourceRange) {
    how = (+how) & 0xffff;
    if (how > 3) throw new DOMException('The comparison is not supported.', 'NotSupportedError');
    if (!(sourceRange instanceof Range)) throw new TypeError('The value is not a Range.');
    const mine = rangeState(this), theirs = rangeState(sourceRange);
    if (rangeRoot(mine) !== rangeRoot(theirs)) throw wrongDocument();
    const point = [mine.start, mine.end, mine.end, mine.start][how];
    return comparePoints(point, [theirs.start, theirs.start, theirs.end, theirs.end][how]);
  }

  comparePoint(node, offset) {
    const state = rangeState(this), point = rangePoint(state, node, offset);
    if (!point) throw wrongDocument();
    if (comparePoints(point, state.start) < 0) return -1;
    return comparePoints(point, state.end) > 0 ? 1 : 0;
  }

  isPointInRange(node, offset) {
    const state = rangeState(this), point = rangePoint(state, node, offset);
    return !!point && comparePoints(point, state.start) >= 0 && comparePoints(point, state.end) <= 0;
  }

  intersectsNode(node) {
    idOf(node);
    const state = rangeState(this);
    if (node.getRootNode() !== rangeRoot(state)) return false;
    const parent = node.parentNode;
    if (!parent) return true;
    const offset = childIndex(node);
    return comparePoints([parent, offset], state.end) < 0 && comparePoints([parent, offset + 1], state.start) > 0;
  }

  createContextualFragment(html) {
    const fragment = wrap(native.parseFragment(String(html), native.htmlChild('body') ?? native.documentElement()));
    parsedMarkup(String(html), fragment);
    return fragment;
  }
}

const sameBoundaries = (a, b) => {
  if (!a || !b) return a === b;
  const first = rangeState(a), second = rangeState(b);
  return first.start[0] === second.start[0] && first.start[1] === second.start[1] &&
    first.end[0] === second.end[0] && first.end[1] === second.end[1];
};
const associateRange = (range, direction = 'none', focusHost = true) => {
  if (selectedRange === range && selectionDirection === direction) return;
  // A selection set where it already is neither fires `selectionchange` nor
  // focuses its editing host, as in Chromium; the new range still becomes
  // the selection's.
  const moved = selectionDirection !== direction || !sameBoundaries(selectedRange, range);
  selectedRange = range;
  selectionDirection = direction;
  native.setSelection(range ? rangeIds.get(range) : null);
  if (!moved) return;
  selectionChanged();
  if (range && focusHost) focusEditingHost(range);
};
const selectionBoundary = (anchor) => {
  if (!selectedRange) return [null, 0];
  const state = rangeState(selectedRange);
  return (anchor !== (selectionDirection === 'backward')) ? state.start : state.end;
};
// The Selection holds the direction; the live endpoints remain native.
const selectionField = (anchor, offset) => {
  if (!selectedRange) return offset ? 0 : null;
  const start = anchor !== (selectionDirection === 'backward');
  const value = native.rangeField(rangeIds.get(selectedRange), (start ? 0 : 2) + (offset ? 1 : 0));
  return offset ? value : wrap(value);
};
const selectEndpoints = (anchor, focus) => {
  const order = comparePoints(anchor, focus);
  const backwards = order > 0;
  associateRange(order === null ? createRange(focus) : backwards ? createRange(focus, anchor) : createRange(anchor, focus), backwards ? 'backward' : 'forward');
};

class Selection {
  constructor() { illegal(); }

  get anchorNode() { return selectionField(true, false); }
  get anchorOffset() { return selectionField(true, true); }
  get focusNode() { return selectionField(false, false); }
  get focusOffset() { return selectionField(false, true); }
  get rangeCount() { return selectedRange ? 1 : 0; }
  get isCollapsed() { return !selectedRange || selectedRange.collapsed; }
  get type() { return !selectedRange ? 'None' : selectedRange.collapsed ? 'Caret' : 'Range'; }
  get direction() { return selectedRange ? selectionDirection : 'none'; }

  addRange(range) {
    if (!(range instanceof Range)) throw new TypeError('The value is not a Range.');
    if (selectedRange || range.startContainer.getRootNode() !== document) return;
    associateRange(range);
  }

  getRangeAt(index) {
    if (!selectedRange || ((+index) >>> 0) !== 0) throw new DOMException('The selection has no range at that index.', 'IndexSizeError');
    return selectedRange;
  }

  removeRange(range) {
    if (!(range instanceof Range)) throw new TypeError('The value is not a Range.');
    if (range !== selectedRange) throw new DOMException('The range is not in this selection.', 'NotFoundError');
    this.removeAllRanges();
  }

  removeAllRanges() { associateRange(null); }

  collapse(node, offset = 0) {
    if (node === null) return this.removeAllRanges();
    const point = boundary(node, offset);
    if (node.getRootNode() !== document) return;
    associateRange(createRange(point));
  }

  collapseToStart() {
    if (!selectedRange) throw new DOMException('The selection is empty.', 'InvalidStateError');
    associateRange(createRange(rangeState(selectedRange).start));
  }

  collapseToEnd() {
    if (!selectedRange) throw new DOMException('The selection is empty.', 'InvalidStateError');
    associateRange(createRange(rangeState(selectedRange).end));
  }

  extend(node, offset = 0) {
    idOf(node);
    if (node.getRootNode() !== document) return;
    if (!selectedRange) throw new DOMException('The selection is empty.', 'InvalidStateError');
    selectEndpoints(selectionBoundary(true), boundary(node, offset));
  }

  setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset) {
    const anchor = boundary(anchorNode, anchorOffset), focus = boundary(focusNode, focusOffset);
    if (anchorNode.getRootNode() !== document || focusNode.getRootNode() !== document) return;
    selectEndpoints(anchor, focus);
  }

  selectAllChildren(node) {
    const start = boundary(node, 0);
    if (node.getRootNode() !== document) return;
    associateRange(createRange(start, boundary(node, node.childNodes.length)), 'forward');
  }

  containsNode(node, allowPartialContainment = false) {
    idOf(node);
    if (!selectedRange || node.getRootNode() !== document) return false;
    const { start, end } = rangeState(selectedRange);
    const first = [node, 0], last = [node, boundaryLength(node)];
    return allowPartialContainment
      ? comparePoints(start, last) <= 0 && comparePoints(end, first) >= 0
      : comparePoints(start, first) <= 0 && comparePoints(end, last) >= 0;
  }

  toString() { return selectedRange ? String(selectedRange) : ''; }

  deleteFromDocument() {
    if (selectedRange) selectedRange.deleteContents();
  }
}
for (const [name, value] of Object.entries({ START_TO_START: 0, START_TO_END: 1, END_TO_END: 2, END_TO_START: 3 })) {
  Object.defineProperty(Range, name, { value, enumerable: true });
  Object.defineProperty(Range.prototype, name, { value, enumerable: true });
}
hidden(Selection.prototype, 'empty', Selection.prototype.removeAllRanges);
hidden(Selection.prototype, 'setPosition', Selection.prototype.collapse);
Object.defineProperty(Selection.prototype, Symbol.toStringTag, { value: 'Selection', configurable: true });
Object.defineProperty(Range.prototype, Symbol.toStringTag, { value: 'Range', configurable: true });
const selection = Object.create(Selection.prototype);
