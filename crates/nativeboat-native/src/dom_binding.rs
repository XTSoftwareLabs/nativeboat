//! Minimal engine-free DOM host used by compiled renderer graphs.
//!
//! The application compiler supplies the parsed static document as node specs;
//! framework modules then mutate the same native node arena through ordinary
//! JavaScript object methods. Painting and the full Web API surface are layered
//! on this stable tree in follow-up work.

use crate::object::WeakObject;
use crate::symbol::Symbol;
use crate::{ErrorKind, Key, Object, Property, Realm, Slot, Thrown, Value, ops};
use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

/// A static node emitted by the HTML compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeSpec {
    /// An element and its attributes and descendants.
    Element {
        /// Lowercased HTML tag name.
        name: String,
        /// Namespace URI for HTML, SVG or MathML elements.
        namespace: String,
        /// Source attributes in order.
        attributes: Vec<(String, String)>,
        /// Child nodes.
        children: Vec<NodeSpec>,
    },
    /// Text content.
    Text(String),
    /// Comment content.
    Comment(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Document,
    Element,
    Text,
    Comment,
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    name: String,
    namespace: String,
    data: String,
    attributes: Vec<(String, String)>,
    parent: Option<usize>,
    children: Vec<usize>,
    listeners: Vec<(String, Value)>,
    object: Option<WeakObject>,
}

#[derive(Debug)]
struct Arena {
    nodes: Vec<Node>,
    window: usize,
    id_key: Symbol,
    document: usize,
    html: usize,
    head: usize,
    body: usize,
}

/// A handle to the mutable renderer document installed in a Realm. It lets
/// the native host snapshot the post-initialization DOM without exposing the
/// JavaScript object representation to its layout backend.
#[derive(Debug, Clone)]
pub struct NativeDocument {
    arena: Rc<RefCell<Arena>>,
}

impl NativeDocument {
    /// Copies the current body tree after framework mutations have run.
    pub fn snapshot(&self) -> Vec<NodeSpec> {
        let arena = self.arena.borrow();
        arena.nodes[arena.body]
            .children
            .iter()
            .filter_map(|child| snapshot_node(&arena, *child))
            .collect()
    }

    /// Serializes the current body for the layout DOM's HTML document parser.
    pub fn body_html(&self) -> String {
        let mut output = String::new();
        for node in self.snapshot() {
            serialize_node(&node, &mut output);
        }
        output
    }
}

fn serialize_node(node: &NodeSpec, output: &mut String) {
    match node {
        NodeSpec::Text(text) => escape_text(text, output),
        NodeSpec::Comment(text) => {
            output.push_str("<!--");
            output.push_str(&text.replace("--", "- -"));
            output.push_str("-->");
        }
        NodeSpec::Element {
            name,
            namespace,
            attributes,
            children,
        } => {
            output.push('<');
            output.push_str(name);
            for (name, value) in attributes {
                output.push(' ');
                output.push_str(name);
                output.push_str("=\"");
                escape_attribute(value, output);
                output.push('\"');
            }
            output.push('>');
            if namespace == "http://www.w3.org/1999/xhtml" && is_void_element(name) {
                return;
            }
            for child in children {
                serialize_node(child, output);
            }
            output.push_str("</");
            output.push_str(name);
            output.push('>');
        }
    }
}

fn escape_text(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            character => output.push(character),
        }
    }
}

fn escape_attribute(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&quot;"),
            character => output.push(character),
        }
    }
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

impl Arena {
    fn new(window: usize) -> Arena {
        let mut arena = Arena {
            nodes: Vec::new(),
            window,
            id_key: Symbol::private("nativeboat.node"),
            document: 0,
            html: 0,
            head: 0,
            body: 0,
        };
        arena.document = arena.alloc(Kind::Document, "#document", "", "");
        arena.html = arena.alloc(Kind::Element, "html", "", "http://www.w3.org/1999/xhtml");
        arena.head = arena.alloc(Kind::Element, "head", "", "http://www.w3.org/1999/xhtml");
        arena.body = arena.alloc(Kind::Element, "body", "", "http://www.w3.org/1999/xhtml");
        arena.append(arena.document, arena.html);
        arena.append(arena.html, arena.head);
        arena.append(arena.html, arena.body);
        arena
    }

    fn alloc(&mut self, kind: Kind, name: &str, data: &str, namespace: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            name: name.to_string(),
            namespace: namespace.to_string(),
            data: data.to_string(),
            attributes: Vec::new(),
            parent: None,
            children: Vec::new(),
            listeners: Vec::new(),
            object: None,
        });
        id
    }

    fn append(&mut self, parent: usize, child: usize) {
        self.detach(child);
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
        self.invalidate();
    }

    fn insert_before(&mut self, parent: usize, child: usize, before: Option<usize>) {
        self.detach(child);
        let index = before
            .and_then(|before| {
                self.nodes[parent]
                    .children
                    .iter()
                    .position(|id| *id == before)
            })
            .unwrap_or(self.nodes[parent].children.len());
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.insert(index, child);
        self.invalidate();
    }

    fn detach(&mut self, child: usize) {
        if let Some(parent) = self.nodes[child].parent.take() {
            self.nodes[parent].children.retain(|id| *id != child);
            self.invalidate();
        }
    }

    fn invalidate(&self) {
        if self.window != 0 {
            crate::app_host::invalidate_window(crate::app_host::Window::from_raw(self.window));
        }
    }

    fn descendants(&self, root: usize) -> impl Iterator<Item = usize> + '_ {
        let mut found = Vec::new();
        let mut pending: Vec<usize> = self.nodes[root].children.iter().rev().copied().collect();
        while let Some(id) = pending.pop() {
            found.push(id);
            pending.extend(self.nodes[id].children.iter().rev().copied());
        }
        found.into_iter()
    }

    fn text_content(&self, id: usize) -> String {
        let node = &self.nodes[id];
        if matches!(node.kind, Kind::Text | Kind::Comment) {
            return node.data.clone();
        }
        let mut value = String::new();
        for child in &node.children {
            if self.nodes[*child].kind != Kind::Comment {
                value.push_str(&self.text_content(*child));
            }
        }
        value
    }

    fn find(&self, selector: &str) -> Option<usize> {
        if selector.is_empty() {
            return None;
        }
        self.descendants(self.document).find(|id| {
            let node = &self.nodes[*id];
            if node.kind != Kind::Element {
                return false;
            }
            if let Some(name) = selector.strip_prefix('#') {
                node.attributes
                    .iter()
                    .any(|(key, value)| key == "id" && value == name)
            } else if let Some(class) = selector.strip_prefix('.') {
                node.attributes.iter().any(|(key, value)| {
                    key == "class" && value.split_ascii_whitespace().any(|token| token == class)
                })
            } else {
                node.name.eq_ignore_ascii_case(selector)
            }
        })
    }
}

