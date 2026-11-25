//! Disclosure state changes and exclusive groups across all DOM mutations.

use crate::prelude::*;

use crate::{Dom, Namespace, NodeId};

impl Dom {
    /// Drains details state changes for asynchronous `ToggleEvent` delivery.
    /// Each entry contains the element, previous open state and new open state.
    pub fn take_details_toggles(&mut self) -> Vec<(NodeId, bool, bool)> {
        core::mem::take(&mut self.details_toggles)
    }

    pub(crate) fn details_attribute_changed(
        &mut self,
        node: NodeId,
        name: &str,
        old: bool,
        new: bool,
    ) {
        if self.local_name(node) != Some("details") || self.namespace(node) != Some(Namespace::Html)
        {
            return;
        }
        if name == "open" && old != new {
            self.details_toggles.push((node, old, new));
        }
        if name != "open" && name != "name" {
            return;
        }
        let named_open = self.has_attribute(node, "open")
            && self
                .get_attribute(node, "name")
                .is_some_and(|name| !name.is_empty());
        if named_open {
            self.named_open_details.insert(node);
            if name == "name" || (name == "open" && !old) {
                self.enforce_details_group(node, name == "open");
            }
        } else {
            self.named_open_details.remove(&node);
        }
    }

    pub(crate) fn details_inserted(&mut self, subtree: NodeId) {
        // Inspect only named open disclosures, rather than walking every
        // subtree on every insertion in a large document.
        let inserted: Vec<NodeId> = self
            .named_open_details
            .iter()
            .copied()
            .filter(|node| self.contains(subtree, *node))
            .collect();
        for node in inserted {
            self.enforce_details_group(node, false);
        }
    }

    fn enforce_details_group(&mut self, node: NodeId, close_others: bool) {
        let name = self.get_attribute(node, "name");
        let mut root = node;
        while let Some(parent) = self.parent(root) {
            root = parent;
        }
        let other = self.named_open_details.iter().copied().find(|other| {
            *other != node
                && self.get_attribute(*other, "name") == name
                && self.contains(root, *other)
        });
        if let Some(other) = other {
            self.remove_attribute(if close_others { other } else { node }, "open");
        }
    }
}

#[cfg(test)]
mod tests;
