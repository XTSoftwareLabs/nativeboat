use super::*;

const FORM: &str = r#"<form id=f action="/save"><label id=l1 for=t>T</label><input id=t name=t value=start required maxlength=5>
<label id=l2><input id=c type=checkbox name=c checked></label><input id=r1 type=radio name=r value=1><input id=r2 type=radio name=r value=2 checked>
<textarea id=ta name=ta>text</textarea><select id=s name=s><option value=a>A</option><option selected> B  b </option></select>
<select id=m multiple name=m><option selected>x</option><option>y</option></select><button id=b>Go</button><input id=reset type=reset>
<input type=file name=file><input id=dis name=dis disabled value=1><fieldset></fieldset></form>
<input id=outside form=f name=o value=v><input id=num type=number value=3 step=2><input id=weird type=WEIRD><progress id=pr value=1 max=4></progress>"#;

#[test]
fn inputs_and_text_areas() {
    let mut page = fixture(FORM);
    page.check(
        r#"
        const t = ids('#t'); const c = ids('#c'); const f = ids('#f');
        eq(t.value, 'start', 'value'); t.value = 'next'; eq(t.value, 'next', 'set value'); t.value = null; eq(t.value, '', 'null value');
        eq(t.defaultValue, 'start', 'default'); t.defaultValue = 'd'; eq(t.getAttribute('value'), 'd', 'set default');
        eq(t.type, 'text', 'type'); eq(ids('#weird').type, 'text', 'unknown type'); t.type = 'TEXT'; eq(t.type, 'text', 'set type');
        eq(t.maxLength, 5, 'max length'); eq(ids('#num').maxLength, -1, 'default max'); eq(t.size, 20, 'size');
        t.maxLength = 7; eq(t.getAttribute('maxlength'), '7', 'set number');
        assert(t.required && !t.disabled && ids('#dis').disabled, 'booleans'); t.required = false; assert(!t.hasAttribute('required'), 'unset boolean');
        eq(t.form, f, 'form'); eq(ids('#outside').form, f, 'form attribute'); eq(t.labels.length, 1, 'labels'); eq(t.labels[0].id, 'l1', 'label');
        t.value = 'hello'; eq(t.selectionStart, 5, 'selection start'); eq(t.selectionEnd, 5, 'selection end'); eq(t.selectionDirection, 'none', 'direction');
        t.setSelectionRange(1, 3, 'forward'); eq([t.selectionStart, t.selectionEnd, t.selectionDirection], [1, 3, 'forward'], 'range');
        t.selectionStart = 2; t.selectionEnd = 4; t.selectionDirection = 'backward';
        eq([t.selectionStart, t.selectionEnd, t.selectionDirection], [2, 4, 'backward'], 'set selection');
        t.setSelectionRange(9, -1); eq([t.selectionStart, t.selectionEnd], [5, 5], 'clamped'); t.setSelectionRange('x', 'y'); eq(t.selectionStart, 0, 'nan');
        let selected = false; t.addEventListener('select', () => { selected = true; }); t.select(); assert(selected, 'select event'); eq(t.selectionEnd, 5, 'select all');
        t.setRangeText('J', 0, 1); eq(t.value, 'Jello', 'range text'); t.setRangeText('!'); eq(t.value.length, 6, 'range default');
        const num = ids('#num'); eq(num.valueAsNumber, 3, 'number'); num.stepUp(); eq(num.value, '5', 'step up'); num.stepDown(2); eq(num.value, '1', 'step down');
        num.valueAsNumber = 7; eq(num.value, '7', 'set number'); t.value = 'x'; assert(Number.isNaN(t.valueAsNumber), 'nan number'); eq(t.valueAsDate, null, 'date');
        const file = document.querySelector('[type=file]'); eq(file.files.length, 0, 'files'); eq(file.files.item(0), null, 'file item'); eq(t.files, null, 'no files');
        eq(t.list, null, 'list'); t.showPicker();
        assert(c.checked && c.defaultChecked, 'checked'); c.checked = false; assert(!c.checked && c.defaultChecked, 'set checked');
        c.defaultChecked = false; assert(!c.hasAttribute('checked'), 'default unchecked');
        c.indeterminate = true; assert(c.indeterminate, 'indeterminate'); eq(c.labels[0].id, 'l2', 'wrapping label'); eq(ids('#l2').control, c, 'label control');
        eq(ids('#l1').control, t, 'for control'); eq(ids('#l1').htmlFor, 't', 'html for'); eq(ids('#l1').form, f, 'label form');
        const lone = document.createElement('label'); lone.htmlFor = 'f'; eq(lone.control, null, 'not labelable'); lone.htmlFor = 'pr'; eq(lone.control, ids('#pr'), 'progress labelable');
        lone.htmlFor = 'none'; eq(lone.control, null, 'missing control'); eq(lone.form, null, 'no form'); eq(document.createElement('label').control, null, 'empty label');
        const ta = ids('#ta'); eq(ta.type, 'textarea', 'textarea type'); eq(ta.value, 'text', 'textarea'); ta.value = 'new'; eq(ta.defaultValue, 'text', 'textarea default');
        ta.defaultValue = 'changed'; eq(ta.textContent, 'changed', 'set default'); eq(ta.textLength, 3, 'length'); ta.value = null; eq(ta.value, '', 'null');
        eq(ta.rows + ta.cols, 22, 'rows cols');
        const progress = ids('#pr'); eq(progress.value, 1, 'progress'); eq(progress.max, 4, 'max'); eq(progress.position, 0.25, 'position');
        progress.value = 2; progress.max = 0; eq(progress.max, 1, 'bad max'); progress.removeAttribute('value'); eq(progress.value, 0, 'no value');
        eq(progress.position, -1, 'indeterminate');
        "#,
    );
}