/// Installs `document` on a renderer Realm and hydrates its body from compiled
/// HTML node descriptions.
pub fn install(realm: &mut Realm, body: &[NodeSpec]) -> NativeDocument {
    install_for_window(realm, body, 0)
}

/// Installs a DOM arena that requests repaint when renderer mutations change it.
pub fn install_for_window(realm: &mut Realm, body: &[NodeSpec], window: usize) -> NativeDocument {
    let arena = Rc::new(RefCell::new(Arena::new(window)));
    {
        let mut dom = arena.borrow_mut();
        for spec in body {
            let child = hydrate(&mut dom, spec);
            let body = dom.body;
            dom.append(body, child);
        }
    }
    let document = document_object(realm, arena.clone());
    realm.set_global("document", Value::Object(document));
    NativeDocument { arena }
}

fn snapshot_node(arena: &Arena, id: usize) -> Option<NodeSpec> {
    let node = &arena.nodes[id];
    match node.kind {
        Kind::Element => Some(NodeSpec::Element {
            name: node.name.clone(),
            namespace: node.namespace.clone(),
            attributes: node.attributes.clone(),
            children: node
                .children
                .iter()
                .filter_map(|child| snapshot_node(arena, *child))
                .collect(),
        }),
        Kind::Text => Some(NodeSpec::Text(node.data.clone())),
        Kind::Comment => Some(NodeSpec::Comment(node.data.clone())),
        Kind::Document => None,
    }
}

fn hydrate(arena: &mut Arena, spec: &NodeSpec) -> usize {
    match spec {
        NodeSpec::Element {
            name,
            namespace,
            attributes,
            children,
        } => {
            let id = arena.alloc(Kind::Element, name, "", namespace);
            arena.nodes[id].attributes = attributes.clone();
            for child in children {
                let child = hydrate(arena, child);
                arena.append(id, child);
            }
            id
        }
        NodeSpec::Text(text) => arena.alloc(Kind::Text, "#text", text, ""),
        NodeSpec::Comment(text) => arena.alloc(Kind::Comment, "#comment", text, ""),
    }
}

