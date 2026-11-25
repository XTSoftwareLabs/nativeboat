//! The logical DOM.
//!
//! An arena of nodes with the tree operations, attributes, form control
//! state, selector queries and event dispatch that application code and
//! frameworks such as React rely on. There is no browser engine behind it:
//! the runtime turns the tree into native controls and painted content.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub(crate) mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::format;
    pub use alloc::string::String;
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

mod constraints;
mod custom;
mod details;
mod element;
pub mod events;
mod html;
mod journal;
pub mod mutation;
mod ownership;
mod query;
mod range;
mod shadow;

pub use constraints::{PatternMatcher, validity};
pub use custom::is_custom_element_name;
pub use element::{InlineStyle, Namespace};
pub use events::{Event, Listener, ListenerOptions, Phase};
pub use journal::Journal;
pub use mutation::{MutationKind, MutationRecord, ObserveOptions};
pub use query::ElementRef;
pub use range::Boundary;
pub use shadow::{ShadowRootData, ShadowRootMode};

use alloc::borrow::ToOwned;
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use core::fmt;

/// Identifies a node of a [`Dom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The node as a stable integer, used by the native UI tree.
    pub fn index(self) -> u64 {
        u64::from(self.0)
    }
}

/// A DOM exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomError {
    /// `HierarchyRequestError`
    HierarchyRequest(String),
    /// `NotFoundError`
    NotFound(String),
    /// `InvalidCharacterError`
    InvalidCharacter(String),
    /// `SyntaxError`
    Syntax(String),
    /// `InvalidStateError`
    InvalidState(String),
    /// `IndexSizeError`
    IndexSize(String),
    /// `NotSupportedError`
    NotSupported(String),
}

impl DomError {
    /// The DOMException name.
    pub fn name(&self) -> &'static str {
        match self {
            DomError::HierarchyRequest(_) => "HierarchyRequestError",
            DomError::NotFound(_) => "NotFoundError",
            DomError::InvalidCharacter(_) => "InvalidCharacterError",
            DomError::Syntax(_) => "SyntaxError",
            DomError::InvalidState(_) => "InvalidStateError",
            DomError::IndexSize(_) => "IndexSizeError",
            DomError::NotSupported(_) => "NotSupportedError",
        }
    }

    /// The message.
    pub fn message(&self) -> &str {
        match self {
            DomError::HierarchyRequest(message)
            | DomError::NotFound(message)
            | DomError::InvalidCharacter(message)
            | DomError::Syntax(message)
            | DomError::InvalidState(message)
            | DomError::IndexSize(message)
            | DomError::NotSupported(message) => message,
        }
    }
}

impl fmt::Display for DomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.name(), self.message())
    }
}

/// The kind of a node and its own data.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// The document.
    Document,
    /// A document fragment.
    Fragment,
    /// An element.
    Element(element::ElementData),
    /// Character data.
    Text(String),
    /// A comment.
    Comment(String),
    /// The window, an event target outside the tree.
    Window,
    /// A detached event target such as `new EventTarget()`.
    EventTarget,
}

