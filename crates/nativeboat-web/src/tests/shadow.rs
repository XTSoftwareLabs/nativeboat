use super::*;

#[test]
fn shadow_roots_attach_to_hosts_and_hide_closed_ones() {
    let mut page = fixture("<body><div id=open></div><div id=closed></div><img id=image></body>");
    page.check(
        r#"
const host = ids('#open');
eq(host.shadowRoot, null, 'no shadow root yet');
const root = host.attachShadow({ mode: 'open' });
eq(root instanceof ShadowRoot && root instanceof DocumentFragment, true, 'interfaces');
eq(root.nodeType, 11, 'a fragment');
eq(root.host === host, true, 'host');
eq(root.mode, 'open', 'mode');
eq(root.delegatesFocus, false, 'delegatesFocus');
eq(root.slotAssignment, 'named', 'slotAssignment');
eq(host.shadowRoot === root, true, 'open roots are exposed');
eq(root.parentNode, null, 'no parent');
throws(() => host.attachShadow({ mode: 'open' }), 'NotSupportedError', 'one root per host');
throws(() => ids('#image').attachShadow({ mode: 'open' }), 'NotSupportedError', 'img cannot host');
let threw = null;
try { ids('#closed').attachShadow({ mode: 'sideways' }); } catch (error) { threw = error; }
eq(threw instanceof TypeError, true, 'mode is checked');
const closed = ids('#closed').attachShadow({ mode: 'closed', delegatesFocus: true });
eq(ids('#closed').shadowRoot, null, 'closed roots are hidden');
eq(closed.mode, 'closed', 'closed mode');
eq(closed.delegatesFocus, true, 'delegatesFocus');
throws(() => root.cloneNode(true), 'NotSupportedError', 'shadow roots are not cloned');
throws(() => document.body.appendChild(root), 'HierarchyRequestError', 'shadow roots are not children');
let constructed = null;
try { new ShadowRoot(); } catch (error) { constructed = error; }
eq(constructed instanceof TypeError, true, 'not constructible');
"#,
    );
}

#[test]
fn shadow_trees_are_scoped_but_connected() {
    let mut page = fixture("<body><div id=host><b id=light>light</b></div></body>");
    page.check(
        r#"
const host = ids('#host');
const root = host.attachShadow({ mode: 'open' });
root.innerHTML = '<p id=inner><span class=deep>deep</span></p><slot></slot>';
eq(root.innerHTML, '<p id="inner"><span class="deep">deep</span></p><slot></slot>', 'innerHTML');
const deep = root.querySelector('.deep');
eq(deep.textContent, 'deep', 'querySelector inside');
eq(root.getElementById('inner').localName, 'p', 'getElementById');
eq(document.querySelector('.deep'), null, 'the document does not see into shadow trees');
eq(document.getElementById('inner'), null, 'nor its ids');
eq(deep.isConnected, true, 'connected through the host');
eq(document.contains(deep), false, 'not contained by the document');
eq(deep.getRootNode() === root, true, 'tree root');
eq(deep.getRootNode({ composed: true }) === document, true, 'composed root');
eq(host.children.length, 1, 'host children are unchanged');
host.remove();
eq(deep.isConnected, false, 'disconnected with the host');
"#,
    );
}

#[test]
fn host_children_are_assigned_to_slots_by_name() {
    let mut page = fixture(
        "<body><div id=host>text<h1 slot=title id=title>t</h1><p id=plain>p</p></div><div id=closed><i id=hidden>x</i></div></body>",
    );
    page.check(
        r#"
const host = ids('#host');
const root = host.attachShadow({ mode: 'open' });
root.innerHTML = '<slot name=title id=named></slot><slot id=default></slot><slot name=none>fallback</slot>';
const named = root.getElementById('named');
const fallback = root.querySelector('[name=none]');
eq(ids('#title').assignedSlot === named, true, 'named slot');
eq(ids('#plain').assignedSlot === root.getElementById('default'), true, 'default slot');
eq(host.firstChild.assignedSlot === root.getElementById('default'), true, 'text is slotted');
eq(named.name, 'title', 'slot name');
eq(named.assignedNodes().map((node) => node.id), ['title'], 'assignedNodes');
eq(root.getElementById('default').assignedNodes().length, 2, 'text and element');
eq(root.getElementById('default').assignedElements().map((node) => node.id), ['plain'], 'assignedElements');
eq(fallback.assignedNodes(), [], 'nothing assigned');
eq(fallback.assignedNodes({ flatten: true }).map((node) => node.data), ['fallback'], 'fallback when flattened');
ids('#plain').slot = 'title';
eq(named.assignedElements().map((node) => node.id), ['title', 'plain'], 'reassigned by the slot attribute');
const closed = ids('#closed').attachShadow({ mode: 'closed' });
closed.innerHTML = '<slot></slot>';
eq(ids('#hidden').assignedSlot, null, 'slots of closed trees are hidden');
"#,
    );
}

