use super::*;

#[test]
fn dispatch_follows_the_event_path() {
    let mut page = fixture("<div id=outer><button id=inner>x</button></div>");
    page.check(
        r#"
        const outer = ids('#outer'); const inner = ids('#inner'); const log = [];
        const record = (name) => (event) => log.push(`${name}:${event.eventPhase}:${event.currentTarget === window ? 'window' : event.currentTarget.id || event.currentTarget.nodeName}`);
        window.addEventListener('ping', record('wc'), true); document.addEventListener('ping', record('dc'), { capture: true });
        outer.addEventListener('ping', record('oc'), true); inner.addEventListener('ping', record('tc'), true);
        inner.addEventListener('ping', record('tb')); outer.addEventListener('ping', record('ob')); window.addEventListener('ping', record('wb'));
        const event = new Event('ping', { bubbles: true, cancelable: true });
        eq(event.eventPhase, 0, 'idle phase'); eq(event.composedPath(), [], 'no path');
        assert(inner.dispatchEvent(event), 'not cancelled');
        eq(log, ['wc:1:window', 'dc:1:#document', 'oc:1:outer', 'tc:2:inner', 'tb:2:inner', 'ob:3:outer', 'wb:3:window'], 'order');
        eq(event.target, inner, 'target'); eq(event.srcElement, inner, 'src'); eq(event.currentTarget, null, 'current cleared'); eq(event.eventPhase, 0, 'phase cleared');
        log.length = 0; inner.dispatchEvent(new Event('ping')); eq(log, ['wc:1:window', 'dc:1:#document', 'oc:1:outer', 'tc:2:inner', 'tb:2:inner'], 'no bubbling');
        log.length = 0; const stopped = new Event('ping', { bubbles: true });
        outer.addEventListener('ping', (event) => event.stopPropagation(), true); inner.dispatchEvent(stopped);
        eq(log, ['wc:1:window', 'dc:1:#document', 'oc:1:outer'], 'stopped'); assert(!stopped.cancelBubble, 'flag cleared');
        const detached = document.createElement('p'); let reached = false; detached.addEventListener('x', () => { reached = true; }); detached.dispatchEvent(new Event('x', { bubbles: true })); assert(reached, 'detached');
        const loaded = []; window.addEventListener('load', () => loaded.push('window')); document.addEventListener('load', () => loaded.push('document'));
        document.dispatchEvent(new Event('load', { bubbles: true })); eq(loaded, ['document'], 'load stops at document');
        throws(() => inner.dispatchEvent({}), 'TypeError', 'not an event');
        const busy = new Event('busy'); inner.addEventListener('busy', () => { throws(() => inner.dispatchEvent(busy), 'InvalidStateError', 'redispatch'); }); inner.dispatchEvent(busy);
        throws(() => new Event(), 'TypeError', 'event needs a type');
        "#,
    );
}