fn document_object(realm: &mut Realm, arena: Rc<RefCell<Arena>>) -> Object {
    let document = realm.object();
    let (html_id, head_id, body_id) = {
        let dom = arena.borrow();
        (dom.html, dom.head, dom.body)
    };
    let html = node_object(realm, &arena, html_id);
    let head = node_object(realm, &arena, head_id);
    let body = node_object(realm, &arena, body_id);
    document.define(
        Key::name("documentElement"),
        Property::data(Value::Object(html)),
    );
    document.define(Key::name("head"), Property::data(Value::Object(head)));
    document.define(Key::name("body"), Property::data(Value::Object(body)));
    document.define(Key::name("nodeType"), Property::data(Value::Number(9.0)));

    // The document owns the arena. Other node methods use weak references so
    // detached node handles do not keep an entire page alive on their own.
    let create_element_arena = arena.clone();
    let create_element = realm.function("createElement", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "element name")?;
        if !valid_name(&name) {
            return Err(realm.throw(ErrorKind::Type, "invalid element name"));
        }
        let arena = create_element_arena.clone();
        let id = arena.borrow_mut().alloc(
            Kind::Element,
            &name.to_ascii_lowercase(),
            "",
            "http://www.w3.org/1999/xhtml",
        );
        Ok(Value::Object(node_object(realm, &arena, id)))
    });
    document.define(
        Key::name("createElement"),
        Property::data(Value::Object(create_element)),
    );

    let create_text_arena = Rc::downgrade(&arena);
    let create_text = realm.function("createTextNode", 1, false, move |realm, _, args| {
        let text = string_argument(realm, args.first(), "text")?;
        let arena = create_text_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let id = arena.borrow_mut().alloc(Kind::Text, "#text", &text, "");
        Ok(Value::Object(node_object(realm, &arena, id)))
    });
    document.define(
        Key::name("createTextNode"),
        Property::data(Value::Object(create_text)),
    );

    let create_comment_arena = Rc::downgrade(&arena);
    let create_comment = realm.function("createComment", 1, false, move |realm, _, args| {
        let text = string_argument(realm, args.first(), "comment")?;
        let arena = create_comment_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let id = arena
            .borrow_mut()
            .alloc(Kind::Comment, "#comment", &text, "");
        Ok(Value::Object(node_object(realm, &arena, id)))
    });
    document.define(
        Key::name("createComment"),
        Property::data(Value::Object(create_comment)),
    );

    let create_ns_arena = Rc::downgrade(&arena);
    let create_ns = realm.function("createElementNS", 2, false, move |realm, _, args| {
        let namespace = string_argument(realm, args.first(), "namespace")?;
        let name = string_argument(realm, args.get(1), "element name")?;
        if !valid_name(&name) {
            return Err(realm.throw(ErrorKind::Type, "invalid element name"));
        }
        let arena = create_ns_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let local_name = if namespace == "http://www.w3.org/1999/xhtml" {
            name.to_ascii_lowercase()
        } else {
            name
        };
        let id = arena
            .borrow_mut()
            .alloc(Kind::Element, &local_name, "", &namespace);
        Ok(Value::Object(node_object(realm, &arena, id)))
    });
    document.define(
        Key::name("createElementNS"),
        Property::data(Value::Object(create_ns)),
    );

    let query_arena = Rc::downgrade(&arena);
    let query_key = arena.borrow().id_key.clone();
    let query = realm.function("querySelector", 1, false, move |realm, _, args| {
        let selector = string_argument(realm, args.first(), "selector")?;
        let arena = query_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let found = arena.borrow().find(&selector);
        Ok(found.map_or(Value::Null, |id| {
            Value::Object(cached_node(realm, &arena, id, &query_key))
        }))
    });
    document.define(
        Key::name("querySelector"),
        Property::data(Value::Object(query)),
    );

    let id_arena = Rc::downgrade(&arena);
    let id_key = arena.borrow().id_key.clone();
    let get_by_id = realm.function("getElementById", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "id")?;
        let arena = id_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let found = arena
            .borrow()
            .nodes
            .iter()
            .enumerate()
            .find_map(|(id, node)| {
                node.attributes
                    .iter()
                    .any(|(key, value)| key == "id" && value == &name)
                    .then_some(id)
            });
        Ok(found.map_or(Value::Null, |id| {
            Value::Object(cached_node(realm, &arena, id, &id_key))
        }))
    });
    document.define(
        Key::name("getElementById"),
        Property::data(Value::Object(get_by_id)),
    );
    document
}

fn node_object(realm: &mut Realm, arena: &Rc<RefCell<Arena>>, id: usize) -> Object {
    let id_key = arena.borrow().id_key.clone();
    cached_node(realm, arena, id, &id_key)
}

fn cached_node(
    realm: &mut Realm,
    arena: &Rc<RefCell<Arena>>,
    id: usize,
    id_key: &Symbol,
) -> Object {
    if let Some(object) = arena.borrow().nodes[id]
        .object
        .as_ref()
        .and_then(WeakObject::upgrade)
    {
        return object;
    }
    let object = realm.object();
    object.define(
        Key::Symbol(id_key.clone()),
        Property::hidden(Value::Number(id as f64)),
    );
    let name = arena.borrow().nodes[id].name.clone();
    let kind = arena.borrow().nodes[id].kind;
    let namespace = arena.borrow().nodes[id].namespace.clone();
    let display_name = if kind == Kind::Element {
        name.to_ascii_uppercase()
    } else {
        name.clone()
    };
    object.define(
        Key::name("nodeType"),
        Property::data(Value::Number(node_type(kind) as f64)),
    );
    object.define(
        Key::name("nodeName"),
        Property::data(Value::str(&display_name)),
    );
    object.define(
        Key::name("namespaceURI"),
        Property::data(Value::str(&namespace)),
    );
    if kind == Kind::Element {
        object.define(
            Key::name("tagName"),
            Property::data(Value::str(&name.to_ascii_uppercase())),
        );
        object.define(Key::name("localName"), Property::data(Value::str(&name)));
        define_attribute_property(realm, &object, Rc::downgrade(arena), id, "id", "id");
        define_attribute_property(
            realm,
            &object,
            Rc::downgrade(arena),
            id,
            "className",
            "class",
        );
        let style = style_object(realm, Rc::downgrade(arena), id);
        object.define(Key::name("style"), Property::data(Value::Object(style)));
        install_element_methods(realm, &object, Rc::downgrade(arena), id);
    }
    install_node_methods(realm, &object, Rc::downgrade(arena), id);
    arena.borrow_mut().nodes[id].object = Some(object.downgrade());
    object
}

