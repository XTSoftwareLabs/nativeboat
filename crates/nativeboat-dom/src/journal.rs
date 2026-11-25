//! Edit journals: the changes made to an editing host while an edit runs,
//! which undo reverts. Recording what changed keeps an edit's cost to the
//! edit, where snapshots of the whole host cost the whole document.

use crate::prelude::*;

use crate::{Dom, NodeId};

/// One change to the document.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// Text or comment data replaced.
    Data { node: NodeId, old: String },
    /// An attribute set or removed.
    Attribute {
        node: NodeId,
        name: String,
        old: Option<String>,
    },
    /// Children added or removed between two siblings.
    Children {
        parent: NodeId,
        added: Vec<NodeId>,
        removed: Vec<NodeId>,
        next: Option<NodeId>,
    },
}

/// The changes an edit made, in order: to its editing host, and to the
/// nodes outside the document that the edit worked on, as fragments and
/// new elements before they went in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Journal {
    host: Option<NodeId>,
    changes: Vec<Change>,
}

impl Journal {
    /// Whether the edit changed nothing.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// This journal followed by a later one, as one edit.
    pub fn then(&self, later: &Journal) -> Journal {
        Journal {
            host: self.host.or(later.host),
            changes: self.changes.iter().chain(&later.changes).cloned().collect(),
        }
    }
}

/// A host's recording in progress and its count of changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recording {
    id: u64,
    host: NodeId,
    journal: Journal,
}

impl Dom {
    /// Starts recording the changes inside `host`; returns the recording's
    /// id for [`Dom::end_journal`]. From now on the host's changes are also
    /// counted, as [`Dom::host_changes`] tells.
    pub fn begin_journal(&mut self, host: NodeId) -> u64 {
        self.watch_host(host);
        // Edits nest only a few deep; recordings a script never ended stop
        // costing anything here.
        if self.recordings.len() >= 16 {
            self.recordings.remove(0);
        }
        self.next_recording += 1;
        let id = self.next_recording;
        self.recordings.push(Recording {
            id,
            host,
            journal: Journal {
                host: Some(host),
                changes: Vec::new(),
            },
        });
        id
    }

    /// Ends a recording and returns what changed; `None` for an unknown id.
    pub fn end_journal(&mut self, id: u64) -> Option<Journal> {
        let position = self
            .recordings
            .iter()
            .position(|recording| recording.id == id)?;
        Some(self.recordings.remove(position).journal)
    }

    /// How many times the content of a host has changed since its first
    /// recording; zero for hosts never recorded.
    pub fn host_changes(&self, host: NodeId) -> u64 {
        self.watched_hosts
            .iter()
            .find(|(watched, _)| *watched == host)
            .map_or(0, |(_, changes)| *changes)
    }

    fn watch_host(&mut self, host: NodeId) {
        if !self
            .watched_hosts
            .iter()
            .any(|(watched, _)| *watched == host)
        {
            self.watched_hosts.push((host, 0));
        }
    }

    /// Where a change to `target` falls: the watched hosts around it, and
    /// whether it is outside the document. `own` tells whether a change to
    /// a host itself changes its content; its attributes do not.
    fn place(&self, target: NodeId, own: bool) -> (Vec<NodeId>, bool) {
        let mut hosts = Vec::new();
        let mut root = target;
        let mut current = if own {
            Some(target)
        } else {
            self.parent(target)
        };
        while let Some(node) = current {
            if self.watched_hosts.iter().any(|(host, _)| *host == node) {
                hosts.push(node);
            }
            root = node;
            current = self.parent(node);
        }
        (hosts, root != Dom::DOCUMENT)
    }

    /// Undoes a journal's changes, latest first. Nodes the document shows
    /// outside the host are left as they are now: they belong to the
    /// application, not to the edit.
    pub fn revert_journal(&mut self, journal: &Journal) {
        for change in journal.changes.iter().rev() {
            let (target, own) = match change {
                Change::Data { node, .. } => (*node, true),
                Change::Attribute { node, .. } => (*node, false),
                Change::Children { parent, .. } => (*parent, true),
            };
            if let Some(host) = journal.host {
                let (hosts, detached) = self.place(target, own);
                if !detached && !hosts.contains(&host) {
                    continue;
                }
            }
            match change {
                Change::Data { node, old } => self.set_data(*node, old),
                Change::Attribute { node, name, old } => match old {
                    Some(value) => {
                        let _ = self.set_attribute(*node, name, value);
                    }
                    None => self.remove_attribute(*node, name),
                },
                Change::Children {
                    parent,
                    added,
                    removed,
                    next,
                } => {
                    for node in added {
                        if self.parent(*node) == Some(*parent) {
                            let _ = self.remove_child(*parent, *node);
                        }
                    }
                    let reference = next.filter(|next| self.parent(*next) == Some(*parent));
                    for node in removed {
                        let _ = self.insert_before(*parent, *node, reference);
                    }
                }
            }
        }
    }

    /// Notes a change for the recordings and counts of the hosts around
    /// `target`.
    /// `own` tells whether a change to a host itself is a change to its
    /// content; its attributes are not.
    fn journal(&mut self, target: NodeId, own: bool, change: impl FnOnce(&Dom) -> Change) {
        if self.watched_hosts.is_empty() {
            return;
        }
        let (inside, detached) = self.place(target, own);
        for (host, changes) in &mut self.watched_hosts {
            if inside.contains(host) {
                *changes += 1;
            }
        }
        let recorded = |recording: &Recording| detached || inside.contains(&recording.host);
        if !self.recordings.iter().any(recorded) {
            return;
        }
        let change = change(self);
        for recording in &mut self.recordings {
            if detached || inside.contains(&recording.host) {
                recording.journal.changes.push(change.clone());
            }
        }
    }

    pub(crate) fn journal_data(&mut self, node: NodeId, old: &str) {
        self.journal(node, true, |_| Change::Data {
            node,
            old: old.to_owned(),
        });
    }

    pub(crate) fn journal_attribute(&mut self, node: NodeId, name: &str, old: Option<&str>) {
        self.journal(node, false, |_| Change::Attribute {
            node,
            name: name.to_owned(),
            old: old.map(str::to_owned),
        });
    }

    pub(crate) fn journal_children(
        &mut self,
        parent: NodeId,
        added: &[NodeId],
        removed: &[NodeId],
        next: Option<NodeId>,
    ) {
        self.journal(parent, true, |_| Change::Children {
            parent,
            added: added.to_vec(),
            removed: removed.to_vec(),
            next,
        });
    }
}

#[cfg(test)]
mod tests;
