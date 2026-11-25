use super::*;

#[test]
fn inline_and_computed_styles() {
    let mut page = fixture("<p id=p style='color: blue'>x</p>");
    page.check(
        r#"
        const p = ids('#p');
        const style = p.style;
        eq(style.color, 'blue', 'camel get'); eq(style.cssText, 'color: blue;', 'text'); eq(style.length, 1, 'length');
        style.backgroundColor = 'red'; eq(p.getAttribute('style').includes('background-color: red'), true, 'camel set');
        style.setProperty('margin-top', '4px', 'important'); eq(style.getPropertyPriority('margin-top'), 'important', 'priority');
        eq(style.getPropertyValue('margin-top'), '4px', 'value'); eq(style.item(0), 'color', 'item'); eq(style[0], 'color', 'index'); eq(style.item(99), '', 'no item');
        eq(style.removeProperty('color'), 'blue', 'remove'); style.setProperty('padding', ''); style.setProperty('padding', null); style.width = '';
        style.cssFloat = 'left'; eq(style.cssFloat, 'left', 'float'); style.WebkitTransform = 'none';
        style.setProperty('--brand', 'red'); eq(style.getPropertyValue('--brand'), 'red', 'custom'); eq(style['--brand'], 'red', 'custom get');
        assert('color' in style && 'cssText' in style && !(Symbol.iterator in {}), 'has'); eq([...style].length, style.length, 'iterate');
        style.cssText = 'width: 10px'; eq(p.getAttribute('style'), 'width: 10px', 'set text');
        style[Symbol.toStringTag] = 'x'; eq(style.parentRule, null, 'parent rule');
        p.style = 'height: 5px'; eq(p.style.height, '5px', 'assign style'); eq(p.style, style, 'cached');
        throws(() => new CSSStyleDeclaration(), 'TypeError', 'illegal');
        const computed = getComputedStyle(p); eq(computed.color, 'rgb(255, 0, 0)', 'computed'); eq(computed.length, 2, 'computed length');
        eq(computed.cssText, 'color: rgb(255, 0, 0); position: relative;', 'computed text'); eq(computed.getPropertyPriority('color'), '', 'computed priority');
        eq(computed.margin, '', 'computed missing');
        throws(() => { computed.color = 'red'; }, 'NoModificationAllowedError', 'read only');
        throws(() => computed.removeProperty('color'), 'NoModificationAllowedError', 'read only remove');
        throws(() => { computed.cssText = ''; }, 'NoModificationAllowedError', 'read only text');
        eq(getComputedStyle(document).length, 0, 'not an element');
        "#,
    );
}

#[test]
fn style_sheets() {
    let mut page = fixture("<style>p { color: red } /* note */ .a { margin: 0 }</style><p>x</p>");
    page.check(
        r#"
        const element = document.querySelector('style'); const sheet = element.sheet; eq(sheet, document.styleSheets[0], 'sheets');
        eq(document.styleSheets.item(0), sheet, 'item'); eq(document.styleSheets.item(5), null, 'no item');
        eq(sheet.cssRules.length, 2, 'rules'); eq(sheet.cssRules[0].selectorText, 'p', 'selector'); eq(sheet.cssRules[0].style.color, 'red', 'rule style');
        eq(sheet.cssRules[0].style.cssText, 'color: red', 'rule text'); eq(sheet.cssRules[0].style.length, 1, 'rule length'); eq(sheet.cssRules[0].style.margin, '', 'no value');
        throws(() => { sheet.cssRules[0].style.color = 'x'; }, 'NoModificationAllowedError', 'rule read only');
        eq(sheet.cssRules[1].cssText, '.a { margin: 0 }', 'comment stripped'); eq(sheet.rules.length, 2, 'rules alias');
        eq(sheet.insertRule('@media print { p { color: black } }', 2), 2, 'insert'); eq(sheet.cssRules[2].type, 4, 'media');
        sheet.insertRule('@import url(x.css);', 2); eq(sheet.cssRules[2].type, 3, 'import');
        sheet.insertRule('@layer x;', 2); eq(sheet.cssRules[2].type, 0, 'unknown at');
        sheet.addRule('i', 'color: green'); eq(sheet.cssRules.length, 6, 'add rule'); sheet.deleteRule(2); sheet.removeRule(2);
        throws(() => sheet.insertRule('x {}', 99), 'IndexSizeError', 'insert range'); throws(() => sheet.deleteRule(0), 'IndexSizeError', 'delete own');
        eq(sheet.ownerNode, element, 'owner'); eq(sheet.type + sheet.href + sheet.disabled, 'text/cssnullfalse', 'props'); eq(sheet.media.length, 0, 'media');
        eq(web.styleSheets()[0][1].includes('@media print'), true, 'renderer text'); eq(web.styleSheets()[0][1].split('\n').length, 3, 'inserted lines');
        const constructed = new CSSStyleSheet(); constructed.replaceSync('a { color: red } b { color: blue }'); eq(constructed.cssRules.length, 2, 'replace');
        eq(constructed.ownerNode, null, 'no owner'); constructed.insertRule('c {}'); constructed.deleteRule(0);
        throws(() => new StyleSheet(), 'TypeError', 'style sheet'); throws(() => new CSSRule(), 'TypeError', 'rule');
        constructed.replace('d { x: "}" }').then((result) => { globalThis.replaced = result.cssRules.length; });
        eq(document.createElement('style').sheet, null, 'detached sheet'); eq(document.createElement('link').sheet, null, 'link sheet');
        const quoted = document.createElement('style'); quoted.textContent = 'a::after { content: "}" } b { c: d '; document.head.appendChild(quoted);
        eq(quoted.sheet.cssRules.length, 1, 'quoted brace'); eq(web.styleSheets().length, 2, 'two sheets'); eq(web.styleSheets()[1][1], quoted.textContent, 'plain text');
        const escaped = document.createElement('style'); escaped.textContent = "a { content: '\\'' } /* open"; document.head.appendChild(escaped);
        eq(escaped.sheet.cssRules.length, 1, 'escaped');
        "#,
    );
    page.check("eq(replaced, 1, 'async replace');");
    let rules: Vec<String> = page
        .page
        .calls
        .borrow()
        .iter()
        .filter(|call| call.starts_with("rules "))
        .map(|call| call.split(' ').nth(2).unwrap().to_owned())
        .collect();
    assert_eq!(rules, ["1", "2", "3", "4", "3", "2"]);
}
