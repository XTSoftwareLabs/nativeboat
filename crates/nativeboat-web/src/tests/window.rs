use super::*;

#[test]
fn fragments_scroll_to_ids_names_and_top() {
    let mut page = fixture(
        "<a id=link href='#destination'>go</a><div id=destination></div><a name=legacy></a>",
    );
    page.check(
        r##"
        document.getElementById('link').click();
        location.hash='#legacy';
        location.hash='#missing';
        location.hash='#top';
        location.hash='';
        const target=document.getElementById('destination');
        target.scrollIntoView(false); target.scrollIntoView({block:'center',inline:'end'});
        target.scrollIntoViewIfNeeded();
    "##,
    );
    assert!(
        page.page
            .calls
            .borrow()
            .iter()
            .filter(|call| call.starts_with("scroll "))
            .count()
            >= 5
    );
    for (name, expected) in [
        ("start", ScrollAlign::Start),
        ("center", ScrollAlign::Center),
        ("end", ScrollAlign::End),
        ("nearest", ScrollAlign::Nearest),
        ("invalid", ScrollAlign::Nearest),
    ] {
        assert_eq!(ScrollAlign::parse(name), expected);
    }
}

#[test]
fn urls_and_search_params() {
    let mut page = fixture("<p>x</p>");
    page.check(
        r#"
        const url = new URL('HTTPS://User:Pw@Example.COM:443/a/./b/../c d?x=1&y=2#frag');
        eq([url.protocol, url.username, url.password, url.host, url.hostname, url.port, url.pathname, url.search, url.hash],
           ['https:', 'User', 'Pw', 'example.com', 'example.com', '', '/a/c%20d', '?x=1&y=2', '#frag'], 'parts');
        eq(url.href, 'https://User:Pw@example.com/a/c%20d?x=1&y=2#frag', 'href'); eq(url.origin, 'https://example.com', 'origin'); eq(String(url), url.href, 'string'); eq(url.toJSON(), url.href, 'json');
        eq(new URL('http://h:8080/').port, '8080', 'port'); eq(new URL('http://h').pathname, '/', 'root path'); eq(new URL('ws://h/x').origin, 'ws://h', 'ws');
        const base = 'http://h/dir/page.html?q#f';
        eq(new URL('other', base).href, 'http://h/dir/other', 'relative'); eq(new URL('../up', base).href, 'http://h/up', 'parent'); eq(new URL('/abs', base).href, 'http://h/abs', 'absolute');
        eq(new URL('?n', base).href, 'http://h/dir/page.html?n', 'query'); eq(new URL('#g', base).href, 'http://h/dir/page.html?q#g', 'hash'); eq(new URL('', base).href, 'http://h/dir/page.html?q', 'empty');
        eq(new URL('//other/x', base).href, 'http://other/x', 'protocol relative'); eq(new URL('\\\\other\\x', base).href, 'http://other/x', 'backslashes');
        eq(new URL('./', base).href, 'http://h/dir/', 'dot'); eq(new URL('a/..', base).href, 'http://h/dir/', 'trailing dots'); eq(new URL('%2e%2e/%2E', base).href, 'http://h/', 'encoded dots');
        eq(new URL('?', base).search, '', 'empty query'); eq(new URL('#', base).hash, '', 'empty hash'); eq(new URL('x?#', base).href, 'http://h/dir/x', 'empty tail');
        eq(new URL('mailto:me@x.org?subject=hi').pathname, 'me@x.org', 'opaque'); eq(new URL('mailto:a').origin, 'null', 'opaque origin');
        eq(new URL('#top', 'about:blank').href, 'about:blank#top', 'opaque hash'); throws(() => new URL('x', 'about:blank'), 'TypeError', 'opaque relative');
        eq(new URL('mailto:a?#').href, 'mailto:a', 'opaque empty tail'); eq(new URL('custom://host:9/p').host, 'host:9', 'non special');
        eq(new URL('file:///C:/app/x.html').pathname, '/C:/app/x.html', 'file'); eq(new URL('file://server/share').hostname, 'server', 'file host');
        eq(new URL('file://C:/x').pathname, '/C:/x', 'drive host'); eq(new URL('file:C:/x').pathname, '/C:/x', 'file no slashes'); eq(new URL('file://server').pathname, '/', 'file empty path');
        eq(new URL('file:///x?#').href, 'file:///x', 'file empty tail'); eq(new URL('file:///x').origin, 'null', 'file origin');
        throws(() => new URL('nope'), 'TypeError', 'invalid'); throws(() => new URL('x', 'bad base'), 'TypeError', 'invalid base'); throws(() => new URL('http://'), 'TypeError', 'no host');
        assert(URL.canParse('http://x') && !URL.canParse('x'), 'can parse');
        url.protocol = 'http'; url.protocol = '1bad'; eq(url.protocol, 'http:', 'protocol'); url.username = 'u'; url.password = 'p'; url.host = 'New:81'; eq(url.host, 'new:81', 'host');
        url.hostname = 'HOST'; url.port = '80'; eq(url.port, '', 'default port'); url.port = '82'; url.pathname = 'p q'; eq(url.pathname, '/p%20q', 'pathname'); url.pathname = '/z';
        url.search = '?a=1'; eq(url.searchParams.get('a'), '1', 'search params'); url.search = ''; eq(url.search, '', 'cleared'); url.hash = 'h'; eq(url.hash, '#h', 'hash'); url.hash = '';
        url.searchParams.append('k', 'v w'); eq(url.search, '?k=v+w', 'params update url'); url.searchParams.delete('k'); eq(url.search, '', 'params cleared');
        url.href = 'http://reset/?r=1'; eq(url.searchParams.get('r'), '1', 'href reparses'); throws(() => { url.href = 'bad'; }, 'TypeError', 'bad href');
        url.username = ''; eq(url.href, 'http://reset/?r=1', 'no credentials'); url.username = 'only'; eq(url.href, 'http://only@reset/?r=1', 'user only');
        const params = new URLSearchParams('?a=1&b=2&a=3&c&d=%zz&e=x%2By+z');
        eq(params.getAll('a'), ['1', '3'], 'get all'); eq(params.get('c'), '', 'no value'); eq(params.get('d'), '%zz', 'bad escape'); eq(params.get('e'), 'x+y z', 'plus');
        eq(params.get('none'), null, 'missing'); assert(params.has('a') && params.has('a', '3') && !params.has('a', '9'), 'has'); eq(params.size, 6, 'size');
        params.set('a', 'x'); eq(params.getAll('a'), ['x'], 'set'); params.set('new', '1'); params.delete('b', 'nope'); params.delete('b', '2'); params.sort();
        eq([...params.keys()], ['a', 'c', 'd', 'e', 'new'], 'sorted'); eq(params.toString(), "a=x&c=&d=%25zz&e=x%2By+z&new=1", 'serialize');
        eq(new URLSearchParams({ x: '!' }).toString(), 'x=%21', 'record'); eq(new URLSearchParams([['y', "'()~"]]).toString(), 'y=%27%28%29%7E', 'pairs');
        eq(new URLSearchParams(params).get('a'), 'x', 'copy'); eq(new URLSearchParams(new Map([['m', 1]])).get('m'), '1', 'iterable');
        let count = 0; params.forEach(() => count++); eq(count, 5, 'forEach'); eq([...params.values()].length + [...params.entries()].length + [...params].length, 15, 'iterators');
        eq(new URLSearchParams().toString(), '', 'empty params'); eq(new URLSearchParams('a=1&&b').size, 2, 'empty parts');
        eq(['a', 'b', 'a'].sort().length, 3, 'sort sanity'); const tie = new URLSearchParams('b=1&a=2&b=0'); tie.sort(); eq(tie.toString(), 'a=2&b=1&b=0', 'stable sort');
        assert(URL.createObjectURL(new Blob()).startsWith('blob:'), 'object url'); URL.revokeObjectURL('x');
        "#,
    );
}

