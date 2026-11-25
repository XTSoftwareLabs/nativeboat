//! Tree construction from tokens.

use crate::prelude::*;
use crate::tokenizer::{LineIndex, Token, TokenKind, tokenize};
use crate::{Attribute, Document, Element, Node, Text, is_void_element};

/// Elements that belong in `head` when they appear before the body content.
fn is_head_element(name: &str) -> bool {
    matches!(
        name,
        "base" | "link" | "meta" | "noscript" | "script" | "style" | "template" | "title"
    )
}

/// Start tags that close an open `p` element.
fn closes_paragraph(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "dialog"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "summary"
            | "table"
            | "ul"
    )
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// SVG element names that keep their mixed case.
const SVG_ELEMENTS: &[&str] = &[
    "altGlyph",
    "altGlyphDef",
    "altGlyphItem",
    "animateColor",
    "animateMotion",
    "animateTransform",
    "clipPath",
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDistantLight",
    "feDropShadow",
    "feFlood",
    "feFuncA",
    "feFuncB",
    "feFuncG",
    "feFuncR",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMergeNode",
    "feMorphology",
    "feOffset",
    "fePointLight",
    "feSpecularLighting",
    "feSpotLight",
    "feTile",
    "feTurbulence",
    "foreignObject",
    "glyphRef",
    "linearGradient",
    "radialGradient",
    "textPath",
];

/// SVG attribute names that keep their mixed case.
const SVG_ATTRIBUTES: &[&str] = &[
    "attributeName",
    "attributeType",
    "baseFrequency",
    "baseProfile",
    "calcMode",
    "clipPathUnits",
    "diffuseConstant",
    "edgeMode",
    "filterUnits",
    "glyphRef",
    "gradientTransform",
    "gradientUnits",
    "kernelMatrix",
    "kernelUnitLength",
    "keyPoints",
    "keySplines",
    "keyTimes",
    "lengthAdjust",
    "limitingConeAngle",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "maskContentUnits",
    "maskUnits",
    "numOctaves",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "preserveAlpha",
    "preserveAspectRatio",
    "primitiveUnits",
    "refX",
    "refY",
    "repeatCount",
    "repeatDur",
    "requiredExtensions",
    "requiredFeatures",
    "specularConstant",
    "specularExponent",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stitchTiles",
    "surfaceScale",
    "systemLanguage",
    "tableValues",
    "targetX",
    "targetY",
    "textLength",
    "viewBox",
    "viewTarget",
    "xChannelSelector",
    "yChannelSelector",
    "zoomAndPan",
];

fn adjust_case(name: String, table: &[&str]) -> String {
    table
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(&name))
        .map_or(name, |candidate| (*candidate).to_owned())
}

/// A stack of open elements whose bottom entry is the insertion root.
#[derive(Debug)]
struct OpenElements {
    stack: Vec<Element>,
}

impl OpenElements {
    fn new(root: Element) -> Self {
        Self { stack: vec![root] }
    }

    fn current(&mut self) -> &mut Element {
        self.stack.last_mut().expect("insertion root")
    }

    fn current_name(&self) -> &str {
        &self.stack[self.stack.len() - 1].name
    }

    fn in_foreign_content(&self) -> bool {
        self.stack
            .iter()
            .any(|element| element.name == "svg" || element.name == "math")
    }

    /// Pops the current element into its parent.
    fn pop(&mut self) {
        let element = self.stack.pop().expect("open element");
        self.current().children.push(Node::Element(element));
    }

    /// Finds the nearest open element named `name`, not crossing `boundaries`.
    fn find(&self, names: &[&str], boundaries: &[&str]) -> Option<usize> {
        for index in (1..self.stack.len()).rev() {
            let name = self.stack[index].name.as_str();
            if names.contains(&name) {
                return Some(index);
            }
            if boundaries.contains(&name) {
                return None;
            }
        }
        None
    }

    /// Closes elements until the element at `index` is closed.
    fn close_to(&mut self, index: usize) {
        while self.stack.len() > index {
            self.pop();
        }
    }

    fn close_nearest(&mut self, names: &[&str], boundaries: &[&str]) {
        if let Some(index) = self.find(names, boundaries) {
            self.close_to(index);
        }
    }

