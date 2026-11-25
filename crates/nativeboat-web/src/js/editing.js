// Editing hosts: the caret and the edits a user makes in contenteditable
// content the page paints — typing, deleting, breaking lines and
// paragraphs, moving the caret by keys and selecting with the mouse. Edits
// follow Chromium's: they fire beforeinput and input at the editing host,
// keep blocks from collapsing with a placeholder <br> and keep collapsible
// spaces visible with no-break spaces. ---------------------------------------

const editingHostOf = (node) => wrap(native.editingHost(node[ID]));
const childAt = (node, index) => wrap(native.children(node[ID])[index] ?? null);
const beforeNode = (node) => [node.parentNode, childIndex(node)];
const afterNode = (node) => [node.parentNode, childIndex(node) + 1];
const styleValue = (element, name) => page.computedStyleValue(element[ID], name);

// The selection when it lies in the focused editing host: the host and the
// selection's boundary points in document order.
const editingSelection = () => {
  if (!selectedRange) return null;
  const { start, end } = rangeState(selectedRange);
  const host = editingHostOf(start[0]);
  if (!host || host[ID] !== native.focused() || editingHostOf(end[0]) !== host) return null;
  return { host, start, end, collapsed: comparePoints(start, end) === 0 };
};

// Selecting content of an editing host focuses it.
const focusEditingHost = (range) => {
  const { start, end } = rangeState(range);
  const host = editingHostOf(start[0]);
  if (host && editingHostOf(end[0]) === host && host[ID] !== native.focused()) focusNode(host, { preventScroll: true });
};

// Content the caret passes as a whole: replaced elements and content that
// is not editable. Leaves are such atoms and text that is drawn.
const replacedElements = new Set([
  'br', 'img', 'hr', 'input', 'textarea', 'select', 'button', 'video', 'audio', 'canvas', 'iframe', 'svg',
  'object', 'embed', 'math', 'meter', 'progress',
]);
const isAtom = (node, host) =>
  node.nodeType === 1 && node !== host && (replacedElements.has(node.localName) || editingHostOf(node) !== host);
const isShownText = (node) =>
  node.nodeType === 3 && node.data.length > 0 && page.textRects(node[ID], 0, node.data.length).length > 0;
// A block with nothing in it that still draws a line, as generated content
// or a height gives it, holds the caret at its start.
const holdsContent = (node, host) => [...node.childNodes].some((child) => isShownText(child) ||
  (child.nodeType === 1 && (isAtom(child, host) || isBlock(child) || holdsContent(child, host))));
const isEmptyLine = (node, host) => {
  if (node.nodeType !== 1 || node === host || isAtom(node, host) || !isBlock(node)) return false;
  const rect = page.rect(node[ID]);
  return !!rect && rect[3] > 0 && !holdsContent(node, host);
};
const isLeaf = (node, host) => isShownText(node) || isAtom(node, host) || isEmptyLine(node, host);
const isBreak = (node) => node.nodeType === 1 && node.localName === 'br';

// Tree order inside a host, never entering atoms.
const nextInHost = (node, host, enter) => {
  if (enter && node.firstChild && !isAtom(node, host)) return node.firstChild;
  for (; node && node !== host; node = node.parentNode) {
    if (node.nextSibling) return node.nextSibling;
  }
  return null;
};
const lastInside = (node, host) => {
  while (node.lastChild && !isAtom(node, host)) node = node.lastChild;
  return node;
};
const previousInHost = (node, host) => {
  if (node === host || !node.parentNode) return null;
  if (node.previousSibling) return lastInside(node.previousSibling, host);
  return node.parentNode === host ? null : node.parentNode;
};
// The first leaf after a boundary point, and the last one before it.
const leafAfter = ([container, offset], host) => {
  let node = isCharacterData(container) || offset >= container.childNodes.length
    ? nextInHost(container, host, false)
    : childAt(container, offset);
  while (node && !isLeaf(node, host)) node = nextInHost(node, host, true);
  return node;
};
const leafBefore = ([container, offset], host) => {
  let node = isCharacterData(container) || offset === 0
    ? previousInHost(container, host)
    : lastInside(childAt(container, offset - 1), host);
  while (node && !isLeaf(node, host)) node = previousInHost(node, host);
  return node;
};
const holdsLeaf = (element, host) => {
  const leaf = leafAfter([element, 0], host);
  return !!leaf && element.contains(leaf);
};