#[test]
fn location_and_history() {
    let mut page = fixture("<p>x</p>");
    page.check(
        r#"
        eq(location.href, 'file:///C:/app/index.html', 'href'); eq(location.origin, 'file://', 'origin'); eq(location.pathname, '/C:/app/index.html', 'pathname');
        eq(String(location), location.href, 'string'); eq(document.URL, location.href, 'document url'); eq(document.documentURI, location.href, 'document uri');
        eq(document.location, location, 'document location'); eq(document.domain, '', 'domain'); eq(document.referrer, '', 'referrer');
        throws(() => new Location(), 'TypeError', 'illegal location'); throws(() => new History(), 'TypeError', 'illegal history');
        const events = []; window.addEventListener('popstate', (event) => events.push('pop:' + JSON.stringify(event.state))); window.addEventListener('hashchange', () => events.push('hash'));
        eq(history.length, 1, 'length'); eq(history.state, null, 'state'); eq(history.scrollRestoration, 'auto', 'restoration'); history.scrollRestoration = 'manual';
        history.pushState({ page: 1 }, '', '/C:/app/one'); eq(location.pathname, '/C:/app/one', 'pushed'); eq(history.state, { page: 1 }, 'state'); eq(history.length, 2, 'two');
        history.pushState({ page: 2 }, '', 'two#x'); history.replaceState({ page: 3 }, ''); eq(history.state, { page: 3 }, 'replaced'); eq(location.hash, '#x', 'kept url');
        history.pushState(null, ''); eq(history.length, 4, 'same url push');
        history.back(); history.go(0); history.go(-9);
        globalThis.historyEvents = events;
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(historyEvents, ['pop:{"page":3}'], 'back is same document'); eq(location.hash, '#x', 'back url');
        history.go(-1); globalThis.wentBack = true;
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(historyEvents.slice(1), ['pop:{"page":1}', 'hash'], 'back across hash'); history.forward();
        location.hash = 'h'; eq(location.hash, '#h', 'set hash'); location.replace('#r'); eq(location.hash, '#r', 'replace hash'); location.replace('#r');
        const navigations = []; web.onnavigate = (url) => navigations.push(url); web.onreload = () => navigations.push('reload');
        location.href = 'http://x/'; location.assign('page2.html'); location.replace('page3.html'); location.search = 'q=1'; location.reload();
        eq(navigations, ['http://x/', 'file:///C:/app/page2.html', 'file:///C:/app/page3.html', 'file:///C:/app/one?q=1#r', 'reload'], 'navigations');
        delete web.onreload; location.reload();
        "#,
    );
}

