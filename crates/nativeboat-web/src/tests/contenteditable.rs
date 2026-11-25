use super::*;

/// Helpers for editing: `where()` describes the selection, `press` sends a
/// key down and up and `type` types text the way the platform does, with
/// the text of a key only when the key was not cancelled.
const EDITING: &str = r#"
const d = (node) => node.nodeType === 3 ? JSON.stringify(node.data) : node.localName + (node.id ? '#' + node.id : '');
const where = () => {
  const s = getSelection();
  if (!s.rangeCount) return 'none';
  return `${d(s.anchorNode)}:${s.anchorOffset}` + (s.isCollapsed ? '' : `-${d(s.focusNode)}:${s.focusOffset}`);
};
const press = (key, modifiers = {}) => {
  const cancelled = web.key(true, { key, ...modifiers });
  web.key(false, { key, ...modifiers });
  return cancelled;
};
const type = (text) => { for (const char of text) if (!press(char)) web.text(char); };
const caret = (node, offset) => getSelection().collapse(node, offset);
const html = (element) => element.innerHTML;
"#;

fn editing(page: &mut Fixture, script: &str) {
    page.check(&format!("{EDITING}\n{script}"));
}

/// The node id of the node a script expression evaluates to.
fn node_id(page: &mut Fixture, expression: &str) -> u32 {
    let value = page.eval(&format!(
        "(() => {{ const node = {expression}; return node[Object.getOwnPropertySymbols(node).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')]; }})()"
    ));
    value.parse().unwrap()
}

#[test]
fn editable_content_reports_its_state_and_takes_focus() {
    let mut page = fixture(
        "<div id=e contenteditable><span id=s>ab</span><span id=f contenteditable=false>x<b id=n contenteditable>y</b></span></div>\
         <div id=o>z</div><p id=p contenteditable=PlainText-Only>q</p><p id=w contenteditable=bogus>w</p>",
    );
    editing(
        &mut page,
        r#"
        const [e, s, f, n, o, p, w] = ['#e', '#s', '#f', '#n', '#o', '#p', '#w'].map(ids);
        eq([e, s, f, n, o, p, w].map((element) => element.contentEditable),
          ['true', 'inherit', 'false', 'true', 'inherit', 'plaintext-only', 'inherit'], 'states');
        eq([e, s, f, n, o, p, w].map((element) => element.isContentEditable),
          [true, true, false, true, false, true, false], 'editable');
        eq([e.tabIndex, s.tabIndex, n.tabIndex, o.tabIndex], [0, -1, 0, -1], 'hosts are focusable');
        o.contentEditable = 'TRUE'; eq([o.getAttribute('contenteditable'), o.isContentEditable], ['true', true], 'made editable');
        o.contentEditable = 'Inherit'; eq([o.hasAttribute('contenteditable'), o.isContentEditable], [false, false], 'inherits');
        o.contentEditable = 'false'; eq(o.contentEditable, 'false', 'not editable');
        throws(() => { o.contentEditable = 'yes'; }, 'SyntaxError', 'bad state');
        e.focus();
        eq([document.activeElement === e, where()], [true, '"ab":0'], 'focus puts the caret at the start');
        caret(s.firstChild, 1); e.focus(); eq(where(), '"ab":1', 'focus keeps a caret inside');
        o.focus(); eq(document.activeElement === e, true, 'not focusable');
        s.focus(); eq(document.activeElement === e, true, 'inside is not focusable');
        caret(n.firstChild, 1); eq(document.activeElement === n, true, 'selecting focuses the host');
        caret(ids('#o').firstChild, 0); eq(document.activeElement === n, true, 'selection elsewhere keeps focus');
        "#,
    );
}

