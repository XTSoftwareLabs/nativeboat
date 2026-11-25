//! Lightweight HTML parser.
//!
//! The parser turns renderer HTML into a small DOM IR. It follows the HTML
//! tokenization rules that matter for application markup (raw text elements,
//! character references, void elements, implied end tags, implicit
//! `html`/`head`/`body`) without trying to be a complete browser parser.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

mod entities;
mod serialize;
mod tokenizer;
mod tree;

use crate::prelude::*;
use serde::{Deserialize, Serialize};

pub use entities::decode_entities;
pub use serialize::{escape_attribute, escape_text, serialize, serialize_children};
pub use tree::{parse_document, parse_fragment, parse_fragment_in, parse_vue_fragment_in};

/// A line and column in the source, both starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Position {
    /// Line number, starting at 1.
    pub line: u32,
    /// Column number in characters, starting at 1.
    pub column: u32,
}

/// An attribute of an element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    /// Attribute name, lowercased for HTML elements.
    pub name: String,
    /// Decoded attribute value; empty for boolean attributes.
    pub value: String,
}

/// An element and its subtree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Element {
    /// Tag name, lowercased for HTML elements.
    pub name: String,
    /// Attributes in source order.
    pub attributes: Vec<Attribute>,
    /// Child nodes.
    pub children: Vec<Node>,
    /// Location of the start tag.
    pub position: Position,
}

impl Element {
    /// Creates an element without attributes or children.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            attributes: Vec::new(),
            children: Vec::new(),
            position: Position::default(),
        }
    }

    /// Returns the value of an attribute.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }

    /// Returns the child elements.
    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(Node::as_element)
    }

    /// Returns the concatenated text of all descendant text nodes.
    pub fn text_content(&self) -> String {
        let mut text = String::new();
        collect_text(&self.children, &mut text);
        text
    }

    /// Visits this element and all descendant elements in document order.
    pub fn walk<'a>(&'a self, visit: &mut dyn FnMut(&'a Element)) {
        visit(self);
        for child in self.child_elements() {
            child.walk(visit);
        }
    }
}

fn collect_text(nodes: &[Node], text: &mut String) {
    for node in nodes {
        match node {
            Node::Text(value) => text.push_str(&value.text),
            Node::Element(element) => collect_text(&element.children, text),
            Node::Comment(_) => {}
        }
    }
}

/// A run of character data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    /// Decoded text.
    pub text: String,
    /// Location of the first character.
    pub position: Position,
}

/// A node of the DOM IR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Node {
    /// An element.
    Element(Element),
    /// Character data.
    Text(Text),
    /// A comment.
    Comment(String),
}

impl Node {
    /// Returns the element if this node is one.
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(element) => Some(element),
            _ => None,
        }
    }
}

/// A parsed HTML document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// The doctype name, if the document declared one.
    pub doctype: Option<String>,
    /// The `html` element, which always contains `head` and `body`.
    pub html: Element,
}

impl Document {
    /// Returns the `head` element.
    pub fn head(&self) -> &Element {
        self.html.children[0].as_element().expect("head element")
    }

    /// Returns the `body` element.
    pub fn body(&self) -> &Element {
        self.html.children[1].as_element().expect("body element")
    }

    /// Returns the text of the first `title` element.
    pub fn title(&self) -> Option<String> {
        let mut title = None;
        self.html.walk(&mut |element| {
            if title.is_none() && element.name == "title" {
                title = Some(element.text_content());
            }
        });
        title
    }

    /// Returns all elements in document order.
    pub fn elements(&self) -> Vec<&Element> {
        let mut elements = Vec::new();
        self.html.walk(&mut |element| elements.push(element));
        elements
    }
}

/// Elements that never have children or an end tag.
pub fn is_void_element(name: &str) -> bool {
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
            | "source"
            | "track"
            | "wbr"
    )
}

/// Elements whose content is raw text without markup or character references.
pub fn is_raw_text_element(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes"
    )
}

/// Elements whose content is text with character references but no markup.
pub fn is_escapable_raw_text_element(name: &str) -> bool {
    matches!(name, "textarea" | "title")
}

#[cfg(test)]
mod tests;
