use super::*;

#[test]
fn detached_documents_parse_sanitize_and_import_without_changing_the_page() {
    let mut page = fixture("<p id=original>keep</p>");
    page.check(r#"
      const original=document.body.innerHTML;
      const doc=new DOMParser().parseFromString('<html><head><title>Detached</title></head><body><p id=p onclick="bad()">Hi<b>bold</b><script>bad()</script></p></body></html>','text/html');
      eq(doc.nodeType,9,'document');eq(doc.ownerDocument,null,'document owner');eq(doc.defaultView,null,'no browsing context');
      eq(doc.title,'Detached','parsed title');eq(doc.contentType,'text/html','HTML type');eq(doc.activeElement,null,'no focused element');
      eq(doc.getElementById('p').ownerDocument,doc,'parsed node owner');eq(doc.getElementById('p').isConnected,false,'detached');
      const iterator=doc.createNodeIterator(doc.body,NodeFilter.SHOW_ELEMENT|NodeFilter.SHOW_TEXT);
      eq(iterator.nextNode(),doc.body,'root included');const p=iterator.nextNode();eq(p,doc.getElementById('p'),'paragraph');
      p.removeAttribute('onclick');let node;while(node=iterator.nextNode())if(node.nodeType===1&&node.localName==='script')node.remove();
      eq(doc.body.innerHTML,'<p id="p">Hi<b>bold</b></p>','sanitized tree');eq(document.body.innerHTML,original,'main page untouched');
      const copy=document.importNode(p,true);eq(copy.ownerDocument,document,'import owner');eq(copy.firstChild.ownerDocument,document,'deep import owner');
      doc.body.removeChild(p);eq(p.ownerDocument,doc,'detached owner kept');document.body.append(copy);eq(copy.isConnected,true,'import connected');
      const adopted=doc.adoptNode(copy);eq(adopted,copy,'adopt identity');eq(copy.ownerDocument,doc,'adopt owner');eq(copy.firstChild.ownerDocument,doc,'deep adopt owner');
      eq(document.getElementById('p'),null,'removed from main page');
      const blank=document.implementation.createHTMLDocument('A <title>');eq(blank.title,'A <title>','safe title');eq(blank.body.childNodes.length,0,'blank body');
      eq(document.implementation,document.implementation,'stable implementation');
      const xml=document.implementation.createDocument('http://www.w3.org/2000/svg','svg');eq(xml.documentElement.namespaceURI,'http://www.w3.org/2000/svg','namespace');eq(xml.contentType,'application/xml','XML type');
      eq(xml.createTextNode('x').ownerDocument,xml,'created text owner');eq(xml.createComment('x').ownerDocument,xml,'comment owner');eq(xml.createDocumentFragment().ownerDocument,xml,'fragment owner');
      const empty=document.implementation.createDocument(null,'');eq(empty.documentElement,null,'empty XML');
      throws(()=>document.implementation.createDocument(null,'a',{}),'NotSupportedError','doctype unsupported');
      throws(()=>new DOMParser().parseFromString('<a/>','application/xml'),'NotSupportedError','XML parser unsupported');
      throws(()=>new DOMParser().parseFromString('x','other'),'TypeError','invalid media type');
    "#);
}

#[test]
fn node_iterators_filter_and_follow_tree_mutations_in_both_directions() {
    let mut page = fixture("<div id=root><p id=a>A</p><p id=b>B</p></div>");
    page.check(r#"
      const root=ids('#root'),a=ids('#a'),b=ids('#b');
      const iterator=document.createNodeIterator(root,NodeFilter.SHOW_ELEMENT,{acceptNode:node=>node===a?NodeFilter.FILTER_REJECT:NodeFilter.FILTER_ACCEPT});
      eq(iterator.nextNode(),root,'root');eq(iterator.nextNode(),b,'filter');eq(iterator.previousNode(),b,'reverse same reference');eq(iterator.previousNode(),root,'reverse root');eq(iterator.previousNode(),null,'start');
      eq(iterator.nextNode(),root,'forward root');eq(iterator.nextNode(),b,'forward b');
      const c=document.createElement('p');c.textContent='C';root.append(c);eq(iterator.nextNode(),c,'new insertion visible');
      c.remove();eq(iterator.nextNode(),null,'removed reference reaches end');
      eq(iterator.previousNode(),b,'removed reference finds predecessor');iterator.detach();
      const texts=document.createNodeIterator(root,NodeFilter.SHOW_TEXT,node=>node.data==='B'?1:3);eq(texts.nextNode().data,'B','function filter');eq(texts.nextNode(),null,'end');
    "#);
}

#[test]
fn native_ownership_preserves_detached_parsed_children_and_fragment_adoption() {
    let mut page = fixture("<p id=main>main</p>");
    page.check(r#"
      const other=document.implementation.createHTMLDocument('other');
      const host=other.createElement('div');host.innerHTML='<b>bold</b><!--note-->';
      const text=host.firstChild.firstChild, comment=host.lastChild;
      host.firstChild.remove();comment.remove();
      eq([text.ownerDocument===other,comment.ownerDocument===other],[true,true],'parsed ownership survives removal');
      const tail=text.splitText(1);eq(tail.ownerDocument,other,'detached split owner');
      const fragment=other.createDocumentFragment();fragment.append(text,comment);
      document.body.append(fragment);
      eq([fragment.ownerDocument===document,text.ownerDocument===document,comment.ownerDocument===document],[true,true,true],'fragment and descendants adopt');
      const empty=other.createDocumentFragment();document.body.append(empty);eq(empty.ownerDocument,document,'empty fragment adoption');
      const copy=other.cloneNode(true);eq(copy.ownerDocument,null,'document clone has no owner');
      const title=copy.querySelector('title');title.remove();eq(title.ownerDocument,copy,'document clone owns descendants');
    "#);
}

#[test]
fn cached_child_collections_stay_live_without_replacing_their_identity() {
    let mut page = fixture("<div id=p>a<b id=first>b</b><!--note--><i id=last>i</i></div>");
    page.check(r#"
      const p=ids('#p'), nodes=p.childNodes, elements=p.children;
      eq(p.childNodes,nodes,'childNodes SameObject');eq(p.children,elements,'children SameObject');
      eq([nodes.length,elements.length,p.childElementCount],[4,2,2],'native counts');
      eq([nodes[1].id,elements[1].id,p.firstElementChild.id,p.lastElementChild.id],['first','last','first','last'],'native indexed children');
      eq(p.firstElementChild.nextElementSibling,p.lastElementChild,'next element skips comments');
      eq(p.lastElementChild.previousElementSibling,p.firstElementChild,'previous element skips comments');
      p.firstChild.remove();p.lastElementChild.remove();
      eq([nodes.length,elements.length,p.childElementCount],[2,1,1],'counts follow removal');
      eq(nodes[99],undefined,'out of range');eq(elements[99],undefined,'missing element');
      eq(nodes['4294967296'],undefined,'indices do not wrap to zero');
      eq(elements['99999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999'],undefined,'infinite index is absent');
      p.innerHTML='<u id=new>replacement</u>';
      eq([nodes.length,elements[0].id],[1,'new'],'cached lists follow replacement');
      eq(p.childNodes,nodes,'replacement preserves child list identity');
      eq(p.children,elements,'replacement preserves element list identity');
      p.textContent='';eq([nodes.length,elements.length,p.firstElementChild,p.lastElementChild],[0,0,null,null],'empty counts');
    "#);
}