// Blocks: elements laid out as blocks, judged by their computed display or,
// without one, by their name.
const blockElements = new Set([
  'address', 'article', 'aside', 'blockquote', 'body', 'caption', 'center', 'dd', 'details', 'dialog', 'div', 'dl',
  'dt', 'fieldset', 'figcaption', 'figure', 'footer', 'form', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'header', 'hgroup',
  'hr', 'html', 'legend', 'li', 'listing', 'main', 'menu', 'nav', 'ol', 'p', 'pre', 'section', 'summary', 'table',
  'tbody', 'td', 'tfoot', 'th', 'thead', 'tr', 'ul', 'xmp',
]);
const isBlock = (node) => {
  if (node.nodeType !== 1) return false;
  const display = styleValue(node, 'display');
  if (!display) return blockElements.has(node.localName);
  return !display.startsWith('inline') && display !== 'contents' && display !== 'none';
};
// The block holding a node, or the host when no block inside it does.
const blockAround = (node, host) => {
  for (let element = node.parentNode; element && element !== host; element = element.parentNode) {
    if (isBlock(element)) return element;
  }
  return host;
};
const blockAt = ([container], host) =>
  container === host || isBlock(container) ? container : blockAround(container, host);
// A <br> that ends its block draws no line of its own.
const endsBlock = (br, host) => {
  const next = leafAfter(afterNode(br), host);
  return !next || isBlock(next) || blockAround(next, host) !== blockAround(br, host);
};

// Character clusters: a code point with the marks, joiners, variation
// selectors and modifiers that extend it, and pairs of regional indicators.
const extending = /[\p{M}\u200c\u200d\ufe0e\ufe0f\u{1f3fb}-\u{1f3ff}\u{e0020}-\u{e007f}]/u;
const isRegional = (point) => point >= 0x1f1e6 && point <= 0x1f1ff;
const codePointEnd = (text, index) => index + (text.codePointAt(index) > 0xffff ? 2 : 1);
const codePointStart = (text, index) =>
  index - (index > 1 && /[\udc00-\udfff]/.test(text[index - 1]) && /[\ud800-\udbff]/.test(text[index - 2]) ? 2 : 1);
const codePointAt = (text, index) => String.fromCodePoint(text.codePointAt(index));
const clusterEnd = (text, index) => {
  let end = codePointEnd(text, index);
  if (isRegional(text.codePointAt(index)) && end < text.length && isRegional(text.codePointAt(end))) {
    return codePointEnd(text, end);
  }
  while (end < text.length && (text[end - 1] === '\u200d' || extending.test(codePointAt(text, end)))) {
    end = codePointEnd(text, end);
  }
  return end;
};
const clusterStart = (text, index) => {
  let start = codePointStart(text, index);
  if (isRegional(text.codePointAt(start)) && start > 0 && isRegional(text.codePointAt(codePointStart(text, start)))) {
    return codePointStart(text, start);
  }
  while (start > 0 && (extending.test(codePointAt(text, start)) || text[start - 1] === '\u200d')) {
    start = codePointStart(text, start);
  }
  return start;
};

// Caret steps. A step reaches a point and passes a character: '\n' for a
// line or block boundary and U+FFFC for an atom.
const lineStartAt = (leaf, host) => {
  if (leaf.nodeType === 3) return { point: [leaf, 0], char: '\n' };
  if (isEmptyLine(leaf, host)) return { point: [leaf, 0], char: '\n' };
  if (isBlock(leaf)) {
    const next = leafAfter(afterNode(leaf), host);
    return next ? lineStartAt(next, host) : null;
  }
  return { point: beforeNode(leaf), char: '\n' };
};
const lineEndAt = (leaf, host) => {
  if (leaf.nodeType === 3) return { point: [leaf, leaf.data.length], char: '\n' };
  if (isEmptyLine(leaf, host)) return { point: [leaf, 0], char: '\n' };
  if (isBlock(leaf)) {
    const previous = leafBefore(beforeNode(leaf), host);
    return previous ? lineEndAt(previous, host) : null;
  }
  return { point: isBreak(leaf) ? beforeNode(leaf) : afterNode(leaf), char: '\n' };
};
const stepForward = (point, host) => {
  const [container, offset] = point;
  if (container.nodeType === 3 && offset < container.data.length) {
    const end = clusterEnd(container.data, offset);
    return { point: [container, end], char: container.data.slice(offset, end) };
  }
  const leaf = leafAfter(point, host);
  if (!leaf) return null;
  if (isBlock(leaf) || blockAround(leaf, host) !== blockAt(point, host)) return lineStartAt(leaf, host);
  if (leaf.nodeType === 3) {
    const end = clusterEnd(leaf.data, 0);
    return { point: [leaf, end], char: leaf.data.slice(0, end) };
  }
  if (!isBreak(leaf)) return { point: afterNode(leaf), char: '\ufffc' };
  if (!endsBlock(leaf, host)) return { point: afterNode(leaf), char: '\n' };
  const next = leafAfter(afterNode(leaf), host);
  return next && lineStartAt(next, host);
};
const stepBackward = (point, host) => {
  const [container, offset] = point;
  if (container.nodeType === 3 && offset > 0) {
    const start = clusterStart(container.data, offset);
    return { point: [container, start], char: container.data.slice(start, offset) };
  }
  const leaf = leafBefore(point, host);
  if (!leaf) return null;
  if (isBlock(leaf) || blockAround(leaf, host) !== blockAt(point, host)) return lineEndAt(leaf, host);
  if (leaf.nodeType === 3) {
    const start = clusterStart(leaf.data, leaf.data.length);
    return { point: [leaf, start], char: leaf.data.slice(start) };
  }
  if (!isBreak(leaf)) return { point: beforeNode(leaf), char: '\ufffc' };
  return endsBlock(leaf, host) ? stepBackward(beforeNode(leaf), host) : { point: beforeNode(leaf), char: '\n' };
};