#[test]
fn storage_and_platform_objects() {
    let saved = FakePage::default();
    *saved.storage.borrow_mut() = Some(r#"[["kept","1"]]"#.to_owned());
    let mut page = fixture_with("<p id=p>x</p>", saved);
    page.check(
        r#"
        eq(localStorage.getItem('kept'), '1', 'loaded'); eq(localStorage.kept, '1', 'property'); eq(localStorage.length, 1, 'length');
        localStorage.setItem('a', 2); localStorage.b = 'x'; eq(localStorage.key(1), 'a', 'key'); eq(localStorage.key(9), null, 'no key'); assert('b' in localStorage && !('z' in localStorage), 'has');
        eq(Object.keys(localStorage), ['kept', 'a', 'b'], 'keys'); delete localStorage.b; localStorage.removeItem('a'); eq(localStorage.getItem('a'), null, 'removed');
        eq(Object.getOwnPropertyDescriptor(localStorage, 'nope'), undefined, 'no descriptor'); eq(Object.getOwnPropertyDescriptor(localStorage, 'kept').value, '1', 'descriptor');
        localStorage[Symbol.toStringTag] = 'S'; delete localStorage[Symbol.iterator]; assert('getItem' in localStorage, 'method in');
        sessionStorage.setItem('s', 1); sessionStorage.clear(); eq(sessionStorage.length, 0, 'session'); throws(() => new Storage(), 'TypeError', 'illegal storage');
        eq(navigator.platform, 'Win32', 'platform'); assert(navigator.userAgent.includes(' Chrome/') && navigator.appVersion === navigator.userAgent.slice(8) && navigator.hardwareConcurrency >= 1 && navigator.userAgentData.brands[1].brand === 'Chromium' && navigator.onLine, 'navigator'); throws(() => new Navigator(), 'TypeError', 'illegal navigator');
        assert(!navigator.sendBeacon() && !navigator.vibrate() && !navigator.javaEnabled(), 'navigator methods');
        assert(navigator.languages === navigator.languages && Object.isFrozen(navigator.languages) && navigator.languages[0] === navigator.language, 'navigator languages');
        navigator.clipboard.writeText('x').catch((error) => { globalThis.clipboard = error.name; }); navigator.clipboard.readText().catch(() => {});
        eq([screen.width, screen.height, screen.availWidth, screen.availHeight], [800, 600, 800, 600], 'screen');
        eq([innerWidth, innerHeight, outerWidth, outerHeight, devicePixelRatio], [800, 600, 800, 600, 1.25], 'viewport');
        eq([scrollX, scrollY, pageXOffset, pageYOffset, screenX, screenY, screenLeft, screenTop, window.event], [0, 0, 0, 0, 0, 0, 0, 0, undefined], 'positions');
        assert(window === globalThis && self === window && top === window && parent === window && frames === window, 'window aliases');
        assert(window instanceof Window && window instanceof EventTarget && !closed && length === 0, 'window class'); throws(() => new Window(), 'TypeError', 'illegal window');
        eq(origin, 'null', 'origin'); assert(isSecureContext, 'secure');
        eq(p, ids('#p'), 'named access'); assert('p' in window && !('nothing' in window), 'named has'); eq(typeof window.nothing, 'undefined', 'no element');
        eq(window[Symbol.iterator], undefined, 'symbol lookup'); globalThis.p2 = 1; eq(p2, 1, 'real global');
        const start = performance.now(); assert(start >= 0 && performance.timeOrigin > 0, 'performance');
        performance.mark('a'); performance.mark('b', { startTime: 5, detail: 1 }); eq(performance.measure('m', 'a', 'b').duration, 5 - performance.getEntriesByName('a')[0].startTime, 'measure');
        eq(performance.measure('n').startTime, 0, 'measure default'); eq(performance.measure('o', 'missing').startTime, 0, 'missing mark');
        eq(performance.getEntriesByType('mark').length, 2, 'marks'); eq(performance.getEntriesByName('m', 'mark').length, 0, 'typed name'); eq(performance.getEntries().length, 5, 'entries');
        performance.clearMarks('a'); performance.clearMeasures('m'); eq(performance.getEntries().length, 3, 'cleared named'); performance.clearMarks(); performance.clearMeasures();
        eq(performance.getEntries().length, 0, 'cleared'); eq(performance.toJSON().timeOrigin, performance.timeOrigin, 'json');
        eq(document.cookie, '', 'cookie'); document.cookie = 'a=1; path=/'; document.cookie = 'b=2'; document.cookie = 'a=3'; eq(document.cookie, 'b=2; a=3', 'cookies');
        document.cookie = 'b=; max-age=0'; eq(document.cookie, 'a=3', 'deleted cookie');
        eq([document.characterSet, document.charset, document.contentType, document.compatMode, document.visibilityState, document.hidden], ['UTF-8', 'UTF-8', 'text/html', 'CSS1Compat', 'visible', false], 'document props');
        assert(document.hasFocus() && document.defaultView === window && document.implementation.hasFeature(), 'document');
        assert(document.implementation.createHTMLDocument() !== document, 'detached html document');
        document.title = 'One'; eq(document.title, 'One', 'title'); eq(document.querySelectorAll('title').length, 1, 'title element'); document.title = ' Two  words '; eq(document.title, 'Two words', 'trimmed');
        eq(document.forms.length + document.images.length + document.links.length + document.scripts.length, 0, 'collections');
        eq(document.scrollingElement, document.documentElement, 'scrolling'); eq(document.getElementsByName('x').length, 0, 'by name');
        assert(!document.execCommand('bold') && !document.queryCommandSupported('bold'), 'commands');
        eq(document.importNode(ids('#p')).id, 'p', 'import'); const adopted = document.adoptNode(document.createElement('i')); eq(adopted.parentNode, null, 'adopt');
        document.body.appendChild(adopted); document.adoptNode(adopted); eq(adopted.parentNode, null, 'adopt detaches');
        "#,
    );
    page.settle();
    page.check("eq(clipboard, 'NotAllowedError', 'clipboard');");
    assert_eq!(
        page.page.storage.borrow().as_deref(),
        Some(r#"[["kept","1"]]"#)
    );
    let mut broken = fixture_with("<p>x</p>", {
        let page = FakePage::default();
        *page.storage.borrow_mut() = Some("not json".to_owned());
        page
    });
    broken.check("eq(localStorage.length, 0, 'bad saved storage');");
    let mut object = fixture_with("<p>x</p>", {
        let page = FakePage::default();
        *page.storage.borrow_mut() = Some("{}".to_owned());
        page
    });
    object.check("eq(localStorage.length, 0, 'non-array storage');");
}

#[test]
fn encoding_and_cloning() {
    let mut page = fixture("<p id=p>x</p>");
    page.check(
        r#"
        const encoder = new TextEncoder(); eq(encoder.encoding, 'utf-8', 'encoding');
        eq([...encoder.encode('aé€😀')], [97, 195, 169, 226, 130, 172, 240, 159, 152, 128], 'utf-8'); eq([...encoder.encode('\ud800')], [239, 191, 189], 'lone surrogate');
        eq(encoder.encode().length, 0, 'empty'); const into = new Uint8Array(2); eq(encoder.encodeInto('abc', into), { read: 3, written: 2 }, 'encode into');
        const decoder = new TextDecoder(); eq(decoder.decode(encoder.encode('aé€😀')), 'aé€😀', 'round trip'); eq(decoder.decode(new Uint8Array([0xef, 0xbb, 0xbf, 65]).buffer), 'A', 'bom');
        eq(new TextDecoder('utf8', { ignoreBOM: true }).decode(new Uint8Array([0xef, 0xbb, 0xbf])), '\ufeff', 'keep bom'); eq(decoder.decode(), '', 'no input');
        eq(decoder.decode(new Uint8Array([0xff, 65, 0xe2, 0x82])), '\ufffdA\ufffd\ufffd', 'invalid'); eq(decoder.decode(new Uint8Array([0xf4, 0x90, 0x80, 0x80])), '\ufffd\ufffd\ufffd\ufffd', 'too large');
        eq([decoder.encoding, decoder.fatal, decoder.ignoreBOM], ['utf-8', false, false], 'decoder props');
        throws(() => new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff])), 'TypeError', 'fatal'); throws(() => new TextDecoder('latin1'), 'RangeError', 'label');
        eq(btoa('hello'), 'aGVsbG8=', 'btoa'); eq(btoa('hi'), 'aGk=', 'btoa two'); eq(btoa(''), '', 'btoa empty'); eq(atob('aGVsbG8='), 'hello', 'atob'); eq(atob('aGk'), 'hi', 'atob unpadded');
        eq(atob(' aG k= '), 'hi', 'atob spaces'); throws(() => btoa('€'), 'InvalidCharacterError', 'btoa range'); throws(() => atob('a'), 'InvalidCharacterError', 'atob length'); throws(() => atob('**'), 'InvalidCharacterError', 'atob chars');
        const source = { date: new Date(5), re: /x/gi, map: new Map([[1, { a: 1 }]]), set: new Set([1]), bytes: new Uint8Array([1, 2]), buffer: new ArrayBuffer(2), list: [1, { b: 2 }], error: new TypeError('t'), plain: new Error('e'), odd: Object.assign(new Error('o'), { name: 'Weird' }) };
        source.self = source;
        const copy = structuredClone(source); assert(copy !== source && copy.self === copy, 'cycle'); eq(copy.date.getTime(), 5, 'date'); eq(copy.re.flags, 'gi', 'regexp');
        eq(copy.map.get(1), { a: 1 }, 'map'); assert(copy.set.has(1) && copy.bytes[1] === 2 && copy.buffer.byteLength === 2, 'collections'); eq(copy.list, [1, { b: 2 }], 'array');
        assert(copy.error instanceof TypeError && copy.error.message === 't' && copy.plain instanceof Error && copy.odd instanceof Error, 'errors');
        eq(structuredClone(5), 5, 'primitive'); eq(structuredClone(null), null, 'null');
        throws(() => structuredClone(() => {}), 'DataCloneError', 'function'); throws(() => structuredClone(function named() {}), 'DataCloneError', 'named function');
        throws(() => structuredClone(Symbol('s')), 'DataCloneError', 'symbol'); 
        const random = crypto.getRandomValues(new Uint32Array(4)); eq(random.length, 4, 'random'); throws(() => crypto.getRandomValues([]), 'TypeError', 'not typed');
        assert(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(crypto.randomUUID()), 'uuid'); eq(crypto.subtle, undefined, 'subtle');
        const blob = new Blob(['ab', new Uint8Array([99]), new Uint8Array([100]).buffer, new Blob(['e'])], { type: 'Text/Plain' });
        eq([blob.size, blob.type], [5, 'text/plain'], 'blob'); blob.text().then((text) => { globalThis.blobText = text; }); blob.slice(1, 3).arrayBuffer().then((buffer) => { globalThis.sliced = buffer.byteLength; });
        blob.bytes().then((bytes) => { globalThis.blobBytes = bytes.length; }); eq(new Blob().size, 0, 'empty blob');
        const file = new File(['x'], 'a.txt', { lastModified: 7 }); eq([file.name, file.lastModified, file.size], ['a.txt', 7, 1], 'file'); assert(new File([], 'b').lastModified > 0, 'file time');
        const reads = []; for (const method of ['readAsText', 'readAsArrayBuffer', 'readAsDataURL']) { const reader = new FileReader(); reader.onload = () => reads.push(typeof reader.result === 'string' ? reader.result : reader.result.byteLength); reader[method](file); }
        const typed = new FileReader(); typed.addEventListener('loadend', () => reads.push(typed.readyState)); typed.readAsDataURL(new Blob(['y'], { type: 'text/plain' })); typed.abort();
        globalThis.reads = reads;
        const channel = new MessageChannel(); channel.port2.onmessage = (event) => { globalThis.message = event.data; }; channel.port1.postMessage({ hello: 1 }); channel.port1.start(); channel.port1.close();
        throws(() => new MessagePort(), 'TypeError', 'illegal port');
        window.addEventListener('message', (event) => { globalThis.posted = event.data; }); postMessage('posted');
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq([blobText, sliced, blobBytes], ['abcde', 2, 5], 'blob reads'); eq(reads, ['x', 1, 'data:application/octet-stream;base64,eA==', 2], 'file reads');
        eq(message, { hello: 1 }, 'message channel'); eq(posted, 'posted', 'post message');
        "#,
    );
}