fn install_node_methods(
    realm: &mut Realm,
    object: &Object,
    arena: Weak<RefCell<Arena>>,
    id: usize,
) {
    let append_arena = arena.clone();
    let append = realm.function("appendChild", 1, false, move |realm, _, args| {
        let child = node_id(realm, args.first(), &append_arena)?;
        let arena = append_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        arena.borrow_mut().append(id, child);
        Ok(args.first().cloned().unwrap_or(Value::Undefined))
    });
    object.define(
        Key::name("appendChild"),
        Property::data(Value::Object(append)),
    );

    let insert_arena = arena.clone();
    let insert = realm.function("insertBefore", 2, false, move |realm, _, args| {
        let child = node_id(realm, args.first(), &insert_arena)?;
        let before = if args.get(1).is_none_or(Value::is_nullish) {
            None
        } else {
            Some(node_id(realm, args.get(1), &insert_arena)?)
        };
        let arena = insert_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        arena.borrow_mut().insert_before(id, child, before);
        Ok(args.first().cloned().unwrap_or(Value::Undefined))
    });
    object.define(
        Key::name("insertBefore"),
        Property::data(Value::Object(insert)),
    );

    let remove_arena = arena.clone();
    let remove = realm.function("removeChild", 1, false, move |realm, _, args| {
        let child = node_id(realm, args.first(), &remove_arena)?;
        let arena = remove_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        arena.borrow_mut().detach(child);
        Ok(args.first().cloned().unwrap_or(Value::Undefined))
    });
    object.define(
        Key::name("removeChild"),
        Property::data(Value::Object(remove)),
    );

    for (property, sibling) in [("parentNode", false), ("nextSibling", true)] {
        let getter_arena = arena.clone();
        let getter = realm.function(property, 0, false, move |realm, _, _| {
            let Some(arena) = getter_arena.upgrade() else {
                return Ok(Value::Null);
            };
            let found = {
                let dom = arena.borrow();
                if sibling {
                    dom.nodes[id].parent.and_then(|parent| {
                        let siblings = &dom.nodes[parent].children;
                        let position = siblings.iter().position(|candidate| *candidate == id)?;
                        siblings.get(position + 1).copied()
                    })
                } else {
                    dom.nodes[id].parent
                }
            };
            Ok(found.map_or(Value::Null, |found| {
                Value::Object(node_object(realm, &arena, found))
            }))
        });
        object.define(
            Key::name(property),
            Property {
                slot: Slot::Accessor {
                    get: Some(getter),
                    set: None,
                },
                enumerable: true,
                configurable: true,
            },
        );
    }

    let text_get_arena = arena.clone();
    let text_get = realm.function("get textContent", 0, false, move |_, _, _| {
        let Some(arena) = text_get_arena.upgrade() else {
            return Ok(Value::Null);
        };
        Ok(Value::str(&arena.borrow().text_content(id)))
    });
    let text_set_arena = arena;
    let text_set = realm.function("set textContent", 1, false, move |realm, _, args| {
        let text = string_argument(realm, args.first(), "textContent")?;
        let arena = text_set_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let mut dom = arena.borrow_mut();
        let old = core::mem::take(&mut dom.nodes[id].children);
        for child in old {
            dom.nodes[child].parent = None;
        }
        if !text.is_empty() && !matches!(dom.nodes[id].kind, Kind::Text | Kind::Comment) {
            let child = dom.alloc(Kind::Text, "#text", &text, "");
            dom.append(id, child);
        } else {
            dom.nodes[id].data = text;
            dom.invalidate();
        }
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("textContent"),
        Property {
            slot: Slot::Accessor {
                get: Some(text_get),
                set: Some(text_set),
            },
            enumerable: true,
            configurable: true,
        },
    );
}

fn install_element_methods(
    realm: &mut Realm,
    object: &Object,
    arena: Weak<RefCell<Arena>>,
    id: usize,
) {
    let set_arena = arena.clone();
    let set = realm.function("setAttribute", 2, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "attribute name")?;
        let value = string_argument(realm, args.get(1), "attribute value")?;
        let arena = set_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        set_attribute_value(&mut arena.borrow_mut(), id, &name, value);
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("setAttribute"),
        Property::data(Value::Object(set)),
    );

    let get_arena = arena.clone();
    let get = realm.function("getAttribute", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "attribute name")?;
        let found = get_arena.upgrade().and_then(|arena| {
            let dom = arena.borrow();
            let name = attribute_name(&dom, id, name);
            dom.nodes[id]
                .attributes
                .iter()
                .find(|(key, _)| key == &name)
                .map(|(_, value)| value.clone())
        });
        Ok(found.map_or(Value::Null, |value| Value::str(&value)))
    });
    object.define(
        Key::name("getAttribute"),
        Property::data(Value::Object(get)),
    );

    let remove_arena = arena.clone();
    let remove = realm.function("removeAttribute", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "attribute name")?;
        if let Some(arena) = remove_arena.upgrade() {
            let mut dom = arena.borrow_mut();
            let name = attribute_name(&dom, id, name);
            dom.nodes[id].attributes.retain(|(key, _)| key != &name);
            dom.invalidate();
        }
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("removeAttribute"),
        Property::data(Value::Object(remove)),
    );

    let has_arena = arena.clone();
    let has = realm.function("hasAttribute", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "attribute name")?;
        let found = has_arena.upgrade().is_some_and(|arena| {
            let dom = arena.borrow();
            let name = attribute_name(&dom, id, name);
            dom.nodes[id].attributes.iter().any(|(key, _)| key == &name)
        });
        Ok(Value::Boolean(found))
    });
    object.define(
        Key::name("hasAttribute"),
        Property::data(Value::Object(has)),
    );

    let add_arena = arena.clone();
    let add = realm.function("addEventListener", 2, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "event type")?;
        let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
        if listener.is_nullish() {
            return Ok(Value::Undefined);
        }
        let arena = add_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let registered = arena.borrow().nodes[id]
            .listeners
            .iter()
            .any(|(event, current)| event == &name && crate::same_value(current, &listener));
        if !registered {
            arena.borrow_mut().nodes[id]
                .listeners
                .push((name, listener));
        }
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("addEventListener"),
        Property::data(Value::Object(add)),
    );

    let remove_arena = arena;
    let remove = realm.function("removeEventListener", 2, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "event type")?;
        let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
        if let Some(arena) = remove_arena.upgrade() {
            arena.borrow_mut().nodes[id]
                .listeners
                .retain(|(event, current)| {
                    event != &name || !crate::same_value(current, &listener)
                });
        }
        Ok(Value::Undefined)
    });
    object.define(
        Key::name("removeEventListener"),
        Property::data(Value::Object(remove)),
    );
}

