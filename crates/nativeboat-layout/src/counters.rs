//! CSS counters: `counter-reset`, `counter-increment` and `counter-set`
//! applied in document order, as CSS Lists scopes them, and resolved for
//! the `counter()` and `counters()` of generated content.

use crate::prelude::*;

use crate::styles::Styles;
use nativeboat_css::types::{Content, ContentItem, Display, ListStyleType};
use nativeboat_css::{ComputedStyle, PseudoElement};
use nativeboat_dom::{Dom, NodeId};

/// The text of each counter item in the generated content of an element's
/// `::before` (0) or `::after` (1), in content order.
pub(crate) type Resolved = HashMap<(NodeId, u8), Vec<String>>;

/// Whether generated content shows counters.
pub(crate) fn shows_counters(items: &[ContentItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item, ContentItem::Counter(..) | ContentItem::Counters(..)))
}

/// Resolves the counters of all generated content in the document.
pub(crate) fn resolve(dom: &Dom, styles: &Styles) -> Resolved {
    let mut walk = Walk {
        dom,
        styles,
        stack: Vec::new(),
        resolved: HashMap::new(),
    };
    if let Some(root) = dom.document_element() {
        walk.element(root);
    }
    walk.resolved
}

/// A counter instance, in the scope of its owner's children: an element's
/// changes are seen by itself, its descendants and its following siblings.
struct Instance {
    name: String,
    value: i64,
    owner: NodeId,
}

struct Walk<'a> {
    dom: &'a Dom,
    styles: &'a Styles,
    /// Counter instances, innermost last.
    stack: Vec<Instance>,
    resolved: Resolved,
}

impl Walk<'_> {
    fn element(&mut self, node: NodeId) {
        let styles = self.styles;
        let Some(style) = styles.get(node) else {
            return;
        };
        // Elements without boxes change no counters.
        if style.display == Display::None {
            return;
        }
        let owner = self.dom.parent(node).unwrap_or(node);
        self.changes(style, owner);
        self.generated(node, &PseudoElement::Before, 0);
        let mut child = self.dom.first_child(node);
        while let Some(current) = child {
            if self.dom.is_element(current) {
                self.element(current);
            }
            child = self.dom.next_sibling(current);
        }
        self.generated(node, &PseudoElement::After, 1);
        self.stack.retain(|instance| instance.owner != node);
    }

    /// A pseudo-element is its element's child: its changes are seen by its
    /// element's children after it.
    fn generated(&mut self, node: NodeId, pseudo: &PseudoElement, key: u8) {
        let styles = self.styles;
        let Some(style) = styles.pseudo(node, pseudo) else {
            return;
        };
        if style.display == Display::None {
            return;
        }
        let Content::Items(items) = &style.content else {
            return;
        };
        self.changes(style, node);
        if !shows_counters(items) {
            return;
        }
        let texts = items
            .iter()
            .filter_map(|item| match item {
                ContentItem::Counter(name, kind) => {
                    let value = self.innermost(name, node).value;
                    Some(counter_text(*kind, value))
                }
                ContentItem::Counters(name, separator, kind) => {
                    self.innermost(name, node);
                    let values: Vec<String> = self
                        .stack
                        .iter()
                        .filter(|instance| instance.name == *name)
                        .map(|instance| counter_text(*kind, instance.value))
                        .collect();
                    Some(values.join(separator))
                }
                _ => None,
            })
            .collect();
        self.resolved.insert((node, key), texts);
    }

    /// Applies an element's counter changes in CSS order: resets, then
    /// increments (with a list item's implicit one), then sets.
    fn changes(&mut self, style: &ComputedStyle, owner: NodeId) {
        for (name, value) in &style.counter_reset.0 {
            // A reset by an element in an instance's scope at the same level
            // replaces it; a deeper one nests a new instance.
            match self
                .stack
                .iter_mut()
                .rev()
                .find(|instance| instance.name == *name)
            {
                Some(instance) if instance.owner == owner => instance.value = i64::from(*value),
                _ => self.stack.push(Instance {
                    name: name.clone(),
                    value: i64::from(*value),
                    owner,
                }),
            }
        }
        let increments = &style.counter_increment.0;
        for (name, step) in increments {
            let instance = self.innermost(name, owner);
            instance.value = instance.value.saturating_add(i64::from(*step));
        }
        if style.display == Display::ListItem
            && !increments.iter().any(|(name, _)| name == "list-item")
        {
            self.innermost("list-item", owner).value += 1;
        }
        for (name, value) in &style.counter_set.0 {
            self.innermost(name, owner).value = i64::from(*value);
        }
    }

    /// The innermost instance of a counter, instantiated at zero when there
    /// is none.
    fn innermost(&mut self, name: &str, owner: NodeId) -> &mut Instance {
        let index = match self
            .stack
            .iter()
            .rposition(|instance| instance.name == name)
        {
            Some(index) => index,
            None => {
                self.stack.push(Instance {
                    name: name.to_owned(),
                    value: 0,
                    owner,
                });
                self.stack.len() - 1
            }
        };
        &mut self.stack[index]
    }
}

/// A number in letters: 1 is `A`, 26 is `Z`, 27 is `AA`.
pub(crate) fn alphabetic(mut value: i64) -> String {
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        letters.push(char::from(b'A' + (value % 26) as u8));
        value /= 26;
    }
    letters.iter().rev().collect()
}

/// A number in Roman numerals, for 1 to 3999.
pub(crate) fn roman(mut value: i64) -> String {
    const NUMERALS: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut text = String::new();
    for (amount, numeral) in NUMERALS {
        while value >= amount {
            text.push_str(numeral);
            value -= amount;
        }
    }
    text
}

/// A counter value in a counter style; values out of a style's range use
/// decimal, as CSS Counter Styles' fallback does.
pub(crate) fn counter_text(kind: ListStyleType, value: i64) -> String {
    let in_range = |limit: i64| value > 0 && value <= limit;
    match kind {
        ListStyleType::None => String::new(),
        ListStyleType::Disc => "\u{2022}".to_owned(),
        ListStyleType::Circle => "\u{25E6}".to_owned(),
        ListStyleType::Square => "\u{25AA}".to_owned(),
        ListStyleType::DisclosureClosed => "\u{25B8}".to_owned(),
        ListStyleType::DisclosureOpen => "\u{25BE}".to_owned(),
        ListStyleType::DecimalLeadingZero if value < 0 => format!("-{:02}", -value),
        ListStyleType::DecimalLeadingZero => format!("{value:02}"),
        ListStyleType::LowerAlpha if in_range(i64::MAX) => alphabetic(value).to_lowercase(),
        ListStyleType::UpperAlpha if in_range(i64::MAX) => alphabetic(value),
        ListStyleType::LowerRoman if in_range(3999) => roman(value).to_lowercase(),
        ListStyleType::UpperRoman if in_range(3999) => roman(value),
        _ => format!("{value}"),
    }
}

#[cfg(test)]
#[path = "counters/tests.rs"]
mod tests;
