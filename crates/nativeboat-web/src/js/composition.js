// An IME owns one live replacement range while the document's selection shows
// its caret. Updates replace that range rather than appending each preedit.
let compositionSession = null;

const interruptComposition = () => {
  if (compositionSession) finishComposition(page.completeComposition() ?? compositionSession.data);
};

const compositionEvent = (session, type, data, cancelable = false) =>
  fire(session.host, CompositionEvent, type, { view: global, bubbles: true, composed: true, cancelable, data });

const compositionCaret = (range, host, cursor) => {
  const state = rangeState(range);
  let point = canonicalPoint(state.start, host), remaining = cursor;
  while (remaining > 0) {
    const step = stepForward(point, host);
    if (!step || comparePoints(step.point, state.end) > 0) return state.end;
    if (remaining < step.char.length && step.point[0].nodeType === 3) {
      return [step.point[0], step.point[1] - step.char.length + remaining];
    }
    remaining -= step.char.length;
    point = step.point;
  }
  return point;
};

const updateComposition = (data, cursor = data.length) => {
  const session = compositionSession;
  if (!session?.range || !session.host.isConnected || native.focused() !== session.host[ID]) return false;
  data = String(data);
  if (session.updated && data === session.data) {
    const point = cursor >= data.length ? rangeState(session.range).end : compositionCaret(session.range, session.host, Math.max(0, cursor));
    associateRange(createRange(point));
    return false;
  }
  const init = { view: global, bubbles: true, composed: true, cancelable: false,
    inputType: 'insertCompositionText', data, isComposing: true };
  fire(session.host, InputEvent, 'beforeinput', init);
  if (compositionSession !== session || !session.host.isConnected) return false;
  compositionEvent(session, 'compositionupdate', data);
  if (compositionSession !== session || !session.host.isConnected) return false;
  const { start, end } = rangeState(session.range);
  const point = comparePoints(start, end) === 0 ? canonicalPoint(start, session.host) : deleteBetween(start, end, session.host);
  const anchor = createRange(point);
  const after = insertEditingText({ host: session.host, start: point, end: point, collapsed: true }, data);
  session.range = createRange(rangeState(anchor).start, after);
  session.data = data;
  session.updated = true;
  const caret = cursor >= data.length ? after : compositionCaret(session.range, session.host, Math.max(0, cursor));
  associateRange(createRange(caret));
  fire(session.host, InputEvent, 'input', init);
  return false;
};

const finishComposition = (data) => {
  const session = compositionSession;
  if (!session) return false;
  data = String(data);
  if (session.range && data !== session.data) updateComposition(data);
  if (compositionSession !== session) return false;
  if (session.range) associateRange(createRange(rangeState(session.range).end));
  compositionSession = null;
  compositionEvent(session, 'compositionend', data);
  if (data !== '') rememberEdit(session.host, session.before, 'insertCompositionText', data, false);
  else forgetEdit(session.host, session.before);
  return false;
};

const beginComposition = () => {
  if (compositionSession) finishComposition(compositionSession.data);
  const editing = editingSelection();
  if (!editing) return true;
  const session = { host: editing.host, range: null, data: '', updated: false, before: historyStart(editing.host) };
  compositionSession = session;
  const allowed = compositionEvent(session, 'compositionstart', String(selectedRange), true).allowed;
  const current = editingSelection();
  if (!allowed || compositionSession !== session || current?.host !== session.host) {
    if (compositionSession === session) finishComposition('');
    return true;
  }
  // compositionstart handlers can move the caret or seed an anchor node.
  session.range = createRange(current.start, current.end);
  return false;
};