#[test]
fn ranges_move_copy_and_compare_their_contents() {
    let mut page = fixture("<p id=p>abc<b id=b>def</b>ghi</p><p id=q>jkl</p><!--c-->");
    editing(
        &mut page,
        r#"
        const p = ids('#p'), b = ids('#b'), q = ids('#q'), [abc, def] = [p.firstChild, b.firstChild];
        eq([Range.START_TO_START, Range.START_TO_END, Range.END_TO_END, Range.END_TO_START, Range.prototype.END_TO_START], [0, 1, 2, 3, 3], 'constants');
        const r = document.createRange(); r.setStart(abc, 1); r.setEnd(def, 2);
        const clone = r.cloneContents();
        eq([[...clone.childNodes].map(d), clone.lastChild.textContent, html(p)], [['"bc"', 'b#b'], 'de', 'abc<b id="b">def</b>ghi'], 'cloned');
        const moved = r.extractContents();
        eq([[...moved.childNodes].map(d), html(p)], [['"bc"', 'b#b'], 'a<b id="b">f</b>ghi'], 'extracted');
        eq([r.collapsed, r.startContainer === p, r.startOffset], [true, true, 1], 'collapsed where the contents were');
        eq(r.extractContents().childNodes.length, 0, 'nothing to extract');
        r.insertNode(document.createTextNode('X'));
        eq([html(p), r.startContainer === p, r.startOffset, r.endOffset], ['aX<b id="b">f</b>ghi', true, 1, 2], 'inserted');
        r.setStart(p.lastChild, 1); r.collapse(true); r.insertNode(document.createElement('i'));
        eq(html(p), 'aX<b id="b">f</b>g<i></i>hi', 'inserted into text');
        const fragment = document.createDocumentFragment(); fragment.append('1', '2');
        r.setStart(p, 0); r.collapse(true); r.insertNode(fragment);
        eq([html(p).slice(0, 3), r.endOffset], ['12a', 2], 'inserted a fragment');
        const i = p.querySelector('i'); r.setStart(p, 0); r.collapse(true); r.insertNode(i);
        eq(html(p).slice(0, 8), '<i></i>1', 'moved a node');
        r.selectNode(i); r.insertNode(i); eq(html(p).slice(0, 8), '<i></i>1', 'inserted itself');
        r.setStart(i, 0); throws(() => r.insertNode(p), 'HierarchyRequestError', 'ancestor');
        throws(() => r.insertNode(document), 'HierarchyRequestError', 'document');
        const comment = [...document.body.childNodes].find((node) => node.nodeType === 8);
        r.setStart(comment, 0); throws(() => r.insertNode(document.createElement('a')), 'HierarchyRequestError', 'comment');
        r.selectNode(i); throws(() => r.insertNode(i.parentNode), 'HierarchyRequestError', 'into itself');
        r.setStart(document, 0); r.collapse(true); throws(() => r.insertNode(document.createTextNode('t')), 'HierarchyRequestError', 'text into the document');
        const orphan = document.createTextNode('o'); r.setStart(orphan, 0);
        throws(() => r.insertNode(document.createElement('a')), 'HierarchyRequestError', 'orphan text');
        throws(() => r.insertNode({}), 'TypeError', 'not a node');

        r.setStart(q.firstChild, 1); r.setEnd(q.firstChild, 2); r.surroundContents(document.createElement('u'));
        eq([html(q), r.startContainer === q, r.startOffset, r.endOffset], ['j<u>k</u>l', true, 1, 2], 'surrounded');
        const old = document.createElement('s'); old.textContent = 'gone';
        r.setStart(q.firstChild, 0); r.setEnd(q.lastChild, 1); r.surroundContents(old);
        eq(html(q), '<s>j<u>k</u>l</s>', 'surrounded with an emptied element');
        r.setStart(old.firstChild, 0); r.setEnd(old.lastChild, 1);
        r.setStart(old.childNodes[1].firstChild, 0);
        throws(() => r.surroundContents(document.createElement('a')), 'InvalidStateError', 'partial element');
        r.selectNodeContents(q); throws(() => r.surroundContents(document.createDocumentFragment()), 'InvalidNodeTypeError', 'fragment');

        const one = document.createRange(), two = document.createRange();
        one.setStart(p, 0); one.setEnd(p, 1); two.setStart(q, 0); two.setEnd(q, 1);
        eq([0, 1, 2, 3].map((how) => one.compareBoundaryPoints(how, two)), [-1, -1, -1, -1], 'before');
        eq([0, 1, 2, 3].map((how) => two.compareBoundaryPoints(how, one)), [1, 1, 1, 1], 'after');
        eq([one.compareBoundaryPoints(0, one), one.compareBoundaryPoints(65536, one), one.compareBoundaryPoints(NaN, one)], [0, 0, 0], 'same');
        throws(() => one.compareBoundaryPoints(4, two), 'NotSupportedError', 'bad comparison');
        throws(() => one.compareBoundaryPoints(-1, two), 'NotSupportedError', 'negative comparison');
        throws(() => one.compareBoundaryPoints(0, {}), 'TypeError', 'not a range');
        const away = document.createRange(), loose = document.createElement('div'); loose.textContent = 'loose';
        away.selectNodeContents(loose);
        throws(() => one.compareBoundaryPoints(0, away), 'WrongDocumentError', 'other root');

        eq([one.comparePoint(document.body, 0), one.comparePoint(p, 0), one.comparePoint(p, 1), one.comparePoint(q, 0)], [-1, 0, 0, 1], 'compare points');
        throws(() => one.comparePoint(loose, 0), 'WrongDocumentError', 'compare elsewhere');
        throws(() => one.comparePoint(p, 99), 'IndexSizeError', 'compare beyond');
        const doctype = document.createComment(''); Object.defineProperty(doctype, 'nodeType', { value: 10 }); document.body.append(doctype);
        throws(() => one.comparePoint(doctype, 0), 'InvalidNodeTypeError', 'doctype');
        eq([one.isPointInRange(p, 1), one.isPointInRange(q, 0), one.isPointInRange(document.body, 0), one.isPointInRange(loose, 0)], [true, false, false, false], 'points in range');
        throws(() => one.isPointInRange(p, 99), 'IndexSizeError', 'point beyond');
        throws(() => one.isPointInRange({}, 0), 'TypeError', 'point not a node');
        eq([one.intersectsNode(p.firstChild), one.intersectsNode(p), one.intersectsNode(q), one.intersectsNode(document), one.intersectsNode(loose)], [true, true, false, true, false], 'intersects');
        throws(() => one.intersectsNode(null), 'TypeError', 'intersects nothing');

        const all = document.createRange(); all.setStart(p, 1); all.setEnd(old.firstChild, 1);
        all.deleteContents();
        eq([html(p), html(q), all.collapsed, all.startContainer === document.body], ['<i></i>', '<s><u>k</u>l</s>', true, true], 'deleted across elements');
        all.deleteContents(); eq(html(p), '<i></i>', 'nothing to delete');
        const text = q.querySelector('u').firstChild; all.setStart(text, 0); all.setEnd(text, 1); all.deleteContents();
        eq(html(q), '<s><u></u>l</s>', 'deleted text');
        all.setStart(old, 0); all.setEnd(old.lastChild, 1);
        eq(all.cloneContents().textContent, 'l', 'clone into text');
        eq(all.extractContents().childNodes.length, 3, 'extract into text');
        const parts = document.createElement('div'); parts.innerHTML = '<a>1<b>2</b></a><c>3</c>'; document.body.append(parts);
        all.setStart(parts.querySelector('b').firstChild, 0); all.setEnd(parts.querySelector('c').firstChild, 1);
        eq([...all.cloneContents().childNodes].map(d), ['a', 'c'], 'clone partial elements');
        all.setStart(parts, 0); all.setEnd(parts, 2);
        eq(all.extractContents().textContent, '123', 'extract children');
        all.selectNode(doctype);
        throws(() => all.extractContents(), 'HierarchyRequestError', 'doctype');
        "#,
    );
}