/// A page's message ports queue messages until they start, move with
/// messages, and tell their peer when they close.
#[test]
fn message_ports_start_move_and_close() {
    let mut page = fixture("<p id=p>x</p>");
    page.check(
        r#"
        const log = globalThis.portLog = [];
        const first = new MessageChannel();
        first.port2.addEventListener('message', (event) => log.push(`listener ${event.data}`));
        first.port1.postMessage('queued');
        const second = new MessageChannel();
        first.port1.postMessage('carry', [second.port2]);
        first.port1.postMessage('options', { transfer: [] });
        throws(() => first.port1.postMessage('self', [first.port1]), 'DataCloneError', 'own port');
        throws(() => first.port1.postMessage('twice', [second.port1, second.port1]), 'DataCloneError', 'duplicate');
        throws(() => first.port1.postMessage('value', [{}]), 'DataCloneError', 'not a port');
        setTimeout(() => { log.push('before start'); first.port2.start(); }, 0);
        first.port2.addEventListener('message', (event) => {
          if (event.data !== 'carry') return;
          const [carried] = event.ports;
          log.push(`carried ${carried === second.port2} ${Object.isFrozen(event.ports)}`);
          carried.onmessage = (message) => log.push(`second ${message.data}`);
          second.port1.postMessage('through');
        });
        const third = new MessageChannel();
        third.port2.addEventListener('close', () => log.push('closed'));
        third.port2.start();
        third.port1.close();
        third.port1.postMessage('after close');
        window.addEventListener('message', (event) => { if (event.data === 'window') log.push(`window ${event.ports.length} ${event.source === window}`); });
        const fourth = new MessageChannel();
        postMessage('window', '*', [fourth.port1]);
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(portLog, ['before start', 'closed', 'window 1 true', 'listener queued', 'listener carry', 'carried true true', 'listener options', 'second through'], 'port log');
        "#,
    );
}