    fn append(&mut self, mut node: Node) {
        let current = self.current();
        // A newline right after the start tag of `pre`, `listing` or
        // `textarea` is not part of its content.
        if let Node::Text(text) = &mut node
            && current.children.is_empty()
            && matches!(current.name.as_str(), "pre" | "listing" | "textarea")
            && text.text.starts_with('\n')
        {
            text.text.remove(0);
            if text.text.is_empty() {
                return;
            }
        }
        if let (Node::Text(text), Some(Node::Text(previous))) = (&node, current.children.last_mut())
        {
            previous.text.push_str(&text.text);
            return;
        }
        current.children.push(node);
    }

    fn start_tag(&mut self, mut element: Element, self_closing: bool, honor_self_closing: bool) {
        let name = element.name.clone();
        const SCOPE: &[&str] = &["button", "table", "td", "th", "template"];
        if closes_paragraph(&name) {
            self.close_nearest(&["p"], SCOPE);
        }
        match name.as_str() {
            "li" => self.close_nearest(&["li"], &["ul", "ol", "table", "td", "th", "template"]),
            "dd" | "dt" => self.close_nearest(&["dd", "dt"], &["dl", "table", "template"]),
            "option" => self.close_nearest(&["option"], &["select", "datalist", "optgroup"]),
            "optgroup" => {
                self.close_nearest(&["option"], &["select", "datalist", "optgroup"]);
                self.close_nearest(&["optgroup"], &["select"]);
            }
            "a" => self.close_nearest(&["a"], SCOPE),
            "button" => self.close_nearest(&["button"], &["table", "td", "th", "template"]),
            "tr" => {
                self.close_nearest(&["tr"], &["table", "tbody", "thead", "tfoot"]);
                if self.current_name() == "table" {
                    self.push_implied("tbody");
                }
            }
            "td" | "th" => {
                self.close_nearest(&["td", "th"], &["tr", "table"]);
                if matches!(self.current_name(), "table" | "tbody" | "thead" | "tfoot") {
                    if self.current_name() == "table" {
                        self.push_implied("tbody");
                    }
                    self.push_implied("tr");
                }
            }
            "tbody" | "thead" | "tfoot" => {
                self.close_nearest(&["tbody", "thead", "tfoot"], &["table"]);
            }
            _ => {}
        }
        if is_heading(&name) && is_heading(self.current_name()) {
            self.pop();
        }
        let foreign = name == "svg" || name == "math" || self.in_foreign_content();
        if foreign {
            element.name = adjust_case(element.name, SVG_ELEMENTS);
            for attribute in &mut element.attributes {
                attribute.name = adjust_case(core::mem::take(&mut attribute.name), SVG_ATTRIBUTES);
            }
        }
        if is_void_element(&element.name) || (self_closing && (foreign || honor_self_closing)) {
            self.current().children.push(Node::Element(element));
        } else {
            self.stack.push(element);
        }
    }

    fn push_implied(&mut self, name: &str) {
        self.stack.push(Element::new(name));
    }