// The caret position a point stands for, in text where there is some: the
// end of the text before it or the start of the text after it on its line,
// or the nearest line when the point lies between blocks.
const canonicalPoint = (point, host) => {
  if (point[0].nodeType === 3) return point;
  const block = blockAt(point, host);
  if (isEmptyLine(block, host)) return point;
  const before = leafBefore(point, host), after = leafAfter(point, host);
  const inBlock = (leaf) => !!leaf && !isBlock(leaf) && blockAround(leaf, host) === block;
  if (inBlock(before) && isBreak(before) && endsBlock(before, host)) return canonicalPoint(beforeNode(before), host);
  if (inBlock(before) && before.nodeType === 3) return [before, before.data.length];
  if (inBlock(after) && after.nodeType === 3) return [after, 0];
  if (inBlock(before) || inBlock(after)) return point;
  const line = (after && lineStartAt(after, host)) || (before && lineEndAt(before, host));
  return line ? canonicalPoint(line.point, host) : point;
};
const edgeCaret = (element, host, end) => canonicalPoint([element, end ? element.childNodes.length : 0], host);

// Script focusing an editing host puts the caret at its start unless the
// selection is already inside it.
const placeEditingCaret = (element) => {
  if (native.focused() !== element[ID] || native.editingHost(element[ID]) !== element[ID]) return;
  if (selectedRange) {
    const { start, end } = rangeState(selectedRange);
    if (element.contains(start[0]) && element.contains(end[0])) return;
  }
  associateRange(createRange(edgeCaret(element, element, false)));
};

// Collapsible spaces. Where white space collapses, a run of spaces stays
// visible as Chromium keeps it: alternating with no-break spaces, which
// also stand at the start and end of a line.
const isSpace = (char) => char === ' ' || char === '\u00a0';
const collapsesSpaces = (text) => {
  const value = text.parentElement ? styleValue(text.parentElement, 'white-space') : '';
  return !value || value === 'normal' || value === 'nowrap' || value === 'pre-line';
};
const lineNeighbour = (text, host, forward) => {
  const leaf = forward ? leafAfter([text, text.data.length], host) : leafBefore([text, 0], host);
  return leaf && !isBreak(leaf) && !isBlock(leaf) && blockAround(leaf, host) === blockAround(text, host) ? leaf : null;
};
const rebalanceSpaces = (text, from, to, host) => {
  if (!collapsesSpaces(text)) return;
  const data = text.data;
  let start = from, end = to;
  while (start > 0 && isSpace(data[start - 1])) start--;
  while (end < data.length && isSpace(data[end])) end++;
  const before = start === 0 ? lineNeighbour(text, host, false) : null;
  const lineStart = start === 0 && !before;
  const lineEnd = end === data.length && !lineNeighbour(text, host, true);
  let previousSpace = !!before && before.nodeType === 3 && before.data.endsWith(' ');
  let rebalanced = '';
  for (let index = start; index < end; index++) {
    if (!isSpace(data[index])) {
      rebalanced += data[index];
      previousSpace = false;
    } else if (previousSpace || (index === 0 && lineStart) || (index === data.length - 1 && lineEnd)) {
      rebalanced += '\u00a0';
      previousSpace = false;
    } else {
      rebalanced += ' ';
      previousSpace = true;
    }
  }
  for (let index = 0; index < rebalanced.length; index++) {
    if (rebalanced[index] !== data[start + index]) text.replaceData(start + index, 1, rebalanced[index]);
  }
};

// Empty text and inline elements an edit leaves behind; `pruneNode`
// removes such a node, or the empty ones at one edge of an inline element
// and then the element when nothing is left of it.
const isEmptyInline = (node) => node.nodeType === 3
  ? node.data === ''
  : node.nodeType === 1 && !node.hasChildNodes() && !replacedElements.has(node.localName) && !isBlock(node);
