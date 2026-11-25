//! Node document ownership survives detachment and changes on adoption.

use crate::prelude::*;

use crate::{Dom, DomError, NodeId, NodeKind};

impl Dom {
    /// The node's document, including while the node is detached. Documents
    /// and event targets themselves have no owner document.
    pub fn owner_document(&self, node: NodeId) -> Option<NodeId> {
        self.node(node).owner_document
    }

    /// Assigns the document of a detached subtree created or imported by the
    /// host. Insertion adopts automatically; callers adopting connected nodes
    /// must detach them first. This does not invalidate layout or add records.
    pub fn set_owner_document(&mut self, node: NodeId, document: NodeId) -> Result<(), DomError> {
        if !self.exists(node)
            || !self.exists(document)
            || !matches!(self.kind(document), NodeKind::Document)
        {
            return Err(DomError::InvalidState("invalid node document".to_owned()));
        }
        self.adopt_document(node, document);
        Ok(())
    }

    pub(crate) fn adopt_document(&mut self, node: NodeId, document: NodeId) {
        if self.owner_document(node).is_none() || self.owner_document(node) == Some(document) {
            return;
        }
        // Ownership is uniform within each ordinary subtree. The usual
        // same-document insertion therefore performs no walk or allocation.
        let mut pending = vec![node];
        while let Some(node) = pending.pop() {
            self.node_mut(node).owner_document = Some(document);
            if let Some(shadow) = self.shadow_root(node) {
                pending.push(shadow);
            }
            let mut child = self.first_child(node);
            while let Some(id) = child {
                pending.push(id);
                child = self.next_sibling(id);
            }
        }
    }
}

#[cfg(test)]
mod tests;
