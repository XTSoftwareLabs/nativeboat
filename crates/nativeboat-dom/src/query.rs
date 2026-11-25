//! Selector matching and element queries.

use crate::prelude::*;

use crate::{Dom, DomError, NodeId};
use core::cell::Cell;
use nativeboat_css::SelectorList;
use nativeboat_css::selector::state;

/// An element of a [`Dom`] as seen by selector matching.
#[derive(Debug, Clone)]
pub struct ElementRef<'d> {
    /// The DOM.
    pub dom: &'d Dom,
    /// The element.
    pub id: NodeId,
    /// The DOM is immutably borrowed for this view's lifetime.
    state: Cell<Option<u32>>,
    /// The element `:scope` matches; the document element when `None`.
    scope: Option<NodeId>,
    /// The ancestors' features, gathered once for all the rules tested.
    filter: Cell<Option<nativeboat_css::selector::AncestorFilter>>,
    /// Whether this is a shadow host as its own shadow tree's rules see it.
    featureless: bool,
}

impl<'d> ElementRef<'d> {
    /// Wraps an element.
    pub fn new(dom: &'d Dom, id: NodeId) -> Self {
        ElementRef::scoped(dom, id, None)
    }

    /// Wraps an element of a query whose `:scope` is `scope`.
    pub fn scoped(dom: &'d Dom, id: NodeId, scope: Option<NodeId>) -> Self {
        ElementRef {
            dom,
            id,
            state: Cell::new(None),
            scope,
            filter: Cell::new(None),
            featureless: false,
        }
    }

    fn with(&self, id: NodeId) -> Self {
        ElementRef::scoped(self.dom, id, self.scope)
    }

    fn host(&self, id: NodeId) -> Self {
        ElementRef {
            featureless: true,
            ..self.with(id)
        }
    }

    /// A shadow root's id as a stylesheet scope.
    fn scope_of(root: NodeId) -> u32 {
        root.0
    }
}

impl nativeboat_css::Element for ElementRef<'_> {
    fn local_name(&self) -> &str {
        self.dom.local_name(self.id).unwrap_or("")
    }

    fn id(&self) -> Option<&str> {
        self.dom.element_id(self.id)
    }

    fn has_class(&self, class: &str) -> bool {
        self.dom.has_class(self.id, class)
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.dom.get_attribute(self.id, name)
    }

    // Above the top of a shadow tree is its host, which only `:host`
    // selectors match; nothing is above or beside that host.
    fn parent_element(&self) -> Option<Self> {
        if self.featureless {
            return None;
        }
        let parent = self.dom.parent(self.id)?;
        if self.dom.is_element(parent) {
            return Some(self.with(parent));
        }
        self.dom.shadow_host(parent).map(|host| self.host(host))
    }

    fn previous_element_sibling(&self) -> Option<Self> {
        if self.featureless {
            return None;
        }
        self.dom
            .previous_element_sibling(self.id)
            .map(|id| self.with(id))
    }

    fn next_element_sibling(&self) -> Option<Self> {
        if self.featureless {
            return None;
        }
        self.dom
            .next_element_sibling(self.id)
            .map(|id| self.with(id))
    }

    fn first_element_child(&self) -> Option<Self> {
        if self.featureless {
            return None;
        }
        self.dom
            .child_elements(self.id)
            .first()
            .map(|id| self.with(*id))
    }

    fn is_same(&self, other: &Self) -> bool {
        self.id == other.id && self.featureless == other.featureless
    }

    fn is_root(&self) -> bool {
        !self.featureless && self.dom.document_element() == Some(self.id)
    }

    fn tree_scope(&self) -> u32 {
        if !self.dom.has_shadow_roots() {
            return 0;
        }
        let root = self.dom.tree_root(self.id);
        if self.dom.is_shadow_root(root) {
            ElementRef::scope_of(root)
        } else {
            0
        }
    }

    fn shadow_scope(&self) -> Option<u32> {
        self.dom.shadow_root(self.id).map(ElementRef::scope_of)
    }

    fn as_featureless_host(&self) -> Option<Self> {
        self.dom.shadow_root(self.id).map(|_| self.host(self.id))
    }

    fn is_featureless_host(&self) -> bool {
        self.featureless
    }

    fn featured(&self) -> Option<Self> {
        Some(self.with(self.id))
    }

    fn host_context_parent(&self) -> Option<Self> {
        let parent = self.dom.parent_or_host(self.id)?;
        let parent = if self.dom.is_element(parent) {
            parent
        } else {
            self.dom.shadow_host(parent)?
        };
        Some(self.with(parent))
    }

    fn assigned_slot(&self) -> Option<Self> {
        if !self.dom.has_shadow_roots() {
            return None;
        }
        self.dom.assigned_slot(self.id).map(|slot| self.with(slot))
    }

    fn language(&self) -> Option<String> {
        self.dom.language(self.id).map(ToOwned::to_owned)
    }

    fn is_rtl(&self) -> bool {
        self.dom.is_rtl(self.id)
    }

    fn is_scope(&self) -> bool {
        match self.scope {
            Some(scope) => scope == self.id,
            None => self.is_root(),
        }
    }

    fn is_empty(&self) -> bool {
        self.dom.children(self.id).into_iter().all(|child| {
            matches!(self.dom.kind(child), crate::NodeKind::Comment(_))
                || self.dom.data(child).is_some_and(str::is_empty)
        })
    }

    fn state(&self) -> u32 {
        self.state.get().unwrap_or_else(|| {
            let flags = self.dom.element_state(self.id);
            self.state.set(Some(flags));
            flags
        })
    }

    fn ancestor_filter(&self) -> Option<nativeboat_css::selector::AncestorFilter> {
        if let Some(filter) = self.filter.get() {
            return Some(filter);
        }
        let mut filter = nativeboat_css::selector::AncestorFilter::default();
        let mut current = self.dom.parent_element(self.id);
        while let Some(ancestor) = current {
            filter.insert_element(
                self.dom.local_name(ancestor).unwrap_or(""),
                self.dom.element_id(ancestor),
                self.dom.get_attribute(ancestor, "class"),
            );
            current = self.dom.parent_element(ancestor);
        }
        self.filter.set(Some(filter));
        Some(filter)
    }
}