#[test]
fn the_selection_deletes_from_the_document() {
    let mut page = fixture("<p id=p>abc</p>");
    editing(
        &mut page,
        r#"
        getSelection().deleteFromDocument();
        const p = ids('#p'); getSelection().setBaseAndExtent(p.firstChild, 1, p.firstChild, 2);
        getSelection().deleteFromDocument(); eq([html(p), where()], ['ac', '"ac":1'], 'deleted');
        "#,
    );
}

#[test]
fn typing_inserts_text_and_keeps_spaces_visible() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>one</p><p id=b>three</p><p id=c><br></p></div><div id=o>out</div>",
    );
    editing(
        &mut page,
        r#"
        const e = ids('#e'), a = ids('#a'), b = ids('#b'), c = ids('#c'), log = [];
        for (const kind of ['beforeinput', 'input']) {
          document.addEventListener(kind, (event) => log.push(`${kind}:${event.inputType}:${event.data}:${event.cancelable}`));
        }
        eq(web.text('x'), false, 'nothing focused');
        e.focus(); type('X');
        eq([html(a), where()], ['Xone', '"Xone":1'], 'typed');
        eq(log, ['beforeinput:insertText:X:true', 'input:insertText:X:false'], 'input events');
        caret(b.firstChild, 5); type(' x  ');
        eq(html(b), 'three x &nbsp;', 'spaces stay visible');
        caret(a.firstChild, 0); type(' ');
        eq(html(a), '&nbsp;Xone', 'a space starting a line');
        caret(c, 0); type('z');
        eq(html(c), 'z', 'typing replaces the placeholder');
        const cancel = (event) => event.preventDefault();
        e.addEventListener('beforeinput', cancel); log.length = 0;
        eq(web.text('Q'), true, 'cancelled typing is taken');
        e.removeEventListener('beforeinput', cancel);
        eq([html(a), log], ['&nbsp;Xone', ['beforeinput:insertText:Q:true']], 'cancelled');
        const move = () => caret(c.firstChild, 0);
        e.addEventListener('beforeinput', move, { once: true }); caret(a.firstChild, 1); type('M');
        eq([html(a), html(c)], ['&nbsp;Xone', 'Mz'], 'typed where the selection moved');
        e.addEventListener('beforeinput', () => caret(ids('#o').firstChild, 0), { once: true }); type('N');
        eq(html(c), 'Mz', 'nothing typed when the selection left');
        e.focus(); getSelection().setBaseAndExtent(a.firstChild, 1, b.firstChild, 2); type('-');
        eq([html(e), where()], ['<p id="a">&nbsp;-ree x &nbsp;</p><p id="c">Mz</p>', '"\u00a0-":2'], 'typing replaces the selection');
        getSelection().setBaseAndExtent(c.firstChild, 0, c.firstChild, 2); web.text('');
        eq(html(c), '<br>', 'empty text deletes the selection');
        caret(c, 0); eq(document.execCommand('insertText', false, 'x\ny\r\nz'), true, 'insert lines');
        eq([html(c), where()], ['x<br>y<br>z', '"z":1'], 'lines');
        caret(c, 0); web.text('');
        eq(html(c), 'x<br>y<br>z', 'nothing typed');
        "#,
    );
}

