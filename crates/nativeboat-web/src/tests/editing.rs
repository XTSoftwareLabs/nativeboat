use super::*;

const PAGE: &str = "<input id=t value='hello world'><textarea id=a>one\ntwo</textarea>\
<input id=ro readonly value=fixed><input id=c type=checkbox><p id=p>text</p>";

/// The node id of the element matching a selector.
fn node(page: &mut Fixture, selector: &str) -> u32 {
    let value = page.eval(&format!(
        "(() => {{ const element = document.querySelector('{selector}'); return element[Object.getOwnPropertySymbols(element).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')]; }})()"
    ));
    value.parse().unwrap()
}

fn clipboard(page: &Fixture) -> Option<String> {
    page.page.clipboard.borrow().clone()
}

#[test]
fn the_clipboard_api_reads_and_writes_text() {
    let mut page = fixture_with(
        PAGE,
        FakePage {
            clipboard: RefCell::new(Some("start".to_owned())),
            ..FakePage::default()
        },
    );
    page.check(
        "navigator.clipboard.readText().then((text) => { globalThis.read = text; });\
         navigator.clipboard.writeText(42).then(() => { globalThis.wrote = true; });",
    );
    page.settle();
    page.check("eq([read, wrote], ['start', true], 'clipboard');");
    assert_eq!(clipboard(&page).as_deref(), Some("42"));
    *page.page.clipboard.borrow_mut() = None;
    page.check(
        "navigator.clipboard.readText().catch((error) => { globalThis.readError = error.name; });\
         navigator.clipboard.writeText('x').catch((error) => { globalThis.writeError = error.name; });",
    );
    page.settle();
    page.check("eq([readError, writeError], ['NotAllowedError', 'NotAllowedError'], 'denied');");
}

#[test]
fn editing_commands_change_text_controls_like_typing() {
    let mut page = fixture_with(
        PAGE,
        FakePage {
            clipboard: RefCell::new(Some("start".to_owned())),
            ..FakePage::default()
        },
    );
    page.check(
        r#"
        globalThis.log = [];
        for (const type of ['copy', 'cut', 'paste', 'input']) {
          document.addEventListener(type, (event) => log.push(`${type}:${event.target.id}:${event.inputType || ''}`));
        }
        const t = ids('#t'); t.focus(); t.setSelectionRange(0, 5);
        eq(document.execCommand('copy'), true, 'copy');
        eq(log, ['copy:t:'], 'copy event');
        "#,
    );
    assert_eq!(clipboard(&page).as_deref(), Some("hello"));
    page.check(
        r#"
        const t = ids('#t'); log.length = 0;
        t.setSelectionRange(5, 11); eq(document.execCommand('Cut'), true, 'cut');
        eq([t.value, t.selectionStart, t.selectionEnd], ['hello', 5, 5], 'after cut');
        eq(log, ['cut:t:', 'input:t:deleteByCut'], 'cut events');
        log.length = 0; t.setSelectionRange(0, 0); eq(document.execCommand('paste'), true, 'paste');
        eq([t.value, t.selectionStart], [' worldhello', 6], 'after paste');
        eq(log, ['paste:t:', 'input:t:insertFromPaste'], 'paste events');
        eq(document.execCommand('insertText', false, '!'), true, 'insert');
        eq([t.value, t.selectionStart], [' world!hello', 7], 'after insert');
        document.execCommand('insertText'); eq(t.value, ' world!hello', 'empty insert');
        eq(document.execCommand('delete'), true, 'delete'); eq(t.value, ' worldhello', 'deleted backward');
        eq(document.execCommand('forwardDelete'), true, 'forward'); eq(t.value, ' worldello', 'deleted forward');
        t.setSelectionRange(0, 1); document.execCommand('delete'); eq(t.value, 'worldello', 'deleted selection');
        t.setSelectionRange(0, 0); document.execCommand('delete'); eq(t.value, 'worldello', 'nothing before');
        t.setSelectionRange(9, 9); document.execCommand('forwardDelete'); eq(t.value, 'worldello', 'nothing after');
        eq(document.execCommand('selectAll'), true, 'select all'); eq([t.selectionStart, t.selectionEnd], [0, 9], 'all selected');
        "#,
    );
    assert_eq!(clipboard(&page).as_deref(), Some(" world"));
    page.check(
        r#"
        const t = ids('#t');
        const custom = (event) => { event.preventDefault(); event.clipboardData.setData('text', 'custom'); };
        document.addEventListener('copy', custom);
        eq(document.execCommand('copy'), true, 'custom copy');
        document.removeEventListener('copy', custom);
        const nothing = (event) => event.preventDefault();
        document.addEventListener('cut', nothing); document.addEventListener('paste', nothing);
        eq(document.execCommand('cut'), true, 'cancelled cut'); eq(t.value, 'worldello', 'kept by cancelled cut');
        eq(document.execCommand('paste'), true, 'cancelled paste'); eq(t.value, 'worldello', 'kept by cancelled paste');
        document.removeEventListener('cut', nothing); document.removeEventListener('paste', nothing);
        const ro = ids('#ro'); ro.focus(); ro.select();
        eq(document.execCommand('insertText', false, 'x'), false, 'read-only insert');
        eq(document.execCommand('delete'), false, 'read-only delete');
        eq(document.execCommand('cut'), true, 'read-only cut'); eq(ro.value, 'fixed', 'read-only kept');
        eq(document.execCommand('paste'), true, 'read-only paste'); eq(ro.value, 'fixed', 'read-only paste kept');
        ids('#c').focus();
        eq(document.execCommand('selectAll'), true, 'select all elsewhere');
        eq(getSelection().anchorNode === document.body && String(getSelection()).endsWith('text'), true, 'document selected');
        getSelection().removeAllRanges();
        eq(document.execCommand('copy'), true, 'copy nothing');
        eq(document.execCommand('insertText', false, 'x'), false, 'insert elsewhere');
        eq(document.execCommand('forwardDelete'), false, 'delete elsewhere');
        eq([document.queryCommandSupported('CoPy'), document.queryCommandEnabled('paste'), document.queryCommandEnabled('bold')], [true, true, false], 'supported');
        eq([document.queryCommandState('bold'), document.queryCommandValue('bold')], [false, ''], 'state');
        eq([document.execCommand('bold'), document.execCommand('constructor')], [false, false], 'unknown');
        "#,
    );
    assert_eq!(clipboard(&page).as_deref(), Some("fixed"));
    *page.page.clipboard.borrow_mut() = None;
    page.check(
        r#"
        const t = ids('#t'); t.focus(); t.setSelectionRange(0, 2);
        eq(document.execCommand('paste'), false, 'paste without clipboard');
        eq(document.execCommand('copy'), false, 'copy without clipboard');
        eq(document.execCommand('cut'), false, 'cut without clipboard'); eq(t.value, 'worldello', 'kept');
        "#,
    );
}

