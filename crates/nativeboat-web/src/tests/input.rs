use super::*;

#[test]
fn external_file_hover_exposes_protected_metadata_and_negotiates_copy() {
    let mut page =
        fixture("<div id=a>Import</div><div id=b>Move only</div><div id=c>Ignored drop</div>");
    let a = node(&mut page, "#a");
    let b = node(&mut page, "#b");
    let c = node(&mut page, "#c");
    let script=r#"
        const a=ids('#a'),b=ids('#b'),c=ids('#c');
        globalThis.externalLog=[];globalThis.heldHover=[];globalThis.droppedText='';
        const protectedEvent=e=>{
          const d=e.dataTransfer;
          assert(e.isTrusted,'trusted external event');eq(d.types,['Files'],'formats');
          eq([...d.items].map(i=>[i.kind,i.type]),[['file','text/plain'],['file','application/pdf']],'metadata');
          eq(d.files.length,0,'protected files');eq(d.items[0].getAsFile(),null,'protected file');
          eq(d.getData('text/plain'),'','no string data');eq(d.effectAllowed,'copy','copy source');
          d.setData('text','forbidden');eq(d.items.add('x','text'),null,'protected writes');
          heldHover.push([d,d.items[0]]);
          externalLog.push(e.type+':'+e.currentTarget.id);
        };
        for(const target of [a,b,c])for(const type of ['dragenter','dragover','dragleave'])target.addEventListener(type,protectedEvent);
        a.addEventListener('dragover',e=>{e.preventDefault();a.classList.add('hover')});
        a.addEventListener('dragleave',()=>a.classList.remove('hover'));
        a.addEventListener('drop',e=>{
          e.preventDefault();a.classList.remove('hover');
          eq(e.dataTransfer.files[0].path,'C:\\drop\\a.txt','dropped path');
          eq(e.dataTransfer.files[0].size,5,'dropped bytes');
          e.dataTransfer.files[0].text().then(text=>droppedText=text);
          externalLog.push('drop:a');
        });
        b.addEventListener('dragover',e=>{e.preventDefault();e.dataTransfer.dropEffect='move'});
        c.addEventListener('dragover',e=>e.preventDefault());
        const paths=[{path:'C:\\drop\\a.txt'},{path:'C:\\drop\\b.pdf'}];
        assert(!web.dragFiles('over',$A,paths,{}),'over without enter');
        assert(web.dragFiles('enter',$A,paths,{ctrlKey:true,clientX:4}),'accepted enter');
        assert(a.classList.contains('hover'),'hover feedback');
        assert(web.dragFiles('over',$A,paths,{}),'accepted over');
        assert(!web.dragFiles('over',$B,paths,{}),'move effect rejected');
        assert(!a.classList.contains('hover'),'previous target cleared');
        assert(!web.dragFiles('leave',null,[],{}),'leave');
        eq(externalLog,['dragenter:a','dragover:a','dragover:a','dragenter:b','dragleave:a','dragover:b','dragleave:b'],'target lifecycle');
        for(const [d,item] of heldHover){eq(d.types,[],'expired view');eq(item.kind,'','expired item')}
        assert(web.dragFiles('enter',$A,paths,{}),'second enter');
        assert(web.dropFiles($A,[{path:'C:\\drop\\a.txt',bytes:'aGVsbG8='},{path:'C:\\drop\\b.pdf',bytes:''}],{}),'accepted drop');
        assert(!a.classList.contains('hover'),'drop feedback cleared');eq(externalLog.at(-1),'drop:a','drop delivered');
        assert(!web.dragFiles('over',$A,paths,{}),'finished external drag');
        assert(web.dragFiles('enter',$C,paths,{}),'ignored target over accepts');
        assert(!web.dropFiles($C,[{path:'C:\\drop\\a.txt',bytes:''}],{}),'uncanceled drop has no copy effect');
        "#.replace("$A",&a.to_string()).replace("$B",&b.to_string()).replace("$C",&c.to_string());
    page.check(&script);
    page.settle();
    page.check("eq(droppedText,'hello','asynchronous drop reading')");
    assert!(
        page.printed().is_empty(),
        "external listeners reported an error"
    );
}

