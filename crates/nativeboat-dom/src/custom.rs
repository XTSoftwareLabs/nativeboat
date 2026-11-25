//! Custom elements: which elements have custom element names, which of
//! them a definition has upgraded (`:defined`), and the elements of a
//! subtree a registry looks at when it upgrades or runs reactions.

use crate::{Dom, Namespace, NodeId};
use alloc::vec::Vec;

/// Names that contain a hyphen but belong to SVG and MathML, which no
/// definition may take.
const RESERVED: [&str; 8] = [
    "annotation-xml",
    "color-profile",
    "font-face",
    "font-face-src",
    "font-face-uri",
    "font-face-format",
    "font-face-name",
    "missing-glyph",
];

/// Whether `name` is a valid custom element name: it starts with a
/// lowercase ASCII letter, contains a hyphen, has no uppercase ASCII
/// letters and is not reserved.
pub fn is_custom_element_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && name.contains('-')
        && !name.chars().any(|char| char.is_ascii_uppercase())
        && name.chars().all(|char| {
            matches!(char, '-' | '.' | '_' | '0'..='9' | 'a'..='z' | '\u{b7}')
                || u32::from(char) >= 0xc0
        })
        && !RESERVED.contains(&name)
}

impl Dom {
    /// Whether the element has a custom element name, in the HTML
    /// namespace.
    fn has_custom_name(&self, id: NodeId) -> bool {
        self.namespace(id) == Some(Namespace::Html)
            && self.local_name(id).is_some_and(is_custom_element_name)
    }

    /// Whether `:defined` fails to match the element: it has a custom
    /// element name and no definition has upgraded it.
    pub fn is_undefined_custom_element(&self, id: NodeId) -> bool {
        self.has_custom_name(id) && !self.custom_defined.contains(&id)
    }

    /// Marks an element as upgraded by its definition, so `:defined`
    /// matches it.
    pub fn set_custom_defined(&mut self, id: NodeId) {
        if self.has_custom_name(id) && self.custom_defined.insert(id) {
            self.version += 1;
        }
    }

    /// The elements of `root`'s subtree and of the shadow trees in it,
    /// `root` first and then in shadow-including tree order, that a
    /// definition could apply to: those with custom element names, and
    /// those with an `is` attribute, which names the definition of a
    /// customized built-in element.
    pub fn custom_element_candidates(&self, root: NodeId) -> Vec<NodeId> {
        core::iter::once(root)
            .chain(self.shadow_including_descendants(root))
            .filter(|node| {
                self.namespace(*node) == Some(Namespace::Html)
                    && (self.has_custom_name(*node) || self.has_attribute(*node, "is"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
