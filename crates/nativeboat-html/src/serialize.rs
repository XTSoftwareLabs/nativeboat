//! HTML serialization used by `innerHTML` and `outerHTML`.

use crate::prelude::*;
use crate::{Element, Node, is_raw_text_element, is_void_element};

/// Escapes text content.
pub fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\u{a0}' => escaped.push_str("&nbsp;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Escapes an attribute value for a double-quoted attribute.
pub fn escape_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\u{a0}' => escaped.push_str("&nbsp;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Serializes a node, including the element itself (`outerHTML`).
pub fn serialize(node: &Node) -> String {
    let mut html = String::new();
    write_node(node, false, &mut html);
    html
}

/// Serializes the children of a node (`innerHTML`).
pub fn serialize_children(element: &Element) -> String {
    let mut html = String::new();
    let raw = is_raw_text_element(&element.name);
    for child in &element.children {
        write_node(child, raw, &mut html);
    }
    html
}

fn write_node(node: &Node, raw_parent: bool, html: &mut String) {
    match node {
        Node::Text(text) if raw_parent => html.push_str(&text.text),
        Node::Text(text) => html.push_str(&escape_text(&text.text)),
        Node::Comment(text) => {
            html.push_str("<!--");
            html.push_str(text);
            html.push_str("-->");
        }
        Node::Element(element) => {
            html.push('<');
            html.push_str(&element.name);
            for attribute in &element.attributes {
                html.push(' ');
                html.push_str(&attribute.name);
                html.push_str("=\"");
                html.push_str(&escape_attribute(&attribute.value));
                html.push('"');
            }
            html.push('>');
            if is_void_element(&element.name) {
                return;
            }
            html.push_str(&serialize_children(element));
            html.push_str("</");
            html.push_str(&element.name);
            html.push('>');
        }
    }
}