#[test]
fn geometry_focus_and_observers() {
    let mut page = fixture(
        "<div id=a tabindex=1><p id=b>x</p><input id=i><a id=link href=x>l</a><span id=s>s</span></div><img id=img src=logo.png>",
    );
    page.check(
        r#"
        const elements = [...document.querySelectorAll('*')]; const even = elements.find((element) => element.getBoundingClientRect().width === 100);
        const odd = elements.find((element) => element.getBoundingClientRect().width === 0);
        const rect = even.getBoundingClientRect(); eq([rect.x, rect.y, rect.width, rect.height, rect.top, rect.right, rect.bottom, rect.left], [10, 20, 100, 49.5, 20, 110, 69.5, 10], 'rect');
        eq(rect.toJSON().right, 110, 'json'); eq(DOMRect.fromRect({ x: 1, width: 2 }).right, 3, 'from rect'); eq(DOMRect.fromRect().width, 0, 'empty from rect');
        eq(new DOMRectReadOnly(5, 5, -2, -2).left, 3, 'negative rect');
        eq(even.getClientRects().length, 1, 'client rects'); eq(odd.getClientRects().length, 0, 'no client rects'); eq(even.getClientRects().item(0).width, 100, 'rect item');
        eq(even.getClientRects().item(3), null, 'no rect item'); eq([...even.getClientRects()].length, 1, 'iterate rects'); throws(() => new DOMRectList(), 'TypeError', 'rect list');
        // Boxes lie at fractions of pixels; these sizes are whole ones.
        eq([even.clientWidth, even.clientHeight, even.clientLeft, even.clientTop], [98, 46, 1, 2], 'client'); eq([odd.clientWidth, odd.clientHeight, odd.clientLeft, odd.clientTop], [0, 0, 0, 0], 'no client');
        eq([even.scrollWidth, even.scrollHeight, odd.scrollWidth, odd.scrollHeight], [300, 400, 0, 0], 'scroll size');
        even.scrollTop = 30; even.scrollLeft = 'x'; eq([even.scrollLeft, even.scrollTop], [0, 30], 'scroll'); even.scrollTo(5, 6); eq([even.scrollLeft, even.scrollTop], [5, 6], 'scroll to');
        even.scrollBy(1, 1); eq(even.scrollTop, 7, 'scroll by'); even.scrollTo({ top: 2 }); eq([even.scrollLeft, even.scrollTop], [6, 2], 'scroll options');
        even.scrollBy({ left: 1 }); eq(even.scrollLeft, 7, 'scroll by options'); even.scroll(); even.scrollTo(-5, -5); eq(even.scrollTop, 0, 'clamped');
        window.scrollTo(0, 50); eq(window.scrollY, 50, 'window scroll'); window.scrollBy(0, 5); window.scroll(0, 1); eq(document.documentElement.scrollTop, 1, 'document scroll');
        even.scrollIntoView(); even.scrollIntoViewIfNeeded();
        const b = ids('#b'); eq(b.offsetParent, ids('#a'), 'offset parent'); eq(document.body.offsetParent, null, 'body offset parent');
        eq([b.offsetWidth, b.offsetHeight].join() === '100,50' || [b.offsetWidth, b.offsetHeight].join() === '0,0', true, 'offset size');
        eq(typeof b.offsetTop + typeof b.offsetLeft, 'numbernumber', 'offsets');
        // Offsets are layout positions from the parent's padding edge: the
        // fake page's boxes stay put, so scrolling the parent adds its scroll.
        const parentOf = ids('#a'); parentOf.scrollTo(0, 0); const unscrolled = [b.offsetLeft, b.offsetTop]; parentOf.scrollTo(3, 4);
        eq([b.offsetLeft - unscrolled[0], b.offsetTop - unscrolled[1]], [parentOf.scrollLeft, parentOf.scrollTop], 'offsets ignore scrolling'); parentOf.scrollTo(0, 0);
        eq(b.getClientRects().length === 0 || b.offsetTop === b.getBoundingClientRect().top - parentOf.getBoundingClientRect().top - parentOf.clientTop, true, 'offsets from the padding edge'); const loose = [document.createElement('p'), document.createElement('p')].find((p) => p.getClientRects().length === 0); eq([loose.offsetTop, loose.offsetLeft, loose.offsetParent], [0, 0, null], 'detached offsets');
        eq(document.elementFromPoint(1, 1), document.documentElement.firstChild === null ? null : document.querySelectorAll('*')[0].ownerDocument.elementFromPoint(1, 1), 'point');
        eq(document.elementsFromPoint(1, 1).length >= 1, true, 'points');
        eq(ids('#a').tabIndex, 1, 'tab index'); eq(ids('#i').tabIndex, 0, 'focusable index'); eq(ids('#s').tabIndex, -1, 'not focusable'); ids('#s').tabIndex = 2; eq(ids('#s').tabIndex, 2, 'set tab index');
        const events = []; for (const type of ['focus', 'blur', 'focusin', 'focusout']) document.addEventListener(type, (event) => events.push(type + ':' + event.target.id + ':' + (event.relatedTarget ? event.relatedTarget.id : '')), true);
        ids('#i').focus(); ids('#link').focus({ preventScroll: true }); ids('#b').focus(); eq(document.activeElement.id, 'link', 'unfocusable ignored');
        ids('#i').blur(); ids('#link').blur(); eq(document.activeElement, document.body, 'blurred');
        eq(events, ['focus:i:', 'focusin:i:', 'blur:i:link', 'focusout:i:link', 'focus:link:i', 'focusin:link:i', 'blur:link:', 'focusout:link:'], 'focus events');
        const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.focus(); svg.blur();
        const img = ids('#img'); eq([img.naturalWidth, img.naturalHeight], [64, 32], 'natural'); eq(img.width === 100 || img.width === 64, true, 'width'); img.width = 10; img.height = 5; eq([img.width, img.height], [10, 5], 'set size');
        assert(img.complete && img.currentSrc.endsWith('logo.png'), 'complete'); img.decode().then(() => { globalThis.decoded = true; });
        const loads = []; const made = new Image(4, 2); eq([made.width, made.height], [4, 2], 'image constructor'); made.onload = () => loads.push('load'); made.onerror = () => loads.push('error');
        made.src = 'logo.png'; const missing = new Image(); missing.addEventListener('error', () => loads.push('missing')); missing.setAttribute('src', 'none.png'); missing.setAttribute('alt', 'a');
        const unset = new Image(); eq([unset.naturalWidth, unset.width], [0, 0], 'no source'); globalThis.loads = loads;
        const records = []; const observer = new MutationObserver((list, self) => { records.push(...list.map((record) => record.type + ':' + (record.attributeName || '') + ':' + (record.oldValue || '') + ':' + record.addedNodes.length)); assert(self === observer, 'self'); });
        observer.observe(ids('#a'), { childList: true, attributeOldValue: true, subtree: true, characterDataOldValue: true, attributeFilter: ['title'] });
        ids('#b').title = 'one'; ids('#b').title = 'two'; ids('#b').className = 'ignored'; ids('#a').appendChild(document.createElement('em')); ids('#b').firstChild.data = 'changed';
        eq(observer.takeRecords().length, 4, 'take records'); ids('#b').title = 'three'; globalThis.mutationRecords = records; globalThis.mutations = observer;
        throws(() => observer.observe(ids('#a'), {}), 'TypeError', 'no options'); throws(() => new MutationObserver(), 'TypeError', 'no callback');
        const failing = new MutationObserver(() => { throw new Error('observer failed'); }); failing.observe(document.body, { attributes: true }); document.body.title = 'x';
        const quiet = new MutationObserver(() => {}); quiet.observe(document.body, { attributes: true }); quiet.takeRecords();
        const resized = []; const resize = new ResizeObserver((entries) => resized.push(entries.length)); resize.observe(even); resize.observe(odd); resize.unobserve(odd);
        globalThis.resizes = resized; globalThis.resizer = resize;
        const seen = []; const intersection = new IntersectionObserver((entries) => seen.push(entries[0].isIntersecting), { threshold: 0.5 }); intersection.observe(even);
        eq(intersection.takeRecords(), [], 'no records'); eq(intersection.thresholds, [0.5], 'thresholds'); globalThis.intersections = seen; globalThis.intersection = intersection;
        const gone = new IntersectionObserver(() => seen.push('unobserved')); gone.observe(even); gone.unobserve(even); gone.observe(odd); gone.disconnect();
        eq(new IntersectionObserver(() => {}).rootMargin, '0px 0px 0px 0px', 'default margin'); eq(new IntersectionObserver(() => {}, { rootMargin: '10px 5%' }).rootMargin, '10px 5% 10px 5%', 'margins');
        eq(new IntersectionObserver(() => {}, { threshold: [1, 0, 0.5] }).thresholds, [0, 0.5, 1], 'sorted thresholds'); eq(new IntersectionObserver(() => {}, { root: document }).root, document, 'document root');
        throws(() => new IntersectionObserver(() => {}, { rootMargin: '1em' }), 'SyntaxError', 'margin units'); throws(() => new IntersectionObserver(() => {}, { rootMargin: '1px 2px 3px 4px 5px' }), 'SyntaxError', 'extra margins');
        throws(() => new IntersectionObserver(() => {}, { threshold: 2 }), 'RangeError', 'threshold range'); throws(() => new IntersectionObserver(() => {}, { root: {} }), 'TypeError', 'root type');
        throws(() => new IntersectionObserver(), 'TypeError', 'no intersection callback'); throws(() => intersection.observe({}), 'TypeError', 'target type');
        eq(new IntersectionObserverEntry({ isIntersecting: true }).intersectionRatio, 0, 'entry');
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(mutationRecords, ['attributes:title:two:0'], 'delivered'); mutations.disconnect(); ids('#b').title = 'four';
        assert(decoded, 'decoded'); eq(loads.sort(), ['load', 'missing'], 'image events');
        web.frame(); eq(resizes, [1], 'resize observed'); web.resize(); web.frame(); eq(resizes, [1], 'unchanged size');
        assert(!web.hasFrames(), 'no check due'); ids('#b').title = 'five'; assert(web.hasFrames(), 'changes are checked'); web.frame(); eq(resizes, [1], 'changed nothing');
        resizer.disconnect();
        const failing = new ResizeObserver(() => { throw new Error('resize failed'); }); failing.observe(ids('#a')); web.frame();
        globalThis.failingResize = failing;
        "#,
    );
    page.settle();
    // Intersections arrive in a task after the frame.
    page.check(
        r#"
        eq(intersections, [true], 'intersection'); intersection.disconnect(); failingResize.disconnect();
        ids('#b').title = 'six'; assert(!web.hasFrames(), 'nothing observed');
        "#,
    );
    let printed = page.printed();
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Uncaught Error: observer failed"))
    );
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Uncaught Error: resize failed"))
    );
    let calls = page.page.calls.borrow().clone();
    assert!(calls.iter().any(|call| call.starts_with("scroll ")));
    assert!(calls.iter().any(|call| call == "focus None"));
}

