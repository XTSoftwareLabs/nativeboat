use super::*;

#[test]
fn definitions_are_kept_by_name_and_constructor_and_validated() {
    let mut page = fixture("<body></body>");
    page.check(
        r#"
class Widget extends HTMLElement {}
eq(customElements instanceof CustomElementRegistry, true, 'registry');
eq(customElements.get('x-widget'), undefined, 'not yet defined');
let resolved = null;
customElements.whenDefined('x-widget').then((value) => { resolved = value; });
customElements.define('x-widget', Widget);
eq(customElements.get('x-widget') === Widget, true, 'get');
eq(customElements.getName(Widget), 'x-widget', 'getName');
eq(customElements.getName(class {}), null, 'getName of another class');
throws(() => customElements.define('x-widget', class extends HTMLElement {}), 'NotSupportedError', 'same name');
throws(() => customElements.define('x-other', Widget), 'NotSupportedError', 'same constructor');
for (const name of ['div', 'X-a', '1-a', 'font-face', 'x a']) {
  throws(() => customElements.define(name, class extends HTMLElement {}), 'SyntaxError', name);
}
throws(() => customElements.define('x-extends', class extends HTMLElement {}, { extends: 'x-widget' }), 'NotSupportedError', 'extends a custom name');
let threw = null;
try { customElements.define('x-number', 1); } catch (error) { threw = error; }
eq(threw instanceof TypeError, true, 'constructor must be a function');
threw = null;
try { new HTMLElement(); } catch (error) { threw = error; }
eq(threw instanceof TypeError, true, 'HTMLElement without a definition');
threw = null;
try { new HTMLDivElement(); } catch (error) { threw = error; }
eq(threw instanceof TypeError, true, 'built-in element constructors');
globalThis.resolvedLater = () => resolved === Widget;
let rejected = null;
customElements.whenDefined('nohyphen').catch((error) => { rejected = error.name; });
globalThis.rejectedLater = () => rejected;
"#,
    );
    page.settle();
    page.check(
        r#"
eq(resolvedLater(), true, 'whenDefined resolves with the constructor');
eq(rejectedLater(), 'SyntaxError', 'whenDefined rejects invalid names');
"#,
    );
}

#[test]
fn elements_are_made_by_their_constructors_and_match_defined() {
    let mut page = fixture("<body></body>");
    page.check(
        r#"
const log = [];
class Counter extends HTMLElement {
  constructor() { super(); this.count = 1; log.push('constructed ' + this.localName); }
}
customElements.define('x-counter', Counter);
const made = document.createElement('x-counter');
eq(made instanceof Counter && made instanceof HTMLElement, true, 'createElement uses the constructor');
eq(made.count, 1, 'constructor fields');
const direct = new Counter();
eq(direct.localName, 'x-counter', 'new makes an element of the name');
eq(direct.ownerDocument === document, true, 'owned by the document');
eq(direct.matches(':defined'), true, 'constructed elements are defined');
const undefinedOne = document.createElement('x-later');
eq(undefinedOne.matches(':defined'), false, 'undefined custom element');
eq(undefinedOne.matches(':not(:defined)'), true, 'not defined');
eq(document.createElement('div').matches(':defined'), true, 'built-ins are defined');
eq(log, ['constructed x-counter', 'constructed x-counter'], 'constructor runs');
// A constructor that breaks the rules gives an unknown element.
customElements.define('x-bad', class extends HTMLElement {
  constructor() { super(); this.setAttribute('a', 'b'); }
});
const bad = document.createElement('x-bad');
eq(bad instanceof HTMLUnknownElement, true, 'failed constructor result');
eq(bad.localName, 'x-bad', 'failed element name');
"#,
    );
    let printed = page.printed().join("\n");
    assert!(
        printed.contains("must not give the element attributes"),
        "{printed}"
    );
}

#[test]
fn definitions_upgrade_document_elements_and_report_attributes_then_connection() {
    let mut page = fixture(
        r#"<body><x-item id=a title=one></x-item><div><x-item id=b></x-item></div></body>"#,
    );
    page.check(
        r#"
const log = [];
const first = ids('#a');
eq(first instanceof HTMLElement, true, 'an HTML element before');
eq(first.matches(':defined'), false, 'undefined before');
class Item extends HTMLElement {
  static get observedAttributes() { return ['title']; }
  constructor() { super(); log.push('construct ' + this.id); }
  connectedCallback() { log.push('connected ' + this.id); }
  disconnectedCallback() { log.push('disconnected ' + this.id); }
  attributeChangedCallback(name, old, value) { log.push(`attribute ${this.id} ${name} ${old} ${value}`); }
}
customElements.define('x-item', Item);
eq(first instanceof Item, true, 'the same wrapper is upgraded');
eq(first.matches(':defined'), true, 'defined after');
eq(log, ['construct a', 'attribute a title null one', 'connected a', 'construct b', 'connected b'], 'upgrade order');
"#,
    );
}