const pruneNode = (node, end) => {
  if (node.nodeType === 1 && !isBlock(node)) {
    let child;
    while ((child = end ? node.lastChild : node.firstChild) && pruneNode(child, end));
  }
  if (!isEmptyInline(node)) return false;
  node.parentNode.removeChild(node);
  return true;
};

// Removes the empty text or inline elements holding a caret range, gives
// the caret's block a placeholder <br> when nothing else draws it a line,
// and returns the caret in text where possible.
const removeEmptyAt = (range, host) => {
  for (let container; (container = rangeState(range).start[0]) !== host && isEmptyInline(container);) {
    container.parentNode.removeChild(container);
  }
};
const settleCaret = (range, host) => {
  removeEmptyAt(range, host);
  const point = rangeState(range).start, block = blockAt(point, host);
  if (block !== host && !holdsLeaf(block, host)) createRange(point).insertNode(document.createElement('br'));
  const caret = canonicalPoint(rangeState(range).start, host);
  if (caret[0].nodeType === 3) rebalanceSpaces(caret[0], caret[1], caret[1], host);
  return caret;
};

// Deletes the content between two points of a host. When they are in
// different blocks the first line of the later block joins the earlier
// one where the deletion started, and the later block goes when nothing
// else is in it.
const deleteBetween = (from, to, host) => {
  const fromBlock = blockAt(from, host), toBlock = blockAt(to, host);
  let caret = createRange(from);
  createRange(from, to).deleteContents();
  if (fromBlock !== toBlock && toBlock.isConnected) {
    removeEmptyAt(caret, host);
    const next = leafAfter(rangeState(caret).start, host);
    if (next && !isBlock(next) && blockAround(next, host) === toBlock) {
      let child = next;
      while (child.parentNode !== toBlock) child = child.parentNode;
      const line = document.createDocumentFragment();
      while (child && !isBlock(child)) {
        const following = child.nextSibling;
        if (isBreak(child)) {
          child.parentNode.removeChild(child);
          break;
        }
        line.appendChild(child);
        child = following;
      }
      let [container, offset] = rangeState(caret).start;
      while (container !== fromBlock && container !== host && (offset === 0 || offset === boundaryLength(container))) {
        [container, offset] = offset === 0 ? beforeNode(container) : afterNode(container);
      }
      caret = createRange([container, offset]);
      caret.insertNode(line);
      caret.collapse(true);
    }
    let empty = toBlock;
    while (empty !== host && !empty.contains(fromBlock) && !holdsLeaf(empty, host)) {
      const parent = empty.parentNode;
      parent.removeChild(empty);
      if (parent.hasChildNodes()) break;
      empty = parent;
    }
  }
  return settleCaret(caret, host);
};

// Inserts text, a line break or a paragraph break at a caret point and
// returns the caret after it.
const insertTextAt = (point, text, host) => {
  let [container, offset] = point;
  if (container.nodeType !== 3) {
    const next = leafAfter(point, host), previous = leafBefore(point, host);
    const block = blockAt(point, host);
    const placeholder = next && isBreak(next) && blockAround(next, host) === block && endsBlock(next, host) &&
      (!previous || isBreak(previous) || isBlock(previous) || blockAround(previous, host) !== block);
    container = document.createTextNode('');
    offset = 0;
    createRange(point).insertNode(container);
    if (placeholder) next.parentNode.removeChild(next);
  }
  container.insertData(offset, text);
  rebalanceSpaces(container, offset, offset + text.length, host);
  return [container, offset + text.length];
};
const insertBreakAt = (point, host) => {
  const br = document.createElement('br');
  createRange(point).insertNode(br);
  for (const side of [br.previousSibling, br.nextSibling]) {
    if (side && side.nodeType === 3 && side.data === '') side.parentNode.removeChild(side);
  }
  if (endsBlock(br, host)) br.parentNode.insertBefore(document.createElement('br'), br.nextSibling);
  return canonicalPoint(afterNode(br), host);
};
const insertParagraphAt = (point, host) => {
  const block = blockAt(point, host);
  let fresh, child;
  if (block === host) {
    child = point[0] === host ? childAt(host, point[1]) : point[0];
    while (child && child.parentNode !== host) child = child.parentNode;
    let next = child;
    while (next && !isBlock(next)) next = next.nextSibling;
    const rest = createRange(point, next ? beforeNode(next) : [host, host.childNodes.length]).extractContents();
    fresh = document.createElement('div');
    fresh.appendChild(rest);
    host.insertBefore(fresh, next);
    while ((child = fresh.previousSibling) && pruneNode(child, true));
    const previous = leafBefore(beforeNode(fresh), host);
    if (!previous || isBlock(previous) || blockAround(previous, host) !== host) {
      host.insertBefore(document.createElement('br'), fresh);
    }
  } else {
    const rest = createRange(point, [block, block.childNodes.length]).extractContents();
    fresh = block.cloneNode(false);
    fresh.removeAttribute('id');
    fresh.appendChild(rest);
    block.parentNode.insertBefore(fresh, block.nextSibling);
    while ((child = block.lastChild) && pruneNode(child, true));
    if (/^h[1-6]$/.test(block.localName) && !holdsLeaf(fresh, host)) {
      const paragraph = document.createElement('div');
      while (fresh.firstChild) paragraph.appendChild(fresh.firstChild);
      fresh.parentNode.replaceChild(paragraph, fresh);
      fresh = paragraph;
    }
    if (!holdsLeaf(block, host)) block.appendChild(document.createElement('br'));
  }
  while ((child = fresh.firstChild) && pruneNode(child, false));
  if (!holdsLeaf(fresh, host)) fresh.appendChild(document.createElement('br'));
  return edgeCaret(fresh, host, false);
};
const insertEditingText = ({ host, start, end, collapsed }, text) => {
  let point = collapsed ? canonicalPoint(start, host) : deleteBetween(start, end, host);
  const lines = text.split(/\r\n|\r|\n/);
  lines.forEach((line, index) => {
    if (index) point = insertBreakAt(point, host);
    if (line) point = insertTextAt(point, line, host);
  });
  return point;
};
const breakEditing = ({ host, start, end, collapsed }, paragraph) => {
  const point = collapsed ? canonicalPoint(start, host) : deleteBetween(start, end, host);
  return paragraph ? insertParagraphAt(point, host) : insertBreakAt(point, host);
};

