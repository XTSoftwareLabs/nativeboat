use super::*;

#[test]
fn range_boundaries_follow_order_roots_and_utf16_offsets() {
    let mut page = fixture("<p id=p>a😀<b id=b>bc</b>d</p><p id=q>ef</p>");
    page.check(r#"
        const p=ids('#p'), b=ids('#b'), q=ids('#q'), a=p.firstChild, c=b.firstChild;
        const range=new Range();
        eq([range.startContainer===document,range.startOffset,range.collapsed], [true,0,true], 'initial');
        range.setStart(a,1); eq([range.startContainer===a,range.endContainer===a,range.endOffset,range.collapsed],[true,true,1,true],'start collapses past end');
        range.setEnd(c,1); eq([String(range),range.collapsed,range.commonAncestorContainer===p],['😀b',false,true],'UTF16 and nested text');
        range.setEnd(a,0); eq([range.startOffset,range.endOffset,range.collapsed],[0,0,true],'end before start collapses');
        range.selectNodeContents(a); eq([range.startOffset,range.endOffset,String(range)],[0,3,'a😀'],'text contents');
        const clone=range.cloneRange(); range.collapse(); eq([range.startOffset,range.endOffset,clone.endOffset,clone.collapsed],[3,3,3,false],'collapse end and clone independence');
        clone.collapse(true); eq([clone.startOffset,clone.endOffset,clone.collapsed],[0,0,true],'collapse start');
        range.selectNode(b); eq([range.startContainer===p,range.startOffset,range.endOffset,String(range)],[true,1,2,'bc'],'select node includes parent boundaries');
        range.selectNodeContents(p); eq([range.endOffset,String(range)],[3,'a😀bcd'],'element contents');
        range.setStartBefore(b); range.setEndAfter(b); eq(String(range),'bc','before after');
        range.setStartAfter(b); range.setEndBefore(q); eq(String(range),'d','ancestor and sibling ordering');
        range.setEndBefore(b); eq(range.collapsed,true,'ancestor offset precedes start');
        const detached=document.createTextNode('xy'); range.setStart(detached,1); eq([range.endContainer===detached,range.endOffset],[true,1],'different root resets end');
        range.setEnd(a,2); eq([range.startContainer===a,range.startOffset],[true,2],'different root resets start');
        range.selectNodeContents(document.createComment('abc')); eq(String(range),'','comments excluded');
        throws(()=>range.setStart(a,4),'IndexSizeError','text offset');
        throws(()=>range.setEnd(p,-1),'IndexSizeError','negative unsigned offset');
        throws(()=>range.setStart({},0),'TypeError','non-node');
        throws(()=>range.setEnd(a,1n),'TypeError','bigint offset');
        throws(()=>range.selectNode(detached),'InvalidNodeTypeError','detached node');
        throws(()=>range.setStartBefore(document),'InvalidNodeTypeError','root has no sibling boundary');
        const doctype=document.createComment('');Object.defineProperty(doctype,'nodeType',{value:10});
        throws(()=>range.selectNodeContents(doctype),'InvalidNodeTypeError','document type boundary validation');
        range.setStart(a,NaN); range.setEnd(a,'2'); eq([range.startOffset,range.endOffset],[0,2],'IDL conversions');
    "#);
}

#[test]
fn selection_retains_range_identity_and_directional_endpoints() {
    let mut page = fixture("<p id=p>abcdef<b id=b>XYZ</b></p>");
    page.check(r#"
        const s=getSelection(), a=ids('#p').firstChild, b=ids('#b').firstChild;
        assert(s instanceof Selection,'global Selection prototype');
        throws(()=>new Selection(),'TypeError','illegal constructor');
        eq([s.rangeCount,s.anchorNode,s.focusNode,s.anchorOffset,s.focusOffset,s.type,s.direction,s.isCollapsed],[0,null,null,0,0,'None','none',true],'empty state');
        const range=document.createRange();range.setStart(a,1);range.setEnd(a,4);s.addRange(range);
        assert(s.getRangeAt(0)===range,'range association is a reference');
        eq([s.anchorNode===a,s.anchorOffset,s.focusNode===a,s.focusOffset,s.type,String(s)],[true,1,true,4,'Range','bcd'],'added range');
        const ignored=new Range();ignored.selectNodeContents(ids('#b'));s.addRange(ignored);assert(s.getRangeAt(0)===range,'single range ignores second');
        range.setEnd(a,5);eq(s.focusOffset,5,'associated range updates endpoints');
        s.extend(a,0); eq([s.anchorOffset,s.focusOffset,s.direction,String(s)],[1,0,'backward','a'],'extend backward preserves anchor');
        assert(s.getRangeAt(0)!==range && range.endOffset===5,'extend replaces range');
        const backwards=s.getRangeAt(0);backwards.setEnd(a,3);eq([s.anchorOffset,s.focusOffset,s.direction],[3,0,'backward'],'range mutation preserves direction');
        s.extend(b,2);eq([s.anchorNode===a,s.anchorOffset,s.focusNode===b,s.focusOffset,s.direction,String(s)],[true,3,true,2,'forward','defXY'],'cross-node extension');
        const previous=s.getRangeAt(0);s.collapseToStart();eq([s.anchorOffset,s.focusOffset,s.type,s.isCollapsed],[3,3,'Caret',true],'collapse to start');
        assert(s.getRangeAt(0)!==previous && !previous.collapsed,'collapse preserves old range');
        s.setBaseAndExtent(b,2,a,2);eq([s.anchorNode===b,s.anchorOffset,s.focusNode===a,s.focusOffset,s.direction,String(s)],[true,2,true,2,'backward','cdefXY'],'base focus ordered independently');
        s.collapseToEnd();eq([s.anchorNode===b,s.anchorOffset,s.focusOffset],[true,2,2],'collapse to end');
        assert(s.empty===s.removeAllRanges && s.setPosition===s.collapse,'aliases');
        s.setPosition(a,2);eq([s.anchorOffset,s.focusOffset],[2,2],'set position');
        s.selectAllChildren(ids('#p'));eq([s.anchorNode===ids('#p'),s.anchorOffset,s.focusOffset,String(s),s.direction],[true,0,2,'abcdefXYZ','forward'],'select all');
        assert(s.containsNode(ids('#b')) && s.containsNode(a),'fully contains descendants');
        s.setBaseAndExtent(a,2,b,1);assert(!s.containsNode(ids('#b')) && s.containsNode(ids('#b'),true),'partial containment');
        s.removeRange(s.getRangeAt(0));eq([s.rangeCount,s.type,String(s)],[0,'None',''],'remove associated range');
        s.collapse(a);s.collapse(null);eq(s.rangeCount,0,'null empties selection');
    "#);
}

#[test]
fn selection_scalar_reads_follow_direction_and_live_utf16_boundaries() {
    let mut page = fixture("<p id=p>a😀b</p><p id=q>xy</p>");
    page.check(r#"
        const s=getSelection(), p=ids('#p').firstChild, q=ids('#q').firstChild;
        const read=()=>[s.anchorNode===q,s.anchorOffset,s.focusNode===p,s.focusOffset];
        s.setBaseAndExtent(q,2,p,3);eq(read(),[true,2,true,3],'backward cross-node scalar reads');
        p.insertData(0,'z');q.deleteData(0,1);eq(read(),[true,1,true,4],'both endpoints stay live');
        s.getRangeAt(0).collapse(true);eq([s.anchorNode===p,s.focusNode===p,s.anchorOffset,s.focusOffset],[true,true,4,4],'backward collapsed range');
        s.setBaseAndExtent(p,1,q,1);eq([s.anchorNode===p,s.anchorOffset,s.focusNode===q,s.focusOffset],[true,1,true,1],'forward reads');
        s.removeAllRanges();eq([s.anchorNode,s.anchorOffset,s.focusNode,s.focusOffset],[null,0,null,0],'empty scalar reads');
    "#);
}

#[test]
fn selection_validates_arguments_and_ignores_disconnected_nodes() {
    let mut page = fixture("<p id=p>hello</p>");
    page.check(r#"
        const s=getSelection(), a=ids('#p').firstChild, detached=document.createTextNode('xx'), range=new Range();
        throws(()=>s.getRangeAt(0),'IndexSizeError','empty range');
        throws(()=>s.collapseToStart(),'InvalidStateError','empty collapse start');
        throws(()=>s.collapseToEnd(),'InvalidStateError','empty collapse end');
        throws(()=>s.extend(a,0),'InvalidStateError','empty extend');
        throws(()=>s.addRange(),'TypeError','range required');
        throws(()=>s.removeRange(),'TypeError','remove requires range');
        throws(()=>s.removeRange(range),'NotFoundError','not associated');
        s.extend(detached,999);eq(s.rangeCount,0,'disconnected extend ignored before empty and offset validation');
        range.selectNodeContents(detached);s.addRange(range);eq(s.rangeCount,0,'disconnected range ignored');
        s.collapse(a,2);const original=s.getRangeAt(0);
        s.collapse(detached,1);s.selectAllChildren(detached);s.setBaseAndExtent(a,1,detached,1);
        assert(s.getRangeAt(0)===original,'disconnected endpoints preserve state');
        throws(()=>s.collapse(detached,3),'IndexSizeError','collapse validates before disconnected check');
        throws(()=>s.setBaseAndExtent(a,1,detached,3),'IndexSizeError','set base validates offsets');
        throws(()=>s.extend(a,6),'IndexSizeError','extend validates');
        const doctype=document.createComment('');Object.defineProperty(doctype,'nodeType',{value:10});document.body.appendChild(doctype);
        throws(()=>s.collapse(doctype),'InvalidNodeTypeError','document type collapse validation');
        throws(()=>s.extend(doctype),'InvalidNodeTypeError','document type extend validation');
        throws(()=>s.selectAllChildren(doctype),'InvalidNodeTypeError','document type select validation');
        throws(()=>s.getRangeAt(-1),'IndexSizeError','negative index');
        throws(()=>s.getRangeAt(1),'IndexSizeError','single range index');
        throws(()=>s.containsNode(null),'TypeError','contains requires node');
        assert(!s.containsNode(detached),'disconnected not contained');
        s.empty();assert(!s.containsNode(a),'empty not contained');
    "#);
}

#[test]
fn selection_change_is_an_asynchronous_coalesced_document_event() {
    let mut page = fixture("<p id=p>hello</p>");
    page.check(r#"
        globalThis.events=[];
        document.addEventListener('selectionchange', e=>events.push([e.target===document,e.bubbles,e.cancelable]));
        const s=getSelection(), a=ids('#p').firstChild;
        s.removeAllRanges();
        s.collapse(a,0);s.extend(a,3);s.getRangeAt(0).setEnd(a,4);
        eq(events,[],'queued task rather than synchronous dispatch');
    "#);
    page.settle();
    page.check("eq(events,[[true,false,false]],'coalesced event');getSelection().empty();getSelection().empty();");
    page.settle();
    page.check("eq(events.length,2,'empty transition fires once');getSelection().empty();");
    page.settle();
    page.check("eq(events.length,2,'no event for already empty');");
}

#[test]
fn ranges_report_rendered_text_and_contained_element_rectangles() {
    let mut page = fixture("<p id=p>abcdef<b id=b>XYZ<i id=i>w</i></b></p><p id=q></p>");
    page.check(r#"
        const p=ids('#p'), a=p.firstChild, b=ids('#b'), c=b.firstChild, w=ids('#i').firstChild;
        const box=(rect)=>[rect.x,rect.y,rect.width,rect.height], boxes=(list)=>[...list].map(box);
        const range=new Range();range.setStart(a,1);range.setEnd(a,4);
        assert(range.getClientRects() instanceof DOMRectList,'rectangle list');
        eq(boxes(range.getClientRects()),[[5,0,15,10]],'selected part of the text');
        eq(box(range.getBoundingClientRect()),[5,0,15,10],'bounds of one rectangle');
        range.collapse(true);
        eq(boxes(range.getClientRects()),[[5,0,0,10]],'collapsed text range is the caret');
        eq(box(range.getBoundingClientRect()),[5,0,0,10],'empty rectangles bound to the first');
        range.setStart(p,1);
        eq([range.collapsed,boxes(range.getClientRects())],[true,[]],'caret between elements has no rectangle, as in Chromium');
        const empty=new Range();empty.setStart(ids('#q'),0);
        eq(empty.getClientRects().length,0,'caret in an empty element has no rectangle');
        eq(box(empty.getBoundingClientRect()),[0,0,0,0],'no rectangles bound to an empty rectangle');
        const own=boxes(b.getClientRects()), inner=[[0,0,15,10],[0,0,5,10]];
        range.selectNode(b);
        eq(boxes(range.getClientRects()),own.concat(inner),'contained element then its text, not nested elements');
        range.setStart(a,2);
        eq(boxes(range.getClientRects()),[[10,0,20,10]].concat(own,inner),'partly selected text first');
        eq(box(range.getBoundingClientRect()),own.length?[0,0,110,70]:[0,0,30,10],'union of rectangles with area');
        range.setStart(c,3);range.setEnd(w,0);
        eq(boxes(range.getClientRects()),[[15,0,0,10],[0,0,0,10]],'boundary texts give carets');
        eq(document.caretPositionFromPoint(1,2),null,'no caret position');
        eq(document.caretRangeFromPoint(1,2),null,'no caret range');
        throws(()=>new CaretPosition(),'TypeError','illegal constructor');
    "#);
    let dom = Dom::from_document(&nativeboat_html::parse_document(
        "<p id=p>abcdef<b id=b>XYZ<i id=i>w</i></b></p><p id=q></p>",
    ));
    let text = dom
        .first_child(dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap())
        .unwrap();
    page.page.caret.set(Some((text.0, 2)));
    page.check(r#"
        const a=ids('#p').firstChild, position=document.caretPositionFromPoint(3,4);
        assert(position instanceof CaretPosition,'caret position');
        eq(Object.prototype.toString.call(position),'[object CaretPosition]','tag');
        eq([position.offsetNode===a,position.offset],[true,2],'caret node and offset');
        eq(position.getClientRect().toJSON(),{x:10,y:0,width:0,height:10,top:0,right:10,bottom:10,left:10},'caret rectangle');
        const caret=document.caretRangeFromPoint(3,4);
        eq([caret instanceof Range,caret.startContainer===a,caret.startOffset,caret.collapsed],[true,true,2,true],'collapsed caret range');
    "#);
    let empty = dom.get_element_by_id(Dom::DOCUMENT, "q").unwrap();
    page.page.caret.set(Some((empty.0, 0)));
    page.check(r#"
        const position=document.caretPositionFromPoint(3,4);
        eq([position.offsetNode===ids('#q'),position.offset,position.getClientRect()],[true,0,null],'caret in an empty element');
    "#);
}

#[test]
fn ranges_and_the_selection_stay_live_across_tree_mutations() {
    let mut page = fixture("<div id=d><p id=a>one</p><p id=b>two</p><p id=c>three</p></div>");
    page.check(r#"
        const d=ids('#d'), a=ids('#a'), b=ids('#b'), c=ids('#c'), s=getSelection();
        const r=new Range(); r.setStart(d,1); r.setEnd(d,3);
        const t=new Range(); t.setStart(b.firstChild,1); t.setEnd(b.firstChild,2);
        s.collapse(c.firstChild,2); const caret=s.getRangeAt(0);
        const x=document.createElement('p'); d.insertBefore(x,a);
        eq([r.startOffset,r.endOffset],[2,4],'insertion before shifts offsets');
        d.append(document.createElement('hr'),'tail'); eq(r.endOffset,4,'appending after the end leaves it');
        x.remove(); eq([r.startOffset,r.endOffset],[1,3],'removal before shifts back');
        b.remove();
        eq([t.startContainer===d,t.startOffset,t.endContainer===d,t.endOffset,r.startOffset,r.endOffset],[true,1,true,1,1,2],'removed content collapses to its place');
        eq([s.anchorNode===c.firstChild,s.anchorOffset,s.getRangeAt(0)===caret],[true,2,true],'selection survives unrelated removal');
        c.before('p','q'); eq([r.startOffset,r.endOffset,t.startOffset],[1,4,1],'fragment insertion counts its children');
        d.appendChild(c); eq([s.anchorNode===d,s.anchorOffset,s.focusOffset,r.endOffset],[true,3,3,3],'moving a node takes the boundaries inside it out');
        d.textContent='new'; eq([r.startOffset,r.endOffset,s.anchorNode===d,s.anchorOffset],[0,0,true,0],'replacing all children');
        s.collapse(d.firstChild,2); d.innerHTML='<i>x</i>'; eq([s.anchorNode===d,s.anchorOffset],[true,0],'inner HTML');
        const loose=document.createElement('div'); loose.innerHTML='<b>1</b><b>2</b>';
        const u=new Range(); u.setStart(loose,2); loose.firstChild.remove(); eq(u.startOffset,1,'detached trees are live too');
    "#);
}

#[test]
fn character_data_edits_split_and_merge_move_range_boundaries() {
    let mut page = fixture("<p id=p>hello</p>");
    page.check(r#"
        const p=ids('#p'), text=p.firstChild, s=getSelection();
        const r=new Range(); r.setStart(text,1); r.setEnd(text,4); s.collapse(text,5);
        text.insertData(0,'>> '); eq([r.startOffset,r.endOffset,s.anchorOffset],[4,7,8],'insertion before moves');
        text.appendData('!'); eq([r.endOffset,s.anchorOffset,text.data],[7,8,'>> hello!'],'appending after leaves');
        text.deleteData(5,2); eq([r.startOffset,r.endOffset,s.anchorOffset,String(r)],[4,5,6,'e'],'deletion collapses into place');
        text.replaceData(3,10,'X'); eq([text.data,r.startOffset,r.endOffset,s.anchorOffset],['>> X',3,3,3],'replacement clamps the count');
        text.data='abc'; eq([r.startOffset,s.anchorOffset],[0,0],'setting the data');
        eq([text.substringData(1,5),text.substringData(3,1)],['bc',''],'substrings');
        throws(()=>text.substringData(4,1),'IndexSizeError','substring past the end');
        throws(()=>text.insertData(9,'x'),'IndexSizeError','insertion past the end');
        throws(()=>text.splitText(4),'IndexSizeError','split past the end');
        const comment=document.createComment('xyz'); comment.deleteData(1,1); eq(comment.data,'xz','comments');
        text.data='a😀b'; text.deleteData(1,2); eq(text.data,'ab','UTF-16 offsets');
        text.data='abcdef'; s.collapse(text,5); r.setStart(text,1); r.setEnd(p,1);
        const tail=text.splitText(3);
        eq([text.data,tail.data,p.childNodes.length,tail.previousSibling===text],['abc','def',2,true],'split');
        eq([text.wholeText,tail.wholeText],['abcdef','abcdef'],'whole text joins adjacent text');
        eq([s.anchorNode===tail,s.anchorOffset,r.startContainer===text,r.startOffset,r.endContainer===p,r.endOffset],[true,2,true,1,true,2],'boundaries follow the split');
        const loose=document.createTextNode('xyz'), end=loose.splitText(1);
        eq([loose.data,end.data,end.parentNode],['x','yz',null],'detached split');
        p.append('','ghi'); s.collapse(p.childNodes[3],1); r.setStart(tail,1); r.setEnd(p,3);
        p.normalize();
        eq([p.childNodes.length,text.data,String(r)],[1,'abcdefghi','ef'],'merged');
        eq([r.startContainer===text,r.startOffset,r.endContainer===text,r.endOffset,s.anchorNode===text,s.anchorOffset],[true,4,true,6,true,7],'boundaries follow the merge');
        const div=document.createElement('div'); div.append('','x'); div.normalize(); eq([div.childNodes.length,div.textContent],[1,'x'],'empty text removed');
    "#);
}

#[test]
fn collected_ranges_release_their_document_boundaries() {
    let mut page = fixture("<p>text</p>");
    page.check("for (let i = 0; i < 100; i++) new Range(); globalThis.kept = new Range();");
    assert_eq!(page.dom.borrow().range_count(), 101);
    boa_gc::force_collect();
    page.check("for (let i = 0; i < 40; i++) new Range(); kept.setStart(document.body, 1);");
    assert!(page.dom.borrow().range_count() <= 42);
    page.check("eq([kept.startContainer===document.body,kept.startOffset],[true,1],'kept range');");
}

#[test]
fn the_document_tracks_the_selected_range() {
    let mut page = fixture("<p id=p>abc</p>");
    page.check(
        "getSelection().setBaseAndExtent(ids('#p').firstChild, 1, ids('#p').firstChild, 2);",
    );
    let text = {
        let dom = page.dom.borrow();
        let p = dom.get_element_by_id(Dom::DOCUMENT, "p").unwrap();
        dom.first_child(p).unwrap()
    };
    assert_eq!(page.dom.borrow().selection(), Some([(text, 1), (text, 2)]));
    page.check("ids('#p').firstChild.insertData(0, 'xy');");
    assert_eq!(page.dom.borrow().selection(), Some([(text, 3), (text, 4)]));
    page.check("getSelection().removeAllRanges();");
    assert_eq!(page.dom.borrow().selection(), None);
}
