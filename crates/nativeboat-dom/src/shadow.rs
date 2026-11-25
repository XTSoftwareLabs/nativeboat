//! Shadow trees: shadow roots attached to host elements, the
//! shadow-including tree they make with the document, retargeting across
//! their boundaries, and the slots their host's children are assigned to.
//!
//! A shadow root is a document fragment that has a host: it is never a
//! child, its children are the shadow tree, and the host's own children
//! show through its slots.

use crate::prelude::*;

use crate::{Dom, DomError, Namespace, NodeId, NodeKind, custom::is_custom_element_name};

/// `ShadowRoot.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowRootMode {
    /// `element.shadowRoot` exposes the root.
    Open,
    /// Only the code that attached the root holds it.
    Closed,
}

/// What a shadow root was attached with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowRootData {
    /// The element the root is attached to.
    pub host: NodeId,
    /// Whether the root is open.
    pub mode: ShadowRootMode,
    /// Whether focusing the host focuses the first focusable element of
    /// its shadow tree.
    pub delegates_focus: bool,
}

/// The HTML elements besides custom elements that can host a shadow root.
const HOSTS: [&str; 18] = [
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];

impl Dom {
    /// `Element.attachShadow`: attaches a new, empty shadow root to `host`.
    pub fn attach_shadow(
        &mut self,
        host: NodeId,
        mode: ShadowRootMode,
        delegates_focus: bool,
    ) -> Result<NodeId, DomError> {
        let name = self.local_name(host).unwrap_or_default();
        if self.namespace(host) != Some(Namespace::Html)
            || !(HOSTS.contains(&name) || is_custom_element_name(name))
        {
            return Err(DomError::NotSupported(format!(
                "a <{name}> element cannot host a shadow root"
            )));
        }
        if self.shadow_roots.contains_key(&host) {
            return Err(DomError::NotSupported(
                "the element already hosts a shadow root".to_owned(),
            ));
        }
        let root = self.create_fragment();
        if let Some(document) = self.owner_document(host) {
            self.node_mut(root).owner_document = Some(document);
        }
        self.shadow_roots.insert(host, root);
        self.shadow_data.insert(
            root,
            ShadowRootData {
                host,
                mode,
                delegates_focus,
            },
        );
        self.touch();
        Ok(root)
    }

    /// The shadow root attached to an element, open or closed.
    pub fn shadow_root(&self, host: NodeId) -> Option<NodeId> {
        self.shadow_roots.get(&host).copied()
    }

    /// What a shadow root was attached with, if `root` is one.
    pub fn shadow_root_data(&self, root: NodeId) -> Option<ShadowRootData> {
        self.shadow_data.get(&root).copied()
    }

    /// The host of a shadow root.
    pub fn shadow_host(&self, root: NodeId) -> Option<NodeId> {
        self.shadow_data.get(&root).map(|data| data.host)
    }

    /// Whether the node is a shadow root.
    pub fn is_shadow_root(&self, node: NodeId) -> bool {
        self.shadow_data.contains_key(&node)
    }

    /// Whether any shadow root was attached.
    pub fn has_shadow_roots(&self) -> bool {
        !self.shadow_data.is_empty()
    }

    /// Every shadow root, in the order they were attached.
    pub fn shadow_roots(&self) -> Vec<NodeId> {
        let mut roots: Vec<NodeId> = self.shadow_data.keys().copied().collect();
        roots.sort();
        roots
    }

    /// The root of the node's tree: the document, a shadow root, or the top
    /// of a detached subtree.
    pub fn tree_root(&self, node: NodeId) -> NodeId {
        let mut root = node;
        while let Some(parent) = self.parent(root) {
            root = parent;
        }
        root
    }

    /// `Node.getRootNode`: the node's tree root, or with `composed` its
    /// shadow-including root, beyond the hosts of its shadow trees.
    pub fn root_node(&self, node: NodeId, composed: bool) -> NodeId {
        let mut root = self.tree_root(node);
        while composed && let Some(host) = self.shadow_host(root) {
            root = self.tree_root(host);
        }
        root
    }

    /// The node's parent, or a shadow root's host.
    pub fn parent_or_host(&self, node: NodeId) -> Option<NodeId> {
        self.parent(node).or_else(|| self.shadow_host(node))
    }

