use super::*;

#[test]
fn grouped_typing_undo_redo_restore_caret_nodes_and_event_listeners() {
    let mut page = fixture("<div id=e contenteditable><p id=p><b id=b>ab</b>cd</p></div>");
    page.check(r#"
      const e=ids('#e'),b=ids('#b'),text=b.firstChild,s=getSelection();e.focus();s.collapse(text,2);
      let clicks=0;b.onclick=()=>clicks++;
      web.text('H');web.text('i');eq(b.textContent,'abHi','typed group');
      eq(web.command('undo'),true,'undo');eq(b.textContent,'ab','group undone');eq(s.anchorOffset,2,'caret restored');
      eq(ids('#b'),b,'element identity');eq(b.firstChild,text,'text identity');b.click();eq(clicks,1,'listener retained');
      eq(web.command('redo'),true,'redo');eq(b.textContent,'abHi','group redone');eq(s.anchorOffset,4,'redo caret');
      web.text('!');eq(web.command('undo'),true,'typing after redo is separate');eq(b.textContent,'abHi','one edit undone');
      web.text('?');eq(web.command('redo'),false,'divergent edit clears redo');
      e.textContent='external';eq(web.command('undo'),false,'external content invalidates history');eq(e.textContent,'external','external content retained');
      eq(web.command('unknown'),false,'unknown command');
      s.collapse(e.firstChild,8);web.text('!');e.innerHTML=e.innerHTML;
      eq(web.command('undo'),false,'externally replaced nodes invalidate history');eq(e.textContent,'external!','replacement kept');
    "#);
}

#[test]
fn structural_edits_and_composition_have_atomic_history() {
    let mut page =
        fixture("<div id=e contenteditable><p id=a>ab</p><p id=b class=keep>cd</p></div>");
    page.check(r#"
      const e=ids('#e'),a=ids('#a'),b=ids('#b'),s=getSelection();e.focus();s.collapse(b.firstChild,0);
      web.key(true,{key:'Backspace'});eq(e.innerHTML,'<p id="a">abcd</p>','paragraphs joined');
      web.command('undo');eq(e.children.length,2,'paragraph restored');eq(e.lastChild,b,'original paragraph');eq(b.className,'keep','attributes restored');
      eq(s.anchorNode,b.firstChild,'caret node restored');eq(s.anchorOffset,0,'caret offset restored');
      web.command('redo');eq(e.children.length,1,'join redone');
      s.collapse(a.firstChild,2);web.key(true,{key:'Enter'});eq(e.children.length,2,'split paragraph');
      web.command('undo');eq(e.textContent,'abcd','split undone');eq(e.children.length,1,'split removed');
      web.command('redo');eq(e.children.length,2,'split redone');
      web.composition('start');web.composition('update','ni');web.composition('update','你好😀');web.composition('end','你好😀');
      const committed=e.innerHTML;web.command('undo');eq(e.textContent,'abcd','whole composition undone');
      web.command('redo');eq(e.innerHTML,committed,'composition redone');
      web.composition('start');web.composition('update','cancel');web.composition('end','');
      web.command('undo');eq(e.textContent,'abcd','cancellation creates no extra history');
    "#);
}

#[test]
fn native_controls_and_user_commands_share_cancellable_history() {
    let mut page = fixture_with(
        "<input id=t value=abcd><input id=r readonly value=keep>",
        FakePage {
            clipboard: RefCell::new(Some("paste".into())),
            ..Default::default()
        },
    );
    page.check(r#"
      const t=ids('#t'),r=ids('#r'),id=t[Object.getOwnPropertySymbols(t).find(symbol=>String(symbol)==='Symbol(nativeboat.node)')];
      t.focus();t.setSelectionRange(2,2);
      web.key(true,{key:'X'},true);t.setSelectionRange(3,3);web.input(id,'abXcd');web.key(false,{key:'X'},true);
      eq(web.key(true,{key:'z',ctrlKey:true},true),true,'Ctrl+Z is taken');eq(t.value,'abcd','native typing undone');eq(t.selectionStart,2,'native old caret');
      eq(web.key(true,{key:'z',ctrlKey:true,shiftKey:true},true),true,'Ctrl+Shift+Z is taken');eq(t.value,'abXcd','native typing redone');eq(t.selectionStart,3,'native new caret');
      const cancel=event=>event.preventDefault();t.addEventListener('beforeinput',cancel);
      web.command('undo');eq(t.value,'abXcd','canceled history');web.command('paste');eq(t.value,'abXcd','canceled paste');
      t.removeEventListener('beforeinput',cancel);web.command('paste');eq(t.value,'abXpastecd','paste');
      web.command('undo');eq(t.value,'abXcd','paste undone');web.command('redo');eq(t.value,'abXpastecd','paste redone');
      r.focus();eq(web.command('undo'),false,'readonly has no history');web.command('paste');eq(r.value,'keep','readonly kept');
    "#);
}

#[test]
fn history_beforeinput_can_be_handled_by_the_application() {
    let mut page = fixture("<div id=e contenteditable>own history</div>");
    page.check(r#"
      const e=ids('#e');e.focus();const log=[];
      e.onbeforeinput=event=>{if(event.inputType.startsWith('history')){log.push([event.inputType,event.cancelable,event.isTrusted]);event.preventDefault();}};
      eq(web.command('undo'),true,'application undo');eq(web.command('redo'),true,'application redo');
      eq(log,[['historyUndo',true,true],['historyRedo',true,true]],'history events');eq(e.textContent,'own history','application owns history');
    "#);
}

#[test]
fn native_history_preserves_comments_attributes_and_outer_host_changes() {
    let mut page =
        fixture("<div id=e contenteditable><p><b data-kind=before>ab</b><!--keep--></p></div>");
    page.check(r#"
      const e=ids('#e'),b=e.querySelector('b'),comment=b.nextSibling,s=getSelection();
      e.focus();s.collapse(b.firstChild,2);
      e.addEventListener('input',()=>{b.setAttribute('data-kind','after');comment.data='changed';},{once:true});
      web.text('!');eq(b.textContent,'ab!','typed');
      e.className='outer-changed';
      eq(web.command('undo'),true,'outer host attributes do not invalidate content history');
      eq(b.getAttribute('data-kind'),'before','nested attribute restored');eq(comment.data,'keep','comment restored');
      eq(e.className,'outer-changed','outer attributes preserved');
      eq(web.command('redo'),true,'redo');eq(b.getAttribute('data-kind'),'after','attribute redone');
      eq(comment.data,'changed','comment redone');eq(b.nextSibling,comment,'comment identity');
    "#);
}
