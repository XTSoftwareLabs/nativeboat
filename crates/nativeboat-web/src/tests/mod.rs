use super::*;
use nativeboat_js::Level;
use std::cell::Cell;

mod documents;
mod editing_history;
mod ranges;

/// A page with fixed geometry that records what it is asked to do. Elements
/// with even ids are rendered.
#[derive(Default)]
struct FakePage {
    calls: RefCell<Vec<String>>,
    storage: RefCell<Option<String>>,
    dark: Cell<bool>,
    /// Selections of elements shown as text controls, by node id.
    selections: RefCell<std::collections::HashMap<u32, (u32, u32)>>,
    /// The clipboard text; `None` when the clipboard is unavailable.
    clipboard: RefCell<Option<String>>,
    clipboard_html: RefCell<Option<String>>,
    /// Canvas drawing calls as `node size commands`.
    canvas: RefCell<Vec<String>>,
    /// The caret position at every point.
    caret: Cell<Option<(u32, u32)>>,
    composition: RefCell<Option<String>>,
    /// Computed values an element has besides the fixed ones, by node id.
    styles: RefCell<std::collections::HashMap<u32, Vec<(String, String)>>>,
}

impl Page for FakePage {
    fn url(&self) -> String {
        "file:///C:/app/index.html".to_owned()
    }

    fn rect(&self, node: NodeId) -> Option<Rect> {
        node.0
            .is_multiple_of(2)
            .then(|| Rect::new(10.0, 20.0, 100.0, 49.5))
    }

    fn client(&self, node: NodeId) -> Option<Rect> {
        node.0
            .is_multiple_of(2)
            .then(|| Rect::new(0.75, 1.5, 97.5, 46.25))
    }

    fn bbox(&self, node: NodeId) -> Option<Rect> {
        node.0
            .is_multiple_of(2)
            .then(|| Rect::new(-3.0, 4.0, 30.0, 40.0))
    }

    fn scroll_size(&self, node: NodeId) -> Option<(f32, f32)> {
        node.0.is_multiple_of(2).then_some((299.5, 400.25))
    }

    fn computed_style(&self, node: NodeId) -> Vec<(String, String)> {
        let mut style = vec![
            ("color".to_owned(), "rgb(255, 0, 0)".to_owned()),
            ("position".to_owned(), "relative".to_owned()),
        ];
        style.extend(
            self.styles
                .borrow()
                .get(&node.0)
                .cloned()
                .unwrap_or_default(),
        );
        style
    }

    fn viewport(&self) -> (f32, f32) {
        (800.0, 600.0)
    }

    fn device_pixel_ratio(&self) -> f32 {
        1.25
    }

    fn element_from_point(&self, _: f32, _: f32) -> Option<NodeId> {
        Some(NodeId(2))
    }

    fn caret_from_point(&self, _: f32, _: f32) -> Option<(NodeId, u32)> {
        self.caret
            .get()
            .map(|(node, offset)| (NodeId(node), offset))
    }

    /// Text is five pixels a character on one line at the top.
    fn text_rects(&self, _: NodeId, start: u32, end: u32) -> Vec<Rect> {
        vec![Rect::new(
            start as f32 * 5.0,
            0.0,
            (end - start) as f32 * 5.0,
            10.0,
        )]
    }

    fn match_media(&self, query: &str) -> bool {
        query.contains("dark") && self.dark.get()
    }

    fn focus(&self, node: Option<NodeId>) {
        self.calls.borrow_mut().push(format!("focus {node:?}"));
    }

    fn complete_composition(&self) -> Option<String> {
        self.composition.borrow_mut().take()
    }

    fn scroll_into_view(&self, node: NodeId, block: ScrollAlign, inline: ScrollAlign) {
        let _ = (block, inline);
        self.calls.borrow_mut().push(format!("scroll {}", node.0));
    }

    fn image_size(&self, url: &str) -> Option<(f32, f32)> {
        url.ends_with("logo.png").then_some((64.0, 32.0))
    }