#[test]
fn listeners_options_and_cancellation() {
    let mut page = fixture("<p id=p>x</p>");
    page.check(
        r#"
        const p = ids('#p'); const log = [];
        const listener = () => log.push('once'); p.addEventListener('go', listener, { once: true }); p.addEventListener('go', listener, { once: true });
        p.dispatchEvent(new Event('go')); p.dispatchEvent(new Event('go')); eq(log, ['once'], 'once');
        const handler = { handleEvent(event) { log.push(event.type); } }; p.addEventListener('obj', handler); p.dispatchEvent(new Event('obj'));
        p.addEventListener('obj', { notAHandler: true }); p.dispatchEvent(new Event('obj')); eq(log.slice(1), ['obj', 'obj'], 'handleEvent');
        p.addEventListener('none', null); p.addEventListener('none', undefined); p.removeEventListener('none', null); p.removeEventListener('never', listener);
        const removed = () => log.push('removed'); p.addEventListener('rm', removed); p.addEventListener('rm', removed, true); p.removeEventListener('rm', removed);
        p.dispatchEvent(new Event('rm')); eq(log.filter((x) => x === 'removed').length, 1, 'capture kept'); p.removeEventListener('rm', removed, { capture: true });
        p.removeEventListener('rm', removed, true);
        let later = 0; const second = () => later++; p.addEventListener('mut', () => p.removeEventListener('mut', second)); p.addEventListener('mut', second);
        p.dispatchEvent(new Event('mut')); eq(later, 0, 'removed during dispatch');
        p.addEventListener('imm', (event) => event.stopImmediatePropagation()); p.addEventListener('imm', () => log.push('after imm')); p.dispatchEvent(new Event('imm'));
        assert(!log.includes('after imm'), 'immediate');
        const cancel = new Event('c', { cancelable: true }); p.addEventListener('c', (event) => event.preventDefault()); assert(!p.dispatchEvent(cancel), 'cancelled');
        assert(cancel.defaultPrevented && !cancel.returnValue, 'prevented');
        const passive = new Event('pv', { cancelable: true }); p.addEventListener('pv', (event) => event.preventDefault(), { passive: true }); assert(p.dispatchEvent(passive), 'passive');
        const plain = new Event('plain'); plain.preventDefault(); assert(!plain.defaultPrevented, 'not cancelable');
        const legacy = new Event('lg', { cancelable: true }); legacy.returnValue = false; assert(legacy.defaultPrevented, 'return value'); legacy.returnValue = true;
        legacy.cancelBubble = true; assert(legacy.cancelBubble, 'cancel bubble'); legacy.cancelBubble = false;
        const controller = new AbortController(); p.addEventListener('sig', () => log.push('signal'), { signal: controller.signal });
        p.dispatchEvent(new Event('sig')); controller.abort(); p.dispatchEvent(new Event('sig')); eq(log.filter((x) => x === 'signal').length, 1, 'aborted');
        p.addEventListener('sig', () => log.push('late'), { signal: controller.signal }); p.dispatchEvent(new Event('sig')); assert(!log.includes('late'), 'already aborted');
        assert(controller.signal.aborted && controller.signal.reason.name === 'AbortError', 'reason'); controller.abort('again');
        throws(() => controller.signal.throwIfAborted(), 'AbortError', 'throw if aborted'); new AbortController().signal.throwIfAborted();
        eq(AbortSignal.abort('why').reason, 'why', 'static abort'); throws(() => new AbortSignal(), 'TypeError', 'illegal signal');
        const timeout = AbortSignal.timeout(0); timeout.onabort = () => { globalThis.timedOut = timeout.reason.name; };
        p.addEventListener('throw', () => { throw new Error('listener failed'); }); p.dispatchEvent(new Event('throw'));
        const init = new Event('init'); init.initEvent('renamed', true, true); eq([init.type, init.bubbles, init.cancelable], ['renamed', true, true], 'initEvent'); init.initEvent('x');
        p.addEventListener('during', (event) => { event.initEvent('changed'); eq(event.type, 'during', 'init while dispatching'); eq(event.composedPath().length, 4, 'path'); });
        p.dispatchEvent(new Event('during'));
        const composed = new Event('c', { composed: true }); assert(composed.composed && composed.timeStamp >= 0 && !composed.isTrusted, 'flags');
        eq(Event.AT_TARGET + Event.prototype.BUBBLING_PHASE, 5, 'phases');
        const custom = new CustomEvent('cu', { detail: { a: 1 } }); eq(custom.detail, { a: 1 }, 'detail'); custom.initCustomEvent('cu2', false, false, 5); eq(custom.detail, 5, 'init custom');
        eq(new CustomEvent('x').detail, null, 'default detail');
        const mouse = new MouseEvent('click', { clientX: 3, clientY: 4, ctrlKey: true, button: 2 });
        eq([mouse.pageX, mouse.pageY, mouse.x, mouse.y, mouse.offsetX, mouse.offsetY, mouse.button, mouse.which], [3, 4, 3, 4, 3, 4, 2, 3], 'mouse');
        eq([new MouseEvent('mousedown').which, new PointerEvent('pointerdown', { button: 1 }).which], [1, 2], 'which');
        assert(mouse.getModifierState('Control') && !mouse.getModifierState('Shift') && !mouse.getModifierState('Fn'), 'modifiers');
        const key = new KeyboardEvent('keydown', { key: 'a', shiftKey: true }); assert(key.getModifierState('Shift') && key.key === 'a', 'keyboard');
        assert(new PointerEvent('pointerdown').isPrimary && new WheelEvent('wheel', { deltaY: 3 }).deltaY === 3, 'pointer wheel');
        eq([new FocusEvent('f').relatedTarget, new InputEvent('i', { data: 'x' }).data, new ErrorEvent('e', { message: 'm' }).message], [null, 'x', 'm'], 'other events');
        for (const constructor of [CompositionEvent, PromiseRejectionEvent, SubmitEvent, ProgressEvent, MessageEvent, StorageEvent, PopStateEvent, HashChangeEvent, AnimationEvent, TransitionEvent, DragEvent, ClipboardEvent, UIEvent]) {
          assert(new constructor('t') instanceof Event, constructor.name);
        }
        for (const name of ['Event', 'Events', 'HTMLEvents', 'CustomEvent', 'UIEvent', 'UIEvents', 'MouseEvent', 'MouseEvents', 'KeyboardEvent', 'FocusEvent']) {
          assert(document.createEvent(name) instanceof Event, name);
        }
        throws(() => document.createEvent('Nope'), 'NotSupportedError', 'create event');
        const target = new EventTarget(); let got = 0; target.addEventListener('t', () => got++); target.dispatchEvent(new Event('t')); target.removeEventListener('t', null); eq(got, 1, 'event target');
        p.onclick = (event) => { log.push('onclick'); return false; }; const click = new MouseEvent('click', { cancelable: true }); p.dispatchEvent(click);
        assert(click.defaultPrevented && typeof p.onclick === 'function', 'handler property'); p.onclick = () => log.push('replaced'); p.dispatchEvent(new MouseEvent('click'));
        p.onclick = null; eq(p.onclick, null, 'cleared'); p.dispatchEvent(new MouseEvent('click')); eq(log.filter((x) => x === 'replaced').length, 1, 'removed handler');
        eq(document.createElement('i').onfocus, null, 'no handler'); p.onfocus = 'not a function'; eq(p.onfocus, null, 'ignored string');
        document.onreadystatechange = () => {}; window.onpopstate = () => {}; eq(typeof window.onpopstate, 'function', 'window handler');
        "#,
    );
    page.settle();
    page.check("eq(timedOut, 'TimeoutError', 'timeout signal');");
    let printed = page.printed();
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Uncaught Error: listener failed"))
    );
}

