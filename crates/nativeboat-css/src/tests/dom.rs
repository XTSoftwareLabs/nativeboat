//! A small DOM over parsed HTML used by selector and cascade tests.

use crate::selector::{Element, state};
use nativeboat_html::Document;

pub(crate) struct Entry<'a> {
    element: &'a nativeboat_html::Element,
    parent: Option<usize>,
    children: Vec<usize>,
}

pub(crate) struct Tree<'a> {
    entries: Vec<Entry<'a>>,
}

impl<'a> Tree<'a> {
    pub(crate) fn new(document: &'a Document) -> Self {
        let mut tree = Tree {
            entries: Vec::new(),
        };
        tree.add(&document.html, None);
        tree
    }

    fn add(&mut self, element: &'a nativeboat_html::Element, parent: Option<usize>) -> usize {
        let index = self.entries.len();
        self.entries.push(Entry {
            element,
            parent,
            children: Vec::new(),
        });
        for child in element.child_elements() {
            let child = self.add(child, Some(index));
            self.entries[index].children.push(child);
        }
        index
    }

    pub(crate) fn by_id(&self, id: &str) -> Node<'_> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.element.attribute("id") == Some(id))
            .unwrap();
        Node { tree: self, index }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Node<'a> {
    pub(crate) tree: &'a Tree<'a>,
    pub(crate) index: usize,
}

impl Node<'_> {
    fn entry(&self) -> &Entry<'_> {
        &self.tree.entries[self.index]
    }

    fn sibling(&self, offset: isize) -> Option<Self> {
        let parent = self.entry().parent?;
        let siblings = &self.tree.entries[parent].children;
        let position = siblings.iter().position(|index| *index == self.index)?;
        let target = position.checked_add_signed(offset)?;
        siblings.get(target).map(|index| Node {
            tree: self.tree,
            index: *index,
        })
    }
}

impl Element for Node<'_> {
    fn local_name(&self) -> &str {
        &self.entry().element.name
    }

    fn id(&self) -> Option<&str> {
        self.entry().element.attribute("id")
    }

    fn has_class(&self, class: &str) -> bool {
        self.entry()
            .element
            .attribute("class")
            .is_some_and(|classes| classes.split_whitespace().any(|name| name == class))
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.entry().element.attribute(name)
    }

    fn parent_element(&self) -> Option<Self> {
        self.entry().parent.map(|index| Node {
            tree: self.tree,
            index,
        })
    }

    fn previous_element_sibling(&self) -> Option<Self> {
        self.sibling(-1)
    }

    fn next_element_sibling(&self) -> Option<Self> {
        self.sibling(1)
    }

    fn first_element_child(&self) -> Option<Self> {
        self.entry().children.first().map(|index| Node {
            tree: self.tree,
            index: *index,
        })
    }

    fn is_same(&self, other: &Self) -> bool {
        self.index == other.index
    }

    fn is_root(&self) -> bool {
        self.entry().parent.is_none()
    }

    fn is_empty(&self) -> bool {
        self.entry()
            .element
            .children
            .iter()
            .all(|child| matches!(child, nativeboat_html::Node::Comment(_)))
    }

    fn state(&self) -> u32 {
        let words = self.entry().element.attribute("data-state").unwrap_or("");
        words
            .split_whitespace()
            .map(|word| match word {
                "hover" => state::HOVER,
                "active" => state::ACTIVE,
                "focus" => state::FOCUS,
                "focus-visible" => state::FOCUS_VISIBLE,
                "focus-within" => state::FOCUS_WITHIN,
                "disabled" => state::DISABLED,
                "checked" => state::CHECKED,
                "indeterminate" => state::INDETERMINATE,
                "required" => state::REQUIRED,
                "editable" => state::EDITABLE,
                "placeholder" => state::PLACEHOLDER_SHOWN,
                "link" => state::LINK,
                "control" => state::FORM_CONTROL,
                other => panic!("unknown state {other}"),
            })
            .fold(0, |flags, flag| flags | flag)
            | if self.local_name() == "a" && self.attribute("href").is_some() {
                state::LINK
            } else {
                0
            }
    }
}