#[test]
fn selects_and_options() {
    let mut page = fixture(FORM);
    page.check(
        r#"
        const s = ids('#s'); const m = ids('#m'); const f = ids('#f');
        eq(s.type, 'select-one', 'select type'); eq(m.type, 'select-multiple', 'multiple type'); eq(s.value, 'B b', 'select value'); eq(s.selectedIndex, 1, 'index');
        s.value = 'a'; eq(s.selectedIndex, 0, 'set value'); s.selectedIndex = 1; eq(s.value, 'B b', 'set index'); s.value = null; eq(s.selectedIndex, -1, 'null value');
        eq(s.length, 2, 'length'); eq(s.options.length, 2, 'options'); eq(s.item(0).value, 'a', 'item'); eq(s.item(4), null, 'no item');
        eq(s.namedItem('none'), null, 'named'); s.options[0].id = 'oa'; eq(s.namedItem('oa').value, 'a', 'named item');
        s.options.selectedIndex = 0; eq(s.options.selectedIndex, 0, 'options index'); eq(m.selectedOptions.length, 1, 'selected options');
        const option = new Option('Third', 'c', true, true); s.options.add(option); eq(s.length, 3, 'add'); eq(s.value, 'c', 'selected new');
        s.add(new Option('Zero', 'z'), 0); eq(s.item(0).value, 'z', 'add before index'); s.add(new Option('Mid'), s.item(1)); s.add(new Option('End'), 9);
        s.options.remove(0); s.remove(0); s.remove(99); eq(s.length, 4, 'removed');
        eq(option.text, 'Third', 'text'); option.text = 'T3'; eq(option.label, 'T3', 'label'); option.label = 'L'; eq(option.label, 'L', 'set label');
        option.value = 'cc'; eq(option.value, 'cc', 'value'); eq(option.index, 2, 'index'); eq(option.form, f, 'option form');
        assert(option.defaultSelected, 'default selected'); option.defaultSelected = false; option.selected = false; assert(!option.selected, 'deselected');
        eq(new Option().text, '', 'empty option'); eq(new Option('x').index, 0, 'detached index'); eq(new Option('x').form, null, 'detached form');
        const detached = document.createElement('select'); document.body.appendChild(detached); detached.remove(); eq(detached.parentNode, null, 'select removed');
        s.multiple = true; eq(s.type, 'select-multiple', 'now multiple');
        "#,
    );
}

#[test]
fn constraint_validation_reports_what_the_document_checks() {
    let mut page = fixture(
        "<input id=email type=email value=me><input id=n type=number max=2 value=5><input id=plain>",
    );
    page.check(
        r#"
        const e = ids('#email'); const n = ids('#n');
        assert(e.validity.typeMismatch && !e.validity.valid, 'email mismatch');
        eq(e.validationMessage, "Please include an '@' in the email address. 'me' is missing an '@'.", 'email message');
        assert(e.matches(':invalid') && !e.matches(':valid'), 'invalid email matches');
        e.value = 'me@example.org'; assert(e.validity.valid && e.matches(':valid'), 'valid email');
        assert(n.validity.rangeOverflow, 'overflow'); eq(n.validationMessage, 'Value must be less than or equal to 2.', 'range message');
        let fired = 0; n.addEventListener('invalid', () => fired++); assert(!n.checkValidity(), 'check'); eq(fired, 1, 'invalid event');
        n.setCustomValidity('nope'); assert(n.validity.customError, 'custom'); eq(n.validationMessage, 'nope', 'custom message');
        n.setCustomValidity(''); n.value = '1'; assert(n.checkValidity() && ids('#plain').willValidate, 'valid number');
        "#,
    );
}