#[test]
fn globals_and_hooks() {
    let mut page = fixture("<p id=p>x</p>");
    page.check(
        r#"
        const calls = []; alert('no hook'); eq(confirm('no hook'), false, 'confirm default'); eq(prompt('no hook'), null, 'prompt default'); close();
        web.onalert = (message) => calls.push('alert ' + message); web.onconfirm = (message) => message === 'yes'; web.onprompt = (message, value) => message + value; web.onclose = () => calls.push('close');
        alert('hi'); alert(); assert(confirm('yes') && !confirm(), 'confirm'); eq(prompt('a', 'b'), 'ab', 'prompt'); eq(prompt(), '', 'prompt empty'); close();
        const opened = []; web.onopen = (url, target) => opened.push(target); eq(open('x.html'), null, 'open'); open('y', ''); open(); eq(opened, ['_blank', '_blank', '_blank'], 'open targets');
        eq(calls, ['alert hi', 'alert ', 'close'], 'hooks'); print(); focus(); blur(); stop(); moveTo(); resizeTo();
        customElements.define('x-y', class extends HTMLElement {}); assert(customElements.get('x-y') !== undefined, 'custom element');
        throws(() => customElements.define('x-y', class {}), 'NotSupportedError', 'redefine'); customElements.upgrade(document); customElements.whenDefined('x-y').then((c) => { globalThis.defined = !!c; });
        assert(CSS.supports('display', 'grid'), 'css supports'); eq(CSS.escape('a.b'), 'a\\.b', 'escape'); eq(CSS.escape('1a'), '\\31 a', 'escape digit');
        assert(document.fonts.check('1em x') && document.fonts.status === 'loaded', 'fonts'); document.fonts.add(); document.fonts.delete(); document.fonts.clear(); document.fonts.forEach(); document.fonts.has();
        document.fonts.load().then(() => {}); document.fonts.ready.then(() => { globalThis.fontsReady = true; }); new FontFace('x', 'url(x)').load().then((face) => { globalThis.face = face.family; });
        eq(getSelection(), document.getSelection(), 'selection'); const selection = getSelection(); const range = document.createRange(); selection.addRange(range); selection.removeRange(range); selection.removeAllRanges(); selection.collapse(null);
        eq(selection.toString(), '', 'selection text'); range.setStart(ids('#p'), 0); range.setEnd(ids('#p'), 1); range.collapse(); range.detach();
        eq(range.getBoundingClientRect().width, 0, 'range rect'); eq(range.getClientRects().length, 0, 'caret between elements');
        range.selectNode(ids('#p')); eq(range.endOffset, 1, 'select node'); eq(range.cloneRange().endOffset, 1, 'clone range'); eq(document.createRange().getBoundingClientRect().width, 0, 'document range');
        eq(document.createRange().createContextualFragment('<b>x</b>').firstChild.localName, 'b', 'contextual');
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT); const names = []; while (walker.nextNode()) names.push(walker.currentNode.localName); eq(names, ['p'], 'walker');
        const texts = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, { acceptNode: () => NodeFilter.FILTER_ACCEPT }); eq(texts.nextNode().data, 'x', 'text walker');
        const filtered = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_ALL, (node) => (node.nodeType === 1 ? 1 : 3)); filtered.firstChild(); filtered.nextSibling();
        filtered.currentNode = ids('#p').firstChild; eq(filtered.parentNode().id, 'p', 'parent'); filtered.currentNode = document.documentElement; eq(filtered.parentNode(), null, 'root parent');
        const empty = document.createTreeWalker(ids('#p').firstChild); eq(empty.firstChild(), null, 'no child'); eq(empty.nextSibling(), null, 'no sibling'); eq(empty.nextNode(), null, 'no next');
        reportError(new Error('reported'));
        let errorEvent = null; window.addEventListener('error', (event) => { errorEvent = event.message; event.preventDefault(); }); reportError(new Error('handled')); eq(errorEvent, 'Uncaught Error: handled', 'error event');
        reportError('plain'); eq(errorEvent, 'Uncaught plain', 'plain error event');
        window.addEventListener('unhandledrejection', (event) => { if (event.reason === 'quiet') event.preventDefault(); }); Promise.reject('quiet'); Promise.reject('loud');
        requestIdleCallback((deadline) => { globalThis.idle = deadline.timeRemaining() >= 0 && !deadline.didTimeout; }); cancelIdleCallback(requestIdleCallback(() => { globalThis.idleRan = true; }, { timeout: 100 }));
        eq(Object.prototype.toString.call(document.createElement('div').dataset) !== '', true, 'dataset');
        "#,
    );
    page.settle();
    page.check("assert(defined && fontsReady && face === 'x' && idle && !globalThis.idleRan, 'async globals');");
    let printed = page.printed();
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Uncaught Error: reported"))
    );
    assert!(
        printed
            .iter()
            .any(|line| line == "Uncaught (in promise) 'loud'")
    );
    assert!(
        !printed
            .iter()
            .any(|line| line.contains("quiet") || line.contains("handled"))
    );
}