#[test]
fn deleting_removes_characters_words_and_joins_blocks() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>ab</p><p id=b>cd<b id=k>ef</b></p>\
         <p id=c>g&#x301;h&#x1F1EF;&#x1F1F5;i&#x1F44D;&#x1F3FD;</p><p id=w>one two, three</p></div>",
    );
    editing(
        &mut page,
        r#"
        const e = ids('#e'), a = ids('#a'), b = ids('#b'), c = ids('#c'), w = ids('#w'), log = [];
        e.addEventListener('beforeinput', (event) => log.push(event.inputType));
        e.focus(); caret(b.firstChild, 0); eq(press('Backspace'), true, 'backspace taken');
        eq([html(a), b.isConnected, where()], ['abcd<b id="k">ef</b>', false, '"ab":2'], 'joined');
        press('Delete'); eq([html(a), where()], ['abd<b id="k">ef</b>', '"ab":2'], 'deleted forward');
        press('Backspace'); press('Backspace');
        eq([html(a), where()], ['d<b id="k">ef</b>', '"d":0'], 'deleted the text');
        press('Backspace'); eq(html(a), 'd<b id="k">ef</b>', 'nothing before');
        eq(log, ['deleteContentBackward', 'deleteContentForward', 'deleteContentBackward', 'deleteContentBackward'], 'input types');
        caret(c.firstChild, 12);
        const sizes = [];
        for (let i = 0; i < 4; i++) { press('Backspace'); sizes.push(c.firstChild.length); }
        eq(sizes, [8, 7, 3, 2], 'whole characters');
        press('Backspace'); eq([html(c), where()], ['<br>', 'p#c:0'], 'a placeholder keeps the line');
        press('Backspace'); eq([html(a), c.isConnected, where()], ['d<b id="k">ef</b>', false, '"ef":2'], 'an empty block goes');
        const t = w.firstChild; caret(t, 0); press('Backspace');
        eq([html(a), w.isConnected, where()], ['d<b id="k">ef</b>one two, three', false, '"ef":2'], 'joined after the bold text');
        caret(t, 14); press('Backspace', { ctrlKey: true });
        eq(t.data, 'one two,\u00a0', 'deleted a word');
        press('Backspace', { ctrlKey: true }); eq(t.data, 'one two', 'deleted punctuation');
        caret(t, 0); press('Delete', { ctrlKey: true }); eq(t.data, 'two', 'deleted the next word');
        caret(t, 3); press('Delete'); eq(t.data, 'two', 'nothing after');
        const cancel = (event) => event.preventDefault();
        e.addEventListener('beforeinput', cancel); press('Backspace'); e.removeEventListener('beforeinput', cancel);
        eq(t.data, 'two', 'cancelled');
        getSelection().setBaseAndExtent(t, 1, t, 2); press('Delete');
        eq(t.data, 'to', 'deleted the selection');
        eq([press('Backspace', { altKey: true }), t.data], [false, 'to'], 'alt is not an edit');
        "#,
    );
}