    fn load_media(&self, url: &str) -> Option<LoadedMedia> {
        self.calls.borrow_mut().push(format!("media {url}"));
        if url.ends_with("clip.mp4") {
            Some((1.5, Some((2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]))))
        } else if url.ends_with("tone.mp3") {
            Some((f64::INFINITY, None))
        } else {
            None
        }
    }

    fn add_frame(&self, key: &str, width: u32, height: u32, pixels: &[u8]) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("frame {key} {width}x{height} {}", pixels.len()));
        true
    }

    fn load_storage(&self) -> Option<String> {
        self.storage.borrow().clone()
    }

    fn save_storage(&self, data: &str) {
        *self.storage.borrow_mut() = Some(data.to_owned());
    }

    fn sheet_rules(&self, owner: NodeId, rules: &str) {
        self.calls
            .borrow_mut()
            .push(format!("rules {} {}", owner.0, rules.lines().count()));
    }

    fn text_selection(&self, node: NodeId) -> Option<(u32, u32)> {
        self.selections.borrow().get(&node.0).copied()
    }

    fn select_text(&self, node: NodeId, start: u32, end: u32) {
        self.selections.borrow_mut().insert(node.0, (start, end));
    }

    fn read_clipboard(&self) -> Option<String> {
        self.clipboard.borrow().clone()
    }

    fn open_select(&self, node: NodeId) {
        self.calls.borrow_mut().push(format!("select {}", node.0));
    }

    fn write_clipboard(&self, text: &str) -> bool {
        self.write_clipboard_contents(Some(text), None)
    }

    fn read_clipboard_contents(&self) -> Option<Vec<(String, String)>> {
        if self.clipboard.borrow().is_none() && self.clipboard_html.borrow().is_none() {
            return None;
        }
        let mut contents = Vec::new();
        if let Some(text) = self.clipboard.borrow().as_ref() {
            contents.push(("text/plain".into(), text.clone()));
        }
        if let Some(html) = self.clipboard_html.borrow().as_ref() {
            contents.push(("text/html".into(), html.clone()));
        }
        Some(contents)
    }

    fn write_clipboard_contents(&self, text: Option<&str>, html: Option<&str>) -> bool {
        if self.clipboard.borrow().is_none() && self.clipboard_html.borrow().is_none() {
            return false;
        }
        *self.clipboard.borrow_mut() = text.map(str::to_owned);
        *self.clipboard_html.borrow_mut() = html.map(str::to_owned);
        true
    }

    fn canvas_draw(&self, node: NodeId, width: u32, height: u32, commands: &str) {
        self.canvas
            .borrow_mut()
            .push(format!("{} {width}x{height} {commands}", node.0));
    }

    fn canvas_read(&self, _: NodeId, (_, _, width, height): (i32, i32, u32, u32)) -> Vec<u8> {
        (0..width * height * 4).map(|index| index as u8).collect()
    }

    fn canvas_write(
        &self,
        node: NodeId,
        (x, y, width, height): (i32, i32, u32, u32),
        pixels: &[u8],
    ) {
        self.canvas.borrow_mut().push(format!(
            "{} write {x},{y} {width}x{height} {pixels:?}",
            node.0
        ));
    }

    fn canvas_url(&self, _: NodeId, jpeg: bool) -> String {
        if jpeg {
            "data:image/jpeg;base64,/9j/".to_owned()
        } else {
            "data:image/png;base64,iVBO".to_owned()
        }
    }

    fn parse_color(&self, text: &str) -> Option<[f32; 4]> {
        match text {
            "red" => Some([255.0, 0.0, 0.0, 1.0]),
            "blue" => Some([0.0, 0.0, 255.0, 1.0]),
            "rgba(0, 0, 255, 0.5)" => Some([0.0, 0.0, 255.0, 0.5]),
            _ => None,
        }
    }

    fn parse_font(&self, text: &str) -> Option<String> {
        text.contains("px")
            .then(|| r#"{"families":["serif"],"size":20.0,"weight":700,"italic":false}"#.to_owned())
    }

    fn measure_text(&self, font: &str, text: &str) -> [f32; 3] {
        let size = if font.contains("\"size\":20") {
            2.0
        } else {
            1.0
        };
        [
            text.chars().count() as f32 * 5.0 * size,
            8.0 * size,
            2.0 * size,
        ]
    }

    fn path_data(&self, data: &str) -> String {
        if data.is_empty() {
            "[]".to_owned()
        } else {
            r#"[{"Move":[0,0]},{"Line":[10,0]},"Close"]"#.to_owned()
        }
    }
}