    /// Whether `ancestor` is `node` or a shadow-including ancestor of it.
    pub fn shadow_including_contains(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.parent_or_host(candidate);
        }
        false
    }

    /// The nodes below `root` in shadow-including tree order: each node,
    /// then its shadow root and that root's tree, then its children.
    pub fn shadow_including_descendants(&self, root: NodeId) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut pending: Vec<NodeId> = self.children(root).into_iter().rev().collect();
        if let Some(shadow) = self.shadow_root(root) {
            pending.push(shadow);
        }
        while let Some(node) = pending.pop() {
            found.push(node);
            pending.extend(self.children(node).into_iter().rev());
            if let Some(shadow) = self.shadow_root(node) {
                pending.push(shadow);
            }
        }
        found
    }

    /// Retargets `node` against `against`, as events and focus are: while
    /// `node` is in a shadow tree that does not contain `against`, it is
    /// replaced by that tree's host.
    pub fn retarget(&self, node: NodeId, against: NodeId) -> NodeId {
        let mut node = node;
        loop {
            let root = self.tree_root(node);
            match self.shadow_host(root) {
                Some(host) if !self.shadow_including_contains(root, against) => node = host,
                _ => return node,
            }
        }
    }

    /// Whether the node is a slot of a shadow tree, which shows the host
    /// children assigned to it in the flat tree.
    pub fn is_assigning_slot(&self, node: NodeId) -> bool {
        self.is_slot(node) && self.is_shadow_root(self.tree_root(node))
    }

    /// A node's children in the flat tree that rendering follows: a host
    /// shows its shadow tree, a slot the nodes assigned to it or, when none
    /// are, its own children.
    pub fn flat_children(&self, node: NodeId) -> Vec<NodeId> {
        if self.shadow_data.is_empty() {
            return self.children(node);
        }
        if let Some(root) = self.shadow_root(node) {
            return self.children(root);
        }
        if self.is_assigning_slot(node) {
            let assigned = self.assigned_nodes(node, false);
            if !assigned.is_empty() {
                return assigned;
            }
        }
        self.children(node)
    }

    /// The element children of a node in the flat tree.
    pub fn flat_child_elements(&self, node: NodeId) -> Vec<NodeId> {
        let mut children = self.flat_children(node);
        children.retain(|child| self.is_element(*child));
        children
    }

    /// A node's parent in the flat tree: its slot, the host of its shadow
    /// tree, or its parent; `None` for a host's child no slot shows and for
    /// a slot's own children while nodes are assigned to it.
    pub fn flat_parent(&self, node: NodeId) -> Option<NodeId> {
        let parent = self.parent(node)?;
        if self.shadow_data.is_empty() {
            return Some(parent);
        }
        if let Some(host) = self.shadow_host(parent) {
            return Some(host);
        }
        if self.shadow_root(parent).is_some() {
            return self.assigned_slot(node);
        }
        if self.is_assigning_slot(parent) && !self.assigned_nodes(parent, false).is_empty() {
            return None;
        }
        Some(parent)
    }

    /// The element a node's flat-tree parent is, if it is one.
    pub fn flat_parent_element(&self, node: NodeId) -> Option<NodeId> {
        self.flat_parent(node)
            .filter(|parent| self.is_element(*parent))
    }

    fn is_slot(&self, node: NodeId) -> bool {
        self.namespace(node) == Some(Namespace::Html) && self.local_name(node) == Some("slot")
    }

    /// The name of a slot, or the name of the slot a node asks for.
    fn slot_name(&self, node: NodeId, attribute: &str) -> String {
        self.get_attribute(node, attribute)
            .unwrap_or_default()
            .to_owned()
    }

    /// The first slot in a shadow tree with a name.
    fn find_slot(&self, root: NodeId, name: &str) -> Option<NodeId> {
        self.descendants(root)
            .into_iter()
            .find(|node| self.is_slot(*node) && self.slot_name(*node, "name") == name)
    }

    /// The slot a host's child is assigned to: the first slot of the host's
    /// shadow tree named as the child's `slot` attribute, empty for text and
    /// for elements without one.
    pub fn assigned_slot(&self, node: NodeId) -> Option<NodeId> {
        let parent = self.parent(node)?;
        let root = self.shadow_root(parent)?;
        let name = match self.kind(node) {
            NodeKind::Element(_) => self.slot_name(node, "slot"),
            NodeKind::Text(_) => String::new(),
            _ => return None,
        };
        self.find_slot(root, &name)
    }

    /// `HTMLSlotElement.assignedNodes`: the host's children assigned to the
    /// slot, or with `flatten` the nodes that show through it, which are its
    /// own children when nothing is assigned and the assigned nodes of the
    /// slots among them.
    pub fn assigned_nodes(&self, slot: NodeId, flatten: bool) -> Vec<NodeId> {
        let host = self.shadow_host(self.tree_root(slot));
        let assigned: Vec<NodeId> = host
            .map(|host| {
                self.children(host)
                    .into_iter()
                    .filter(|child| self.assigned_slot(*child) == Some(slot))
                    .collect()
            })
            .unwrap_or_default();
        if !flatten {
            return assigned;
        }
        let shown = if assigned.is_empty() {
            self.children(slot)
                .into_iter()
                .filter(|child| {
                    matches!(self.kind(*child), NodeKind::Element(_) | NodeKind::Text(_))
                })
                .collect()
        } else {
            assigned
        };
        shown
            .into_iter()
            .flat_map(|node| {
                if self.is_slot(node) && self.is_shadow_root(self.tree_root(node)) {
                    self.assigned_nodes(node, true)
                } else {
                    vec![node]
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