#[test]
fn enter_breaks_lines_and_paragraphs() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>abcd</p><h1 id=h>head</h1></div><div id=f contenteditable>ab</div>\
         <div id=g contenteditable><b>x</b></div>",
    );
    editing(
        &mut page,
        r#"
        const e = ids('#e'), a = ids('#a'), h = ids('#h'), f = ids('#f'), g = ids('#g'), log = [];
        document.addEventListener('beforeinput', (event) => log.push(event.inputType));
        e.focus(); caret(a.firstChild, 2); eq(press('Enter'), true, 'enter taken');
        eq([html(e), where()], ['<p id="a">ab</p><p>cd</p><h1 id="h">head</h1>', '"cd":0'], 'split the paragraph');
        caret(h.firstChild, 4); press('Enter');
        eq([html(e), where()], ['<p id="a">ab</p><p>cd</p><h1 id="h">head</h1><div><br></div>', 'div:0'], 'a heading ends');
        type('x'); press('Enter', { shiftKey: true });
        eq([e.lastChild.innerHTML, where()], ['x<br><br>', 'div:2'], 'a line break at the end');
        type('y'); eq(e.lastChild.innerHTML, 'x<br>y', 'typed on the new line');
        caret(h.firstChild, 2); press('Enter');
        eq(html(e), '<p id="a">ab</p><p>cd</p><h1 id="h">he</h1><h1>ad</h1><div>x<br>y</div>', 'split the heading');
        caret(a.firstChild, 0); press('Enter');
        eq(html(e).slice(0, 36), '<p id="a"><br></p><p>ab</p><p>cd</p>', 'a paragraph before');
        eq(log, ['insertParagraph', 'insertParagraph', 'insertText', 'insertLineBreak', 'insertText', 'insertParagraph', 'insertParagraph'], 'input types');
        f.focus(); caret(f.firstChild, 1); press('Enter');
        eq([html(f), where()], ['a<div>b</div>', '"b":0'], 'a paragraph in the host');
        caret(f.firstChild, 0); press('Enter');
        eq([html(f), where()], ['<br><div>a</div><div>b</div>', '"a":0'], 'a paragraph at the start of the host');
        caret(f, 3); press('Enter');
        eq(html(f), '<br><div>a</div><div>b</div><div><br></div>', 'a paragraph at the end of the host');
        g.focus(); caret(g.firstChild.firstChild, 1); press('Enter');
        eq(html(g), '<b>x</b><div><br></div>', 'no empty inline element');
        getSelection().setBaseAndExtent(f.childNodes[1].firstChild, 0, f.childNodes[2].firstChild, 1);
        press('Enter', { shiftKey: true });
        eq(html(f), '<br><div><br><br></div><div><br></div>', 'replaced the selection with a line break');
        eq([press('Enter', { ctrlKey: true }), html(f)], [false, '<br><div><br><br></div><div><br></div>'], 'ctrl enter');
        "#,
    );
}
#[test]
fn keys_move_the_caret_and_extend_the_selection() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>one two</p><p id=b>x<br>yz</p></div><p id=o>out</p>",
    );
    editing(
        &mut page,
        r#"
        const e = ids('#e'), keys = [];
        // Without a caret the key scrolls the (overflowing) viewport.
        eq(press('ArrowRight'), true, 'nothing to move');
        eq(getSelection().rangeCount, 0, 'no caret');
        e.focus();
        const moves = [
          ['ArrowRight'], ['ArrowRight', { ctrlKey: true }], ['ArrowRight', { ctrlKey: true }], ['ArrowRight'],
          ['End'], ['ArrowRight'], ['Home'], ['End'], ['Home', { shiftKey: true }], ['ArrowLeft'],
          ['ArrowLeft', { shiftKey: true }], ['ArrowRight'], ['Home', { ctrlKey: true }], ['ArrowLeft'],
          ['End', { ctrlKey: true }], ['ArrowRight'], ['ArrowUp'], ['ArrowUp'], ['ArrowUp'], ['ArrowUp'],
          ['ArrowDown'], ['ArrowDown', { ctrlKey: true }], ['ArrowDown', { shiftKey: true }],
          ['ArrowLeft', { ctrlKey: true, shiftKey: true }], ['ArrowDown'], ['ArrowUp', { ctrlKey: true }],
          ['ArrowDown'], ['ArrowDown'], ['ArrowDown'],
        ];
        for (const [key, modifiers] of moves) {
          eq(press(key, modifiers), true, key);
          keys.push(where());
        }
        eq(keys, [
          '"one two":1', '"one two":4', '"one two":7', '"x":0',
          '"x":1', '"yz":0', '"yz":0', '"yz":2', '"yz":2-"yz":0', '"yz":0',
          '"yz":0-"x":1', '"yz":0', '"one two":0', '"one two":0',
          '"yz":2', '"yz":2', '"x":1', '"one two":7', '"one two":0', '"one two":0',
          '"x":0', '"x":1', '"x":1-"yz":0',
          '"x":1', '"yz":0', '"x":0',
          '"yz":0', '"yz":2', '"yz":2',
        ], 'moves');
        getSelection().setBaseAndExtent(ids('#a').firstChild, 1, ids('#a').firstChild, 3);
        press('ArrowDown'); eq(where(), '"x":0', 'down from the end of a selection');
        getSelection().setBaseAndExtent(ids('#b').lastChild, 1, ids('#b').lastChild, 2);
        press('ArrowUp'); eq(where(), '"x":1', 'up from the start of a selection');
        caret(ids('#o').firstChild, 1);
        // Outside editable content the key scrolls the viewport back.
        eq([press('ArrowLeft'), where()], [true, '"out":1'], 'not editing');
        // Alt+arrows are not the host's; Page Down with the caret outside
        // it pages the viewport.
        e.focus(); eq([press('ArrowLeft', { altKey: true }), press('PageDown')], [false, true], 'other keys');
        "#,
    );
}