/// `Node.nodeType` values.
pub mod node_type {
    /// `ELEMENT_NODE`
    pub const ELEMENT: u16 = 1;
    /// `TEXT_NODE`
    pub const TEXT: u16 = 3;
    /// `COMMENT_NODE`
    pub const COMMENT: u16 = 8;
    /// `DOCUMENT_NODE`
    pub const DOCUMENT: u16 = 9;
    /// `DOCUMENT_FRAGMENT_NODE`
    pub const FRAGMENT: u16 = 11;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeData {
    pub(crate) kind: NodeKind,
    pub(crate) owner_document: Option<NodeId>,
    child_count: u32,
    element_child_count: u32,
    pub(crate) parent: Option<NodeId>,
    pub(crate) first_child: Option<NodeId>,
    pub(crate) last_child: Option<NodeId>,
    pub(crate) previous_sibling: Option<NodeId>,
    pub(crate) next_sibling: Option<NodeId>,
    pub(crate) listeners: Vec<Listener>,
}

/// The scrollbar part under the pointer and the one pressed, each by its
/// scroll container and a code for the part.
pub type ScrollbarParts = (Option<(NodeId, u8)>, Option<(NodeId, u8)>);

/// The document tree.
#[derive(Debug, Clone)]
pub struct Dom {
    pub(crate) nodes: Vec<NodeData>,
    pub(crate) focused: Option<NodeId>,
    pub(crate) hovered: Option<NodeId>,
    pub(crate) active: Option<NodeId>,
    /// The scrollbar parts under the pointer and pressed, by element and
    /// part, of scroll containers whose scrollbars the page paints.
    pub(crate) scrollbar_parts: ScrollbarParts,
    pub(crate) focus_visible: bool,
    pub(crate) version: u64,
    pub(crate) details_toggles: Vec<(NodeId, bool, bool)>,
    pub(crate) scrolled: Vec<NodeId>,
    pub(crate) named_open_details: HashSet<NodeId>,
    pub(crate) observers: Vec<mutation::Observer>,
    pub(crate) next_listener: u64,
    /// The platform bitmap of each drawn canvas element and how often it
    /// was drawn on.
    pub(crate) canvases: HashMap<NodeId, (u64, u64)>,
    /// The elements with each `id`, connected or not, for `getElementById`.
    pub(crate) ids: HashMap<String, Vec<NodeId>>,
    pub(crate) ranges: range::Ranges,
    /// Edit journals being recorded.
    pub(crate) recordings: Vec<journal::Recording>,
    /// The editing hosts whose changes are counted, and their counts.
    pub(crate) watched_hosts: Vec<(NodeId, u64)>,
    pub(crate) next_recording: u64,
    /// The messages `setCustomValidity` gave controls.
    pub(crate) custom_validity: HashMap<NodeId, String>,
    /// What `pattern` constraints match with, from the runtime.
    pub(crate) pattern_matcher: Option<constraints::PatternMatcher>,
    /// The custom elements their definitions have upgraded.
    pub(crate) custom_defined: HashSet<NodeId>,
    /// The shadow root of each host.
    pub(crate) shadow_roots: HashMap<NodeId, NodeId>,
    /// What each shadow root was attached with.
    pub(crate) shadow_data: HashMap<NodeId, shadow::ShadowRootData>,
}

impl Default for Dom {
    fn default() -> Self {
        Dom::new()
    }
}

/// The children of a node in order, as [`Dom::child_nodes`] walks them.
#[derive(Debug, Clone)]
pub struct ChildNodes<'a> {
    dom: &'a Dom,
    next: Option<NodeId>,
}

impl Iterator for ChildNodes<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.dom.next_sibling(current);
        Some(current)
    }
}

impl Dom {
    /// The document node.
    pub const DOCUMENT: NodeId = NodeId(0);
    /// The window event target.
    pub const WINDOW: NodeId = NodeId(1);

    /// Creates an empty document.
    pub fn new() -> Dom {
        let mut dom = Dom {
            nodes: Vec::new(),
            focused: None,
            hovered: None,
            active: None,
            scrollbar_parts: (None, None),
            focus_visible: false,
            version: 0,
            details_toggles: Vec::new(),
            scrolled: Vec::new(),
            named_open_details: HashSet::new(),
            observers: Vec::new(),
            next_listener: 1,
            canvases: HashMap::new(),
            ids: HashMap::new(),
            ranges: range::Ranges::default(),
            recordings: Vec::new(),
            watched_hosts: Vec::new(),
            next_recording: 0,
            custom_validity: HashMap::new(),
            pattern_matcher: None,
            custom_defined: HashSet::new(),
            shadow_roots: HashMap::new(),
            shadow_data: HashMap::new(),
        };
        dom.allocate(NodeKind::Document);
        dom.allocate(NodeKind::Window);
        dom
    }

