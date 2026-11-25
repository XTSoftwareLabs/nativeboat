// Edit history records the changes each edit makes to an editing host and
// undoes them on the original nodes, which keep their listeners; an edit
// costs what it changed, not the size of the document. Application changes
// outside an edit invalidate the affected history rather than being undone.
// Text controls keep their values.
const histories = new WeakMap(), nativeEditBefore = new WeakMap();
let applyingHistory = false;
const historySnapshot = (target) => {
  if (isTextControl(target)) return { value: target.value, start: target.selectionStart,
    end: target.selectionEnd, direction: target.selectionDirection };
  const anchor = selectionBoundary(true), focus = selectionBoundary(false);
  return { changes: native.historyChanges(target[ID]),
    anchor: target.contains(anchor[0]) ? anchor : [target, 0],
    focus: target.contains(focus[0]) ? focus : [target, 0] };
};
// The state an edit starts from; for an editing host it also starts
// recording the edit's changes, which rememberEdit ends.
const historyStart = (target) => {
  const before = historySnapshot(target);
  if (before.value === undefined) before.recording = native.historyBegin(target[ID]);
  return before;
};
// Ends an edit that left no change to remember, as a cancelled composition
// does. What it did and undid in between is not an application's change, so
// the history around it stays valid.
const forgetEdit = (target, before) => {
  if (before.recording === undefined) return;
  native.historyEnd(before.recording);
  const history = histories.get(target);
  if (!history) return;
  const { changes } = historySnapshot(target);
  const last = history.undo.at(-1), next = history.redo.at(-1);
  if (last && last.after.changes === before.changes) last.after = { ...last.after, changes };
  if (next && next.before.changes === before.changes) next.before = { ...next.before, changes };
};
const sameHistoryContent = (left, right) => left.value !== undefined
  ? left.value === right.value : left.changes === right.changes;
const sameHistoryCaret = (left, right) => left.value !== undefined
  ? left.start === right.start && left.end === right.end && left.start === left.end
  : samePoint(left.anchor, left.focus) && samePoint(left.anchor, right.anchor) && samePoint(left.focus, right.focus);
const rememberEdit = (target, before, inputType, data, group = userEditing) => {
  const journal = before.recording === undefined ? undefined : native.historyEnd(before.recording);
  if (applyingHistory) return;
  const after = historySnapshot(target);
  if (journal === null || (journal === undefined && sameHistoryContent(before, after))) return;
  let history = histories.get(target);
  if (!history) histories.set(target, history = { undo: [], redo: [] });
  const previous = history.undo.at(-1);
  if (previous && !sameHistoryContent(previous.after, before)) history.undo.length = 0;
  const time = performance.now();
  const grouped = group && ['insertText', 'deleteContentBackward', 'deleteContentForward'].includes(inputType) &&
    !(data && /\s/.test(data));
  if (grouped && history.undo.length && previous.grouped && previous.inputType === inputType &&
      time - previous.time < 1000 && sameHistoryCaret(previous.after, before)) {
    previous.after = after;
    previous.time = time;
    if (journal) previous.journal = native.historyJoin(previous.journal, journal);
  } else {
    history.undo.push({ before, after, journal, inputType, grouped, time });
    if (history.undo.length > 128) history.undo.shift();
  }
  history.redo.length = 0;
};
const restoreValue = (target, snapshot) => {
  native.setValue(target[ID], snapshot.value);
  target.setSelectionRange(snapshot.start, snapshot.end, snapshot.direction);
  edited.add(target);
};
const historyCommand = (redo) => {
  const target = isEditable(document.activeElement) ? document.activeElement : editingSelection()?.host;
  if (!target) return false;
  const inputType = redo ? 'historyRedo' : 'historyUndo';
  const init = { view: global, bubbles: true, composed: true, cancelable: true, inputType, data: null };
  if (!fire(target, InputEvent, 'beforeinput', init).allowed) return true;
  const history = histories.get(target), stack = history?.[redo ? 'redo' : 'undo'];
  const entry = stack?.at(-1);
  if (!entry) return false;
  const from = redo ? entry.before : entry.after, to = redo ? entry.after : entry.before;
  if (!sameHistoryContent(from, historySnapshot(target))) {
    history.undo.length = history.redo.length = 0;
    return false;
  }
  applyingHistory = true;
  try {
    if (entry.journal) {
      // Undoing records the changes that redo, and the other way round.
      const recording = native.historyBegin(target[ID]);
      native.historyRevert(entry.journal);
      const clamp = ([node, offset]) => [node, Math.min(offset, boundaryLength(node))];
      selectEndpoints(clamp(to.anchor), clamp(to.focus));
      fire(target, InputEvent, 'input', { ...init, cancelable: false });
      entry.journal = native.historyEnd(recording) || entry.journal;
    } else {
      restoreValue(target, to);
      fire(target, InputEvent, 'input', { ...init, cancelable: false });
    }
    if (redo) entry.after = historySnapshot(target);
    else entry.before = historySnapshot(target);
  } finally { applyingHistory = false; }
  stack.pop();
  history[redo ? 'undo' : 'redo'].push(entry);
  entry.grouped = false;
  return true;
};