#[test]
fn the_mouse_puts_the_caret_and_selects() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>one two</p><p id=b>three</p></div><p id=t>plain words</p>\
         <input id=i><div id=g draggable=true>drag</div>",
    );
    let a = node_id(&mut page, "document.querySelector('#a').firstChild");
    let b = node_id(&mut page, "document.querySelector('#b').firstChild");
    let t = node_id(&mut page, "document.querySelector('#t').firstChild");
    let element = |page: &mut Fixture, selector: &str| {
        node_id(page, &format!("document.querySelector('{selector}')"))
    };
    let (pa, pt, input, drag) = (
        element(&mut page, "#a"),
        element(&mut page, "#t"),
        element(&mut page, "#i"),
        element(&mut page, "#g"),
    );
    let pointer = format!(
        "const at = (kind, id, init = {{}}) => web.pointer(kind, id, {{ clientX: 1, clientY: 1, button: 0, buttons: 1, ...init }});\
         const [pa, pt, input, drag] = [{pa}, {pt}, {input}, {drag}];"
    );
    let step = |page: &mut Fixture, caret: Option<(u32, u32)>, script: &str| {
        page.page.caret.set(caret);
        editing(page, &format!("{pointer}\n{script}"));
    };
    step(
        &mut page,
        Some((a, 4)),
        "const log = globalThis.log = []; document.addEventListener('selectstart', (event) => log.push(event.target.id));\
         at('down', pa); eq([where(), document.activeElement.id, log], ['\"one two\":4', 'e', ['a']], 'caret');",
    );
    step(
        &mut page,
        Some((b, 2)),
        "at('move', pa); eq(where(), '\"one two\":4-\"three\":2', 'dragged');\
         at('move', pa, { buttons: 0 }); at('up', pa);",
    );
    step(
        &mut page,
        Some((a, 1)),
        "at('move', pa); eq(where(), '\"one two\":4-\"three\":2', 'released');\
         at('down', pa, { shiftKey: true }); eq(where(), '\"one two\":4-\"one two\":1', 'extended'); at('up', pa);\
         at('down', pa, { detail: 2 }); eq(where(), '\"one two\":0-\"one two\":4', 'word'); at('up', pa);\
         at('down', pa, { detail: 3 }); eq(where(), '\"one two\":0-\"one two\":7', 'paragraph'); at('up', pa);\
         at('down', pt); eq(where(), '\"one two\":0-\"one two\":7', 'outside the host');",
    );
    // A double press past the end of a line selects the line break to the
    // next one; on the last letters, or past the host's last line, a word.
    step(
        &mut page,
        Some((a, 7)),
        "at('down', pa, { detail: 2, clientX: 50 }); eq(where(), '\"one two\":7-\"three\":0', 'line break'); at('up', pa);\
         at('down', pa, { detail: 2, clientX: 30 }); eq(where(), '\"one two\":4-\"one two\":7', 'last word'); at('up', pa);",
    );
    step(
        &mut page,
        Some((b, 5)),
        "at('down', pa, { detail: 2, clientX: 50 }); eq(where(), '\"three\":0-\"three\":5', 'last line'); at('up', pa);",
    );
    step(
        &mut page,
        Some((t, 2)),
        "at('down', pt); eq([where(), document.activeElement === document.body], ['\"plain words\":2', true], 'text');\
         at('move', pt, { clientX: 2 }); at('up', pt);\
         at('down', pt, { detail: 2 }); eq(where(), '\"plain words\":0-\"plain words\":6', 'a word of text');\
         at('down', pt, { detail: 3 }); eq(where(), 'p#t:0-p#t:1', 'a paragraph of text');\
         at('down', pa); eq(where(), 'p#t:0-p#t:1', 'not in the host');\
         caret(ids('#t').firstChild, 1); at('down', input); eq(where(), 'body:2', 'form controls');         caret(ids('#t').firstChild, 1);\
         at('down', drag); eq(where(), '\"plain words\":1', 'draggable'); at('up', drag);\
         at('down', pt, { button: 2 }); eq(where(), '\"plain words\":1', 'right button');\
         const cancel = (event) => event.preventDefault();\
         document.addEventListener('selectstart', cancel); at('down', pt); document.removeEventListener('selectstart', cancel);\
         eq(where(), '\"plain words\":1', 'cancelled'); at('move', pt); eq(where(), '\"plain words\":1', 'nothing to extend');\
         document.addEventListener('mousedown', cancel); at('down', pt); document.removeEventListener('mousedown', cancel);\
         eq(where(), '\"plain words\":1', 'cancelled press');",
    );
    step(
        &mut page,
        None,
        "at('down', pt); eq(where(), '\"plain words\":1', 'no caret'); at('move', pt); at('up', pt);",
    );
    step(&mut page, Some((t, 3)), "at('down', pt); at('move', pt);");
    step(
        &mut page,
        Some((a, 3)),
        "at('move', pt); eq(where(), '\"plain words\":3', 'stays out of the host'); at('up', pt);",
    );
    step(&mut page, Some((a, 3)), "at('down', pa); at('move', pa);");
    step(
        &mut page,
        None,
        "at('move', pa); eq(where(), '\"one two\":3', 'nothing under the pointer'); at('up', pa);",
    );
}