#[test]
fn events_leaving_shadow_trees_are_retargeted_to_their_hosts() {
    let mut page = fixture("<body><div id=host></div></body>");
    page.check(
        r#"
const host = ids('#host');
const root = host.attachShadow({ mode: 'open' });
root.innerHTML = '<div class=wrap><button>go</button></div>';
const button = root.querySelector('button');
const seen = [];
const note = (name) => (event) => seen.push([name, event.target === button ? 'button' : event.target === host ? 'host' : String(event.target), event.eventPhase]);
document.addEventListener('ping', note('document'), true);
host.addEventListener('ping', note('host'));
root.addEventListener('ping', note('root'));
button.addEventListener('ping', note('button'));
let path = null;
document.addEventListener('ping', (event) => { path = event.composedPath().length; });
const composed = new Event('ping', { bubbles: true, composed: true });
button.dispatchEvent(composed);
eq(seen, [['document', 'host', 1], ['button', 'button', 2], ['root', 'button', 3], ['host', 'host', 2]], 'composed path and targets');
eq(path, 8, 'composedPath includes the shadow tree');
eq(composed.target === host, true, 'the target outside stays the host');
seen.length = 0;
const local = new Event('ping', { bubbles: true });
button.dispatchEvent(local);
eq(seen, [['button', 'button', 2], ['root', 'button', 3]], 'uncomposed events stay in their tree');
eq(local.target, null, 'targets left inside shadow trees are cleared');
seen.length = 0;
button.dispatchEvent(new Event('ping', { composed: true }));
eq(seen, [['document', 'host', 1], ['button', 'button', 2], ['host', 'host', 2]], 'hosts see non-bubbling composed events at target');
"#,
    );
}

#[test]
fn events_from_slotted_nodes_pass_through_their_slots() {
    let mut page = fixture("<body><div id=host><b id=light>light</b></div></body>");
    page.check(
        r#"
const host = ids('#host');
const root = host.attachShadow({ mode: 'closed' });
root.innerHTML = '<section><slot></slot></section>';
const slot = root.querySelector('slot');
const seen = [];
slot.addEventListener('ping', (event) => seen.push('slot ' + event.target.id));
root.querySelector('section').addEventListener('ping', () => seen.push('section'));
host.addEventListener('ping', (event) => seen.push('host ' + event.target.id + ' ' + event.composedPath().length));
ids('#light').dispatchEvent(new Event('ping', { bubbles: true }));
eq(seen, ['slot light', 'section', 'host light 6'], 'slotted path, closed nodes hidden from the host');
"#,
    );
}

#[test]
fn focus_inside_a_shadow_tree_is_the_hosts_for_the_document() {
    let mut page = fixture("<body><div id=host></div></body>");
    page.check(
        r#"
const host = ids('#host');
const root = host.attachShadow({ mode: 'open' });
root.innerHTML = '<input id=field>';
const field = root.getElementById('field');
field.focus();
eq(document.activeElement === host, true, 'retargeted for the document');
eq(root.activeElement === field, true, 'the shadow root sees the field');
eq(host.matches(':focus-within'), true, 'focus-within on the host');
field.blur();
eq(root.activeElement, null, 'no focus');
"#,
    );
}

#[test]
fn custom_elements_in_shadow_trees_upgrade_and_react() {
    let mut page = fixture("<body><div id=host></div></body>");
    page.check(
        r#"
const root = ids('#host').attachShadow({ mode: 'open' });
root.innerHTML = '<x-early></x-early>';
const log = [];
customElements.define('x-early', class extends HTMLElement { connectedCallback() { log.push('early'); } });
customElements.define('x-late', class extends HTMLElement {
  connectedCallback() { log.push('late connected'); }
  disconnectedCallback() { log.push('late disconnected'); }
});
root.appendChild(document.createElement('x-late'));
ids('#host').remove();
eq(log, ['early', 'late connected', 'late disconnected'], 'reactions in shadow trees');
const other = document.createElement('div');
document.body.appendChild(other);
const otherRoot = other.attachShadow({ mode: 'open' });
otherRoot.innerHTML = '<x-late></x-late>';
eq(log.slice(3), ['late connected'], 'roots attached after a definition are observed');
"#,
    );
}