#[test]
fn disclosure_notifications_are_async_coalesced_and_follow_mutations() {
    let mut page = fixture(
        "<details id=a><summary id=s>Summary</summary>content</details><details id=b></details>",
    );
    page.check(
        r#"
        const a=ids('#a'),b=ids('#b');globalThis.disclosureLog=[];
        new MutationObserver(()=>disclosureLog.push('mutation')).observe(a,{attributes:true});
        document.addEventListener('toggle',e=>disclosureLog.push('capture:'+e.target.id),true);
        document.addEventListener('toggle',()=>disclosureLog.push('bubble'));
        for(const d of [a,b])d.ontoggle=e=>{
          assert(e instanceof ToggleEvent,'toggle class');assert(e.isTrusted,'trusted');
          eq([e.bubbles,e.cancelable,e.composed,e.source],[false,false,false,null],'event flags');
          assert(e.target===d,'target');assert(e.currentTarget===d,'current target');
          e.preventDefault();assert(!e.defaultPrevented,'not cancelable');
          disclosureLog.push([d.id,e.oldState,e.newState]);
        };
        a.open=true;b.open=true;a.open=false;a.open=true;
        a.setAttribute('open','false');a.toggleAttribute('open',true);
        eq(disclosureLog,[],'no synchronous toggle');
        Promise.resolve().then(()=>disclosureLog.push('microtask'));
        "#,
    );
    page.settle();
    page.check(r#"
        eq(disclosureLog,['mutation','microtask','capture:b',['b','closed','open'],'capture:a',['a','closed','open']],'coalescing and task rescheduling');
        disclosureLog.length=0;a.removeAttribute('open');a.open=true;a.open=false;
        "#);
    page.settle();
    page.check(
        r#"
        eq(disclosureLog,['mutation','capture:a',['a','open','closed']],'closed state');
        disclosureLog.length=0;a.open=true;a.open=false;
        "#,
    );
    page.settle();
    page.check(r#"
        eq(disclosureLog,['mutation','capture:a',['a','closed','closed']],'net zero changes still notify');
        disclosureLog.length=0;ids('#s').click();assert(a.open,'summary opens immediately');
        eq(disclosureLog,[],'summary toggle is async');
        "#);
    page.settle();
    page.check(
        r#"
        eq(disclosureLog,['mutation','capture:a',['a','closed','open']],'summary notification');
        ids('#s').addEventListener('click',e=>e.preventDefault(),{once:true});
        disclosureLog.length=0;ids('#s').click();assert(a.open,'canceling click prevents change');
        "#,
    );
    page.settle();
    page.check("eq(disclosureLog,[],'no toggle on canceled click')");
    assert!(
        page.printed().is_empty(),
        "disclosure listener reported an error"
    );
}

#[test]
fn disclosure_parser_cloning_insertion_and_named_groups_notify() {
    let mut page = fixture(
        "<details id=a name=g open><summary>A</summary></details><details id=b name=g open><summary>B</summary></details><details id=c></details>",
    );
    page.check(r#"
        const a=ids('#a'),b=ids('#b');globalThis.groupLog=[];
        for(const d of [a,b])d.addEventListener('toggle',e=>groupLog.push([d.id,e.oldState,e.newState]));
        assert(a.open&&!b.open,'parser first member stays open');eq(a.name,'g','name reflection');
        "#);
    page.settle();
    page.check(r#"
        eq(groupLog,[['a','closed','open'],['b','closed','closed']],'parser notifications coalesce');
        groupLog.length=0;b.open=true;assert(b.open&&!a.open,'opening closes existing member');
        const c=ids('#c');c.open=true;c.name='g';assert(!c.open&&b.open,'changing name closes joining member');
        const copy=b.cloneNode(true);copy.id='copy';
        copy.addEventListener('toggle',e=>groupLog.push(['copy',e.oldState,e.newState]));
        const wrapper=document.createElement('section');wrapper.appendChild(copy);
        assert(copy.open,'detached tree has independent group');
        document.body.appendChild(wrapper);assert(!copy.open&&b.open,'subtree insertion closes new member');
        "#);
    page.settle();
    page.check(r#"
        eq(groupLog,[['b','closed','open'],['a','open','closed'],['copy','closed','closed']],'group and clone notifications');
        const detached=document.createElement('details');globalThis.detachedLog=[];
        detached.addEventListener('toggle',e=>detachedLog.push([e.oldState,e.newState]));
        detached.open=true;
        const parsed=document.createElement('section');
        parsed.innerHTML='<details id=fragment name=g open><summary>Fragment</summary></details>';
        const fragment=parsed.firstElementChild;
        fragment.addEventListener('toggle',e=>groupLog.push(['fragment',e.oldState,e.newState]));
        document.body.appendChild(parsed);assert(!fragment.open,'fragment insertion respects group');
        "#);
    page.settle();
    page.check(
        r#"
        eq(detachedLog,[['closed','open']],'detached notification');
        eq(groupLog.at(-1),['fragment','closed','closed'],'parsed fragment notification');
        "#,
    );
    assert!(
        page.printed().is_empty(),
        "group listener reported an error"
    );
}

#[test]
fn toggle_events_have_readonly_states_and_support_reentrant_changes() {
    let mut page = fixture("<details id=a></details>");
    page.check(r#"
        const a=ids('#a'),empty=new ToggleEvent('toggle');
        eq([empty.oldState,empty.newState,empty.source,empty.isTrusted],['','',null,false],'defaults');
        eq(Object.prototype.toString.call(empty),'[object ToggleEvent]','event tag');
        const event=new ToggleEvent('toggle',{oldState:0,newState:null,source:a,bubbles:true});
        eq([event.oldState,event.newState,event.source===a,event.bubbles],['0','null',true,true],'dictionary coercion');
        throws(()=>new ToggleEvent(),'TypeError','missing type');
        throws(()=>new ToggleEvent('toggle',{source:document}),'TypeError','source type');
        eq(new ToggleEvent('toggle',null).oldState,'','null dictionary');
        eq(new ToggleEvent('toggle',{source:undefined}).source,null,'undefined source');
        throws(()=>{ 'use strict';event.oldState='changed' },'TypeError','readonly old state');
        throws(()=>{ 'use strict';event.newState='changed' },'TypeError','readonly new state');
        throws(()=>{ 'use strict';event.source=null },'TypeError','readonly source');
        globalThis.reentrantLog=[];
        a.ontoggle=e=>{
          reentrantLog.push([e.oldState,e.newState]);
          if(reentrantLog.length===1)a.open=false;
        };
        a.open=true;
        "#);
    page.settle();
    page.check("eq(reentrantLog,[['closed','open'],['open','closed']],'listener changes notify in fresh task')");
    assert!(
        page.printed().is_empty(),
        "reentrant listener reported an error"
    );
}

#[test]
fn a_disclosure_listener_can_coalesce_another_already_due_notification() {
    let mut page = fixture("<details id=a></details><details id=b></details>");
    page.check(
        r#"
        const a=ids('#a'),b=ids('#b');globalThis.dueLog=[];
        a.ontoggle=()=>{dueLog.push('a');b.open=false};
        b.ontoggle=e=>dueLog.push([e.oldState,e.newState]);
        a.open=true;b.open=true;
        "#,
    );
    page.settle();
    page.check("eq(dueLog,['a',['closed','closed']],'old queued task is canceled')");
    assert!(page.printed().is_empty(), "due listener reported an error");
}

#[test]
fn drag_data_views_are_protected_readonly_and_disconnected_between_events() {
    let mut page = fixture("<div id=source draggable=true>drag</div><div id=target>drop</div>");
    let source = node(&mut page, "#source");
    let target = node(&mut page, "#target");
    let script = r#"
        const source=ids('#source'), target=ids('#target');
        const held=[], modes=[], file=new File(['hello'],'a.txt',{type:'text/plain'});
        globalThis.transferResults={strings:[],forbiddenCallbacks:0};
        const remember=d=>held.push([d,d.items,d.files,d.items[0],d.items[1]]);
        const protectedView=e=>{
          const d=e.dataTransfer; modes.push(e.type);
          eq(d.types,['text/plain','Files'],'protected types');
          eq([...d.items].map(item=>[item.kind,item.type]),[['string','text/plain'],['file','text/plain']],'protected metadata');
          eq(d.getData('text'),'','protected string');eq(d.files.length,0,'protected files');
          eq(d.items[1].getAsFile(),null,'protected file item');
          d.items[0].getAsString(()=>transferResults.forbiddenCallbacks++);
          d.setData('text','changed');d.clearData();d.items.clear();
          eq(d.items.add('new','other'),null,'protected add');
          throws(()=>d.items.remove(0),'InvalidStateError','protected remove');
          d.effectAllowed='link';eq(d.effectAllowed,'move','protected effects');
          remember(d);
        };
        source.addEventListener('dragstart',e=>{
          const d=e.dataTransfer;d.effectAllowed='move';d.setData('text','original');d.items.add(file);
          eq(d.getData('text'),'original','writable');eq(d.files[0],file,'writable file');
          assert(d.items===d.items,'same item list');assert(d.files===d.files,'same file list');
          d.effectAllowed='invalid';eq(d.effectAllowed,'move','invalid allowed effect');
          remember(d);
        });
        source.addEventListener('drag',protectedView);
        source.addEventListener('dragend',e=>{protectedView(e);eq(e.dataTransfer.dropEffect,'move','final effect')});
        target.addEventListener('dragenter',protectedView);
        target.addEventListener('dragover',e=>{protectedView(e);e.preventDefault()});
        target.addEventListener('drop',e=>{
          const d=e.dataTransfer;modes.push('drop');
          eq(d.getData('text'),'original','read-only string');eq(d.files[0],file,'read-only file');
          assert(d.files instanceof FileList,'file list type');eq(d.files.item(0),file,'item');eq(d.files.item(9),null,'missing item');
          eq(d.items[1].getAsFile(),file,'read-only file item');
          d.items[0].getAsString(value=>transferResults.strings.push(value));
          d.setData('text','changed');d.clearData('text');d.items.clear();
          eq(d.items.add(file),null,'read-only add');throws(()=>d.items.remove(0),'InvalidStateError','read-only remove');
          eq(d.getData('text'),'original','writes ignored');eq(d.files.length,1,'file clear ignored');
          d.effectAllowed='all';eq(d.effectAllowed,'move','read-only effects');
          d.dropEffect='move';d.dropEffect='invalid';eq(d.dropEffect,'move','invalid drop effect');
          remember(d);e.preventDefault();
        });
        web.pointer('down',$SOURCE,{button:0,clientX:0,clientY:0});
        web.pointer('move',$TARGET,{buttons:1,clientX:20,clientY:0});
        web.pointer('up',$TARGET,{button:0});
        assert(modes.includes('drag')&&modes.includes('dragenter')&&modes.includes('dragover')&&modes.includes('drop')&&modes.includes('dragend'),'all modes');
        assert(new Set(held.map(view=>view[0])).size===held.length,'event views are distinct');
        for(const [d,items,files,stringItem,fileItem] of held){
          eq(d.types,[],'disconnected types');eq(d.getData('text'),'','disconnected data');
          eq(items.length,0,'disconnected items');eq(files.length,0,'disconnected files');
          eq([stringItem.kind,stringItem.type,fileItem.kind,fileItem.type],['','','',''],'disconnected metadata');
          eq(fileItem.getAsFile(),null,'disconnected file');
          stringItem.getAsString(()=>transferResults.forbiddenCallbacks++);
          d.setData('text','changed');d.clearData();items.clear();eq(items.add(file),null,'disconnected add');
          throws(()=>items.remove(0),'InvalidStateError','disconnected remove');
        }
        "#.replace("$SOURCE", &source.to_string()).replace("$TARGET", &target.to_string());
    page.check(&script);
    page.settle();
    page.check("eq(transferResults,{strings:['original'],forbiddenCallbacks:0},'readable callback captures data before disconnection')");
    assert!(
        page.printed().is_empty(),
        "drag listeners reported an error"
    );
}

#[test]
fn user_drag_decides_what_dragging_drags() {
    let mut page = fixture(
        "<a id=link href=x style='-webkit-user-drag: none'>link</a><div id=box style='-webkit-user-drag: element'>box</div><div id=target>drop</div>",
    );
    let link = node(&mut page, "#link");
    let boxed = node(&mut page, "#box");
    let target = node(&mut page, "#target");
    // The fake page computes what the elements' styles set.
    for (id, value) in [(link, "none"), (boxed, "element")] {
        page.page
            .styles
            .borrow_mut()
            .insert(id, vec![("-webkit-user-drag".to_owned(), value.to_owned())]);
    }
    let script = r#"
        const started = [];
        document.addEventListener('dragstart', (event) => started.push(event.target.id));
        for (const source of [$LINK, $BOX]) {
          web.pointer('down', source, {button:0, clientX:0, clientY:0});
          web.pointer('move', $TARGET, {buttons:1, clientX:20, clientY:0});
          web.pointer('up', $TARGET, {button:0});
        }
        eq(started, ['box'], 'only the element that user-drag lets go drags');
        "#
    .replace("$LINK", &link.to_string())
    .replace("$BOX", &boxed.to_string())
    .replace("$TARGET", &target.to_string());
    page.check(&script);
}

#[test]
fn transfer_item_order_live_files_and_removed_item_references() {
    let mut page = fixture("<div></div>");
    page.check(r#"
        const data=new DataTransfer(), files=data.files, items=data.items;
        throws(()=>new FileList(),'TypeError','illegal FileList constructor');
        eq(Object.prototype.toString.call(files),'[object FileList]','file list tag');
        assert(!Array.isArray(files),'FileList is not an array');
        const file=new File(['hello'],'a.txt',{type:'text/plain'}), fileItem=items.add(file);
        const textItem=items.add('first','text/plain');items.add('html','text/html');
        eq([...items].map(item=>item.kind),['file','string','string'],'insertion order');
        assert(files===data.files,'same live file list');assert(items===data.items,'same live item list');
        eq(files.length,1,'live addition');eq(files[0],file,'index');eq(files[1],undefined,'missing index');eq(files['00'],undefined,'canonical index');
        assert(items[0]!==items[0],'indexed reads create new item wrappers');
        data.setData('text','second');eq([textItem.kind,textItem.type],['',''],'replacement removes old item');
        eq([...items].map(item=>item.type),['text/plain','text/html','text/plain'],'replacement appends its new item');
        globalThis.orderedTransferValues=[];items[2].getAsString(value=>orderedTransferValues.push(value));
        const snapshot=data.types;assert(Object.isFrozen(snapshot),'frozen types');
        data.clearData('text');eq([textItem.kind,textItem.type],['',''],'removed item');
        textItem.getAsString(()=>orderedTransferValues.push('removed'));
        data.setData('text','new');textItem.getAsString(()=>orderedTransferValues.push('resurrected'));
        eq(snapshot,['text/html','text/plain','Files'],'types are a snapshot');
        data.setData('url','# comment\r\n\r\nhttps://one.example/\r\nhttps://two.example/');
        eq(data.getData('URL'),'https://one.example/','legacy url returns first URI');
        data.clearData();eq(data.types,['Files'],'clearData retains files');
        items.remove(-1);eq(files.length,1,'negative index converts to unsigned');
        items.clear();eq(files.length,0,'live clear');eq(fileItem.getAsFile(),null,'removed file item');
        data.effectAllowed='copyMove';eq(data.effectAllowed,'none','constructed transfers have no platform drag effects');
        data.dropEffect='link';eq(data.dropEffect,'none','constructed transfer drop effect');
    "#);
    page.settle();
    page.check("eq(orderedTransferValues,['second'],'async callback captures the replacement before removal')");
    assert!(
        page.printed().is_empty(),
        "transfer callbacks reported an error"
    );
}

#[test]
fn incoming_files_are_visible_only_during_the_drop() {
    let mut page = fixture("<div id=target>drop</div>");
    let target = node(&mut page, "#target");
    page.check(&r#"
        const held=[],target=ids('#target');globalThis.incomingFileText='';
        for(const type of ['dragenter','dragover'])target.addEventListener(type,e=>{
          eq(e.dataTransfer.types,['Files'],'external metadata');eq(e.dataTransfer.files.length,0,'external protected files');
          eq(e.dataTransfer.items[0].kind,'file','external item kind');eq(e.dataTransfer.items[0].getAsFile(),null,'external protected file item');
          held.push(e.dataTransfer);if(type==='dragover')e.preventDefault();
        });
        target.addEventListener('drop',e=>{
          const file=e.dataTransfer.files[0];eq(file.name,'a.txt','name');eq(file.path,'C:/a.txt','Electron path');
          eq([...e.dataTransfer.files],[file],'iterable drop files');eq(e.dataTransfer.items[0].getAsFile(),file,'file item');
          file.text().then(value=>incomingFileText=value);held.push(e.dataTransfer);
        });
        web.dropFiles($TARGET,[{path:'C:/a.txt',bytes:'aGVsbG8='}],{});
        eq(held.length,3,'external event views');for(const data of held)eq([data.types,data.files.length,data.items.length],[[],0,0],'external views expire');
    "#.replace("$TARGET", &target.to_string()));
    page.settle();
    page.check("eq(incomingFileText,'hello','captured File stays readable after drop')");
    assert!(
        page.printed().is_empty(),
        "file-drop listeners reported an error"
    );
}

#[test]
fn drag_lifecycle_cancellation_threshold_and_file_transfers() {
    let mut page = fixture(
        "<div id=source draggable=true><b id=child>doc</b></div><div id=target>folder</div><a id=link href='https://example.com/'>link</a>",
    );
    let source = node(&mut page, "#source");
    let child = node(&mut page, "#child");
    let target = node(&mut page, "#target");
    let link = node(&mut page, "#link");
    page.check(&format!(r#"
        const source=ids('#source'), target=ids('#target'), child=ids('#child');
        eq(source.draggable,true,'enumerated attribute');source.draggable=false;eq(source.draggable,false,'false');source.draggable=true;
        const log=[];
        source.addEventListener('dragstart',e=>{{log.push('start');e.dataTransfer.setData('text','doc');e.dataTransfer.effectAllowed='move'}});
        source.addEventListener('dragend',e=>log.push('end:'+e.dataTransfer.dropEffect));
        target.addEventListener('dragover',e=>e.preventDefault());
        target.addEventListener('drop',e=>log.push('drop:'+e.dataTransfer.getData('text/plain')));
        web.pointer('down',{child},{{button:0,clientX:0,clientY:0}});
        web.pointer('move',{target},{{buttons:1,clientX:2,clientY:0}});eq(log,[],'threshold');
        web.pointer('move',{target},{{buttons:1,clientX:20,clientY:0}});
        web.pointer('up',{target},{{button:0}});eq(log,['start','drop:doc','end:move'],'drop');
        log.length=0;
        web.pointer('down',{source},{{button:0,clientX:0,clientY:0}});web.pointer('move',{target},{{buttons:1,clientX:20,clientY:0}});
        web.key(true,{{key:'Escape'}});eq(log,['start','end:none'],'escape');
        log.length=0;
        web.pointer('down',{source},{{button:0,clientX:0,clientY:0}});web.pointer('move',{target},{{buttons:1,clientX:20,clientY:0}});
        web.pointer('leave',null,{{}});web.pointer('up',null,{{button:0}});eq(log,['start','end:none'],'outside');
        const cancel=e=>e.preventDefault();source.addEventListener('dragstart',cancel);
        log.length=0;web.pointer('down',{source},{{button:0,clientX:0,clientY:0}});web.pointer('move',{target},{{buttons:1,clientX:20,clientY:0}});web.pointer('up',{target},{{button:0}});
        eq(log,['start'],'canceled start');
        ids('#link').addEventListener('dragstart',e=>{{eq(e.dataTransfer.getData('url'),'https://example.com/','link url');e.preventDefault()}});
        web.pointer('down',{link},{{button:0,clientX:0,clientY:0}});web.pointer('move',{target},{{buttons:1,clientX:20,clientY:0}});
        const dt=new DataTransfer(), file=new File(['hello'],'a.txt',{{type:'text/plain'}});
        dt.items.add(file);dt.items.add('text','text/plain');eq(dt.types,['text/plain','Files'],'formats');
        eq(dt.items.length,2,'items');eq([...dt.files],[file],'files');eq(dt.items[0].getAsFile(),file,'file item');eq(dt.items[1].getAsFile(),null,'text item');
        dt.items[1].getAsString(value=>eq(value,'text','text item async'));
        let threw=false;try{{dt.items.add('again','text/plain')}}catch(e){{threw=e.name==='NotSupportedError'}}assert(threw,'duplicate string');
        dt.items.remove(1);eq(dt.items.length,1,'remove text');dt.items.remove(0);eq(dt.files.length,0,'remove file');
        dt.items.add(file);dt.items.clear();eq(dt.items.length,0,'clear');
        const synthetic=new File([],'synthetic.txt');eq(synthetic.path,undefined,'synthetic has no path');
        web.dropFiles({target},[{{path:'C:\\drop\\a.txt',bytes:'aGVsbG8='}}],{{clientX:2}});
        eq(log.at(-1),'drop:','external drop');
        web.dropFiles({source},[{{path:'C:\\drop\\a.txt',bytes:''}}],{{}});
    "#));
}

const PAGE: &str = r##"<div id=outer><div id=inner>text</div><button id=b>B</button>
<form id=f><input id=t name=t><input id=c type=checkbox><input id=r1 type=radio name=r checked><input id=r2 type=radio name=r>
<label id=l>Label <input id=lc type=checkbox></label><select id=s><option>a</option><option>b</option></select><input id=range type=range>
<textarea id=ta></textarea><button id=submit>Go</button><button id=reset type=reset>R</button></form>
<form id=g><input id=only></form><form id=h><input id=x><button id=disabled disabled>D</button></form>
<a id=hash href="#section">hash</a><a id=same href="index.html#">same</a><a id=away href="https://example.com/">away</a><a id=blank href="other.html" target=_blank>blank</a>
<details id=d><summary id=sum>S</summary>more</details><div id=focusable tabindex=0>F</div></div>"##;

/// The node id of the element matching a selector.
fn node(page: &mut Fixture, selector: &str) -> u32 {
    let value = page.eval(&format!(
        "(() => {{ const element = document.querySelector('{selector}'); return element[Object.getOwnPropertySymbols(element).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')]; }})()"
    ));
    value.parse().unwrap()
}

#[test]
fn pointer_input_and_hover() {
    let mut page = fixture(PAGE);
    let inner = node(&mut page, "#inner");
    let button = node(&mut page, "#b");
    let focusable = node(&mut page, "#focusable");
    page.check(&format!(
        r#"
        const log = []; const inner = ids('#inner'); const outer = ids('#outer');
        for (const type of ['pointerover', 'mouseover', 'pointerenter', 'mouseenter', 'pointerout', 'mouseout', 'pointerleave', 'mouseleave', 'mousemove', 'pointermove', 'mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'auxclick']) {{
          outer.addEventListener(type, (event) => log.push(type + ':' + event.target.id), true);
        }}
        eq(web.pointer('move', {inner}, {{ clientX: 5 }}), false, 'move');
        eq(log, ['pointerover:inner', 'mouseover:inner', 'pointerenter:outer', 'mouseenter:outer', 'pointerenter:inner', 'mouseenter:inner', 'pointermove:inner', 'mousemove:inner'], 'enter');
        log.length = 0; web.pointer('move', {button}, {{}});
        eq(log, ['pointerout:inner', 'mouseout:inner', 'pointerleave:inner', 'mouseleave:inner', 'pointerover:b', 'mouseover:b', 'pointerenter:b', 'mouseenter:b', 'pointermove:b', 'mousemove:b'], 'move over');
        log.length = 0; eq(web.pointer('leave', null, {{}}), false, 'leave'); eq(log.length, 6, 'left');
        log.length = 0; web.pointer('down', {button}, {{ button: 0 }}); eq(document.activeElement, ids('#b'), 'focused by press');
        web.pointer('up', {button}, {{ button: 0 }}); eq(web.pointer('click', {button}, {{ button: 0 }}), false, 'click');
        web.pointer('click', {button}, {{ button: 2 }}); web.pointer('click', {button}, {{ button: 1 }}); web.pointer('dblclick', {button}, {{}}); eq(web.pointer('other', {button}, {{}}), false, 'unknown');
        assert(log.includes('click:b') && log.includes('contextmenu:b') && log.includes('auxclick:b') && log.includes('dblclick:b'), 'clicks');
        web.pointer('down', {inner}, {{ button: 0 }}); eq(document.activeElement, document.body, 'blurred by press');
        web.pointer('down', {inner}, {{ button: 0 }}); web.pointer('down', {focusable}, {{ button: 2 }}); eq(document.activeElement, document.body, 'right button keeps focus');
        outer.addEventListener('mousedown', (event) => event.preventDefault()); eq(web.pointer('down', {focusable}, {{}}), true, 'cancelled press'); eq(document.activeElement, document.body, 'no focus');
        outer.addEventListener('mousemove', (event) => event.preventDefault()); eq(web.pointer('move', {focusable}, {{}}), true, 'cancelled move');
        outer.addEventListener('mouseup', (event) => event.preventDefault()); eq(web.pointer('up', {focusable}, {{}}), true, 'cancelled up');
        eq(web.wheel({inner}, {{ deltaY: 100 }}), false, 'wheel');
        outer.addEventListener('wheel', (event) => event.preventDefault()); eq(web.wheel({inner}, {{ deltaY: 100 }}), true, 'cancelled wheel');
        "#
    ));
}

#[test]
fn keys_and_implicit_submission() {
    let mut page = fixture(PAGE);
    let text = node(&mut page, "#t");
    let only = node(&mut page, "#only");
    let x = node(&mut page, "#x");
    page.check(&format!(
        r#"
        const log = []; document.addEventListener('keydown', (event) => log.push('down:' + event.key + ':' + event.keyCode + ':' + event.target.nodeName));
        document.addEventListener('keypress', (event) => log.push('press:' + event.charCode)); document.addEventListener('keyup', (event) => log.push('up:' + event.key));
        eq(web.key(true, {{ key: 'a', code: 'KeyA' }}), false, 'down'); eq(web.key(false, {{ key: 'a' }}), false, 'up');
        web.key(true, {{ key: 'Shift' }}); web.key(true, {{ key: 'c', ctrlKey: true }}); web.key(true, {{ key: 'ArrowUp' }}); web.key(true, {{}});
        eq(log, ['down:a:65:BODY', 'press:97', 'up:a', 'down:Shift:16:BODY', 'down:c:67:BODY', 'down:ArrowUp:38:BODY', 'down::0:BODY'], 'keys');
        const codes = []; const record = (event) => codes.push(event.keyCode); document.addEventListener('keydown', record);
        web.key(true, {{ key: 'End', code: 'End' }}); web.key(true, {{ key: ';', code: 'Semicolon' }}); web.key(true, {{ key: '!', code: 'Digit1', shiftKey: true }});
        web.key(true, {{ key: '5', code: 'Numpad5' }}); web.key(true, {{ key: 'F5', code: 'F5' }}); web.key(true, {{ key: 'Home' }});
        web.key(true, {{ key: 'End', code: 'Numpad1' }}); web.key(true, {{ key: 'Delete', code: 'NumpadDecimal' }}); web.key(true, {{ key: '.', code: 'NumpadDecimal' }});
        document.removeEventListener('keydown', record); eq(codes, [35, 186, 49, 101, 116, 36, 35, 46, 110], 'physical key codes');
        const submits = []; for (const form of document.forms) form.addEventListener('submit', (event) => {{ submits.push(form.id + ':' + (event.submitter ? event.submitter.id : '')); event.preventDefault(); }});
        web.focus({text}); eq(document.activeElement.id, 't', 'focused'); web.key(true, {{ key: 'Enter' }}); eq(submits, ['f:submit'], 'submit button');
        web.focus({only}); web.key(true, {{ key: 'Enter' }}); eq(submits[1], 'g:', 'no button');
        web.focus({x}); web.key(true, {{ key: 'Enter' }}); eq(submits.length, 2, 'disabled submitter');
        document.addEventListener('keypress', (event) => event.preventDefault()); eq(web.key(true, {{ key: 'Enter' }}), true, 'cancelled press');
        document.addEventListener('keydown', (event) => event.preventDefault()); eq(web.key(true, {{ key: 'b' }}), true, 'cancelled down');
        document.addEventListener('keyup', (event) => event.preventDefault()); eq(web.key(false, {{ key: 'b' }}), true, 'cancelled up');
        web.focus(null); eq(document.activeElement, document.body, 'blurred'); web.focus(null);
        "#
    ));
}

#[test]
fn sequential_focus_and_keyboard_activation() {
    let mut page = fixture(
        "<button id=one>1</button><a id=link href='#here'>link</a><input id=field><div id=plain>x</div>\
         <div id=late tabindex=0>late</div><div id=first tabindex=2>first</div><div id=early tabindex=1>early</div>\
         <div id=skipped tabindex=-1>no</div><button id=off disabled>off</button><div hidden><button id=gone>gone</button></div>\
         <input id=submit type=submit value=Go><button id=inner><span id=label>in</span></button>",
    );
    page.check(
        r#"
        const order = []; const tab = (shift) => { web.key(true, { key: 'Tab', shiftKey: shift }); order.push(document.activeElement.id); };
        for (let i = 0; i < 9; i++) tab(false);
        eq(order, ['early', 'first', 'one', 'link', 'field', 'late', 'submit', 'inner', 'early'], 'forward');
        order.length = 0; tab(true); tab(true); eq(order, ['inner', 'submit'], 'backward');
        web.focus(null); order.length = 0; tab(true); eq(order, ['inner'], 'from nothing backward');
        eq(web.key(true, { key: 'Tab', ctrlKey: true }), false, 'ctrl+tab is not navigation');
        const clicks = []; document.addEventListener('click', (event) => clicks.push(event.target.id));
        document.getElementById('one').focus(); eq(web.key(true, { key: 'Enter' }), true, 'enter activates');
        web.key(true, { key: ' ' }); web.key(false, { key: ' ' });
        document.getElementById('link').focus(); web.key(true, { key: 'Enter' }); web.key(false, { key: ' ' });
        document.getElementById('submit').focus(); web.key(false, { key: ' ' });
        document.getElementById('plain').tabIndex = 0; document.getElementById('plain').focus(); web.key(true, { key: 'Enter' }); web.key(false, { key: ' ' });
        // Native controls activate themselves.
        document.getElementById('one').focus(); eq(web.key(true, { key: 'Enter' }, true), false, 'native enter'); web.key(false, { key: ' ' }, true);
        eq(clicks, ['one', 'one', 'link', 'submit'], 'activated');
        // Disabled controls and their content get no clicks.
        clicks.length = 0; document.getElementById('off').click(); web.pointer('click', ids('#off')[Object.getOwnPropertySymbols(ids('#off')).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')], {});
        document.getElementById('inner').disabled = true; document.getElementById('label').click(); eq(clicks, [], 'disabled');
        "#,
    );
    // Without focusable elements Tab is left to the platform.
    let mut empty = fixture("<p>text</p>");
    empty.check("eq(web.key(true, { key: 'Tab' }), false, 'nothing to focus');");
}

#[test]
fn drawn_selects_open_and_follow_keys() {
    let mut page = fixture(
        "<select id=s><option>a</option><option disabled>b</option><option>c</option><option>d</option></select>\
         <select id=m multiple><option>x</option></select><select id=big size=3><option>y</option></select>\
         <select id=none></select><select id=off><option>z</option></select>",
    );
    let select = node(&mut page, "#s");
    let multiple = node(&mut page, "#m");
    let big = node(&mut page, "#big");
    page.check(&format!(
        r#"
        const s = ids('#s'); const log = [];
        for (const type of ['input', 'change']) s.addEventListener(type, () => log.push(type + ':' + s.value));
        // Clicks open drop-downs; scripts cannot.
        web.pointer('click', {select}, {{ button: 0 }}); s.click();
        web.pointer('click', {multiple}, {{ button: 0 }}); web.pointer('click', {big}, {{ button: 0 }});
        const key = (key, init = {{}}) => web.key(true, {{ key, ...init }});
        s.focus();
        eq(key('ArrowDown'), true, 'down'); eq(s.value, 'c', 'skips disabled options');
        key('ArrowRight'); key('End'); eq(s.value, 'd', 'last');
        eq(log, ['input:c', 'change:c', 'input:d', 'change:d'], 'events once per change');
        key('Home'); key('ArrowUp'); eq(s.value, 'a', 'first');
        key('ArrowDown'); key('ArrowLeft'); eq(s.value, 'a', 'back');
        eq(key('x'), false, 'other keys');
        key('Enter'); key(' '); key('F4'); key('ArrowDown', {{ altKey: true }}); key('ArrowUp', {{ altKey: true }});
        eq(s.value, 'a', 'opening keeps the value');
        eq(web.key(true, {{ key: 'ArrowDown' }}, true), false, 'native select');
        eq(s.value, 'a', 'native select keys');
        ids('#none').focus(); eq(key('ArrowDown'), false, 'no options');
        ids('#big').focus(); eq(key('Enter'), false, 'list box');
        const off = ids('#off'); off.focus(); off.disabled = true; eq(key('ArrowDown'), false, 'disabled');
        web.pointer('click', {select}, {{ button: 0, detail: 1 }});
        "#
    ));
    let opened = page
        .page
        .calls
        .borrow()
        .iter()
        .filter(|call| call.starts_with("select "))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(opened, vec![format!("select {select}"); 7]);
}

#[test]
fn typing_selecting_and_activation() {
    let mut page = fixture(PAGE);
    let text = node(&mut page, "#t");
    let select = node(&mut page, "#s");
    let range = node(&mut page, "#range");
    let checkbox = node(&mut page, "#c");
    let radio = node(&mut page, "#r2");
    let label = node(&mut page, "#l");
    let submit = node(&mut page, "#submit");
    let reset = node(&mut page, "#reset");
    let summary = node(&mut page, "#sum");
    let hash = node(&mut page, "#hash");
    let same = node(&mut page, "#same");
    let away = node(&mut page, "#away");
    let blank = node(&mut page, "#blank");
    let disabled = node(&mut page, "#disabled");
    page.check(&format!(
        r#"
        const log = []; const record = (event) => log.push(event.type + ':' + event.target.id);
        for (const type of ['input', 'change', 'click', 'submit', 'reset', 'toggle', 'focus', 'blur']) document.addEventListener(type, record, true);
        const t = ids('#t');
        let tracked = ''; Object.defineProperty(t, 'value', {{ get() {{ return tracked; }}, set(value) {{ tracked = value; }}, configurable: true }});
        web.focus({text}); web.input({text}, 'typed'); eq(tracked, '', 'setter bypassed'); delete t.value; eq(t.value, 'typed', 'native value');
        web.focus(null); eq(log, ['focus:t', 'input:t', 'blur:t', 'change:t'], 'change on blur'); log.length = 0;
        web.focus({text}); web.focus(null); eq(log, ['focus:t', 'blur:t'], 'no change without edit'); log.length = 0;
        web.select({select}, [1]); eq(ids('#s').value, 'b', 'selected'); eq(log, ['input:s', 'change:s'], 'select events'); log.length = 0;
        web.value({range}, 30); eq(ids('#range').value, '30', 'range'); eq(log, ['input:range', 'change:range'], 'range events'); log.length = 0;
        eq(web.activate({checkbox}), true, 'activate'); assert(ids('#c').checked, 'checked'); eq(log, ['click:c', 'input:c', 'change:c'], 'checkbox events'); log.length = 0;
        ids('#c').addEventListener('click', (event) => event.preventDefault()); eq(web.activate({checkbox}), false, 'cancelled'); assert(ids('#c').checked, 'restored'); log.length = 0;
        ids('#c').indeterminate = true; ids('#c').click(); assert(ids('#c').indeterminate, 'indeterminate restored');
        web.activate({radio}); assert(ids('#r2').checked && !ids('#r1').checked, 'radio'); eq(log.slice(-2), ['input:r2', 'change:r2'], 'radio events'); log.length = 0;
        web.activate({radio}); eq(log, ['click:r2'], 'already checked radio');
        ids('#r1').addEventListener('click', (event) => event.preventDefault()); ids('#r1').click(); assert(ids('#r2').checked && !ids('#r1').checked, 'radio restored');
        const lone = document.createElement('input'); lone.type = 'radio'; document.body.appendChild(lone); lone.addEventListener('click', (event) => event.preventDefault()); lone.click(); assert(!lone.checked, 'lone radio restored');
        const ungrouped = document.createElement('input'); ungrouped.type = 'radio'; ungrouped.name = ''; document.body.appendChild(ungrouped); ungrouped.click(); assert(ungrouped.checked, 'ungrouped radio');
        log.length = 0; web.activate({label}); assert(ids('#lc').checked, 'label activates control'); eq(log, ['click:l', 'click:lc', 'input:lc', 'change:lc'], 'label events');
        log.length = 0; ids('#lc').click(); eq(log.filter((entry) => entry.startsWith('click')).length, 1, 'no loop through label');
        document.addEventListener('submit', (event) => event.preventDefault()); log.length = 0; web.activate({submit}); eq(log, ['click:submit', 'submit:f'], 'submit');
        ids('#t').value = 'x'; web.activate({reset}); eq(ids('#t').value, '', 'reset'); web.activate({disabled}); const orphan = document.createElement('button'); orphan.type = 'reset'; orphan.click();
        const formless = document.createElement('button'); document.body.appendChild(formless); formless.click();
        web.activate({summary}); assert(ids('#d').open, 'details opened'); ids('#sum').click(); assert(!ids('#d').open, 'details closed');
        const floating = document.createElement('summary'); floating.click();
        const hashes = []; window.addEventListener('hashchange', (event) => hashes.push(event.newURL)); window.addEventListener('popstate', () => hashes.push('pop'));
        web.activate({hash}); eq(location.hash, '#section', 'hash'); eq(hashes, ['pop', 'file:///C:/app/index.html#section'], 'hash events');
        web.activate({same}); eq(location.hash, '', 'same document');
        const opened = []; web.onopen = (url, target) => opened.push(url + ' ' + target); web.onnavigate = (url) => opened.push('nav ' + url);
        web.activate({blank}); web.activate({away}); eq(opened, ['file:///C:/app/other.html _blank', 'nav https://example.com/'], 'navigation');
        delete web.onopen; delete web.onnavigate; web.activate({blank}); web.activate({away});
        const plain = document.createElement('a'); plain.click(); web.activate({hash});
        "#
    ));
}

#[test]
fn page_lifecycle_and_frames() {
    let page_state = FakePage::default();
    page_state.dark.set(false);
    let mut page = fixture_with("<p id=p autofocus tabindex=0>x</p>", page_state);
    page.check(
        r#"
        const log = []; document.addEventListener('readystatechange', () => log.push(document.readyState));
        document.addEventListener('DOMContentLoaded', () => log.push('dom')); window.addEventListener('load', () => log.push('load'));
        eq(document.readyState, 'loading', 'loading'); web.ready(); eq(log, ['interactive', 'dom', 'complete', 'load'], 'ready'); eq(document.activeElement.id, 'p', 'autofocus');
        web.ready();
        const dark = matchMedia('(prefers-color-scheme: dark)'); eq(dark.matches, false, 'light'); eq(dark.media, '(prefers-color-scheme: dark)', 'media');
        const changes = []; dark.addListener((event) => changes.push('listener')); dark.onchange = () => changes.push('onchange');
        window.addEventListener('resize', () => changes.push('resize')); globalThis.darkList = dark; globalThis.mediaChanges = changes;
        throws(() => new MediaQueryList(), 'TypeError', 'illegal list');
        let frameTime = null; const frame = requestAnimationFrame((time) => { frameTime = time; }); const dropped = requestAnimationFrame(() => { throw new Error('never'); });
        cancelAnimationFrame(dropped); assert(web.hasFrames(), 'frames'); web.frame(12); eq(frameTime, 12, 'frame time'); assert(!web.hasFrames(), 'no frames');
        requestAnimationFrame(() => { throw new Error('frame failed'); }); web.frame(); throws(() => requestAnimationFrame(1), 'TypeError', 'bad frame');
        web.scroll(null); web.scroll(document.documentElement[Object.getOwnPropertySymbols(document.documentElement)[0]]); web.scroll(ids('#p')[Object.getOwnPropertySymbols(ids('#p'))[0]]);
        let leaving = true; window.addEventListener('beforeunload', (event) => { if (!leaving) event.preventDefault(); }); assert(web.beforeUnload(), 'leave');
        leaving = false; assert(!web.beforeUnload(), 'stay'); const unloads = []; window.addEventListener('pagehide', () => unloads.push('hide')); window.onunload = () => unloads.push('unload');
        web.unload(); eq(unloads, ['hide', 'unload'], 'unload');
        "#,
    );
    page.page.dark.set(true);
    page.check("web.resize(); eq(mediaChanges, ['resize', 'listener', 'onchange'], 'media change'); web.resize(); eq(mediaChanges.length, 4, 'no second change');");
    page.check("darkList.removeListener(() => {});");
    assert!(
        page.printed()
            .iter()
            .any(|line| line.starts_with("Uncaught Error: frame failed"))
    );
}

#[test]
fn uncancelled_context_clicks_describe_their_target_to_the_application() {
    let mut page = fixture(
        "<div id=rich contenteditable><b id=word>bold</b> text</div><textarea id=area>abcdef</textarea>\
         <a id=link href=\"https://example.com/x\">go</a><img id=pic alt=Pic title=Tip><p id=plain>plain</p>",
    );
    let host = page.host.host_object();
    page.host
        .context()
        .global_object()
        .set(js_string!("hostObject"), host, false, page.host.context())
        .unwrap();
    let [word, area, link, pic, plain] =
        ["#word", "#area", "#link", "#pic", "#plain"].map(|selector| node(&mut page, selector));
    page.check(&format!(
        r#"
        const menus = []; hostObject.contextMenu = (params) => menus.push(params);
        const range = document.createRange(); range.selectNodeContents(ids('#word').firstChild);
        getSelection().removeAllRanges(); getSelection().addRange(range);
        eq(web.pointer('click', {word}, {{ button: 2, clientX: 10.4, clientY: 20.6 }}), false, 'rich');
        const rich = menus.pop();
        eq([rich.x, rich.y, rich.isEditable, rich.selectionText, rich.formControlType, rich.mediaType, rich.menuSourceType], [10, 21, true, 'bold', 'none', 'none', 'mouse'], 'rich params');
        eq(rich.editFlags, {{ canUndo: true, canRedo: true, canCut: true, canCopy: true, canPaste: true, canDelete: true, canSelectAll: true, canEditRichly: true }}, 'rich flags');
        eq(rich.inputFieldType, undefined, 'no input field type');
        const area = ids('#area'); area.setSelectionRange(1, 3);
        web.pointer('click', {area}, {{ button: 2 }});
        const field = menus.pop();
        eq([field.isEditable, field.selectionText, field.formControlType, field.editFlags.canEditRichly, field.spellcheckEnabled], [true, 'bc', 'text-area', false, true], 'field');
        getSelection().removeAllRanges();
        web.pointer('click', {link}, {{ button: 2 }});
        const anchor = menus.pop();
        eq([anchor.linkURL, anchor.linkText, anchor.isEditable, anchor.editFlags.canCopy, anchor.editFlags.canPaste], ['https://example.com/x', 'go', false, false, false], 'link');
        web.pointer('click', {pic}, {{ button: 2 }});
        const image = menus.pop();
        eq([image.mediaType, image.altText, image.titleText, image.hasImageContents, image.mediaFlags.canSave], ['image', 'Pic', 'Tip', false, true], 'image');
        // Pages that cancel the event keep the application's menu closed.
        ids('#plain').addEventListener('contextmenu', (event) => event.preventDefault());
        eq(web.pointer('click', {plain}, {{ button: 2 }}), true, 'cancelled');
        eq(menus.length, 0, 'no menu');
        delete hostObject.contextMenu;
        eq(web.pointer('click', {link}, {{ button: 2 }}), false, 'without an application');
        "#
    ));
}