// Runs an edit of the focused editing host: a cancelable beforeinput for
// the user's input (not for commands scripts run), then the edit, the caret
// it leaves and input. `plan` returns the edit for the current selection,
// or null when there is nothing to do.
let userEditing = false;
const asUser = (action) => {
  const previous = userEditing;
  userEditing = true;
  try {
    return action();
  } finally {
    userEditing = previous;
  }
};
const editHost = (inputType, data, plan, dataTransfer = null) => {
  let editing = editingSelection();
  if (!editing || !plan(editing)) return false;
  const init = { bubbles: true, composed: true, inputType, data, dataTransfer };
  if (userEditing) {
    if (!fire(editing.host, InputEvent, 'beforeinput', { ...init, cancelable: true }).allowed) return true;
    editing = editingSelection();
  }
  const edit = editing && plan(editing);
  if (!edit) return true;
  const before = historyStart(editing.host);
  try {
    associateRange(createRange(edit()));
    fire(editing.host, InputEvent, 'input', init);
  } finally {
    rememberEdit(editing.host, before, inputType, data);
  }
  if (userEditing) revealCaret();
  return true;
};
const deletePlan = (forward, word) => (editing) => {
  const { host } = editing;
  if (!editing.collapsed) return () => deleteBetween(editing.start, editing.end, host);
  const point = canonicalPoint(editing.start, host);
  const target = word
    ? (forward ? wordForward : wordBackward)(point, host)
    : (forward ? stepForward : stepBackward)(point, host)?.point;
  if (!target) return null;
  return () => (forward ? deleteBetween(point, target, host) : deleteBetween(target, point, host));
};
const deleteType = (forward, word) => `delete${word ? 'Word' : 'Content'}${forward ? 'Forward' : 'Backward'}`;
const insertPlan = (text) => (editing) => (text || !editing.collapsed) && (() => insertEditingText(editing, text));
const breakPlan = (paragraph) => (editing) => () => breakEditing(editing, paragraph);
const clearPlan = (editing) => !editing.collapsed && (() => deleteBetween(editing.start, editing.end, editing.host));