#[test]
fn forms_buttons_and_validity() {
    let mut page = fixture(FORM);
    page.check(
        r#"
        const t = ids('#t'); const c = ids('#c'); const m = ids('#m'); const f = ids('#f'); const ta = ids('#ta');
        eq(ids('#b').type, 'submit', 'button type'); ids('#b').type = 'reset'; eq(ids('#b').type, 'reset', 'set button type');
        ids('#b').type = 'button'; eq(ids('#b').type, 'button', 'button'); ids('#b').type = 'submit';
        eq(f.elements.length, 13, 'elements'); eq(f.length, 13, 'form length'); eq(f.method, 'get', 'method'); f.method = 'POST'; eq(f.method, 'post', 'post');
        eq(f.action, 'file:///save', 'action'); t.value = ''; assert(!f.checkValidity(), 'invalid form'); t.value = 'ok'; eq(t.validity.valueMissing, false, 'valid');
        t.value = ''; assert(t.validity.valueMissing && !t.checkValidity(), 'missing'); eq(t.validationMessage, 'Please fill out this field.', 'message');
        t.setCustomValidity('custom'); eq(t.validationMessage, 'custom', 'custom'); assert(!t.reportValidity(), 'report'); t.setCustomValidity(''); t.value = 'v';
        eq(t.validationMessage, '', 'no message'); assert(t.willValidate && !ids('#dis').willValidate, 'will validate');
        const radio = ids('#r1'); radio.required = true; eq(radio.validity.valueMissing, false, 'radio group checked'); c.required = true; c.checked = true;
        eq(c.validity.valid, true, 'checked checkbox valid'); assert(f.reportValidity() === f.checkValidity(), 'form report');
        const data = new FormData(f); eq(data.getAll('r'), ['2'], 'form data radio'); eq(data.get('s'), 'B b', 'select data');
        eq(data.get('dis'), null, 'disabled data'); eq([...data.keys()].includes('file'), false, 'file skipped'); eq(data.get('o'), 'v', 'outside');
        eq(data.get('c'), 'on', 'checked box'); eq(data.getAll('m'), ['x'], 'multiple');
        data.append('x', 1); data.set('x', 2); eq(data.get('x'), '2', 'set'); assert(data.has('x') && !data.has('y'), 'has'); data.delete('x');
        eq([...data.values()].length, [...data].length, 'values'); let entries = 0; data.forEach(() => entries++); eq(entries, [...data.entries()].length, 'forEach');
        data.append('blob', new Blob(['a'])); assert(data.get('blob') instanceof Blob, 'blob value'); eq(new FormData().get('x'), null, 'empty form data');
        c.checked = false; t.value = 'zzz'; ta.value = 'zzz'; m.options[1].selected = true; ids('#s').value = 'a';
        f.addEventListener('reset', (event) => { if (globalThis.blockReset) event.preventDefault(); });
        globalThis.blockReset = true; f.reset(); eq(t.value, 'zzz', 'reset blocked'); globalThis.blockReset = false; f.reset();
        eq(t.value, 'start', 'reset text'); eq(ta.value, 'text', 'reset textarea'); assert(c.checked, 'reset checkbox'); eq(m.selectedOptions.length, 1, 'reset select');
        eq(ids('#s').value, 'B b', 'reset single select');
        let submitted = null; f.addEventListener('submit', (event) => { submitted = event.submitter; if (!globalThis.allowSubmit) event.preventDefault(); });
        f.requestSubmit(ids('#b')); eq(submitted, ids('#b'), 'request submit'); globalThis.allowSubmit = true; f.requestSubmit(); f.submit();
        "#,
    );
    let printed = page.printed();
    assert_eq!(
        printed
            .iter()
            .filter(|line| line.contains("Form submission to 'file:///save' is not supported"))
            .count(),
        2
    );
}
