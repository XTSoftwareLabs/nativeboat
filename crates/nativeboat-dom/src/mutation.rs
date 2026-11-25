//! Mutation observers.

use crate::prelude::*;

use crate::{Dom, NodeId};

/// Options of `MutationObserver.observe`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObserveOptions {
    /// Report child list changes.
    pub child_list: bool,
    /// Report attribute changes.
    pub attributes: bool,
    /// Report character data changes.
    pub character_data: bool,
    /// Observe descendants too.
    pub subtree: bool,
    /// Record old attribute values.
    pub attribute_old_value: bool,
    /// Record old character data.
    pub character_data_old_value: bool,
    /// Only report these attributes.
    pub attribute_filter: Option<Vec<String>>,
}

/// The kind of a mutation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    /// `childList`
    ChildList,
    /// `attributes`
    Attributes,
    /// `characterData`
    CharacterData,
}

/// A `MutationRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRecord {
    /// The kind.
    pub kind: MutationKind,
    /// The changed node.
    pub target: NodeId,
    /// Added nodes.
    pub added: Vec<NodeId>,
    /// Removed nodes.
    pub removed: Vec<NodeId>,
    /// The sibling before the change.
    pub previous_sibling: Option<NodeId>,
    /// The sibling after the change.
    pub next_sibling: Option<NodeId>,
    /// The changed attribute.
    pub attribute_name: Option<String>,
    /// The old value, when requested.
    pub old_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observer {
    id: u64,
    registrations: Vec<(NodeId, ObserveOptions)>,
    records: Vec<MutationRecord>,
}

impl Dom {
    /// Creates a mutation observer and returns its id.
    pub fn create_observer(&mut self) -> u64 {
        let id = self.observers.len() as u64 + 1;
        self.observers.push(Observer {
            id,
            registrations: Vec::new(),
            records: Vec::new(),
        });
        id
    }

    fn observer_mut(&mut self, id: u64) -> Option<&mut Observer> {
        self.observers.iter_mut().find(|observer| observer.id == id)
    }

    /// `observe`; a new registration for the same target replaces the old one.
    pub fn observe(&mut self, observer: u64, target: NodeId, options: ObserveOptions) {
        if let Some(observer) = self.observer_mut(observer) {
            observer.registrations.retain(|(node, _)| *node != target);
            observer.registrations.push((target, options));
        }
    }

    /// `disconnect`.
    pub fn disconnect(&mut self, observer: u64) {
        if let Some(observer) = self.observer_mut(observer) {
            observer.registrations.clear();
            observer.records.clear();
        }
    }

    /// `takeRecords`.
    pub fn take_records(&mut self, observer: u64) -> Vec<MutationRecord> {
        self.observer_mut(observer)
            .map(|observer| core::mem::take(&mut observer.records))
            .unwrap_or_default()
    }

    /// Observers with pending records.
    pub fn pending_observers(&self) -> Vec<u64> {
        self.observers
            .iter()
            .filter(|observer| !observer.records.is_empty())
            .map(|observer| observer.id)
            .collect()
    }

    fn queue_record(
        &mut self,
        record: MutationRecord,
        wants: &dyn Fn(&ObserveOptions) -> Option<bool>,
    ) {
        if self.observers.is_empty() {
            return;
        }
        let mut ancestors = vec![record.target];
        let mut current = self.parent(record.target);
        while let Some(node) = current {
            ancestors.push(node);
            current = self.parent(node);
        }
        for observer in &mut self.observers {
            let matched = observer.registrations.iter().find_map(|(node, options)| {
                let position = ancestors.iter().position(|ancestor| ancestor == node)?;
                if position > 0 && !options.subtree {
                    return None;
                }
                wants(options)
            });
            if let Some(keep_old) = matched {
                let mut record = record.clone();
                if !keep_old {
                    record.old_value = None;
                }
                observer.records.push(record);
            }
        }
    }

    pub(crate) fn record_child_list(
        &mut self,
        target: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        previous_sibling: Option<NodeId>,
        next_sibling: Option<NodeId>,
    ) {
        self.journal_children(target, &added, &removed, next_sibling);
        let record = MutationRecord {
            kind: MutationKind::ChildList,
            target,
            added,
            removed,
            previous_sibling,
            next_sibling,
            attribute_name: None,
            old_value: None,
        };
        self.queue_record(record, &|options| options.child_list.then_some(false));
    }

    pub(crate) fn record_attribute(
        &mut self,
        target: NodeId,
        name: &str,
        old_value: Option<String>,
    ) {
        if self.get_attribute(target, name) != old_value.as_deref() {
            self.journal_attribute(target, name, old_value.as_deref());
        }
        let record = MutationRecord {
            kind: MutationKind::Attributes,
            target,
            added: Vec::new(),
            removed: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
            attribute_name: Some(name.to_owned()),
            old_value,
        };
        self.queue_record(record, &|options| {
            let filtered = options
                .attribute_filter
                .as_ref()
                .is_some_and(|filter| !filter.iter().any(|allowed| allowed == name));
            (options.attributes && !filtered).then_some(options.attribute_old_value)
        });
    }

    pub(crate) fn record_character_data(&mut self, target: NodeId, old_value: String) {
        if self.data(target) != Some(old_value.as_str()) {
            self.journal_data(target, &old_value);
        }
        let record = MutationRecord {
            kind: MutationKind::CharacterData,
            target,
            added: Vec::new(),
            removed: Vec::new(),
            previous_sibling: None,
            next_sibling: None,
            attribute_name: None,
            old_value: Some(old_value),
        };
        self.queue_record(record, &|options| {
            options
                .character_data
                .then_some(options.character_data_old_value)
        });
    }
}

#[cfg(test)]
mod tests;
