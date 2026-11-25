//! Live ranges and the character data operations that keep them pointing at
//! the same content.
//!
//! The DOM standard moves every live range boundary when nodes are inserted
//! or removed and when character data is replaced, split or merged, so that
//! a selection survives the edits an editor makes around it. Offsets into
//! character data count UTF-16 code units, as script sees them.

use crate::prelude::*;

use crate::{Dom, DomError, NodeId, NodeKind};

/// A boundary point: a node and an offset into its children or, for
/// character data, its UTF-16 code units.
pub type Boundary = (NodeId, u32);

/// The live ranges of a document; freed slots are reused.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ranges {
    slots: Vec<Option<[Boundary; 2]>>,
    free: Vec<usize>,
    /// The range the document's selection is made of.
    selection: Option<u32>,
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

impl Dom {
    /// Creates a live range and returns its id.
    pub fn create_range(&mut self, start: Boundary, end: Boundary) -> u32 {
        let ranges = &mut self.ranges;
        let slot = ranges.free.pop().unwrap_or(ranges.slots.len());
        if slot == ranges.slots.len() {
            ranges.slots.push(None);
        }
        ranges.slots[slot] = Some([start, end]);
        slot as u32
    }

    /// The start and end of a live range.
    pub fn range(&self, id: u32) -> Option<[Boundary; 2]> {
        self.ranges.slots.get(id as usize).copied().flatten()
    }

    /// Moves a live range.
    pub fn set_range(&mut self, id: u32, start: Boundary, end: Boundary) {
        if let Some(slot @ Some(_)) = self.ranges.slots.get_mut(id as usize) {
            *slot = Some([start, end]);
        }
    }

    /// Forgets a live range, once nothing can use it any more.
    pub fn drop_range(&mut self, id: u32) {
        if let Some(slot @ Some(_)) = self.ranges.slots.get_mut(id as usize) {
            *slot = None;
            self.ranges.free.push(id as usize);
            if self.ranges.selection == Some(id) {
                self.ranges.selection = None;
            }
        }
    }

    /// Makes a live range the document's selection, or clears the selection.
    pub fn set_selection(&mut self, id: Option<u32>) {
        self.ranges.selection = id.filter(|id| self.range(*id).is_some());
    }

    /// The start and end of the document's selection, which moves with the
    /// content like any live range.
    pub fn selection(&self) -> Option<[Boundary; 2]> {
        self.range(self.ranges.selection?)
    }

    /// The child at an index.
    fn child_at(&self, node: NodeId, index: u32) -> Option<NodeId> {
        let mut child = self.first_child(node);
        for _ in 0..index {
            child = self.next_sibling(child?);
        }
        child
    }

    /// The node after `node` in tree order, after its descendants when
    /// `skip` is set.
    fn following(&self, node: NodeId, skip: bool) -> Option<NodeId> {
        if !skip && let Some(child) = self.first_child(node) {
            return Some(child);
        }
        let mut current = node;
        loop {
            if let Some(next) = self.next_sibling(current) {
                return Some(next);
            }
            current = self.parent(current)?;
        }
    }

    /// The text nodes a range covers, in tree order, each with the part of
    /// its data the range covers in UTF-16 code units; empty parts are left
    /// out.
    pub fn range_texts(
        &self,
        [(start, from), (end, to)]: [Boundary; 2],
    ) -> Vec<(NodeId, u32, u32)> {
        let is_text = |node: NodeId| matches!(self.kind(node), NodeKind::Text(_));
        let length =
            |node: NodeId| self.data(node).unwrap_or_default().encode_utf16().count() as u32;
        let mut texts = Vec::new();
        let mut add = |node: NodeId, from: u32, to: u32| {
            if from < to {
                texts.push((node, from, to));
            }
        };
        if start == end && is_text(start) {
            add(start, from, to);
            return texts;
        }
        let mut current = if is_text(start) {
            add(start, from, length(start));
            self.following(start, true)
        } else {
            self.child_at(start, from)
                .or_else(|| self.following(start, true))
        };
        let stop = if is_text(end) {
            Some(end)
        } else {
            self.child_at(end, to).or_else(|| self.following(end, true))
        };
        while let Some(node) = current.filter(|node| Some(*node) != stop) {
            if is_text(node) {
                add(node, 0, length(node));
            }
            current = self.following(node, false);
        }
        if is_text(end) && current == Some(end) {
            add(end, 0, to);
        }
        texts
    }

    /// The number of live ranges.
    pub fn range_count(&self) -> usize {
        self.ranges.slots.len() - self.ranges.free.len()
    }

    /// The node's index among its siblings.
    pub fn index(&self, node: NodeId) -> u32 {
        let mut index = 0;
        let mut current = self.previous_sibling(node);
        while let Some(sibling) = current {
            index += 1;
            current = self.previous_sibling(sibling);
        }
        index
    }

    fn boundaries(&self) -> impl Iterator<Item = &Boundary> {
        self.ranges.slots.iter().flatten().flatten()
    }