#[test]
fn xml_http_requests() {
    let mut page = fixture("<p>x</p>");
    page.check(
        r#"
        globalThis.calls = []; globalThis.log = [];
        globalThis.fetch = (url, init) => {
          calls.push([url, init.method, init.body ?? null, JSON.stringify(init.headers), init.credentials].join(' '));
          if (url.endsWith('/fail')) return Promise.reject(new TypeError('fetch failed'));
          if (url.endsWith('/slow')) return new Promise((resolve, reject) => init.signal.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))));
          const body = url.endsWith('/bad') ? 'not json' : '{"a":1}';
          return Promise.resolve({ status: 200, statusText: 'OK', url, headers: new Map([['Content-Type', 'application/json'], ['X-A', '1']]), arrayBuffer: () => Promise.resolve(new TextEncoder().encode(body).buffer) });
        };
        eq([XMLHttpRequest.DONE, new XMLHttpRequest().UNSENT, new XMLHttpRequest().readyState], [4, 0, 0], 'states');
        const request = new XMLHttpRequest(); const states = [];
        request.onreadystatechange = () => states.push(request.readyState); request.addEventListener('loadstart', () => log.push('loadstart'));
        request.onprogress = (event) => log.push('progress ' + event.loaded + ' ' + event.lengthComputable);
        request.onload = () => log.push(['load', request.status, request.statusText, request.responseText, request.response, request.getResponseHeader('x-a'), request.getResponseHeader('none'), JSON.stringify(request.getAllResponseHeaders()), request.responseURL.endsWith('/data')].join('|'));
        request.onloadend = () => log.push('loadend ' + states.join());
        request.open('post', 'data'); request.setRequestHeader('X-Test', 'yes'); request.overrideMimeType('text/plain'); request.withCredentials = true; request.send('body');
        throws(() => request.send(), 'InvalidStateError', 'sent twice'); throws(() => request.setRequestHeader('a', 'b'), 'InvalidStateError', 'header after send');
        const json = new XMLHttpRequest(); json.responseType = 'json'; json.onload = () => log.push('json ' + json.response.a + ' ' + (() => { try { return json.responseText; } catch (error) { return error.name; } })()); json.open('GET', '/x'); json.send('ignored');
        const bad = new XMLHttpRequest(); bad.responseType = 'json'; bad.onload = () => log.push('bad json ' + bad.response); bad.open('GET', '/bad'); bad.send();
        const buffer = new XMLHttpRequest(); buffer.responseType = 'arraybuffer'; log.push('early ' + buffer.response); buffer.onload = () => log.push('buffer ' + buffer.response.byteLength); buffer.open('GET', '/b'); buffer.send();
        const blob = new XMLHttpRequest(); blob.responseType = 'blob'; blob.onload = () => log.push('blob ' + blob.response.size + ' ' + blob.response.type); blob.open('GET', '/c'); blob.send();
        const documentType = new XMLHttpRequest(); documentType.responseType = 'document'; documentType.onload = () => log.push('document ' + documentType.response); documentType.open('GET', '/d'); documentType.send();
        const failed = new XMLHttpRequest(); failed.onerror = () => log.push('error ' + failed.readyState + ' ' + failed.status); failed.open('GET', '/fail'); failed.send();
        const timed = new XMLHttpRequest(); timed.timeout = 1; timed.ontimeout = () => log.push('timeout ' + timed.readyState); timed.open('GET', '/slow'); timed.send();
        const aborted = new XMLHttpRequest(); aborted.onabort = () => log.push('abort ' + aborted.readyState); aborted.onloadend = () => log.push('abort end'); aborted.open('GET', '/slow'); aborted.send(); aborted.abort();
        log.push('after abort ' + aborted.readyState); aborted.abort(); new XMLHttpRequest().abort();
        const sync = new XMLHttpRequest(); sync.open('GET', '/x', false); throws(() => sync.send(), 'NotSupportedError', 'sync');
        const custom = new XMLHttpRequest(); custom.open('Purge', '/p'); custom.send(); assert(custom.upload instanceof XMLHttpRequestUpload && custom.upload instanceof XMLHttpRequestEventTarget, 'upload');
        "#,
    );
    page.settle();
    page.settle();
    page.check(
        r#"
        const expected = ['loadstart', 'progress 7 true', 'load|200|OK|{"a":1}|{"a":1}|1||"content-type: application/json\\r\\nx-a: 1\\r\\n"|true', 'loadend 1,2,3,4',
          'json 1 InvalidStateError', 'bad json null', 'early null', 'buffer 7', 'blob 7 application/json', 'document null', 'error 4 0', 'timeout 4', 'abort 4', 'abort end', 'after abort 0'];
        for (const line of expected) assert(log.includes(line), line + ' missing from ' + JSON.stringify(log));
        assert(calls[0] === 'file:///C:/app/data POST body [["X-Test","yes"]] include', calls[0]);
        assert(calls.some((call) => call.includes('/x GET  [] same-origin')) && calls.some((call) => call.includes(' Purge ')), JSON.stringify(calls));
        "#,
    );
}

#[test]
fn notifications_are_granted_and_report_their_events() {
    let mut page = fixture("<main></main>");
    page.check(
        r#"
        eq(Notification.permission, 'granted', 'permission'); throws(() => new Notification(), 'TypeError', 'title required');
        Notification.requestPermission((granted) => { globalThis.granted = granted; }).then((granted) => { globalThis.resolved = granted; });
        const note = new Notification('Done', { body: 'video.mp4', tag: 't', data: { n: 1 }, silent: true });
        eq([note.title, note.body, note.tag, note.data, note.silent, note.dir, note.icon], ['Done', 'video.mp4', 't', { n: 1 }, true, 'auto', ''], 'fields');
        eq(new Notification('Bare').silent, null, 'silent default');
        // Without an application to show it, a notification fails.
        const events = []; note.onerror = () => events.push('error'); globalThis.events = events;
        "#,
    );
    page.settle();
    // An application shows them, and sends their events back.
    let notify = boa_engine::object::FunctionObjectBuilder::new(
        page.host.context().realm(),
        boa_engine::NativeFunction::from_fn_ptr(|_, _, _| Ok(true.into())),
    )
    .build();
    let host = page.host.host_object();
    host.set(js_string!("notify"), notify, false, page.host.context())
        .unwrap();
    page.check(
        r#"
        eq([granted, resolved], ['granted', 'granted'], 'request permission');
        const shown = new Notification('Shown'); shown.onclick = () => events.push('click'); shown.addEventListener('close', () => events.push('close'));
        web.notification(3, 'click'); web.notification(3, 'close'); web.notification(3, 'click'); web.notification(99, 'show');
        eq(events, ['error', 'click', 'close'], 'events');
        "#,
    );
}

