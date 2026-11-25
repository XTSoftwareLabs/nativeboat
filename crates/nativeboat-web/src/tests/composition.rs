use super::*;

#[test]
fn ime_preedit_replaces_one_live_range_and_commits_unicode() {
    let mut page = fixture("<div id=e contenteditable>ab</div>");
    page.check(r#"
      const e=ids('#e'),s=getSelection();e.focus();s.collapse(e.firstChild,1);
      globalThis.events=[];
      for(const type of ['compositionstart','compositionupdate','compositionend','beforeinput','input','keydown','keyup']) {
        e.addEventListener(type,event=>events.push([type,event.data??null,event.isComposing??null,event.inputType??null,event.cancelable,event.isTrusted,event.keyCode??null]));
      }
      eq(web.composition('start'),false,'start allowed');
      web.composition('update','ni',1);eq(e.textContent,'anib','preedit');eq(s.anchorOffset,2,'IME cursor');
      web.composition('update','ni',2);eq(s.anchorOffset,3,'cursor-only update');eq(events.length,4,'cursor does not fire input');
      web.key(true,{key:'Backspace'});web.key(false,{key:'Backspace'});web.text('x');eq(e.textContent,'anib','IME owns edits');
      web.composition('update','你好😀',4);eq(e.textContent,'a你好😀b','Unicode replaces preedit');
      web.composition('end','你好😀');eq(e.textContent,'a你好😀b','committed once');eq(s.anchorOffset,5,'UTF16 caret');
      eq(events.map(event=>event[0]),['compositionstart','beforeinput','compositionupdate','input','keydown','keyup','beforeinput','compositionupdate','input','compositionend'],'event order');
      eq(events[1].slice(1,5),['ni',true,'insertCompositionText',false],'noncancelable composing input');
      eq(events[4][2],true,'composing key');eq(events[4][6],229,'IME key code');
      eq(events[5][2],true,'composing key up');eq(events[9][1],'你好😀','final data');
      eq(events.every(event=>event[5]),true,'trusted events');
      web.text('!');eq(e.textContent,'a你好😀!b','ordinary typing resumes');
    "#);
}

#[test]
fn ime_cancellation_selection_and_compositionstart_anchor_mutation() {
    let mut page =
        fixture("<div id=e contenteditable>before word after</div><button id=b>Other</button>");
    page.check(r#"
      const e=ids('#e'),s=getSelection();e.focus();s.setBaseAndExtent(e.firstChild,7,e.firstChild,11);
      web.composition('start');web.composition('update','zhong');web.composition('update','中');web.composition('end','中文');
      eq(e.textContent,'before 中文 after','selection replaced and final text applied');
      web.composition('start');web.composition('update','cancel');web.composition('end','');
      eq(e.textContent,'before 中文 after','canceled preedit removed');
      e.textContent='';e.focus();s.collapse(e,0);
      e.addEventListener('compositionstart',()=>{const anchor=document.createTextNode('\u200b');e.append(anchor);s.collapse(anchor,0);},{once:true});
      web.composition('start');web.composition('update','你');web.composition('end','你');eq(e.textContent,'你\u200b','handler seed preserved');
      web.composition('start');web.composition('update','好');ids('#b').focus();
      eq(e.textContent,'你好\u200b','focus change commits preedit');
      web.composition('update','late');eq(e.textContent,'你好\u200b','stale update ignored');
      e.focus();s.collapse(e.firstChild,0);
      const ended=[];e.addEventListener('compositionend',event=>ended.push(event.data));
      e.addEventListener('compositionstart',event=>event.preventDefault(),{once:true});
      eq(web.composition('start'),true,'cancel start');web.composition('update','rejected');eq(ended,[''],'canceled start ends once');
      eq(e.textContent,'你好\u200b','canceled start keeps content');
      eq(web.composition('unknown'),false,'unknown phase');
      ids('#b').focus();eq(web.composition('start'),true,'non-editable focus cancels native IME');
    "#);
}

#[test]
fn ime_focus_and_pointer_changes_commit_native_candidate_before_moving_selection() {
    let mut page = fixture("<div id=e contenteditable>ab</div><button id=b>Other</button>");
    page.check(
        r#"
      globalThis.e=ids('#e');globalThis.s=getSelection();globalThis.ended=[];e.focus();s.collapse(e.firstChild,1);
      e.oncompositionend=event=>ended.push([event.data,document.activeElement===e]);
      web.composition('start');web.composition('update','ni',1);
    "#,
    );
    *page.page.composition.borrow_mut() = Some("你".into());
    page.check(r#"
      ids('#b').focus();eq(e.textContent,'a你b','converted candidate before blur');eq(ended,[['你',true]],'end on old focused host');
      e.focus();s.collapse(e.firstChild,2);web.composition('start');web.composition('update','hao');
    "#);
    *page.page.composition.borrow_mut() = Some("好😀".into());
    page.check(r#"
      web.pointer('down',e[Object.getOwnPropertySymbols(e).find(symbol=>String(symbol)==='Symbol(nativeboat.node)')],{button:0,clientX:0,clientY:0});
      eq(e.textContent,'a你好😀b','converted candidate before pointer selection');
      eq(ended,[['你',true],['好😀',true]],'each ends once');
      web.composition('end','late');eq(e.textContent,'a你好😀b','stale result ignored');
    "#);
}