// Words: runs of letters, digits and marks, of punctuation, and of spaces.
// Moving by word on Windows passes a word and the spaces after it.
const wordClass = (char) => {
  if (char === '\n') return 3;
  if (/^[\s\u00a0]$/.test(char)) return 0;
  return /^[\p{L}\p{N}\p{M}_]/u.test(char) ? 1 : 2;
};
const wordForward = (point, host) => {
  let step = stepForward(point, host);
  if (!step || step.char === '\n') return step && step.point;
  let current = step.point;
  const kind = wordClass(step.char);
  while (kind && (step = stepForward(current, host)) && step.char !== '\n' && wordClass(step.char) === kind) current = step.point;
  while ((step = stepForward(current, host)) && step.char !== '\n' && wordClass(step.char) === 0) current = step.point;
  return current;
};
const wordBackward = (point, host) => {
  let step = stepBackward(point, host);
  if (!step || step.char === '\n') return step && step.point;
  let current = step.point, kind = wordClass(step.char);
  while (!kind) {
    step = stepBackward(current, host);
    if (!step || step.char === '\n') return current;
    current = step.point;
    kind = wordClass(step.char);
  }
  while ((step = stepBackward(current, host)) && step.char !== '\n' && wordClass(step.char) === kind) current = step.point;
  return current;
};
// The word of a text node around a point, with the spaces after it.
// A double press past the end of a line's text selects the line break
// after it, up to the next line, as Chromium does; the last line has none,
// so its last word is selected.
const lineBreakAfter = (caret, host, x) => {
  const [node, offset] = caret;
  if (node.nodeType !== 3 || offset !== node.data.length) return null;
  const rect = caretRect(caret);
  if (!rect || x <= rect.x) return null;
  const step = stepForward(caret, host);
  return step && step.char === '\n' ? [caret, step.point] : null;
};
const wordAround = ([node, offset]) => {
  if (node.nodeType !== 3 || !node.data) return [[node, offset], [node, offset]];
  const data = node.data, at = Math.min(offset, data.length - 1), kind = wordClass(data[at]);
  let start = at, end = at + 1;
  while (start > 0 && wordClass(data[start - 1]) === kind) start--;
  while (end < data.length && wordClass(data[end]) === kind) end++;
  while (kind && kind !== 3 && end < data.length && wordClass(data[end]) === 0) end++;
  return [[node, start], [node, end]];
};

// The caret's rectangle at a point: its rectangle in text, or else the
// edge of the content beside it or of its container.
const edgeRect = (node, end) => {
  if (!node) return null;
  if (node.nodeType === 3) {
    const at = end ? node.data.length : 0, [rect] = page.textRects(node[ID], at, at);
    return rect ? new DOMRect(rect[0], rect[1], 0, rect[3]) : null;
  }
  if (node.nodeType !== 1 || !page.rect(node[ID])) return null;
  const rect = node.getBoundingClientRect();
  return new DOMRect(end ? rect.right : rect.left, rect.top, 0, rect.height);
};
const caretRect = (point) => {
  const [rect] = rangeRects({ start: point, end: point });
  if (rect) return rect;
  const [container, offset] = point;
  if (container.nodeType !== 1) return edgeRect(container.parentNode, false);
  return edgeRect(childAt(container, offset), false) ||
    (offset > 0 && edgeRect(childAt(container, offset - 1), true)) || edgeRect(container, false);
};

// Caret movement. Up and down keep the caret's horizontal position while
// it moves from line to line; the line found by probing points below or
// above it, or else by walking to the next line.
let caretGoal = null;
const walkLine = (point, host, forward) => {
  let current = point, step;
  while ((step = forward ? stepForward(current, host) : stepBackward(current, host)) && step.char !== '\n') {
    current = step.point;
  }
  return { current, step };
};
const verticalCaret = (point, host, down) => {
  const rect = caretRect(point);
  if (rect) {
    const x = caretGoal && samePoint(caretGoal.point, point) ? caretGoal.x : rect.left;
    const line = Math.max(rect.height, 1), height = viewport()[1];
    const first = down ? rect.bottom + 1 : rect.top - 1, last = first + (down ? 6 : -6) * line;
    for (let y = first; y >= 0 && y < height && (down ? y <= last : y >= last); y += (down ? line : -line) / 2) {
      const candidate = caretPoint(x, y);
      if (!candidate || editingHostOf(candidate[0]) !== host) continue;
      const found = caretRect(candidate), order = comparePoints(candidate, point);
      if (found && (down ? order > 0 && found.top > rect.top : order < 0 && found.top < rect.top)) {
        return { point: canonicalPoint(candidate, host), x };
      }
    }
  }
  const { step } = walkLine(point, host, down);
  const next = step ? canonicalPoint(step.point, host) : edgeCaret(host, host, down);
  return { point: next, x: rect ? rect.left : 0 };
};
// Control with up or down moves to the start or end of the paragraph, or
// of the next one when the caret is there already.
const paragraphCaret = (point, host, forward) => {
  const { current, step } = walkLine(point, host, forward);
  if (!step || !samePoint(current, point)) return current;
  return walkLine(canonicalPoint(step.point, host), host, forward).current;
};
const lineEdgeCaret = (point, host, end) => {
  const rect = caretRect(point), block = blockAt(point, host);
  if (rect && page.rect(block[ID])) {
    const box = block.getBoundingClientRect();
    const candidate = caretPoint(end ? box.right - 1 : box.left + 1, rect.top + rect.height / 2);
    const found = candidate && editingHostOf(candidate[0]) === host && caretRect(candidate);
    if (found && Math.abs(found.top - rect.top) < Math.max(rect.height / 2, 1)) {
      // Chromium's line start is the line box's first caret position and
      // its end the last, wherever they show: text without width at the
      // edge, as Muya's markers hidden at font-size 0, and text pulled past
      // it by a negative margin are on the line too.
      let edge = canonicalPoint(candidate, host);
      const onLine = (next) => {
        const middle = next.top + next.height / 2;
        return middle >= found.top - 0.5 && middle <= found.top + found.height + 0.5
          && (end ? next.left >= found.left - found.height : next.left <= found.left + found.height);
      };
      for (;;) {
        const step = end ? stepForward(edge, host) : stepBackward(edge, host);
        if (!step || step.char === '\n') break;
        const next = caretRect(step.point);
        if (!next || !onLine(next)) break;
        edge = step.point;
      }
      return edge;
    }
  }
  return walkLine(point, host, end).current;
};