    fn adjust(&mut self, mut step: impl FnMut(&Dom, &mut Boundary)) {
        let mut ranges = core::mem::take(&mut self.ranges);
        for boundary in ranges.slots.iter_mut().flatten().flatten() {
            step(self, boundary);
        }
        self.ranges = ranges;
    }

    /// The removing steps, run while `child` is still a child of `parent`:
    /// boundaries inside it move to where it was.
    pub(crate) fn ranges_removing(&mut self, parent: NodeId, child: NodeId) {
        if !self
            .boundaries()
            .any(|(node, _)| *node == parent || self.contains(child, *node))
        {
            return;
        }
        let index = self.index(child);
        self.adjust(|dom, (node, offset)| {
            if dom.contains(child, *node) {
                *node = parent;
                *offset = index;
            } else if *node == parent && *offset > index {
                *offset -= 1;
            }
        });
    }

    /// The insertion steps, run before `count` nodes are inserted into
    /// `parent` before `reference`.
    pub(crate) fn ranges_inserting(
        &mut self,
        parent: NodeId,
        reference: Option<NodeId>,
        count: u32,
    ) {
        if !self.boundaries().any(|(node, _)| *node == parent) {
            return;
        }
        let index = match reference {
            Some(reference) => self.index(reference),
            None => self.children(parent).len() as u32,
        };
        self.adjust(|_, (node, offset)| {
            if *node == parent && *offset > index {
                *offset += count;
            }
        });
    }

    /// `CharacterData.replaceData`; nothing happens for other nodes.
    pub fn replace_data(
        &mut self,
        id: NodeId,
        offset: u32,
        count: u32,
        data: &str,
    ) -> Result<(), DomError> {
        let Some(old) = self.data(id) else {
            return Ok(());
        };
        let old = units(old);
        let length = old.len() as u32;
        if offset > length {
            return Err(DomError::IndexSize(
                "the offset exceeds the length of the data".to_owned(),
            ));
        }
        let count = count.min(length - offset);
        let added = units(data);
        let mut text = old[..offset as usize].to_vec();
        text.extend_from_slice(&added);
        text.extend_from_slice(&old[(offset + count) as usize..]);
        self.write_data(id, String::from_utf16_lossy(&text));
        let added = added.len() as u32;
        self.adjust(|_, (node, at)| {
            if *node == id && *at > offset + count {
                *at = *at + added - count;
            } else if *node == id && *at > offset {
                *at = offset;
            }
        });
        Ok(())
    }

    /// `Text.splitText`: the data after `offset` moves into a new text node
    /// inserted after this one, together with the boundaries in it.
    pub fn split_text(&mut self, id: NodeId, offset: u32) -> Result<NodeId, DomError> {
        let data = units(self.data(id).unwrap_or_default());
        let length = data.len() as u32;
        if offset > length {
            return Err(DomError::IndexSize(
                "the offset exceeds the length of the text".to_owned(),
            ));
        }
        let tail = self.create_text(&String::from_utf16_lossy(&data[offset as usize..]));
        self.node_mut(tail).owner_document = self.owner_document(id);
        if let Some(parent) = self.parent(id) {
            let next = self.next_sibling(id);
            self.insert_before(parent, tail, next)?;
            let index = self.index(id);
            self.adjust(|_, (node, at)| {
                if *node == id && *at > offset {
                    *node = tail;
                    *at -= offset;
                } else if *node == parent && *at == index + 1 {
                    *at += 1;
                }
            });
        }
        self.replace_data(id, offset, length - offset, "")?;
        Ok(tail)
    }

    /// `Node.normalize`: removes empty text nodes and merges adjacent ones,
    /// keeping the boundaries in the merged text.
    pub fn normalize(&mut self, id: NodeId) {
        for node in self.descendants(id) {
            if !matches!(self.kind(node), NodeKind::Text(_)) || self.parent(node).is_none() {
                continue;
            }
            let mut length = units(self.data(node).unwrap_or_default()).len() as u32;
            if length == 0 {
                self.detach(node);
                continue;
            }
            let mut following = Vec::new();
            let mut current = self.next_sibling(node);
            while let Some(sibling) =
                current.filter(|sibling| matches!(self.kind(*sibling), NodeKind::Text(_)))
            {
                following.push(sibling);
                current = self.next_sibling(sibling);
            }
            if following.is_empty() {
                continue;
            }
            let data: String = following
                .iter()
                .filter_map(|sibling| self.data(*sibling))
                .collect();
            let _ = self.replace_data(node, length, 0, &data);
            let parent = self.parent(node);
            for sibling in &following {
                let index = self.index(*sibling);
                self.adjust(|_, (boundary, at)| {
                    if *boundary == *sibling {
                        *boundary = node;
                        *at += length;
                    } else if Some(*boundary) == parent && *at == index {
                        *boundary = node;
                        *at = length;
                    }
                });
                length += units(self.data(*sibling).unwrap_or_default()).len() as u32;
            }
            for sibling in following {
                self.detach(sibling);
            }
        }
    }
}

#[cfg(test)]
mod tests;
