//! Building the DOM from parsed HTML and `innerHTML`/`outerHTML`.

use crate::prelude::*;

use crate::element::Namespace;
use crate::{Dom, NodeId, NodeKind};
use nativeboat_html::{
    Attribute, Element, Node, Position, Text, parse_fragment_in, serialize, serialize_children,
};

impl Dom {
    /// Parses a detached HTML document in this arena, without changing the page.
    pub fn parse_html_document(&mut self, source: &str) -> NodeId {
        let source = nativeboat_html::parse_document(source);
        let document = self.allocate(NodeKind::Document);
        let html = self.import_element(&source.html, Namespace::Html);
        self.link(document, html, None);
        document
    }

    /// Creates a DOM from a parsed document.
    pub fn from_document(document: &nativeboat_html::Document) -> Dom {
        let mut dom = Dom::new();
        let html = dom.import_element(&document.html, Namespace::Html);
        dom.link(Dom::DOCUMENT, html, None);
        dom
    }

    fn import_element(&mut self, source: &Element, namespace: Namespace) -> NodeId {
        let namespace = match source.name.as_str() {
            "svg" => Namespace::Svg,
            "math" => Namespace::MathMl,
            _ => namespace,
        };
        let node = self.allocate(NodeKind::Element(crate::element::ElementData::new(
            namespace,
            source.name.clone(),
        )));
        for attribute in &source.attributes {
            let _ = self.set_attribute(node, &attribute.name, &attribute.value);
        }
        let encoding = source
            .attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case("encoding"))
            .map(|attribute| attribute.value.as_str());
        let child_namespace = children_namespace(&source.name, namespace, encoding);
        for child in &source.children {
            let imported = self.import_node(child, child_namespace);
            self.link(node, imported, None);
        }
        node
    }

    fn import_node(&mut self, source: &Node, namespace: Namespace) -> NodeId {
        match source {
            Node::Element(element) => self.import_element(element, namespace),
            Node::Text(text) => self.create_text(&text.text),
            Node::Comment(text) => self.create_comment(text),
        }
    }

    fn export_node(&self, id: NodeId) -> Node {
        match self.kind(id) {
            NodeKind::Text(text) => Node::Text(Text {
                text: text.clone(),
                position: Position::default(),
            }),
            NodeKind::Comment(text) => Node::Comment(text.clone()),
            _ => Node::Element(self.export_element(id)),
        }
    }

    fn export_element(&self, id: NodeId) -> Element {
        let name = self.local_name(id).unwrap_or("#fragment").to_owned();
        Element {
            name,
            attributes: self
                .attributes(id)
                .into_iter()
                .map(|(name, value)| Attribute { name, value })
                .collect(),
            children: self
                .children(id)
                .into_iter()
                .map(|child| self.export_node(child))
                .collect(),
            position: Position::default(),
        }
    }

    /// `innerHTML`.
    pub fn inner_html(&self, id: NodeId) -> String {
        serialize_children(&self.export_element(id))
    }

    /// `outerHTML`.
    pub fn outer_html(&self, id: NodeId) -> String {
        serialize(&self.export_node(id))
    }

    /// Parses HTML into a fragment of new nodes.
    pub fn parse_html_fragment(&mut self, html: &str, context: NodeId) -> NodeId {
        let namespace = self.namespace(context).unwrap_or(Namespace::Html);
        let namespace = match self.local_name(context) {
            Some(name) => {
                children_namespace(name, namespace, self.get_attribute(context, "encoding"))
            }
            None => namespace,
        };
        let fragment = self.create_fragment();
        let document = if matches!(self.kind(context), NodeKind::Document) {
            context
        } else {
            self.owner_document(context).unwrap_or(Self::DOCUMENT)
        };
        self.adopt_document(fragment, document);
        let context = match namespace {
            Namespace::Html => "#fragment",
            Namespace::Svg => "svg",
            Namespace::MathMl => "math",
        };
        for node in parse_fragment_in(html, context) {
            let imported = self.import_node(&node, namespace);
            self.link(fragment, imported, None);
        }
        fragment
    }

    /// Sets `innerHTML`.
    pub fn set_inner_html(&mut self, id: NodeId, html: &str) {
        let fragment = self.parse_html_fragment(html, id);
        self.clear_children(id);
        let _ = self.insert_before(id, fragment, None);
    }
}

/// The namespace of the elements parsed inside an element: HTML inside the
/// HTML integration points of SVG and MathML (the HTML parser's foreign
/// content rules), the element's own namespace otherwise.
fn children_namespace(name: &str, namespace: Namespace, encoding: Option<&str>) -> Namespace {
    let integration = match namespace {
        Namespace::Svg => matches!(name, "foreignObject" | "desc" | "title"),
        Namespace::MathMl => {
            matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext")
                || (name == "annotation-xml"
                    && encoding.is_some_and(|encoding| {
                        encoding.eq_ignore_ascii_case("text/html")
                            || encoding.eq_ignore_ascii_case("application/xhtml+xml")
                    }))
        }
        Namespace::Html => false,
    };
    if integration {
        Namespace::Html
    } else {
        namespace
    }
}

#[cfg(test)]
mod tests;