fn attribute_name(arena: &Arena, id: usize, name: String) -> String {
    if arena.nodes[id].namespace == "http://www.w3.org/1999/xhtml" {
        name.to_ascii_lowercase()
    } else {
        name
    }
}

fn define_attribute_property(
    realm: &mut Realm,
    object: &Object,
    arena: Weak<RefCell<Arena>>,
    id: usize,
    property: &'static str,
    attribute: &'static str,
) {
    let get_arena = arena.clone();
    let getter = realm.function(property, 0, false, move |_, _, _| {
        let value = get_arena.upgrade().and_then(|arena| {
            let dom = arena.borrow();
            let attribute = attribute_name(&dom, id, attribute.to_string());
            dom.nodes[id]
                .attributes
                .iter()
                .find(|(name, _)| name == &attribute)
                .map(|(_, value)| value.clone())
        });
        Ok(Value::str(value.as_deref().unwrap_or_default()))
    });
    let set_arena = arena;
    let setter = realm.function(property, 1, false, move |realm, _, args| {
        let value = string_argument(realm, args.first(), property)?;
        let arena = set_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        set_attribute_value(&mut arena.borrow_mut(), id, attribute, value);
        Ok(Value::Undefined)
    });
    object.define(
        Key::name(property),
        Property {
            slot: Slot::Accessor {
                get: Some(getter),
                set: Some(setter),
            },
            enumerable: true,
            configurable: true,
        },
    );
}

fn style_object(realm: &mut Realm, arena: Weak<RefCell<Arena>>, id: usize) -> Object {
    let target = realm.object();
    let get_text_arena = arena.clone();
    let get_text = realm.function("get cssText", 0, false, move |_, _, _| {
        let value = get_text_arena.upgrade().and_then(|arena| {
            let dom = arena.borrow();
            dom.nodes[id]
                .attributes
                .iter()
                .find(|(name, _)| name == "style")
                .map(|(_, value)| value.clone())
        });
        Ok(Value::str(value.as_deref().unwrap_or_default()))
    });
    let set_text_arena = arena.clone();
    let set_text = realm.function("set cssText", 1, false, move |realm, _, args| {
        let value = string_argument(realm, args.first(), "cssText")?;
        let arena = set_text_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        set_attribute_value(&mut arena.borrow_mut(), id, "style", value);
        Ok(Value::Undefined)
    });
    target.define(
        Key::name("cssText"),
        Property {
            slot: Slot::Accessor {
                get: Some(get_text),
                set: Some(set_text),
            },
            enumerable: true,
            configurable: true,
        },
    );

    let set_property_arena = arena.clone();
    let set_property = realm.function("setProperty", 2, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "CSS property name")?;
        let value = string_argument(realm, args.get(1), "CSS property value")?;
        let priority = args
            .get(2)
            .map(|value| ops::to_string(realm, value).map(|text| text.to_rust_lossy()))
            .transpose()?;
        let arena = set_property_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        set_style_property(
            &mut arena.borrow_mut(),
            id,
            &name,
            &value,
            priority.as_deref(),
        );
        Ok(Value::Undefined)
    });
    target.define(
        Key::name("setProperty"),
        Property::data(Value::Object(set_property)),
    );

    let get_property_arena = arena.clone();
    let get_property = realm.function("getPropertyValue", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "CSS property name")?;
        let value = get_property_arena
            .upgrade()
            .map(|arena| style_property(&arena.borrow(), id, &name))
            .unwrap_or_default();
        Ok(Value::str(&value))
    });
    target.define(
        Key::name("getPropertyValue"),
        Property::data(Value::Object(get_property)),
    );

    let remove_property_arena = arena.clone();
    let remove_property = realm.function("removeProperty", 1, false, move |realm, _, args| {
        let name = string_argument(realm, args.first(), "CSS property name")?;
        let arena = remove_property_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        let mut dom = arena.borrow_mut();
        let previous = style_property(&dom, id, &name);
        set_style_property(&mut dom, id, &name, "", None);
        Ok(Value::str(&previous))
    });
    target.define(
        Key::name("removeProperty"),
        Property::data(Value::Object(remove_property)),
    );

    let get_arena = arena.clone();
    let handler = realm.object();
    let get = realm.function("get", 3, false, move |realm, _, args| {
        let target = args.first().cloned().unwrap_or(Value::Undefined);
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        let key = match key {
            Value::String(key) => Key::from_js(&key),
            Value::Symbol(key) => Key::Symbol(key),
            _ => return Err(realm.throw(ErrorKind::Type, "invalid CSS property key")),
        };
        if target
            .as_object()
            .is_some_and(|target| target.own_property(&key).is_some())
        {
            return ops::get(realm, &target, &key);
        }
        let Some(name) = (match &key {
            Key::String(name) => Some(name.to_rust_lossy()),
            _ => None,
        }) else {
            return Ok(Value::Undefined);
        };
        let value = get_arena
            .upgrade()
            .map(|arena| style_property(&arena.borrow(), id, &name))
            .unwrap_or_default();
        Ok(Value::str(&value))
    });
    handler.define(Key::name("get"), Property::data(Value::Object(get)));

    let set_arena = arena.clone();
    let set = realm.function("set", 4, false, move |realm, _, args| {
        let key = args.get(1).cloned().unwrap_or(Value::Undefined);
        let Value::String(name) = key else {
            return Ok(Value::Boolean(true));
        };
        let value = args.get(2).cloned().unwrap_or(Value::Undefined);
        let value = ops::to_string(realm, &value)?.to_rust_lossy();
        let arena = set_arena
            .upgrade()
            .ok_or_else(|| realm.throw(ErrorKind::Reference, "document was released"))?;
        if name.to_rust_lossy() == "cssText" {
            set_attribute_value(&mut arena.borrow_mut(), id, "style", value);
        } else {
            set_style_property(
                &mut arena.borrow_mut(),
                id,
                &name.to_rust_lossy(),
                &value,
                None,
            );
        }
        Ok(Value::Boolean(true))
    });
    handler.define(Key::name("set"), Property::data(Value::Object(set)));

    let has = realm.function("has", 2, false, |_, _, _| Ok(Value::Boolean(true)));
    handler.define(Key::name("has"), Property::data(Value::Object(has)));
    let global = Value::Object(realm.global_object());
    let proxy_constructor = ops::get_named(realm, &global, "Proxy")
        .expect("the Proxy constructor is installed in every realm");
    ops::construct(
        realm,
        &proxy_constructor,
        &[Value::Object(target), Value::Object(handler)],
    )
    .expect("style proxy construction uses ordinary objects")
    .as_object()
    .expect("Proxy construction returns an object")
    .clone()
}