    pub(crate) fn allocate(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        let owner_document = (!matches!(
            kind,
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget
        ))
        .then_some(Self::DOCUMENT);
        self.nodes.push(NodeData {
            kind,
            owner_document,
            child_count: 0,
            element_child_count: 0,
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
            listeners: Vec::new(),
        });
        id
    }

    pub(crate) fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.0 as usize]
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.0 as usize]
    }

    /// Whether `id` names a node of this DOM.
    pub fn exists(&self, id: NodeId) -> bool {
        (id.0 as usize) < self.nodes.len()
    }

    /// Increments with every change that can affect rendering.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn touch(&mut self) {
        self.version += 1;
    }

    /// Records that a resource the document shows, such as an image, has
    /// loaded, so that the document is laid out and painted again.
    pub fn resource_loaded(&mut self) {
        self.touch();
    }

    /// Records that a canvas element was drawn into the platform bitmap
    /// `key`, so that it is painted again.
    pub fn canvas_drawn(&mut self, id: NodeId, key: u64) {
        let drawn = self.canvases.get(&id).map_or(0, |(_, count)| *count);
        self.canvases.insert(id, (key, drawn + 1));
        self.touch();
    }

    /// The platform bitmap of a canvas element and how often it was drawn
    /// on, once a script drew on it.
    pub fn canvas(&self, id: NodeId) -> Option<(u64, u64)> {
        self.canvases.get(&id).copied()
    }

    /// The node's kind.
    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.node(id).kind
    }

    /// `Node.nodeType`.
    pub fn node_type(&self, id: NodeId) -> u16 {
        match self.kind(id) {
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget => node_type::DOCUMENT,
            NodeKind::Fragment => node_type::FRAGMENT,
            NodeKind::Element(_) => node_type::ELEMENT,
            NodeKind::Text(_) => node_type::TEXT,
            NodeKind::Comment(_) => node_type::COMMENT,
        }
    }

    /// `Node.nodeName`.
    pub fn node_name(&self, id: NodeId) -> String {
        match self.kind(id) {
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget => "#document".to_owned(),
            NodeKind::Fragment => "#document-fragment".to_owned(),
            NodeKind::Element(element) => element.qualified_name(),
            NodeKind::Text(_) => "#text".to_owned(),
            NodeKind::Comment(_) => "#comment".to_owned(),
        }
    }

    /// Whether the node is an element.
    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.kind(id), NodeKind::Element(_))
    }

    /// The element's local name, or `None` for other nodes.
    pub fn local_name(&self, id: NodeId) -> Option<&str> {
        match self.kind(id) {
            NodeKind::Element(element) => Some(&element.name),
            _ => None,
        }
    }

    /// Creates an element in the HTML namespace.
    pub fn create_element(&mut self, name: &str) -> Result<NodeId, DomError> {
        self.create_element_ns(Namespace::Html, name)
    }

    /// Creates an element in a namespace.
    pub fn create_element_ns(
        &mut self,
        namespace: Namespace,
        name: &str,
    ) -> Result<NodeId, DomError> {
        let valid = name
            .chars()
            .next()
            .is_some_and(|first| first.is_alphabetic() || first == '_' || first == ':')
            && name.chars().all(|character| {
                !character.is_whitespace()
                    && !matches!(character, '<' | '>' | '"' | '\'' | '/' | '=')
            });
        if !valid {
            return Err(DomError::InvalidCharacter(format!(
                "\"{name}\" is not a valid element name"
            )));
        }
        let name = if namespace == Namespace::Html {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        };
        Ok(self.allocate(NodeKind::Element(element::ElementData::new(
            namespace, name,
        ))))
    }

    /// Creates a text node.
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.allocate(NodeKind::Text(text.to_owned()))
    }

    /// Creates a comment.
    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.allocate(NodeKind::Comment(text.to_owned()))
    }

    /// Creates a document fragment.
    pub fn create_fragment(&mut self) -> NodeId {
        self.allocate(NodeKind::Fragment)
    }

    /// Creates a detached event target.
    pub fn create_event_target(&mut self) -> NodeId {
        self.allocate(NodeKind::EventTarget)
    }

    /// `Node.parentNode`.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// `Node.parentElement`.
    pub fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        self.parent(id).filter(|parent| self.is_element(*parent))
    }

    /// `Node.firstChild`.
    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    /// `Node.lastChild`.
    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).last_child
    }

    /// The number of immediate children, maintained at link/detach time.
    pub fn child_count(&self, id: NodeId) -> u32 {
        self.node(id).child_count
    }

    /// The number of immediate element children, without allocating a list.
    pub fn element_child_count(&self, id: NodeId) -> u32 {
        self.node(id).element_child_count
    }

    /// `Node.previousSibling`.
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).previous_sibling
    }

    /// `Node.nextSibling`.
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).next_sibling
    }

    /// `Node.childNodes`.
    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.child_nodes(id).collect()
    }

    /// The children of a node in order, without collecting them.
    pub fn child_nodes(&self, id: NodeId) -> ChildNodes<'_> {
        ChildNodes {
            dom: self,
            next: self.first_child(id),
        }
    }

    /// `ParentNode.children`.
    pub fn child_elements(&self, id: NodeId) -> Vec<NodeId> {
        self.child_nodes(id)
            .filter(|child| self.is_element(*child))
            .collect()
    }

    /// `previousElementSibling`.
    pub fn previous_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.previous_sibling(id);
        while let Some(sibling) = current {
            if self.is_element(sibling) {
                return Some(sibling);
            }
            current = self.previous_sibling(sibling);
        }
        None
    }

    /// `nextElementSibling`.
    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.next_sibling(id);
        while let Some(sibling) = current {
            if self.is_element(sibling) {
                return Some(sibling);
            }
            current = self.next_sibling(sibling);
        }
        None
    }

    /// The `<html>` element.
    pub fn document_element(&self) -> Option<NodeId> {
        self.child_elements(Dom::DOCUMENT).into_iter().next()
    }

    /// The child of `<html>` with the given name, such as `head` or `body`.
    pub fn html_child(&self, name: &str) -> Option<NodeId> {
        let html = self.document_element()?;
        self.child_elements(html)
            .into_iter()
            .find(|child| self.local_name(*child) == Some(name))
    }

    /// Whether `ancestor` is `node` or contains it.
    pub fn contains(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.parent(candidate);
        }
        false
    }

    /// Whether the node is in the document tree, or in a shadow tree of an
    /// element that is.
    pub fn is_connected(&self, node: NodeId) -> bool {
        self.shadow_including_contains(Dom::DOCUMENT, node)
    }

    /// Descendants of `root` in tree order, excluding `root`.
    pub fn descendants(&self, root: NodeId) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut pending: Vec<NodeId> = self.children(root).into_iter().rev().collect();
        while let Some(node) = pending.pop() {
            found.push(node);
            pending.extend(self.children(node).into_iter().rev());
        }
        found
    }

    fn validate_insert(
        &self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<(), DomError> {
        let parent_ok = matches!(
            self.kind(parent),
            NodeKind::Document | NodeKind::Fragment | NodeKind::Element(_)
        );
        if !parent_ok {
            return Err(DomError::HierarchyRequest(
                "this node cannot have children".to_owned(),
            ));
        }
        if self.shadow_including_contains(child, parent) {
            return Err(DomError::HierarchyRequest(
                "the new child contains the parent".to_owned(),
            ));
        }
        if self.is_shadow_root(child) {
            return Err(DomError::HierarchyRequest(
                "a shadow root cannot be inserted".to_owned(),
            ));
        }
        if let Some(reference) = reference
            && self.parent(reference) != Some(parent)
        {
            return Err(DomError::NotFound(
                "the reference node is not a child of this node".to_owned(),
            ));
        }
        match self.kind(child) {
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget => {
                return Err(DomError::HierarchyRequest(
                    "this node cannot be inserted".to_owned(),
                ));
            }
            NodeKind::Text(_) if parent == Dom::DOCUMENT => {
                return Err(DomError::HierarchyRequest(
                    "text cannot be a child of the document".to_owned(),
                ));
            }
            NodeKind::Element(_)
                if parent == Dom::DOCUMENT
                    && self.document_element().is_some_and(|root| root != child) =>
            {
                return Err(DomError::HierarchyRequest(
                    "the document already has an element".to_owned(),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.parent(child) else {
            return;
        };
        let element = self.is_element(child);
        self.node_mut(parent).child_count -= 1;
        self.node_mut(parent).element_child_count -= u32::from(element);
        self.ranges_removing(parent, child);
        let previous = self.previous_sibling(child);
        let next = self.next_sibling(child);
        match previous {
            Some(previous) => self.node_mut(previous).next_sibling = next,
            None => self.node_mut(parent).first_child = next,
        }
        match next {
            Some(next) => self.node_mut(next).previous_sibling = previous,
            None => self.node_mut(parent).last_child = previous,
        }
        let node = self.node_mut(child);
        node.parent = None;
        node.previous_sibling = None;
        node.next_sibling = None;
        self.record_child_list(parent, Vec::new(), vec![child], previous, next);
        if self
            .focused
            .is_some_and(|node| self.shadow_including_contains(child, node))
        {
            self.focused = None;
        }
        if self
            .hovered
            .is_some_and(|node| self.shadow_including_contains(child, node))
        {
            self.hovered = None;
        }
        if self
            .active
            .is_some_and(|node| self.shadow_including_contains(child, node))
        {
            self.active = None;
        }
        self.touch();
    }

    fn link(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        let element = self.is_element(child);
        self.node_mut(parent).child_count += 1;
        self.node_mut(parent).element_child_count += u32::from(element);
        let document = if matches!(self.kind(parent), NodeKind::Document) {
            parent
        } else {
            self.owner_document(parent).unwrap_or(Self::DOCUMENT)
        };
        self.adopt_document(child, document);
        let previous = match reference {
            Some(reference) => self.previous_sibling(reference),
            None => self.last_child(parent),
        };
        {
            let node = self.node_mut(child);
            node.parent = Some(parent);
            node.previous_sibling = previous;
            node.next_sibling = reference;
        }
        match previous {
            Some(previous) => self.node_mut(previous).next_sibling = Some(child),
            None => self.node_mut(parent).first_child = Some(child),
        }
        match reference {
            Some(reference) => self.node_mut(reference).previous_sibling = Some(child),
            None => self.node_mut(parent).last_child = Some(child),
        }
        self.details_inserted(child);
    }

    /// `Node.insertBefore`; inserting a fragment moves its children.
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        reference: Option<NodeId>,
    ) -> Result<NodeId, DomError> {
        self.validate_insert(parent, child, reference)?;
        if reference == Some(child) {
            return Ok(child);
        }
        // A fragment is adopted even when it has no children to insert.
        let document = if matches!(self.kind(parent), NodeKind::Document) {
            parent
        } else {
            self.owner_document(parent).unwrap_or(Self::DOCUMENT)
        };
        self.adopt_document(child, document);
        let nodes = if matches!(self.kind(child), NodeKind::Fragment) {
            let children = self.children(child);
            for node in &children {
                self.detach(*node);
            }
            children
        } else {
            self.detach(child);
            vec![child]
        };
        self.ranges_inserting(parent, reference, nodes.len() as u32);
        for node in &nodes {
            self.link(parent, *node, reference);
        }
        let previous = nodes
            .first()
            .and_then(|first| self.previous_sibling(*first));
        self.record_child_list(parent, nodes, Vec::new(), previous, reference);
        self.touch();
        Ok(child)
    }

    /// `Node.appendChild`.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<NodeId, DomError> {
        self.insert_before(parent, child, None)
    }

    /// `Node.removeChild`.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> Result<NodeId, DomError> {
        if self.parent(child) != Some(parent) {
            return Err(DomError::NotFound(
                "the node is not a child of this node".to_owned(),
            ));
        }
        self.detach(child);
        Ok(child)
    }

    /// `Node.replaceChild`.
    pub fn replace_child(
        &mut self,
        parent: NodeId,
        new_child: NodeId,
        old_child: NodeId,
    ) -> Result<NodeId, DomError> {
        if self.parent(old_child) != Some(parent) {
            return Err(DomError::NotFound(
                "the node is not a child of this node".to_owned(),
            ));
        }
        if new_child == old_child {
            return Ok(old_child);
        }
        let reference = self.next_sibling(old_child);
        let reference = if reference == Some(new_child) {
            self.next_sibling(new_child)
        } else {
            reference
        };
        self.validate_insert(parent, new_child, None)?;
        self.detach(old_child);
        self.insert_before(parent, new_child, reference)?;
        Ok(old_child)
    }

    /// `ChildNode.remove`.
    pub fn remove(&mut self, node: NodeId) {
        self.detach(node);
    }

    /// Removes all children of a node.
    pub fn clear_children(&mut self, parent: NodeId) {
        for child in self.children(parent) {
            self.detach(child);
        }
    }

    /// `Node.textContent`.
    pub fn text_content(&self, id: NodeId) -> Option<String> {
        match self.kind(id) {
            NodeKind::Text(text) | NodeKind::Comment(text) => Some(text.clone()),
            NodeKind::Element(_) | NodeKind::Fragment => {
                let mut text = String::new();
                for node in self.descendants(id) {
                    if let NodeKind::Text(data) = self.kind(node) {
                        text.push_str(data);
                    }
                }
                Some(text)
            }
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget => None,
        }
    }

    /// Sets `Node.textContent`.
    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        match self.kind(id) {
            NodeKind::Text(_) | NodeKind::Comment(_) => self.set_data(id, text),
            NodeKind::Element(_) | NodeKind::Fragment => {
                self.clear_children(id);
                if !text.is_empty() {
                    let node = self.create_text(text);
                    self.link(id, node, None);
                    self.record_child_list(id, vec![node], Vec::new(), None, None);
                    self.touch();
                }
            }
            NodeKind::Document | NodeKind::Window | NodeKind::EventTarget => {}
        }
    }

    /// Character data of a text or comment node.
    pub fn data(&self, id: NodeId) -> Option<&str> {
        match self.kind(id) {
            NodeKind::Text(text) | NodeKind::Comment(text) => Some(text),
            _ => None,
        }
    }

    /// Sets the character data of a text or comment node, replacing all of
    /// it as the `data` setter does.
    pub fn set_data(&mut self, id: NodeId, text: &str) {
        let length = self
            .data(id)
            .map_or(0, |data| data.encode_utf16().count() as u32);
        let _ = self.replace_data(id, 0, length, text);
    }

    pub(crate) fn write_data(&mut self, id: NodeId, text: String) {
        // Identical writes still notify observers and adjust live ranges,
        // but cannot change styles or geometry.
        let changed = self.data(id) != Some(text.as_str());
        if let NodeKind::Text(data) | NodeKind::Comment(data) = &mut self.node_mut(id).kind {
            let old = core::mem::replace(data, text);
            self.record_character_data(id, old);
        }
        if changed {
            self.touch();
        }
    }

    /// `Node.cloneNode`.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let kind = match self.kind(id) {
            NodeKind::Element(element) => NodeKind::Element(element.clone_for_copy()),
            other => other.clone(),
        };
        let copy = self.allocate(kind);
        self.node_mut(copy).owner_document = self.owner_document(id);
        if let Some(value) = self.element_id(copy).map(str::to_owned) {
            self.index_id(copy, None, Some(&value));
        }
        if self.has_attribute(copy, "open") {
            self.details_attribute_changed(copy, "open", false, true);
        }
        if deep {
            for child in self.children(id) {
                let child_copy = self.clone_node(child, true);
                self.link(copy, child_copy, None);
            }
        }
        copy
    }

    /// The focused element.
    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// The element under the pointer.
    pub fn hovered(&self) -> Option<NodeId> {
        self.hovered
    }

    /// Sets the element under the pointer.
    pub fn set_hovered(&mut self, node: Option<NodeId>) {
        if self.hovered != node {
            self.hovered = node;
            self.touch();
        }
    }

    /// The scrollbar part under the pointer and the one pressed, each by
    /// its scroll container and a code for the part.
    pub fn scrollbar_parts(&self) -> ScrollbarParts {
        self.scrollbar_parts
    }

    /// Sets the scrollbar parts under the pointer and pressed.
    pub fn set_scrollbar_parts(&mut self, parts: ScrollbarParts) {
        if self.scrollbar_parts != parts {
            self.scrollbar_parts = parts;
            self.touch();
        }
    }

    /// The element being activated (pressed).
    pub fn active(&self) -> Option<NodeId> {
        self.active
    }

    /// Sets the element being activated.
    pub fn set_active(&mut self, node: Option<NodeId>) {
        if self.active != node {
            self.active = node;
            self.touch();
        }
    }

    /// Whether this is a details element's first direct summary child.
    pub fn is_disclosure_summary(&self, id: NodeId) -> bool {
        if self.local_name(id) != Some("summary") {
            return false;
        }
        let Some(parent) = self.parent_element(id) else {
            return false;
        };
        self.local_name(parent) == Some("details")
            && self
                .child_elements(parent)
                .into_iter()
                .find(|child| self.local_name(*child) == Some("summary"))
                == Some(id)
    }

    /// Whether the element can receive focus.
    pub fn is_focusable(&self, id: NodeId) -> bool {
        let Some(name) = self.local_name(id) else {
            return false;
        };
        if self.has_attribute(id, "disabled")
            && matches!(name, "button" | "input" | "select" | "textarea")
        {
            return false;
        }
        if self
            .get_attribute(id, "tabindex")
            .is_some_and(|index| index.trim().parse::<i32>().is_ok())
        {
            return true;
        }
        match name {
            "button" | "select" | "textarea" | "iframe" => true,
            "input" => self.get_attribute(id, "type") != Some("hidden"),
            "a" | "area" => self.has_attribute(id, "href"),
            "summary" => self.is_disclosure_summary(id),
            _ => self.editable_state(id) == Some(true) && self.editing_host(id) == Some(id),
        }
    }

    /// What an element's `contenteditable` attribute sets: `Some(true)` for
    /// editable content, `Some(false)` for content that is not, and `None`
    /// when it inherits its parent's.
    fn editable_state(&self, id: NodeId) -> Option<bool> {
        match self
            .get_attribute(id, "contenteditable")?
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "true" | "plaintext-only" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    /// The editing host holding a node: the outermost element of the
    /// editable content around it, or `None` if the node is not editable.
    pub fn editing_host(&self, id: NodeId) -> Option<NodeId> {
        let mut host = None;
        let mut node = Some(id);
        while let Some(current) = node {
            match self.editable_state(current) {
                Some(true) => host = Some(current),
                Some(false) => break,
                None => {}
            }
            node = self.parent(current);
        }
        host
    }

    /// Moves focus; returns the previously focused element, or `None` if focus did not change.
    pub fn set_focus(&mut self, node: Option<NodeId>, visible: bool) -> Option<Option<NodeId>> {
        if let Some(node) = node
            && (!self.is_focusable(node) || !self.is_connected(node))
        {
            return None;
        }
        if self.focused == node {
            return None;
        }
        let previous = self.focused;
        self.focused = node;
        self.focus_visible = visible;
        self.touch();
        Some(previous)
    }
}

#[cfg(test)]
mod tests;