// Keys that move the caret or extend the selection, and the keys that
// delete; returns whether the key was the editing host's.
const caretKeys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'];
const moveCaret = (editing, key, ctrl, shift) => {
  const { host } = editing;
  const focus = canonicalPoint(selectionBoundary(false), host);
  const forward = key === 'ArrowRight' || key === 'ArrowDown' || key === 'End';
  if (!shift && !editing.collapsed && (key === 'ArrowLeft' || key === 'ArrowRight')) {
    caretGoal = null;
    associateRange(createRange(forward ? editing.end : editing.start));
    return;
  }
  let target, goal = null;
  if (key === 'ArrowLeft' || key === 'ArrowRight') {
    target = ctrl
      ? (forward ? wordForward : wordBackward)(focus, host)
      : (forward ? stepForward : stepBackward)(focus, host)?.point;
    target = target && canonicalPoint(target, host);
  } else if (key === 'ArrowUp' || key === 'ArrowDown') {
    const from = shift || editing.collapsed ? focus : forward ? editing.end : editing.start;
    goal = ctrl ? null : verticalCaret(from, host, forward);
    target = ctrl ? paragraphCaret(from, host, forward) : goal.point;
  } else {
    target = ctrl ? edgeCaret(host, host, forward) : lineEdgeCaret(focus, host, forward);
  }
  target = target || focus;
  if (shift) selectEndpoints(selectionBoundary(true), target);
  else associateRange(createRange(target));
  caretGoal = goal && { point: selectionBoundary(false), x: goal.x };
};
const editingKey = (init) => {
  const editing = editingSelection();
  if (!editing || init.altKey || init.metaKey) return false;
  const { key } = init, ctrl = !!init.ctrlKey, shift = !!init.shiftKey;
  if (key === 'Backspace' || key === 'Delete') {
    asUser(() => editHost(deleteType(key === 'Delete', ctrl), null, deletePlan(key === 'Delete', ctrl)));
    return true;
  }
  if (caretKeys.includes(key)) {
    moveCaret(editing, key, ctrl, shift);
    revealCaret();
    return true;
  }
  if ((key === 'PageUp' || key === 'PageDown') && !ctrl) {
    pageCaret(editing, key === 'PageDown', shift);
    return true;
  }
  return false;
};
// After the user moves the caret or edits, Chromium reveals the caret: each
// scroll container around it, then the viewport, scrolls the least that
// shows it, and not at all when it shows already.
const revealCaret = () => {
  const point = selectionBoundary(false);
  if (!point[0]) return;
  const rect = caretRect(point);
  if (!rect) return;
  let { top, left } = rect;
  const height = rect.height, width = Math.max(rect.width, 1);
  const shift = (start, size, low, high) => start < low || size > high - low ? start - low
    : start + size > high ? start + size - high : 0;
  const root = document.documentElement;
  const scrolls = (element) => ['overflow-x', 'overflow-y'].some((name) =>
    ['auto', 'scroll', 'overlay', 'hidden'].includes(styleValue(element, name)));
  let element = point[0].nodeType === 1 ? point[0] : point[0].parentElement;
  for (; element; element = element.parentElement || (element.parentNode && element.parentNode.host)) {
    let scroller = element, port;
    if (element === root) {
      if (!userScrollable(root, 1) && !userScrollable(root, 0)) break;
      scroller = document.scrollingElement || root;
      port = { top: 0, left: 0, height: innerHeight, width: innerWidth };
    } else if (scrolls(element)) {
      const box = element.getBoundingClientRect();
      port = { top: box.top + element.clientTop, left: box.left + element.clientLeft, height: element.clientHeight, width: element.clientWidth };
    } else continue;
    const [x, y] = [scroller.scrollLeft, scroller.scrollTop];
    const down = shift(top, height, port.top, port.top + port.height);
    const across = shift(left, width, port.left, port.left + port.width);
    if (down) scroller.scrollTop = y + down;
    if (across) scroller.scrollLeft = x + across;
    top -= scroller.scrollTop - y;
    left -= scroller.scrollLeft - x;
  }
};
// Page Up and Page Down scroll the editing host's scroller a page and move
// the caret to where it showed before, as Chromium does.
const pageCaret = (editing, down, shift) => {
  const { host } = editing;
  const focus = canonicalPoint(selectionBoundary(false), host);
  const rect = caretRect(focus);
  if (!rect || !keyboardScroll(host, 1, down ? 1 : -1, 'page')) return;
  const point = caretPoint(rect.left, rect.top + rect.height / 2);
  if (!point || editingHostOf(point[0]) !== host) return;
  const target = canonicalPoint(point, host);
  if (shift) selectEndpoints(selectionBoundary(true), target);
  else associateRange(createRange(target));
  caretGoal = null;
};