    fn end_tag(&mut self, name: &str, position: crate::Position) {
        if name == "br" {
            let mut element = Element::new("br");
            element.position = position;
            self.start_tag(element, false, false);
            return;
        }
        let index = self
            .stack
            .iter()
            .rposition(|element| element.name.eq_ignore_ascii_case(name));
        match index {
            Some(index) if index > 0 => self.close_to(index),
            _ if name == "p" => {
                let mut element = Element::new("p");
                element.position = position;
                self.current().children.push(Node::Element(element));
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Element {
        self.close_to(1);
        self.stack.pop().expect("insertion root")
    }
}

fn merge_attributes(target: &mut Element, attributes: Vec<Attribute>) {
    for attribute in attributes {
        if target.attribute(&attribute.name).is_none() {
            target.attributes.push(attribute);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
}

/// Preprocesses the input stream as the HTML standard does: carriage return
/// and line feed pairs, and lone carriage returns, become line feeds.
fn normalize_newlines(source: &str) -> alloc::borrow::Cow<'_, str> {
    if source.contains('\r') {
        alloc::borrow::Cow::Owned(source.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        alloc::borrow::Cow::Borrowed(source)
    }
}

/// Parses a complete document, creating `html`, `head` and `body` if needed.
pub fn parse_document(source: &str) -> Document {
    let source = &*normalize_newlines(source);
    let index = LineIndex::new(source);
    let mut doctype = None;
    let mut html = Element::new("html");
    let mut head = OpenElements::new(Element::new("head"));
    let mut body = OpenElements::new(Element::new("body"));
    let mut mode = Mode::BeforeHead;
    for Token { kind, offset } in tokenize(source) {
        let position = index.position(offset);
        match kind {
            TokenKind::Doctype(name) => {
                if mode == Mode::BeforeHead && doctype.is_none() {
                    doctype = Some(name);
                }
            }
            TokenKind::Comment(text) => match mode {
                Mode::BeforeHead => {}
                Mode::InHead | Mode::AfterHead => head.append(Node::Comment(text)),
                Mode::InBody => body.append(Node::Comment(text)),
            },
            TokenKind::Text(text) => {
                if mode < Mode::InBody {
                    let in_raw_head = mode == Mode::InHead && head.stack.len() > 1;
                    if in_raw_head {
                        head.append(Node::Text(Text { text, position }));
                        continue;
                    }
                    if text.trim().is_empty() {
                        continue;
                    }
                    mode = Mode::InBody;
                    let trimmed = text.trim_start();
                    let skipped = text.len() - trimmed.len();
                    let position = index.position(offset + skipped);
                    body.append(Node::Text(Text {
                        text: trimmed.to_owned(),
                        position,
                    }));
                    continue;
                }
                body.append(Node::Text(Text { text, position }));
            }
            TokenKind::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                if name == "html" {
                    merge_attributes(&mut html, attributes);
                    continue;
                }
                if name == "head" {
                    if mode == Mode::BeforeHead {
                        mode = Mode::InHead;
                        merge_attributes(&mut head.stack[0], attributes);
                        head.stack[0].position = position;
                    }
                    continue;
                }
                if name == "body" {
                    if mode < Mode::InBody {
                        mode = Mode::InBody;
                        body.stack[0].position = position;
                    }
                    merge_attributes(&mut body.stack[0], attributes);
                    continue;
                }
                let element = Element {
                    name,
                    attributes,
                    children: Vec::new(),
                    position,
                };
                if mode < Mode::InBody && is_head_element(&element.name) {
                    if mode == Mode::BeforeHead {
                        mode = Mode::InHead;
                    }
                    head.start_tag(element, self_closing, false);
                    continue;
                }
                mode = Mode::InBody;
                body.start_tag(element, self_closing, false);
            }
            TokenKind::EndTag(name) => match mode {
                _ if name == "html" || name == "body" => {}
                Mode::BeforeHead | Mode::InHead | Mode::AfterHead if name == "head" => {
                    head.close_to(1);
                    mode = Mode::AfterHead;
                }
                Mode::InHead => head.end_tag(&name, position),
                Mode::BeforeHead | Mode::AfterHead => {}
                Mode::InBody => body.end_tag(&name, position),
            },
        }
    }
    html.children.push(Node::Element(head.finish()));
    html.children.push(Node::Element(body.finish()));
    Document { doctype, html }
}

/// Parses an HTML fragment such as the value assigned to `innerHTML`.
pub fn parse_fragment(source: &str) -> Vec<Node> {
    parse_fragment_in(source, "#fragment")
}

/// Parses a fragment as the content of an element named `context`, such as `svg`.
pub fn parse_fragment_in(source: &str, context: &str) -> Vec<Node> {
    parse_fragment_in_mode(source, context, false)
}

/// Parses a template fragment using Vue's XML-style self-closing tag syntax.
pub fn parse_vue_fragment_in(source: &str, context: &str) -> Vec<Node> {
    parse_fragment_in_mode(source, context, true)
}

fn parse_fragment_in_mode(source: &str, context: &str, honor_self_closing: bool) -> Vec<Node> {
    let source = &*normalize_newlines(source);
    let index = LineIndex::new(source);
    let mut open = OpenElements::new(Element::new(context));
    for Token { kind, offset } in tokenize(source) {
        let position = index.position(offset);
        match kind {
            TokenKind::Doctype(_) => {}
            TokenKind::Comment(text) => open.append(Node::Comment(text)),
            TokenKind::Text(text) => open.append(Node::Text(Text { text, position })),
            TokenKind::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                if matches!(name.as_str(), "html" | "head" | "body") {
                    continue;
                }
                let element = Element {
                    name,
                    attributes,
                    children: Vec::new(),
                    position,
                };
                open.start_tag(element, self_closing, honor_self_closing);
            }
            TokenKind::EndTag(name) => open.end_tag(&name, position),
        }
    }
    open.finish().children
}

#[cfg(test)]
mod tests;