#[test]
fn text_selection_follows_the_platform() {
    let mut page = fixture(PAGE);
    let area = node(&mut page, "#a");
    page.check("const a = ids('#a'); eq([a.selectionStart, a.selectionEnd, a.selectionDirection], [7, 7, 'none'], 'default');");
    page.page.selections.borrow_mut().insert(area, (1, 3));
    page.check("const a = ids('#a'); eq([a.selectionStart, a.selectionEnd], [1, 3], 'shown');");
    page.page.selections.borrow_mut().insert(area, (10, 20));
    page.check("const a = ids('#a'); eq([a.selectionStart, a.selectionEnd], [7, 7], 'clamped');");
    page.check("const a = ids('#a'); a.setSelectionRange(2, 4, 'backward'); eq([a.selectionStart, a.selectionEnd, a.selectionDirection], [2, 4, 'backward'], 'set');");
    assert_eq!(page.page.selections.borrow().get(&area), Some(&(2, 4)));
}

#[test]
fn data_transfers_hold_data_by_format() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const data = new DataTransfer();
        eq([data.dropEffect, data.effectAllowed, [...data.files], data.types], ['none', 'none', [], []], 'empty');
        data.setData('Text', 'a'); data.setData('URL', 'u'); data.setData('text/html', 1); data.setDragImage();
        eq(data.types, ['text/plain', 'text/uri-list', 'text/html'], 'types');
        eq([data.getData('text/plain'), data.getData('url'), data.getData('text/html'), data.getData('missing')], ['a', 'u', '1', ''], 'data');
        data.clearData('URL'); eq(data.types.length, 2, 'cleared one'); data.clearData(); eq(data.types, [], 'cleared');
        const event = new ClipboardEvent('copy', { clipboardData: data }); eq(event.clipboardData, data, 'event data');
        "#,
    );
}