// Mouse selection: a press puts the caret at the point, or selects the
// word or paragraph there on a double or triple press, and moving with the
// button held extends the selection; editable content keeps it inside its
// host.
let mouseSelection = null;
const formParts = ['input', 'textarea', 'select', 'button', 'option'];
const selectableTarget = (target) => {
  for (let element = target; element && element.nodeType === 1; element = element.parentElement) {
    if (formParts.includes(element.localName)) return false;
    const value = styleValue(element, 'user-select');
    if (value === 'none') return false;
    if ((value && value !== 'auto') || native.editingHost(element[ID]) === element[ID]) return true;
  }
  return true;
};
const describeNode = (node) => !node ? 'none' : node.nodeType === 3 ? `"${node.data.slice(0, 20)}"`
  : `<${node.localName}${node.id ? '#' + node.id : ''}${node.className ? '.' + String(node.className).split(' ')[0] : ''}>`;
const pointerSelect = (target, init) => {
  mouseSelection = null;
  const tracing = native.tracing();
  const skip = (why) => { if (tracing) native.trace(`pointer selection skipped: ${why}`); };
  if (!target || target.nodeType !== 1) return skip('no element');
  if (dragCandidate) return skip('drag candidate');
  if (!selectableTarget(target)) return skip('not selectable');
  const point = caretPoint(init.clientX, init.clientY);
  if (!point) return skip('no caret position');
  const host = editingHostOf(point[0]);
  if (editingHostOf(target) !== host) {
    return skip(`hosts differ: ${describeNode(editingHostOf(target))} ${describeNode(host)} at ${describeNode(point[0])}`);
  }
  if (!fire(target, Event, 'selectstart', { bubbles: true, cancelable: true }).allowed) return skip('selectstart cancelled');
  if (tracing) native.trace(`pointer caret ${describeNode(point[0])} ${point[1]}`);
  const caret = host ? canonicalPoint(point, host) : point;
  const detail = init.detail ?? 1;
  if (detail >= 3) {
    const block = host ? blockAt(caret, host) : blockAround(caret[0], null) || document.body;
    if (host) selectEndpoints(edgeCaret(block, host, false), edgeCaret(block, host, true));
    else selectEndpoints([block, 0], [block, block.childNodes.length]);
  } else if (detail === 2) {
    selectEndpoints(...(lineBreakAfter(caret, host, init.clientX) || wordAround(caret)));
  } else if (init.shiftKey && selectedRange) {
    selectEndpoints(selectionBoundary(true), caret);
  } else {
    associateRange(createRange(caret));
  }
  mouseSelection = { anchor: selectionBoundary(true), host };
};
// A context press in editable content outside the selection puts the
// caret at the point, as Chromium does on Windows; inside it the selection
// stays for the menu.
const contextSelect = (target, init) => {
  if (!target || target.nodeType !== 1 || !selectableTarget(target)) return;
  const point = caretPoint(init.clientX, init.clientY);
  if (!point) return;
  const host = editingHostOf(point[0]);
  if (!host || editingHostOf(target) !== host) return;
  if (selectedRange && !selectedRange.collapsed && selectedRange.isPointInRange(point[0], point[1])) return;
  associateRange(createRange(canonicalPoint(point, host)));
};
const pointerExtend = (init) => {
  if (!mouseSelection || !((init.buttons ?? 0) & 1)) return;
  const point = caretPoint(init.clientX, init.clientY);
  if (!point || editingHostOf(point[0]) !== mouseSelection.host) return;
  selectEndpoints(mouseSelection.anchor, mouseSelection.host ? canonicalPoint(point, mouseSelection.host) : point);
};

// Edit commands for editing hosts, which `document.execCommand`, the
// keyboard and the clipboard share with text controls.
const hostCommand = (inputType, data, plan, dataTransfer) => {
  if (!editingSelection()) return false;
  editHost(inputType, data, plan, dataTransfer);
  return true;
};
const selectHostContents = () => {
  const active = document.activeElement;
  if (!active || native.editingHost(active[ID]) !== active[ID]) return false;
  selectEndpoints(edgeCaret(active, active, false), edgeCaret(active, active, true));
  return true;
};