const TEXT_INPUT_TYPES: &[&str] = &[
    "text",
    "search",
    "url",
    "tel",
    "email",
    "password",
    "number",
    "date",
    "time",
    "datetime-local",
    "month",
    "week",
];

/// Whether a character is strongly right-to-left: Hebrew, Arabic, Syriac,
/// Thaana, N'Ko and the other scripts of those blocks and their forms.
fn strong_rtl(character: char) -> bool {
    matches!(
        u32::from(character),
        0x0590..=0x08ff | 0xfb1d..=0xfdff | 0xfe70..=0xfeff | 0x10800..=0x10fff | 0x1e800..=0x1efff
    )
}

impl Dom {
    /// The language of an element, `:lang()` tests: the `lang` or
    /// `xml:lang` attribute of the element or of its nearest shadow-including
    /// ancestor element that has one.
    pub fn language(&self, id: NodeId) -> Option<&str> {
        let mut current = Some(id);
        while let Some(node) = current {
            if self.is_element(node)
                && let Some(language) = self
                    .get_attribute(node, "xml:lang")
                    .or_else(|| self.get_attribute(node, "lang"))
            {
                return Some(language);
            }
            current = self.parent_or_host(node);
        }
        None
    }

    /// Whether an element's directionality, which `:dir()` tests, is
    /// right-to-left: by the `dir` attribute of the element or its nearest
    /// shadow-including ancestor element that has one, where `auto`, and
    /// `<bdi>` without one, take the direction of the first strongly
    /// directional character of the text inside.
    pub fn is_rtl(&self, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if self.is_element(node) {
                let value = self.get_attribute(node, "dir").map(str::to_ascii_lowercase);
                match value.as_deref() {
                    Some("rtl") => return true,
                    Some("ltr") => return false,
                    Some("auto") => return self.first_strong_rtl(node),
                    _ if self.local_name(node) == Some("bdi") => {
                        return self.first_strong_rtl(node);
                    }
                    _ => {}
                }
            }
            current = self.parent_or_host(node);
        }
        false
    }

    fn first_strong_rtl(&self, node: NodeId) -> bool {
        self.text_content(node)
            .unwrap_or_default()
            .chars()
            .find(|character| strong_rtl(*character) || character.is_alphabetic())
            .is_some_and(strong_rtl)
    }

    /// Dynamic state flags used by pseudo-classes.
    pub fn element_state(&self, id: NodeId) -> u32 {
        let Some(name) = self.local_name(id) else {
            return 0;
        };
        let mut flags = 0;
        // A label hovered or pressed hovers or presses its control too, as
        // in Blink. What happens in a shadow tree happens to its host.
        let contains = |state: Option<NodeId>| {
            state.is_some_and(|node| {
                self.shadow_including_contains(id, node) || self.labels_under(node, id)
            })
        };
        if contains(self.hovered) {
            flags |= state::HOVER;
        }
        if contains(self.active) {
            flags |= state::ACTIVE;
        }
        if contains(self.focused) {
            flags |= state::FOCUS_WITHIN;
        }
        let input_type = self
            .get_attribute(id, "type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        let text_entry = name == "textarea"
            || (name == "input" && TEXT_INPUT_TYPES.contains(&input_type.as_str()));
        // A host whose shadow tree has focus matches `:focus` too.
        let focused_inside = self.shadow_root(id).is_some_and(|root| {
            self.focused
                .is_some_and(|node| self.shadow_including_contains(root, node))
        });
        if focused_inside {
            flags |= state::FOCUS;
        }
        if self.focused == Some(id) {
            flags |= state::FOCUS;
            if self.focus_visible || text_entry {
                flags |= state::FOCUS_VISIBLE;
            }
        }
        let control = matches!(
            name,
            "button" | "input" | "select" | "textarea" | "option" | "optgroup" | "fieldset"
        );
        if control {
            flags |= state::FORM_CONTROL;
            if self.has_attribute(id, "disabled") {
                flags |= state::DISABLED;
            }
        }
        if (self.is_checkable(id) && self.checked(id))
            || (name == "option" && self.option_selected(id))
        {
            flags |= state::CHECKED;
        }
        if self.indeterminate(id) {
            flags |= state::INDETERMINATE;
        }
        if self.has_attribute(id, "required") {
            flags |= state::REQUIRED;
        }
        let editable = text_entry
            && !self.has_attribute(id, "readonly")
            && !self.has_attribute(id, "disabled");
        if editable || self.editing_host(id).is_some() {
            flags |= state::EDITABLE;
        }
        if text_entry && self.has_attribute(id, "placeholder") && self.value(id).is_empty() {
            flags |= state::PLACEHOLDER_SHOWN;
        }
        if matches!(name, "a" | "area" | "link") && self.has_attribute(id, "href") {
            flags |= state::LINK;
        }
        match self.validation_state(id) {
            Some(true) => flags |= state::VALID,
            Some(false) => flags |= state::INVALID,
            None => {}
        }
        if self.is_undefined_custom_element(id) {
            flags |= state::UNDEFINED;
        }
        flags
    }

    fn parse_selectors(selectors: &str) -> Result<SelectorList, DomError> {
        SelectorList::parse(selectors).map_err(|error| {
            DomError::Syntax(format!("'{selectors}' is not a valid selector: {error}"))
        })
    }

    /// `Element.matches`.
    pub fn matches(&self, id: NodeId, selectors: &str) -> Result<bool, DomError> {
        let list = Dom::parse_selectors(selectors)?;
        Ok(self.is_element(id) && list.matches(&ElementRef::scoped(self, id, Some(id))))
    }

    /// `Element.closest`.
    pub fn closest(&self, id: NodeId, selectors: &str) -> Result<Option<NodeId>, DomError> {
        let list = Dom::parse_selectors(selectors)?;
        let mut current = Some(id);
        while let Some(node) = current {
            if self.is_element(node) && list.matches(&ElementRef::scoped(self, node, Some(id))) {
                return Ok(Some(node));
            }
            current = self.parent(node);
        }
        Ok(None)
    }

    /// `querySelectorAll`.
    pub fn query_selector_all(
        &self,
        root: NodeId,
        selectors: &str,
    ) -> Result<Vec<NodeId>, DomError> {
        let list = Dom::parse_selectors(selectors)?;
        let scope = self.is_element(root).then_some(root);
        Ok(self
            .descendants(root)
            .into_iter()
            .filter(|node| {
                self.is_element(*node) && list.matches(&ElementRef::scoped(self, *node, scope))
            })
            .collect())
    }

    /// `querySelector`.
    pub fn query_selector(
        &self,
        root: NodeId,
        selectors: &str,
    ) -> Result<Option<NodeId>, DomError> {
        Ok(self.query_selector_all(root, selectors)?.into_iter().next())
    }

    /// Whether `node` or one of its ancestors is a label of `control`.
    fn labels_under(&self, node: NodeId, control: NodeId) -> bool {
        if !self.labelable(control) {
            return false;
        }
        let mut current = Some(node);
        while let Some(element) = current {
            if self.local_name(element) == Some("label")
                && self.label_control(element) == Some(control)
            {
                return true;
            }
            current = self.parent_element(element);
        }
        false
    }

    /// `HTMLLabelElement.control`: the element its `for` names, or else its
    /// first labelable descendant.
    pub fn label_control(&self, label: NodeId) -> Option<NodeId> {
        if let Some(target) = self.get_attribute(label, "for") {
            let mut root = label;
            while let Some(parent) = self.parent(root) {
                root = parent;
            }
            return self
                .get_element_by_id(root, target)
                .filter(|control| self.labelable(*control));
        }
        self.descendants(label)
            .into_iter()
            .find(|node| self.labelable(*node))
    }

    /// Whether a label can name the element.
    fn labelable(&self, id: NodeId) -> bool {
        match self.local_name(id) {
            Some("button" | "meter" | "output" | "progress" | "select" | "textarea") => true,
            Some("input") => self.input_type(id) != "hidden",
            _ => false,
        }
    }

    /// `getElementById`: the first of the elements with the id, in tree
    /// order, that `root` contains. Unknown ids, as names looked up on the
    /// window are, cost no walk of the tree.
    pub fn get_element_by_id(&self, root: NodeId, id: &str) -> Option<NodeId> {
        let mut found: Option<NodeId> = None;
        for &node in self.ids.get(id)? {
            if node != root
                && self.contains(root, node)
                && found.is_none_or(|first| self.precedes(node, first))
            {
                found = Some(node);
            }
        }
        found
    }

    /// Moves an element in the id index from its old id to its new one.
    pub(crate) fn index_id(&mut self, node: NodeId, old: Option<&str>, new: Option<&str>) {
        if let Some(old) = old
            && let Some(nodes) = self.ids.get_mut(old)
        {
            nodes.retain(|candidate| *candidate != node);
            if nodes.is_empty() {
                self.ids.remove(old);
            }
        }
        if let Some(new) = new {
            self.ids.entry(new.to_owned()).or_default().push(node);
        }
    }

    /// Whether `node` comes before `other` in tree order.
    fn precedes(&self, node: NodeId, other: NodeId) -> bool {
        let lineage = |mut node: NodeId| {
            let mut nodes = vec![node];
            while let Some(parent) = self.parent(node) {
                nodes.push(parent);
                node = parent;
            }
            nodes.reverse();
            nodes
        };
        let (mine, theirs) = (lineage(node), lineage(other));
        let shared = mine.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
        if shared == mine.len() {
            // An ancestor comes first.
            return true;
        }
        if shared == theirs.len() || shared == 0 {
            return false;
        }
        let mut sibling = self.next_sibling(mine[shared]);
        while let Some(next) = sibling {
            if next == theirs[shared] {
                return true;
            }
            sibling = self.next_sibling(next);
        }
        false
    }

    /// `getElementsByTagName`.
    pub fn get_elements_by_tag_name(&self, root: NodeId, name: &str) -> Vec<NodeId> {
        let name = name.to_ascii_lowercase();
        self.descendants(root)
            .into_iter()
            .filter(|node| {
                self.local_name(*node)
                    .is_some_and(|local| name == "*" || local.eq_ignore_ascii_case(&name))
            })
            .collect()
    }

    /// `getElementsByClassName`.
    pub fn get_elements_by_class_name(&self, root: NodeId, classes: &str) -> Vec<NodeId> {
        let wanted: Vec<&str> = classes.split_ascii_whitespace().collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        self.descendants(root)
            .into_iter()
            .filter(|node| wanted.iter().all(|class| self.has_class(*node, class)))
            .collect()
    }
}

#[cfg(test)]
mod tests;