#[test]
fn compiled_handler_attributes_follow_set_attribute() {
    let mut page = fixture("<p id=p>x</p>");
    page.check(r#"
      const p = ids('#p'); const log = [];
      globalThis.__nativeboatHandlers = new Map([['onclick' + String.fromCharCode(0) + 'go()', function (event) { log.push(this.id + ':' + event.type); return false; }]]);
      p.setAttribute('onclick', 'go()'); const click = new MouseEvent('click', { cancelable: true }); p.dispatchEvent(click);
      eq(log.join(), 'p:click', 'compiled handler runs with this'); eq(click.defaultPrevented, true, 'false prevents');
      p.setAttribute('onclick', 'other()'); eq(typeof p.onclick, 'function', 'unknown text keeps the handler');
      p.removeAttribute('onclick'); eq(p.onclick, null, 'removing the attribute removes the handler');
      p.setAttribute('one', 'go()'); eq(p.one, undefined, 'only event handler names');
      delete globalThis.__nativeboatHandlers;
    "#);
}

/// Script elements a page inserts load their source once they are in the
/// document, once: a script this engine has not compiled fires `error`.
/// Detached scripts and scripts of other types do not load.
#[test]
fn inserted_scripts_load_once_connected() {
    let mut page = fixture("<div id=host></div>");
    page.check(
        r#"
        window.seen = [];
        const script = document.createElement('script');
        script.addEventListener('load', () => seen.push('load'));
        script.addEventListener('error', () => seen.push('error'));
        script.setAttribute('src', 'node_modules/pkg/main.js');
        document.createElement('div').appendChild(script);
        document.head.appendChild(script);
        document.head.appendChild(script);
        const data = document.createElement('script');
        data.setAttribute('type', 'application/json');
        data.setAttribute('src', 'x.json');
        data.addEventListener('error', () => seen.push('json'));
        document.body.appendChild(data);
        "#,
    );
    page.settle();
    page.check("eq(seen.join(','), 'error', 'events');");
}