#[test]
fn tab_and_shortcuts_edit_hosts() {
    let mut page = fixture_with(
        "<input id=i><div id=e contenteditable><p id=a>one</p><p id=b>two</p></div>",
        FakePage {
            clipboard: RefCell::new(Some("pasted".to_owned())),
            ..FakePage::default()
        },
    );
    editing(
        &mut page,
        r#"
        const e = ids('#e'), a = ids('#a'), b = ids('#b'), log = [];
        for (const kind of ['copy', 'cut', 'paste', 'beforeinput']) {
          document.addEventListener(kind, (event) => log.push(`${kind}:${event.inputType ?? ''}`));
        }
        ids('#i').focus(); press('Tab');
        eq([document.activeElement === e, where()], [true, '"one":0'], 'tab places the caret');
        caret(b.firstChild, 3); eq(press('v', { ctrlKey: true }), true, 'paste');
        eq([html(b), where()], ['twopasted', '"twopasted":9'], 'pasted');
        press('Insert', { shiftKey: true }); eq(html(b), 'twopastedpasted', 'pasted again');
        eq(press('a', { ctrlKey: true }), true, 'select all');
        eq(where(), '"one":0-"twopastedpasted":15', 'all selected');
        press('c', { ctrlKey: true }); press('Insert', { ctrlKey: true });
        getSelection().setBaseAndExtent(a.firstChild, 1, a.firstChild, 3); press('x', { ctrlKey: true });
        eq(html(a), 'o', 'cut');
        getSelection().setBaseAndExtent(a.firstChild, 0, a.firstChild, 1); press('Delete', { shiftKey: true });
        eq([html(a), where()], ['<br>', 'p#a:0'], 'cut with shift delete');
        eq(log, ['paste:', 'beforeinput:insertFromPaste', 'paste:', 'beforeinput:insertFromPaste', 'copy:', 'copy:',
          'cut:', 'beforeinput:deleteByCut', 'cut:', 'beforeinput:deleteByCut'], 'events');
        eq([press('v', { ctrlKey: true, altKey: true }), press('z', { ctrlKey: true }), press('Insert', { ctrlKey: true, shiftKey: true })],
          [false, true, false], 'other shortcuts and undo');
        eq(press('y', { ctrlKey: true }), true, 'redo cut');
        e.addEventListener('keydown', (event) => event.preventDefault(), { once: true });
        eq([press('v', { ctrlKey: true }), html(b)], [true, 'twopastedpasted'], 'cancelled shortcut');
        document.addEventListener('paste', (event) => event.preventDefault(), { once: true });
        press('v', { ctrlKey: true }); eq(html(b), 'twopastedpasted', 'cancelled paste');
        eq(document.execCommand('insertParagraph'), true, 'paragraph command');
        eq(document.execCommand('insertLineBreak'), true, 'line break command');
        eq(html(e), '<p id="a"><br></p><p><br><br></p><p id="b">twopastedpasted</p>', 'commands');
        "#,
    );
}
#[test]
fn empty_blocks_that_draw_a_line_hold_the_caret() {
    let mut page = fixture(
        "<div id=e contenteditable><p id=a>ab</p><p id=x><span id=s></span></p><!----><p id=y></p><p id=c>cd</p></div>",
    );
    // Only nodes with even ids have boxes on the fake page: `x` draws a line
    // and `y` draws none.
    assert_eq!(node_id(&mut page, "document.querySelector('#x')") % 2, 0);
    assert_eq!(node_id(&mut page, "document.querySelector('#y')") % 2, 1);
    editing(
        &mut page,
        r#"
        const e = ids('#e'), a = ids('#a'), x = ids('#x'), s = ids('#s'), c = ids('#c');
        e.focus(); caret(x, 0); eq(where(), 'p#x:0', 'stays in the empty line');
        caret(s, 0); eq(where(), 'span#s:0', 'stays inside the empty line');
        caret(a.firstChild, 2); press('ArrowRight'); eq(where(), 'p#x:0', 'right into the empty line');
        press('ArrowRight'); eq(where(), '"cd":0', 'right past the line without a box');
        press('ArrowLeft'); eq(where(), 'p#x:0', 'left into the empty line');
        press('ArrowLeft'); eq(where(), '"ab":2', 'left out of it');
        press('ArrowRight'); press('End'); eq(where(), 'p#x:0', 'line ends at its start');
        press('Home'); eq(where(), 'p#x:0', 'line starts at its start');
        type('z'); eq([html(x), where()], ['z<span id="s"></span>', '"z":1'], 'typed into the line');
        "#,
    );
}