#[test]
fn connection_reactions_follow_insertions_removals_and_moves() {
    let mut page = fixture(r#"<body><div id=one></div><div id=two></div></body>"#);
    page.check(
        r#"
const log = [];
class Probe extends HTMLElement {
  connectedCallback() { log.push('connected ' + this.id); }
  disconnectedCallback() { log.push('disconnected ' + this.id); }
}
customElements.define('x-probe', Probe);
const probe = document.createElement('x-probe');
probe.id = 'p';
const detached = document.createElement('section');
detached.appendChild(probe);
eq(log, [], 'no reactions in a detached tree');
ids('#one').appendChild(detached);
eq(log, ['connected p'], 'connected with its ancestor');
ids('#two').appendChild(probe);
eq(log, ['connected p', 'disconnected p', 'connected p'], 'a move disconnects and connects');
probe.remove();
eq(log.slice(3), ['disconnected p'], 'removed');
ids('#one').innerHTML = '<x-probe id=q></x-probe>';
const parsed = ids('#q');
eq(parsed instanceof Probe, true, 'parsed into the document and upgraded');
eq(log.slice(4), ['connected q'], 'parsed connection');
ids('#one').textContent = '';
eq(log.slice(5), ['disconnected q'], 'replaced children disconnect');
const holder = document.createElement('div');
holder.innerHTML = '<x-probe id=r></x-probe>';
eq(holder.firstChild instanceof Probe, false, 'not upgraded while detached');
customElements.upgrade(holder);
eq(holder.firstChild instanceof Probe, true, 'upgrade() upgrades detached trees');
eq(log.length, 6, 'upgrading a detached element does not connect it');
const clone = probe.cloneNode(false);
eq(clone instanceof Probe, true, 'clones are upgraded');
"#,
    );
}

#[test]
fn attribute_reactions_report_only_observed_attributes_that_change() {
    let mut page = fixture("<body></body>");
    page.check(
        r#"
const log = [];
class Field extends HTMLElement {
  static observedAttributes = ['value', 'disabled', 'style'];
  attributeChangedCallback(name, old, value) { log.push([name, old, value]); }
}
customElements.define('x-field', Field);
const field = document.createElement('x-field');
field.setAttribute('value', 'a');
field.setAttribute('VALUE', 'b');
field.setAttribute('other', 'c');
field.removeAttribute('missing');
field.removeAttribute('value');
field.toggleAttribute('disabled');
field.toggleAttribute('disabled', true);
field.toggleAttribute('disabled');
field.style.color = 'red';
eq(log, [
  ['value', null, 'a'],
  ['value', 'a', 'b'],
  ['value', 'b', null],
  ['disabled', null, ''],
  ['disabled', '', null],
  ['style', null, 'color: red;'],
], 'observed changes');
"#,
    );
}

#[test]
fn customized_built_in_elements_extend_their_element_classes() {
    let mut page = fixture(r#"<body><button is=x-press id=parsed>go</button></body>"#);
    page.check(
        r#"
class Press extends HTMLButtonElement {
  connectedCallback() { this.dataset.connected = 'yes'; }
}
customElements.define('x-press', Press, { extends: 'button' });
const parsed = ids('#parsed');
eq(parsed instanceof Press && parsed instanceof HTMLButtonElement, true, 'parsed button upgraded');
eq(parsed.dataset.connected, 'yes', 'connected');
const made = document.createElement('button', { is: 'x-press' });
eq(made instanceof Press, true, 'createElement with is');
eq(made.getAttribute('is'), 'x-press', 'is attribute');
const direct = new Press();
eq(direct.localName, 'button', 'new makes the extended element');
eq(document.createElement('x-press') instanceof Press, false, 'not by its own name');
"#,
    );
}

#[test]
fn reactions_that_throw_are_reported_and_other_reactions_still_run() {
    let mut page = fixture("<body></body>");
    page.check(
        r#"
const log = [];
customElements.define('x-throws', class extends HTMLElement {
  connectedCallback() { log.push(this.id); throw new Error('reaction failed ' + this.id); }
});
document.body.innerHTML = '<x-throws id=a></x-throws><x-throws id=b></x-throws>';
eq(log, ['a', 'b'], 'both connected');
"#,
    );
    let printed = page.printed().join("\n");
    assert!(printed.contains("reaction failed a"), "{printed}");
    assert!(printed.contains("reaction failed b"), "{printed}");
}