#[test]
fn copy_and_cut_preserve_separate_empty_and_html_only_formats() {
    let mut page = fixture_with(
        "<div id=h contenteditable><b>rich😀</b> text</div>",
        FakePage {
            clipboard: RefCell::new(Some("old".into())),
            ..FakePage::default()
        },
    );
    page.check(
        r#"
        const h=ids('#h');h.focus();const r=new Range();r.selectNodeContents(h);
        getSelection().removeAllRanges();getSelection().addRange(r);
        const ignored=e=>e.clipboardData.setData('text/html','ignored');
        document.addEventListener('copy',ignored);
        eq(document.execCommand('copy'),true,'default rich copy');
        document.removeEventListener('copy',ignored);
    "#,
    );
    assert_eq!(clipboard(&page).as_deref(), Some("rich😀 text"));
    assert_eq!(
        page.page.clipboard_html.borrow().as_deref(),
        Some("<b>rich😀</b> text")
    );
    page.check(r#"
        globalThis.saved=null;
        const rich=e=>{saved=e.clipboardData;eq(saved.types,[],'empty write store');
          saved.setData('text/html','<strong>only HTML</strong>');e.preventDefault();};
        document.addEventListener('cut',rich);
        eq(document.execCommand('cut'),true,'html only cut');
        document.removeEventListener('cut',rich);
        eq(ids('#h').innerHTML,'<b>rich😀</b> text','cancelled cut kept');
        eq([saved.types,saved.getData('text/html'),saved.items.length],[[],'',0],'disconnected cut');
    "#);
    assert_eq!(clipboard(&page), None);
    assert_eq!(
        page.page.clipboard_html.borrow().as_deref(),
        Some("<strong>only HTML</strong>")
    );
    page.check(r#"
        const empty=e=>{e.clipboardData.setData('text/plain','');e.clipboardData.setData('text/html','');e.preventDefault();};
        document.addEventListener('copy',empty);
        eq(document.execCommand('copy'),true,'explicit empty formats');
        document.removeEventListener('copy',empty);
        const nothing=e=>e.preventDefault();document.addEventListener('copy',nothing);
        eq(document.execCommand('copy'),true,'cancelled without data');document.removeEventListener('copy',nothing);
    "#);
    assert_eq!(clipboard(&page).as_deref(), Some(""));
    assert_eq!(page.page.clipboard_html.borrow().as_deref(), Some(""));
    *page.page.clipboard.borrow_mut() = None;
    *page.page.clipboard_html.borrow_mut() = None;
    page.check("eq(document.execCommand('copy'),false,'unavailable write');");
    assert!(page.printed().is_empty());
}

#[test]
fn paste_formats_are_readonly_then_disconnected_and_reach_beforeinput() {
    let mut page = fixture_with(
        "<div id=h contenteditable>old</div><input id=t value=control>",
        FakePage {
            clipboard: RefCell::new(Some("plain😀".into())),
            clipboard_html: RefCell::new(Some("<b>rich😀</b>".into())),
            ..FakePage::default()
        },
    );
    page.check(r#"
        const h=ids('#h');h.focus();getSelection().collapse(h.firstChild,3);
        globalThis.saved=null;globalThis.later=null;
        globalThis.readPaste=e=>{saved=e.clipboardData;
          eq(saved.types,['text/plain','text/html'],'paste types');
          eq([saved.getData('text/plain'),saved.getData('text/html')],['plain😀','<b>rich😀</b>'],'paste values');
          saved.setData('text/plain','changed');saved.clearData();saved.items.clear();
          eq(saved.items.add('new','text/plain'),null,'readonly add');
          let error='';try{saved.items.remove(0);}catch(e){error=e.name;}
          eq(error,'InvalidStateError','readonly remove');
          eq(saved.getData('text/plain'),'plain😀','readonly value');
          saved.items[1].getAsString(value=>later=value);
        };
        document.addEventListener('paste',readPaste);
        globalThis.beforePaste=e=>{if(e.inputType==='insertFromPaste') {
          eq(e.dataTransfer.getData('text/html'),'<b>rich😀</b>','beforeinput formats');e.preventDefault();
        }};h.addEventListener('beforeinput',beforePaste);
    "#);
    page.check("eq(web.command('paste'),true,'native paste');");
    page.check(r#"
        eq(ids('#h').textContent,'old','cancelled beforeinput');
        eq([saved.types,saved.getData('text/plain'),saved.items.length],[[],'',0],'disconnected paste');
        document.removeEventListener('paste',readPaste);ids('#h').removeEventListener('beforeinput',beforePaste);
        ids('#t').focus();ids('#t').select();
        eq(document.execCommand('paste'),true,'control paste');eq(ids('#t').value,'plain😀','control uses plain');
    "#);
    page.settle();
    page.check("eq(later,'<b>rich😀</b>','scheduled snapshot');");
    assert_eq!(clipboard(&page).as_deref(), Some("plain😀"));
    assert_eq!(
        page.page.clipboard_html.borrow().as_deref(),
        Some("<b>rich😀</b>")
    );
    *page.page.clipboard.borrow_mut() = None;
    page.check(r#"
        const html=e=>{eq(e.clipboardData.types,['text/html'],'html only');e.preventDefault();};
        document.addEventListener('paste',html);eq(document.execCommand('paste'),true,'html only paste');
        document.removeEventListener('paste',html);
    "#);
    *page.page.clipboard_html.borrow_mut() = None;
    page.check("eq(document.execCommand('paste'),false,'unavailable snapshot');");
    assert!(page.printed().is_empty());
}