#[test]
fn context_presses_move_the_caret_only_outside_the_selection() {
    let mut page = fixture("<div id=e contenteditable><p id=a>one two</p></div><p id=t>plain</p>");
    let a = node_id(&mut page, "document.querySelector('#a').firstChild");
    let t = node_id(&mut page, "document.querySelector('#t').firstChild");
    let pa = node_id(&mut page, "document.querySelector('#a')");
    let pt = node_id(&mut page, "document.querySelector('#t')");
    let pointer = format!(
        "const right = (id) => web.pointer('down', id, {{ clientX: 1, clientY: 1, button: 2, buttons: 2 }});\
         const [pa, pt] = [{pa}, {pt}];"
    );
    let step = |page: &mut Fixture, caret: Option<(u32, u32)>, script: &str| {
        page.page.caret.set(caret);
        editing(page, &format!("{pointer}\n{script}"));
    };
    step(
        &mut page,
        Some((a, 5)),
        "right(pa); eq([where(), document.activeElement.id], ['\"one two\":5', 'e'], 'caret moved');\
         const range = document.createRange(); range.setStart(ids('#a').firstChild, 0); range.setEnd(ids('#a').firstChild, 3);\
         getSelection().removeAllRanges(); getSelection().addRange(range);",
    );
    step(
        &mut page,
        Some((a, 1)),
        "right(pa); eq(where(), '\"one two\":0-\"one two\":3', 'inside the selection');",
    );
    step(
        &mut page,
        Some((t, 1)),
        "right(pt); eq(where(), '\"one two\":0-\"one two\":3', 'not editable'); right(pa); eq(where(), '\"one two\":0-\"one two\":3', 'caret elsewhere');",
    );
    step(
        &mut page,
        None,
        "right(pa); eq(where(), '\"one two\":0-\"one two\":3', 'no caret');",
    );
}