#[test]
fn parsed_markup_binds_compiled_handlers_and_loads_its_images() {
    let mut page = fixture("<main id=m></main><img id=own src=missing.png>");
    page.check(
        r#"
        const events = []; globalThis.events = events;
        ids('#own').addEventListener('error', () => events.push('own error'));
        web.ready();
        globalThis.__nativeboatHandlers = new Map([
          ['onload\u0000loaded(this)', function () { events.push('load ' + this.id); }],
          ['onerror\u0000failed(this)', function () { events.push('error ' + this.id); this.onerror = null; }],
          ['onclick\u0000hit()', function (event) { events.push('click ' + event.type); }],
        ]);
        const main = ids('#m');
        main.innerHTML = '<img id=a src=logo.png onload="loaded(this)"><button id=b onclick="hit()">b</button>';
        ids('#b').click();
        main.insertAdjacentHTML('beforeend', '<span><img id=c src=missing.png onerror="failed(this)"></span>');
        main.firstChild.outerHTML = '<img id=d src=logo.png onload="loaded(this)">';
        main.appendChild(document.createRange().createContextualFragment('<img id=e src=missing.png onerror="failed(this)">'));
        // Unknown handler text binds nothing; templates stay inert.
        main.insertAdjacentHTML('beforeend', '<img id=f src=logo.png onload="other()"><template><img src=missing.png onerror="failed(this)"></template>');
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(events.slice().sort(), ['click click', 'error c', 'error e', 'load a', 'load d', 'own error'], 'events');
        "#,
    );
}

#[test]
fn media_elements_load_their_metadata_and_first_frame() {
    let mut page = fixture("<div id=box></div>");
    page.check(
        r#"
        const log = []; const types = ['loadstart', 'progress', 'suspend', 'abort', 'error', 'emptied', 'loadedmetadata', 'loadeddata', 'canplay', 'canplaythrough', 'durationchange', 'resize'];
        const watch = (media) => { for (const type of types) media.addEventListener(type, () => log.push(media.id + ' ' + type + ' ' + media.readyState + media.networkState)); return media; };
        const video = watch(document.createElement('video')); video.id = 'v';
        eq([video.readyState, video.networkState, video.preload, video.videoWidth, video.currentSrc], [0, 0, 'metadata', 0, ''], 'empty');
        assert(Number.isNaN(video.duration) && video.paused && video.buffered.length === 0, 'nothing loaded');
        video.src = 'clip.mp4';
        eq([video.networkState, video.currentSrc], [3, ''], 'waiting to load');
        const none = watch(document.createElement('video')); none.id = 'n'; none.preload = 'none'; none.src = 'clip.mp4';
        const broken = watch(document.createElement('video')); broken.id = 'b'; broken.src = 'gone.mp4';
        const audio = watch(new Audio('tone.mp3')); audio.id = 'a'; eq(audio.preload, 'auto', 'audio preload');
        ids('#box').innerHTML = '<video id=s><source src=clip.mp4></video>'; const parsed = watch(ids('#s'));
        eq([HTMLMediaElement.HAVE_ENOUGH_DATA, video.NETWORK_NO_SOURCE, MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED], [4, 3, 4], 'constants');
        eq(['video/mp4', 'video/mp4; codecs="avc1.42E01E"', 'video/webm', 'video/webm; codecs="vp9"', 'audio/mpeg', 'video/ogg', 'video/x-matroska', 'video/mp4; codecs="hev1"'].map((type) => video.canPlayType(type)),
          ['maybe', 'probably', 'maybe', 'probably', 'probably', '', '', ''], 'playable types');
        throws(() => new MediaError(), 'TypeError', 'no media errors made');
        throws(() => new HTMLAudioElement(), 'TypeError', 'audio elements made by Audio only');
        throws(() => Audio(), 'TypeError', 'Audio needs new');
        eq([Audio.name, Audio.prototype === HTMLAudioElement.prototype, audio instanceof HTMLAudioElement], ['Audio', true, true], 'Audio');
        globalThis.media = { video, none, broken, audio, parsed, log };
        "#,
    );
    page.settle();
    page.check(
        r#"
        const { video, none, broken, audio, parsed, log } = media;
        const loaded = (id, video) => [id + ' loadstart 02', id + ' durationchange 41', ...(video ? [id + ' resize 41'] : []), ...['loadedmetadata', 'progress', 'suspend', 'loadeddata', 'canplay', 'canplaythrough'].map((type) => id + ' ' + type + ' 41')];
        eq(log.filter((line) => line.startsWith('v ')), loaded('v', true), 'video events');
        eq(log.filter((line) => line.startsWith('n ')), ['n loadstart 01', 'n suspend 01'], 'not preloaded');
        eq(log.filter((line) => line.startsWith('b ')), ['b loadstart 02', 'b error 03'], 'unreadable');
        eq(log.filter((line) => line.startsWith('a ')), loaded('a', false), 'audio events');
        eq(log.filter((line) => line.startsWith('s ')).length, 9, 'source child');
        eq([video.videoWidth, video.videoHeight, video.duration, video.readyState, video.networkState], [2, 1, 1.5, 4, 1], 'loaded video');
        assert(video.currentSrc.endsWith('/clip.mp4') && video.buffered.length === 1 && video.buffered.end(0) === 1.5 && video.seekable.length === 1, 'loaded ranges');
        throws(() => video.buffered.start(1), 'IndexSizeError', 'range index');
        eq([broken.error.code, broken.readyState], [4, 0], 'media error');
        eq([audio.duration, audio.videoWidth], [Infinity, undefined], 'audio');
        log.length = 0; video.src = 'gone.mp4';
        "#,
    );
    page.settle();
    page.check(
        r#"
        eq(media.log, ['v abort 41', 'v emptied 00', 'v durationchange 00', 'v loadstart 02', 'v error 03'], 'reloaded');
        eq([media.video.videoWidth, media.video.error.code], [0, 4], 'reset');
        "#,
    );
    let calls = page.page.calls.borrow().clone();
    let frames: Vec<&String> = calls
        .iter()
        .filter(|call| call.starts_with("frame "))
        .collect();
    assert_eq!(
        frames,
        [
            "frame nativeboat-video-frame:clip.mp4 2x1 8",
            "frame nativeboat-video-frame:clip.mp4 2x1 8"
        ]
    );
}

#[test]
fn performance_observers_get_marks_and_measures_in_a_later_task() {
    let mut page = fixture("<p>x</p>");
    page.check(r#"
      eq(PerformanceObserver.supportedEntryTypes.join(),'mark,measure','supported types');
      throws(()=>new PerformanceObserver(1),'TypeError','callback required');
      const seen=[];const observer=new PerformanceObserver((list,self)=>{seen.push(list.getEntries().map((entry)=>entry.entryType+':'+entry.name).join(),self===observer);});
      throws(()=>observer.observe({}),'TypeError','types required');
      observer.observe({entryTypes:['mark','measure','longtask']});
      performance.mark('a');performance.measure('m','a');eq(seen.length,0,'delivered in a later task');
      const other=new PerformanceObserver(()=>{});other.observe({type:'mark',buffered:true});eq(other.takeRecords().map((entry)=>entry.name).join(),'a','buffered marks');
      globalThis.performanceSeen=seen;
    "#);
    page.settle();
    page.check("eq(performanceSeen.join('|'),'mark:a,measure:m|true','observed entries');");
}