type Output = Rc<RefCell<Vec<(Level, String)>>>;

struct Fixture {
    host: Host,
    dom: Rc<RefCell<Dom>>,
    page: Rc<FakePage>,
    output: Output,
}

const HELPERS: &str = r#"
const assert = (condition, message) => { if (!condition) throw new Error('assertion failed: ' + message); };
const eq = (actual, expected, message) => {
  const a = JSON.stringify(actual), e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${message}: ${a} !== ${e}`);
};
const throws = (callback, name, message) => {
  try { callback(); } catch (error) { eq(error.name, name, message); return error; }
  throw new Error(message + ': did not throw');
};
const ids = (selector) => document.querySelector(selector);
"#;

fn fixture_with(html: &str, page: FakePage) -> Fixture {
    let output: Output = Rc::default();
    let sink = output.clone();
    let mut host = Host::new(
        false,
        Rc::new(move |level, text: &str| sink.borrow_mut().push((level, text.to_owned()))),
    );
    let dom = Rc::new(RefCell::new(Dom::from_document(
        &nativeboat_html::parse_document(html),
    )));
    let page = Rc::new(page);
    install(&mut host, dom.clone(), page.clone());
    Fixture {
        host,
        dom,
        page,
        output,
    }
}

fn fixture(html: &str) -> Fixture {
    fixture_with(html, FakePage::default())
}

impl Fixture {
    fn eval(&mut self, code: &str) -> String {
        let value = self
            .host
            .eval(code, "test.js")
            .unwrap_or_else(|error| panic!("{error}"));
        self.host.run_microtasks();
        value
            .to_string(self.host.context())
            .unwrap()
            .to_std_string_escaped()
    }

    /// Runs a script with the assertion helpers; `web` is the renderer API.
    fn check(&mut self, script: &str) {
        let web = self
            .host
            .host_object()
            .get(js_string!("web"), self.host.context())
            .unwrap();
        self.host
            .context()
            .global_object()
            .set(js_string!("web"), web, false, self.host.context())
            .unwrap();
        let code = format!("(() => {{ {HELPERS}\n{script}\n; return 'ok'; }})()");
        assert_eq!(self.eval(&code), "ok");
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.host.run_microtasks();
            std::thread::sleep(std::time::Duration::from_millis(2));
            self.host.run_timers();
        }
    }

    fn printed(&self) -> Vec<String> {
        self.output
            .borrow_mut()
            .drain(..)
            .map(|(_, text)| text)
            .collect()
    }
}

use boa_engine::js_string;

mod canvas;
mod composition;
mod contenteditable;
mod custom_elements;
mod editing;
mod events;
mod forms;
mod input;
mod shadow;
mod styles;
mod window;

#[test]
fn nodes_and_the_tree() {
    let mut page = fixture("<div id=root><p id=first>one</p><!--c--><span>two</span></div>");
    page.check(
        r#"
        const root = ids('#root');
        eq(root.nodeType, 1, 'type'); eq(root.nodeName, 'DIV', 'name'); eq(root.tagName, 'DIV', 'tag');
        eq(root.childNodes.length, 3, 'children'); eq(root.childNodes[1].nodeType, 8, 'comment');
        eq(root.childNodes.item(5), null, 'missing item'); eq(root.childNodes[7], undefined, 'missing index');
        assert(0 in root.childNodes && !(9 in root.childNodes) && 'length' in root.childNodes, 'has');
        eq(Object.keys(root.childNodes).slice(0, 3), ['0', '1', '2'], 'keys');
        eq([...root.childNodes].length, 3, 'iterate'); let seen = 0; root.childNodes.forEach(() => seen++); eq(seen, 3, 'forEach');
        eq([...root.childNodes.keys()].length + [...root.childNodes.values()].length + [...root.childNodes.entries()].length, 9, 'iterators');
        eq(root.children.length, 2, 'elements'); eq(root.children.item(0).id, 'first', 'item'); eq(root.children.namedItem('first').id, 'first', 'named');
        eq(root.children.namedItem('none'), null, 'unnamed'); eq(root.children.item(9), null, 'no item'); eq([...root.children].length, 2, 'iterate children');
        eq(root.childElementCount, 2, 'count'); eq(root.firstElementChild.id, 'first', 'first'); eq(root.lastElementChild.localName, 'span', 'last');
        const first = root.firstChild;
        eq(first.nextSibling.nodeValue, 'c', 'comment value'); eq(root.lastChild.previousSibling.nodeType, 8, 'previous');
        eq(first.nextElementSibling.localName, 'span', 'next element'); eq(root.lastChild.previousElementSibling.id, 'first', 'previous element');
        eq(first.firstChild.nextElementSibling, null, 'text siblings'); eq(first.firstChild.previousElementSibling, null, 'text previous');
        eq(first.parentNode, root, 'parent'); eq(first.parentElement, root, 'parent element'); eq(document.documentElement.parentElement, null, 'root parent');
        eq(first.ownerDocument === document, true, 'owner'); eq(document.ownerDocument, null, 'document owner');
        assert(root.isConnected && root.hasChildNodes() && !first.firstChild.hasChildNodes(), 'connected');
        const text = document.createTextNode('x');
        eq(text.textContent, 'x', 'text'); text.nodeValue = 'y'; eq(text.data, 'y', 'data'); text.nodeValue = null; eq(text.data, '', 'null value');
        root.nodeValue = 'ignored'; eq(root.nodeValue, null, 'element value');
        text.data = 'hello'; eq(text.length, 5, 'length'); text.appendData('!'); text.insertData(0, '>'); text.deleteData(0, 1); text.replaceData(0, 1, 'H');
        eq(text.data, 'Hello!', 'edited'); eq(text.substringData(1, 3), 'ell', 'substring'); eq(text.wholeText, 'Hello!', 'whole');
        root.appendChild(text); const tail = text.splitText(5); eq(tail.data, '!', 'split'); eq(text.nextSibling, tail, 'split sibling');
        new Text('detached').splitText(2);
        eq(new Text('a').data + new Comment('b').data, 'ab', 'constructors'); eq(new Text().data, '', 'empty text');
        eq(new DocumentFragment().nodeType, 11, 'fragment'); eq(document.createComment('k').nodeName, '#comment', 'comment');
        const fragment = document.createDocumentFragment(); fragment.append('a', document.createElement('b'), 'c');
        eq(fragment.childNodes.length, 3, 'append'); fragment.prepend('z'); eq(fragment.firstChild.data, 'z', 'prepend');
        fragment.lastElementChild.id = 'inside'; eq(fragment.getElementById('inside').localName, 'b', 'fragment id');
        root.appendChild(fragment); eq(fragment.childNodes.length, 0, 'moved');
        const marker = document.createElement('i'); root.insertBefore(marker, first); eq(root.firstChild, marker, 'insert');
        root.insertBefore(document.createElement('u'), null); eq(root.lastChild.localName, 'u', 'insert at end');
        marker.after('after'); marker.before('before'); eq(marker.previousSibling.data + marker.nextSibling.data, 'beforeafter', 'before after');
        marker.replaceWith(document.createElement('em')); eq(marker.parentNode, null, 'replaced');
        marker.before('x'); marker.after('y'); marker.replaceWith('z'); marker.remove();
        text.before('t'); text.after('u'); text.replaceWith('v'); eq(text.parentNode, null, 'text replaced'); text.remove();
        const removed = root.removeChild(first); eq(removed, first, 'removed'); eq(first.parentNode, null, 'detached');
        const swapped = root.replaceChild(first, root.firstChild); eq(first.parentNode, root, 'replace child'); eq(swapped.parentNode, null, 'swapped');
        root.replaceChildren(document.createElement('a'), 'text'); eq(root.childNodes.length, 2, 'replace children'); root.replaceChildren(); eq(root.childNodes.length, 0, 'cleared');
        throws(() => root.appendChild(document.documentElement), 'HierarchyRequestError', 'cycle');
        const error = throws(() => root.removeChild(first), 'NotFoundError', 'not a child'); eq(error.code, 8, 'code'); assert(error instanceof DOMException, 'instance');
        throws(() => document.createElement('1bad'), 'InvalidCharacterError', 'bad name');
        throws(() => root.appendChild({}), 'TypeError', 'not a node');
        throws(() => new Node(), 'TypeError', 'illegal');
        eq(new DOMException('m').name, 'Error', 'default name'); eq(new DOMException('m', 'Custom').code, 0, 'unknown code');
        const clone = root.cloneNode(); eq(clone.id, 'root', 'clone'); assert(clone !== root, 'distinct');
        root.textContent = 'plain'; eq(root.innerHTML, 'plain', 'text content'); root.textContent = null; eq(root.innerHTML, '', 'cleared text');
        document.textContent = 'ignored'; eq(document.textContent, null, 'document text');
        root.innerHTML = '<b>1</b><i>2</i>'; const [b, i] = root.children;
        eq(b.compareDocumentPosition(i), 4, 'following'); eq(i.compareDocumentPosition(b), 2, 'preceding');
        eq(root.compareDocumentPosition(b), 20, 'contained'); eq(b.compareDocumentPosition(root), 10, 'contains'); eq(b.compareDocumentPosition(b), 0, 'same');
        eq(b.compareDocumentPosition(document.createElement('x')), 35, 'disconnected');
        eq(Node.ELEMENT_NODE + Node.prototype.DOCUMENT_POSITION_FOLLOWING, 5, 'constants');
        assert(root.contains(b) && !b.contains(root) && !root.contains(null), 'contains');
        eq(b.getRootNode(), document, 'root node'); assert(b.isSameNode(b) && b.isEqualNode(b.cloneNode(true)) && !b.isEqualNode(null), 'equality');
        b.normalize(); eq(b.lookupNamespaceURI(), null, 'namespace lookup'); eq(b.baseURI, 'file:///C:/app/index.html', 'base');
        "#,
    );
}

#[test]
fn elements_and_attributes() {
    let mut page =
        fixture("<div id=a class='x y' data-user-id=7 title=t><span class=y>1</span></div>");
    page.check(
        r#"
        const a = ids('#a');
        eq(a.getAttribute('title'), 't', 'get'); eq(a.getAttribute('none'), null, 'missing'); eq(a.getAttributeNS(null, 'title'), 't', 'ns');
        a.setAttribute('role', 'note'); a.setAttributeNS(null, 'aria-label', 'L'); assert(a.hasAttribute('role') && a.hasAttributeNS(null, 'aria-label'), 'set');
        a.removeAttribute('role'); a.removeAttributeNS(null, 'aria-label'); assert(!a.hasAttribute('role'), 'removed');
        eq(a.toggleAttribute('hidden'), true, 'toggle on'); eq(a.toggleAttribute('hidden'), false, 'toggle off');
        eq(a.toggleAttribute('hidden', true), true, 'force on'); eq(a.toggleAttribute('hidden', false), false, 'force off');
        throws(() => a.setAttribute('a b', 'x'), 'InvalidCharacterError', 'bad attribute');
        throws(() => a.toggleAttribute('a b'), 'InvalidCharacterError', 'bad toggle');
        assert(a.hasAttributes() && !document.createElement('b').hasAttributes(), 'has attributes');
        eq(a.getAttributeNames(), ['id', 'class', 'data-user-id', 'title'], 'names');
        eq(a.attributes.length, 4, 'attributes'); eq(a.attributes[0].name, 'id', 'first'); eq(a.attributes.getNamedItem('TITLE').value, 't', 'named');
        eq(a.attributes.item(1).value, 'x y', 'item'); eq(a.attributes.getNamedItem('none'), null, 'none'); eq(a.attributes.item(9), null, 'none item');
        eq([...a.attributes].map((attr) => attr.ownerElement === a).length, 4, 'iterate'); throws(() => new Attr(), 'TypeError', 'attr');
        eq(a.getAttributeNode('title').value, 't', 'node'); eq(a.getAttributeNode('none'), null, 'no node');
        eq(a.id, 'a', 'id'); a.id = 'b'; eq(a.id, 'b', 'set id'); a.id = 'a';
        eq(a.className, 'x y', 'class'); a.className = 'x y'; eq(document.createElement('p').className, '', 'no class');
        const list = a.classList; eq(list.length, 2, 'tokens'); eq(list[0], 'x', 'index'); eq(list.item(1), 'y', 'item'); eq(list.item(5), null, 'no item');
        assert(list.contains('x') && !list.contains('z'), 'contains'); list.add('z', 'x'); eq(a.className, 'x y z', 'add'); list.remove('x'); eq(list.value, 'y z', 'remove');
        eq(list.toggle('q'), true, 'toggle'); eq(list.toggle('q'), false, 'toggle back'); eq(list.toggle('y', true), true, 'force'); eq(list.toggle('w', false), false, 'force off');
        assert(list.replace('z', 'y') && !list.replace('none', 'v'), 'replace'); eq(String(list), 'y', 'dedupe');
        list.value = 'm n'; eq([...list], ['m', 'n'], 'value'); let count = 0; list.forEach(() => count++); eq(count, 2, 'forEach');
        assert(list.supports('x'), 'supports'); eq(Object.prototype.toString.call(list), '[object DOMTokenList]', 'tag');
        throws(() => list.add(''), 'SyntaxError', 'empty token'); throws(() => list.add('a b'), 'InvalidCharacterError', 'space token');
        eq(a.classList, list, 'cached'); throws(() => new DOMTokenList(), 'TypeError', 'illegal list');
        eq(a.dataset.userId, '7', 'dataset'); a.dataset.fooBar = 'x'; eq(a.getAttribute('data-foo-bar'), 'x', 'set dataset');
        assert('fooBar' in a.dataset && !('nope' in a.dataset) && !(Symbol.iterator in a.dataset), 'has dataset');
        eq(Object.keys(a.dataset), ['userId', 'fooBar'], 'keys'); delete a.dataset.fooBar; eq(a.dataset.fooBar, undefined, 'deleted');
        eq(a.dataset[Symbol.toStringTag], undefined, 'symbol get'); eq(Object.getOwnPropertyDescriptor(a.dataset, 'nope'), undefined, 'descriptor');
        eq(a.dataset, a.dataset, 'dataset cached'); throws(() => new DOMStringMap(), 'TypeError', 'illegal map');
        a.innerHTML = '<b>bold</b>'; eq(a.innerHTML, '<b>bold</b>', 'inner'); a.innerHTML = null; eq(a.innerHTML, '', 'null inner');
        a.innerHTML = '<b>x</b>'; eq(a.outerHTML.startsWith('<div id="a"'), true, 'outer');
        const b = a.firstChild; b.outerHTML = '<i>1</i><i>2</i>'; eq(a.innerHTML, '<i>1</i><i>2</i>', 'set outer');
        document.createElement('p').outerHTML = '<x></x>';
        const top = document.createElement('section'); document.body.appendChild(top); top.outerHTML = '<aside></aside>';
        eq(document.body.lastElementChild.localName, 'aside', 'outer in body');
        a.insertAdjacentHTML('afterbegin', '<u>0</u>'); a.insertAdjacentHTML('beforeend', '<u>3</u>');
        a.insertAdjacentText('beforeend', '!'); eq(a.textContent, '012' + '3!', 'adjacent');
        const outside = document.createElement('hr'); eq(a.insertAdjacentElement('beforebegin', outside), outside, 'before begin');
        eq(document.createElement('p').insertAdjacentElement('afterend', document.createElement('q')), null, 'no parent after');
        eq(document.createElement('p').insertAdjacentElement('beforebegin', document.createElement('q')), null, 'no parent before');
        a.insertAdjacentElement('afterend', document.createElement('hr')); a.insertAdjacentHTML('afterend', '<br>');
        throws(() => a.insertAdjacentHTML('middle', 'x'), 'SyntaxError', 'bad position');
        eq(a.matches('div.y'), false, 'matches'); assert(a.webkitMatchesSelector('#a'), 'webkit');
        eq(a.querySelector('u').textContent, '0', 'query'); eq(a.querySelectorAll('u').length, 2, 'query all'); eq(a.querySelector('table'), null, 'none');
        eq(a.firstChild.closest('div').id, 'a', 'closest'); eq(a.closest('table'), null, 'no closest');
        throws(() => a.querySelector('['), 'SyntaxError', 'bad selector'); throws(() => a.matches('['), 'SyntaxError', 'bad match');
        throws(() => a.querySelectorAll('['), 'SyntaxError', 'bad all'); throws(() => a.closest('['), 'SyntaxError', 'bad closest');
        const tags = document.getElementsByTagName('u'); eq(tags.length, 2, 'by tag'); a.insertAdjacentHTML('beforeend', '<u>4</u>'); eq(tags.length, 3, 'live');
        eq(document.getElementsByClassName('m').length, 1, 'by class'); eq(a.getElementsByTagName('i').length, 2, 'element by tag'); eq(a.getElementsByClassName('none').length, 0, 'element by class');
        eq(document.getElementById('a'), a, 'by id'); eq(document.getElementById('none'), null, 'no id');
        const tpl = document.createElement('template'); tpl.innerHTML = '<b>x</b><i>y</i>'; eq([tpl.content.childNodes.length, tpl.childNodes.length, tpl.innerHTML, tpl.content === tpl.content].join(), '2,0,<b>x</b><i>y</i>,true', 'template content');
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); assert(svg instanceof SVGSVGElement && svg.namespaceURI.endsWith('svg'), 'svg');
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path'); assert(path instanceof SVGElement && !(path instanceof HTMLElement), 'svg element');
        svg.appendChild(path); eq(path.ownerSVGElement, svg, 'owner svg'); eq(svg.ownerSVGElement, null, 'outermost svg'); path.className.baseVal = 'a b'; eq([path.getAttribute('class'), path.className.animVal, path.className instanceof SVGAnimatedString, typeof a.className].join(), 'a b,a b,true,string', 'svg class name'); assert(!(document instanceof ShadowRoot) && ShadowRoot.prototype instanceof DocumentFragment, 'shadow root interface'); const box = path.getBBox(); eq([box.x, box.width].join(), path[Object.getOwnPropertySymbols(path)[0]] % 2 === 0 ? '-3,30' : '0,0', 'bbox');
        path.style = 'fill: red'; eq(path.style.fill, 'red', 'svg style'); eq(path.dataset.x, undefined, 'svg dataset');
        const m = svg.createSVGMatrix().translate(2, 3).rotate(90); eq([m.a, m.e, m.f].map(Math.round).join(), '0,2,3', 'svg matrix');
        const p = svg.createSVGPoint(); p.x = 1; const q = p.matrixTransform(m); eq([q.x, q.y].map(Math.round).join(), '2,4', 'svg point');
        eq(svg.createSVGRect().width, 0, 'svg rect'); eq(svg.viewBox.baseVal.width, 0, 'no view box'); svg.setAttribute('viewBox', '-8 -4, 100 50'); eq([svg.viewBox.baseVal.x, svg.viewBox.baseVal.height].join(), '-8,50', 'view box'); eq(path.getCTM().a, 1, 'ctm'); eq(typeof path.getScreenCTM().e, 'number', 'screen ctm');
        const math = document.createElementNS('http://www.w3.org/1998/Math/MathML', 'math'); assert(math instanceof Element && !(math instanceof HTMLElement), 'mathml');
        eq(document.createElementNS(null, 'div').localName, 'div', 'null ns'); throws(() => document.createElementNS('', '1'), 'InvalidCharacterError', 'bad ns name');
        eq(a.localName + a.prefix + a.slot, 'divnull', 'names'); eq(a.namespaceURI, 'http://www.w3.org/1999/xhtml', 'html ns');
        throws(() => document.createElement('img').attachShadow({ mode: 'open' }), 'NotSupportedError', 'shadow');
        a.setPointerCapture(1); a.releasePointerCapture(1); assert(!a.hasPointerCapture(1), 'capture');
        eq(a.getAnimations().length, 0, 'animations'); const animation = a.animate(); animation.cancel(); animation.finish(); animation.pause(); animation.play(); animation.reverse();
        eq(document.createElement('custom-thing') instanceof HTMLElement, true, 'unknown element');
        eq(document.createElement('div', { is: 'fancy-div' }).getAttribute('is'), 'fancy-div', 'is');
        "#,
    );
    page.check(
        "ids('#a').requestFullscreen().catch((error) => { globalThis.fullscreen = error.name; });",
    );
    page.check("eq(fullscreen, 'NotSupportedError', 'fullscreen');");
}

#[test]
fn natives_reject_unknown_nodes() {
    let dom = RefCell::new(Dom::new());
    let mut context = Context::default();
    let (_, kind) = natives::DOM
        .iter()
        .find(|(name, _)| *name == "kind")
        .unwrap();
    for value in [JsValue::from(99999), JsValue::from(1.5), JsValue::from(-1)] {
        let error = kind(&dom, &[value], &mut context).unwrap_err();
        assert!(error.to_string().starts_with("TypeError: not a node"));
    }
    assert!(kind(&dom, &[JsValue::from(0)], &mut context).is_ok());
}

#[test]
fn xml_serializer_writes_namespaces_prefixes_and_escapes() {
    let mut page = fixture("<p>x</p>");
    page.check(
        r##"
        const ns = 'http://www.w3.org/2000/svg';
        const svg = document.createElementNS(ns, 'svg'); svg.setAttribute('viewBox', '0 0 10 10');
        const text = document.createElementNS(ns, 'text'); text.setAttribute('data-x', 'a<b"c'); text.textContent = '1 < 2 & 3';
        const use = document.createElementNS(ns, 'use'); use.setAttribute('xlink:href', '#a');
        const object = document.createElementNS(ns, 'foreignObject'); object.innerHTML = '<div>hi<br></div>';
        svg.append(text, use, object);
        eq(new XMLSerializer().serializeToString(svg),
          '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text data-x="a&lt;b&quot;c">1 &lt; 2 &amp; 3</text>' +
          '<use xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="#a"/>' +
          '<foreignObject><div xmlns="http://www.w3.org/1999/xhtml">hi<br /></div></foreignObject></svg>', 'svg');
        const fragment = document.createDocumentFragment(); fragment.append('a', document.createComment('c'));
        eq(new XMLSerializer().serializeToString(fragment), 'a<!--c-->', 'fragment');
        throws(() => new XMLSerializer().serializeToString({}), 'TypeError', 'not a node');
        "##,
    );
}