fn set_attribute_value(arena: &mut Arena, id: usize, name: &str, value: String) {
    let name = attribute_name(arena, id, name.to_string());
    let attributes = &mut arena.nodes[id].attributes;
    if let Some((_, current)) = attributes.iter_mut().find(|(key, _)| key == &name) {
        *current = value;
    } else {
        attributes.push((name, value));
    }
    arena.invalidate();
}

fn style_property(arena: &Arena, id: usize, name: &str) -> String {
    let name = css_name(name);
    arena.nodes[id]
        .attributes
        .iter()
        .find(|(key, _)| key == "style")
        .and_then(|(_, style)| {
            style.split(';').find_map(|declaration| {
                let (property, value) = declaration.split_once(':')?;
                (property.trim() == name).then(|| value.trim().to_string())
            })
        })
        .unwrap_or_default()
}

fn set_style_property(
    arena: &mut Arena,
    id: usize,
    name: &str,
    value: &str,
    priority: Option<&str>,
) {
    let name = css_name(name);
    let previous = arena.nodes[id]
        .attributes
        .iter()
        .find(|(key, _)| key == "style")
        .map(|(_, style)| style.clone())
        .unwrap_or_default();
    let mut declarations = Vec::new();
    for declaration in previous.split(';') {
        let Some((property, current)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_string();
        if property != name {
            declarations.push((property, current.trim().to_string()));
        }
    }
    if !value.is_empty() {
        let value = if priority.is_some_and(|priority| priority.eq_ignore_ascii_case("important")) {
            alloc::format!("{} !important", value.trim())
        } else {
            value.trim().to_string()
        };
        declarations.push((name, value));
    }
    let text = declarations
        .iter()
        .map(|(name, value)| alloc::format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ");
    set_attribute_value(arena, id, "style", text);
}

fn css_name(name: &str) -> String {
    if name.starts_with("--") {
        return name.to_string();
    }
    let mut result = String::new();
    for character in name.chars() {
        if character.is_ascii_uppercase() {
            result.push('-');
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn node_id(
    realm: &Realm,
    value: Option<&Value>,
    arena: &Weak<RefCell<Arena>>,
) -> Result<usize, Thrown> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Err(realm.throw(ErrorKind::Type, "argument is not a native DOM node"));
    };
    let Some(arena) = arena.upgrade() else {
        return Err(realm.throw(ErrorKind::Reference, "document was released"));
    };
    let id = object
        .own_property(&Key::Symbol(arena.borrow().id_key.clone()))
        .and_then(|property| match property.slot {
            Slot::Data {
                value: Value::Number(id),
                ..
            } if id >= 0.0 => Some(id as usize),
            _ => None,
        })
        .filter(|id| *id < arena.borrow().nodes.len())
        .ok_or_else(|| realm.throw(ErrorKind::Type, "argument is not a native DOM node"))?;
    Ok(id)
}

fn string_argument(
    realm: &mut Realm,
    value: Option<&Value>,
    label: &str,
) -> Result<String, Thrown> {
    let value = value.cloned().unwrap_or(Value::Undefined);
    ops::to_string(realm, &value)
        .map(|value| value.to_rust_lossy())
        .map_err(|_| {
            let message = alloc::format!("{label} must be stringable");
            realm.throw(ErrorKind::Type, &message)
        })
}

fn valid_name(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_' || first == ':')
        && name.chars().all(|character| {
            !character.is_whitespace() && !matches!(character, '<' | '>' | '"' | '\'' | '/' | '=')
        })
}

fn node_type(kind: Kind) -> u16 {
    match kind {
        Kind::Element => 1,
        Kind::Text => 3,
        Kind::Comment => 8,
        Kind::Document => 9,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(realm: &mut Realm, object: &Object, name: &str, args: &[Value]) -> Value {
        let function = ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap();
        ops::call(realm, &function, Value::Object(object.clone()), args).unwrap()
    }

    fn property(realm: &mut Realm, object: &Object, name: &str) -> Value {
        ops::get(realm, &Value::Object(object.clone()), &Key::name(name)).unwrap()
    }

    #[test]
    fn compiled_document_nodes_support_hydration_selection_and_mutation() {
        let mut realm = Realm::new();
        let native_document = install(
            &mut realm,
            &[NodeSpec::Element {
                name: "div".into(),
                namespace: "http://www.w3.org/1999/xhtml".into(),
                attributes: vec![
                    ("id".into(), "app".into()),
                    ("class".into(), "entry".into()),
                ],
                children: vec![
                    NodeSpec::Text("ready".into()),
                    NodeSpec::Comment("anchor".into()),
                ],
            }],
        );
        let global = realm.global_object();
        let document = ops::get(&mut realm, &Value::Object(global), &Key::name("document"))
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        let selected = method(
            &mut realm,
            &document,
            "querySelector",
            &[Value::str(".entry")],
        );
        let selected = selected.as_object().unwrap().clone();
        let by_id = method(
            &mut realm,
            &document,
            "getElementById",
            &[Value::str("app")],
        );
        assert!(matches!(by_id, Value::Object(ref object) if *object == selected));
        let text = ops::get(
            &mut realm,
            &Value::Object(selected.clone()),
            &Key::name("textContent"),
        )
        .unwrap();
        assert!(matches!(text, Value::String(ref text) if text.to_rust_lossy() == "ready"));
        assert!(matches!(
            method(&mut realm, &selected, "getAttribute", &[Value::str("id")]),
            Value::String(ref value) if value.to_rust_lossy() == "app"
        ));
        assert!(matches!(
            method(
                &mut realm,
                &selected,
                "hasAttribute",
                &[Value::str("class")]
            ),
            Value::Boolean(true)
        ));
        method(
            &mut realm,
            &selected,
            "removeAttribute",
            &[Value::str("class")],
        );
        assert!(matches!(
            method(
                &mut realm,
                &selected,
                "hasAttribute",
                &[Value::str("class")]
            ),
            Value::Boolean(false)
        ));
        assert!(matches!(
            method(
                &mut realm,
                &document,
                "querySelector",
                &[Value::str("#missing")]
            ),
            Value::Null
        ));
        assert_eq!(
            native_document.snapshot(),
            vec![NodeSpec::Element {
                name: "div".into(),
                namespace: "http://www.w3.org/1999/xhtml".into(),
                attributes: vec![("id".into(), "app".into())],
                children: vec![
                    NodeSpec::Text("ready".into()),
                    NodeSpec::Comment("anchor".into()),
                ],
            }]
        );
        assert_eq!(
            native_document.body_html(),
            "<div id=\"app\">ready<!--anchor--></div>"
        );
    }

    #[test]
    fn document_snapshot_serialization_escapes_content_and_respects_svg_void_names() {
        let mut html = String::new();
        for node in [
            NodeSpec::Text("<&>".into()),
            NodeSpec::Comment("left--right".into()),
            NodeSpec::Element {
                name: "img".into(),
                namespace: "http://www.w3.org/1999/xhtml".into(),
                attributes: vec![("title".into(), "\"&<".into())],
                children: Vec::new(),
            },
            NodeSpec::Element {
                name: "img".into(),
                namespace: "http://www.w3.org/2000/svg".into(),
                attributes: Vec::new(),
                children: vec![NodeSpec::Text("child".into())],
            },
        ] {
            serialize_node(&node, &mut html);
        }
        assert_eq!(
            html,
            "&lt;&amp;&gt;<!--left- -right--><img title=\"&quot;&amp;&lt;\"><img>child</img>"
        );
    }

    #[test]
    fn live_vue_nodes_cover_mutation_text_namespaces_and_attribute_case() {
        let mut realm = Realm::new();
        install(&mut realm, &[]);
        let global = realm.global_object();
        let document = property(&mut realm, &global, "document")
            .as_object()
            .unwrap()
            .clone();
        let body = property(&mut realm, &document, "body")
            .as_object()
            .unwrap()
            .clone();
        let section = method(
            &mut realm,
            &document,
            "createElement",
            &[Value::str("SECTION")],
        )
        .as_object()
        .unwrap()
        .clone();
        method(
            &mut realm,
            &section,
            "setAttribute",
            &[Value::str("ID"), Value::str("live")],
        );
        method(
            &mut realm,
            &section,
            "setAttribute",
            &[Value::str("class"), Value::str("root")],
        );
        let text = method(
            &mut realm,
            &document,
            "createTextNode",
            &[Value::str("Hello")],
        )
        .as_object()
        .unwrap()
        .clone();
        let comment = method(
            &mut realm,
            &document,
            "createComment",
            &[Value::str("ignored")],
        )
        .as_object()
        .unwrap()
        .clone();
        method(
            &mut realm,
            &section,
            "appendChild",
            &[Value::Object(comment.clone())],
        );
        method(
            &mut realm,
            &section,
            "appendChild",
            &[Value::Object(text.clone())],
        );
        method(
            &mut realm,
            &body,
            "appendChild",
            &[Value::Object(section.clone())],
        );
        assert!(matches!(
            property(&mut realm, &section, "textContent"),
            Value::String(ref value) if value.to_rust_lossy() == "Hello"
        ));
        assert!(matches!(
            method(&mut realm, &document, "getElementById", &[Value::str("live")]),
            Value::Object(ref node) if *node == section
        ));
        assert!(matches!(
            method(&mut realm, &document, "querySelector", &[Value::str("section")]),
            Value::Object(ref node) if *node == section
        ));
        assert!(matches!(
            method(
                &mut realm,
                &section,
                "getAttribute",
                &[Value::str("id")]
            ),
            Value::String(ref value) if value.to_rust_lossy() == "live"
        ));

        ops::set(
            &mut realm,
            &Value::Object(section.clone()),
            &Key::name("textContent"),
            Value::str("Updated"),
        )
        .unwrap();
        assert!(matches!(
            property(&mut realm, &section, "textContent"),
            Value::String(ref value) if value.to_rust_lossy() == "Updated"
        ));
        assert!(matches!(
            property(&mut realm, &comment, "parentNode"),
            Value::Null
        ));

        let svg = method(
            &mut realm,
            &document,
            "createElementNS",
            &[Value::str("http://www.w3.org/2000/svg"), Value::str("svg")],
        )
        .as_object()
        .unwrap()
        .clone();
        method(
            &mut realm,
            &svg,
            "setAttribute",
            &[Value::str("viewBox"), Value::str("0 0 2 2")],
        );
        assert!(matches!(
            property(&mut realm, &svg, "namespaceURI"),
            Value::String(ref value) if value.to_rust_lossy().ends_with("svg")
        ));
        assert!(matches!(
            method(
                &mut realm,
                &svg,
                "getAttribute",
                &[Value::str("viewBox")]
            ),
            Value::String(ref value) if value.to_rust_lossy() == "0 0 2 2"
        ));
        method(
            &mut realm,
            &body,
            "insertBefore",
            &[Value::Object(svg.clone()), Value::Object(section.clone())],
        );
        assert!(matches!(
            property(&mut realm, &svg, "nextSibling"),
            Value::Object(ref node) if *node == section
        ));
        assert!(matches!(
            property(&mut realm, &section, "parentNode"),
            Value::Object(ref node) if *node == body
        ));
        method(
            &mut realm,
            &body,
            "removeChild",
            &[Value::Object(svg.clone())],
        );
        assert!(matches!(
            property(&mut realm, &svg, "parentNode"),
            Value::Null
        ));

        let invalid = ops::get(
            &mut realm,
            &Value::Object(document.clone()),
            &Key::name("createElement"),
        )
        .unwrap();
        assert!(
            ops::call(
                &mut realm,
                &invalid,
                Value::Object(document),
                &[Value::str("1bad")]
            )
            .is_err()
        );
    }

    #[test]
    fn vue_patch_properties_update_class_and_inline_style() {
        let mut realm = Realm::new();
        install(&mut realm, &[]);
        let global = realm.global_object();
        let document = property(&mut realm, &global, "document")
            .as_object()
            .unwrap()
            .clone();
        let element = method(&mut realm, &document, "createElement", &[Value::str("div")])
            .as_object()
            .unwrap()
            .clone();

        let listener = realm.function("listener", 0, false, |_, _, _| Ok(Value::Undefined));
        method(
            &mut realm,
            &element,
            "addEventListener",
            &[Value::str("click"), Value::Null],
        );
        method(
            &mut realm,
            &element,
            "addEventListener",
            &[Value::str("click"), Value::Object(listener.clone())],
        );
        method(
            &mut realm,
            &element,
            "addEventListener",
            &[Value::str("click"), Value::Object(listener.clone())],
        );
        method(
            &mut realm,
            &element,
            "removeEventListener",
            &[Value::str("click"), Value::str("different listener")],
        );
        method(
            &mut realm,
            &element,
            "removeEventListener",
            &[Value::str("click"), Value::Object(listener)],
        );

        ops::set(
            &mut realm,
            &Value::Object(element.clone()),
            &Key::name("className"),
            Value::str("editor active"),
        )
        .unwrap();
        assert!(matches!(
            method(&mut realm, &element, "getAttribute", &[Value::str("class")]),
            Value::String(ref value) if value.to_rust_lossy() == "editor active"
        ));

        let style = property(&mut realm, &element, "style")
            .as_object()
            .unwrap()
            .clone();
        ops::set(
            &mut realm,
            &Value::Object(style.clone()),
            &Key::name("cssText"),
            Value::str("color: blue"),
        )
        .unwrap();
        assert!(matches!(
            property(&mut realm, &style, "color"),
            Value::String(ref value) if value.to_rust_lossy() == "blue"
        ));
        assert!(matches!(
            ops::get(
                &mut realm,
                &Value::Object(style.clone()),
                &Key::Symbol(crate::Symbol::new(None)),
            )
            .unwrap(),
            Value::Undefined
        ));
        ops::set(
            &mut realm,
            &Value::Object(style.clone()),
            &Key::name("backgroundColor"),
            Value::str("rgb(1, 2, 3)"),
        )
        .unwrap();
        ops::set(
            &mut realm,
            &Value::Object(style.clone()),
            &Key::name("--editor-gap"),
            Value::str("8px"),
        )
        .unwrap();
        assert!(matches!(
            property(&mut realm, &style, "backgroundColor"),
            Value::String(ref value) if value.to_rust_lossy() == "rgb(1, 2, 3)"
        ));
        assert!(matches!(
            method(
                &mut realm,
                &style,
                "getPropertyValue",
                &[Value::str("--editor-gap")]
            ),
            Value::String(ref value) if value.to_rust_lossy() == "8px"
        ));
        assert!(matches!(
            method(
                &mut realm,
                &style,
                "setProperty",
                &[
                    Value::str("marginTop"),
                    Value::str("4px"),
                    Value::str("important")
                ]
            ),
            Value::Undefined
        ));
        assert!(matches!(
            property(&mut realm, &style, "cssText"),
            Value::String(ref value) if value.to_rust_lossy().contains("margin-top: 4px !important;")
        ));
        assert!(matches!(
            method(
                &mut realm,
                &style,
                "removeProperty",
                &[Value::str("background-color")]
            ),
            Value::String(ref value) if value.to_rust_lossy() == "rgb(1, 2, 3)"
        ));
        assert!(matches!(
            property(&mut realm, &style, "backgroundColor"),
            Value::String(ref value) if value.is_empty()
        ));
    }
}
