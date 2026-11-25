//! Elements: attributes, class lists, inline styles and form control state.

use crate::prelude::*;

use crate::{Dom, DomError, NodeId, NodeKind};
use nativeboat_css::shorthand::{expand, longhands_of};
use nativeboat_css::values::parse_component_values;
use nativeboat_css::{Declaration, parse_declaration_list};

/// Element namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    /// HTML.
    Html,
    /// SVG.
    Svg,
    /// MathML.
    MathMl,
}

impl Namespace {
    /// The namespace URI.
    pub fn uri(self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }

    /// Looks up a namespace URI; unknown and empty URIs map to HTML.
    pub fn from_uri(uri: &str) -> Namespace {
        match uri {
            "http://www.w3.org/2000/svg" => Namespace::Svg,
            "http://www.w3.org/1998/Math/MathML" => Namespace::MathMl,
            _ => Namespace::Html,
        }
    }
}

/// A shorthand's value from its longhands' values: four sides collapse as
/// browsers serialize them, and other lists collapse to one value when every
/// longhand has it.
fn serialize_shorthand(name: &str, values: Vec<String>) -> String {
    let sided = matches!(
        name,
        "margin"
            | "padding"
            | "inset"
            | "border-width"
            | "border-style"
            | "border-color"
            | "scroll-margin"
            | "scroll-padding"
    );
    if sided && values.len() == 4 {
        let [top, right, bottom, left] = [&values[0], &values[1], &values[2], &values[3]];
        let parts: &[&String] = if right != left {
            &[top, right, bottom, left]
        } else if top != bottom {
            &[top, right, bottom]
        } else if top != right {
            &[top, right]
        } else {
            &[top]
        };
        return parts
            .iter()
            .map(|part| part.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    if values.iter().all(|value| *value == values[0]) {
        return values[0].clone();
    }
    values.join(" ")
}

/// The `style` attribute as a declaration list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InlineStyle {
    declarations: Vec<(String, String, bool)>,
}

impl InlineStyle {
    /// Parses a `style` attribute value.
    pub fn parse(text: &str) -> InlineStyle {
        let mut style = InlineStyle::default();
        for declaration in parse_declaration_list(text) {
            style.set(&declaration.name, &declaration.value, declaration.important);
        }
        style
    }

    /// `cssText`.
    pub fn css_text(&self) -> String {
        self.declarations
            .iter()
            .map(|(name, value, important)| {
                if *important {
                    format!("{name}: {value} !important;")
                } else {
                    format!("{name}: {value};")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `getPropertyValue`; longhands set through a shorthand are expanded.
    pub fn get(&self, name: &str) -> String {
        if let Some(index) = self
            .declarations
            .iter()
            .rposition(|(candidate, _, _)| candidate == name)
        {
            // A shorthand's own value holds unless later declarations
            // changed some of its longhands.
            let overridden = longhands_of(name).is_some_and(|longhands| {
                self.declarations[index + 1..].iter().any(|(later, _, _)| {
                    let names = longhands_of(later)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|id| id.name())
                        .collect::<Vec<_>>();
                    longhands
                        .iter()
                        .any(|id| id.name() == later || names.contains(&id.name()))
                })
            });
            if !overridden {
                return self.declarations[index].1.clone();
            }
        }
        for (shorthand, value, _) in self.declarations.iter().rev() {
            if longhands_of(shorthand).is_none() {
                continue;
            }
            let expanded = expand(shorthand, &parse_component_values(value)).unwrap_or_default();
            if let Some((_, values)) = expanded.iter().find(|(id, _)| id.name() == name) {
                return render(values);
            }
        }
        // A shorthand whose longhands are all set, as `border` sets
        // `border-width`, serializes from them.
        if name == "border" {
            // Only sides that agree make a `border`.
            let mut parts = Vec::new();
            for part in ["border-width", "border-style", "border-color"] {
                let sides: Vec<String> = longhands_of(part)
                    .unwrap_or_default()
                    .iter()
                    .map(|id| self.get(id.name()))
                    .collect();
                if sides[0].is_empty() || sides.iter().any(|side| *side != sides[0]) {
                    return String::new();
                }
                parts.push(sides[0].clone());
            }
            return parts.join(" ");
        }
        if let Some(longhands) = longhands_of(name) {
            let values: Vec<String> = longhands.iter().map(|id| self.get(id.name())).collect();
            if values.iter().any(String::is_empty) {
                return String::new();
            }
            return serialize_shorthand(name, values);
        }
        String::new()
    }

    /// `getPropertyPriority`.
    pub fn priority(&self, name: &str) -> &'static str {
        match self
            .declarations
            .iter()
            .find(|(candidate, _, _)| candidate == name)
        {
            Some((_, _, true)) => "important",
            _ => "",
        }
    }

    /// `setProperty`; an empty value removes the property.
    pub fn set(&mut self, name: &str, value: &str, important: bool) {
        let name = if name.starts_with("--") {
            name.to_owned()
        } else {
            name.to_ascii_lowercase()
        };
        let value = value.trim();
        self.remove(&name);
        if value.is_empty() {
            return;
        }
        if let Some(longhands) = longhands_of(&name) {
            self.declarations
                .retain(|(existing, _, _)| !longhands.iter().any(|id| id.name() == existing));
        }
        self.declarations.push((name, value.to_owned(), important));
    }

    /// `removeProperty`; returns the old value.
    pub fn remove(&mut self, name: &str) -> String {
        let old = self.get(name);
        self.declarations
            .retain(|(existing, _, _)| existing != name);
        old
    }

    /// The number of declarations.
    pub fn len(&self) -> usize {
        self.declarations.len()
    }

    /// Whether there are no declarations.
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }

    /// The property name at `index`.
    pub fn item(&self, index: usize) -> Option<&str> {
        self.declarations
            .get(index)
            .map(|(name, _, _)| name.as_str())
    }

    /// The declarations for the cascade.
    pub fn declarations(&self) -> Vec<Declaration> {
        self.declarations
            .iter()
            .map(|(name, value, important)| Declaration {
                name: name.clone(),
                value: value.clone(),
                important: *important,
                position: nativeboat_css::Position::default(),
            })
            .collect()
    }
}

fn render(values: &[nativeboat_css::values::ComponentValue]) -> String {
    use nativeboat_css::values::ComponentValue;
    let mut text = String::new();
    for value in values {
        match value {
            ComponentValue::Token(token) => text.push_str(&token.to_css()),
            ComponentValue::Function(name, arguments) => {
                text.push_str(name);
                text.push('(');
                text.push_str(&render(arguments));
                text.push(')');
            }
            ComponentValue::Parens(inner) => {
                text.push('(');
                text.push_str(&render(inner));
                text.push(')');
            }
            ComponentValue::Brackets(inner) => {
                text.push('[');
                text.push_str(&render(inner));
                text.push(']');
            }
        }
    }
    text.trim().to_owned()
}

/// Element data.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementData {
    pub(crate) namespace: Namespace,
    pub(crate) name: String,
    pub(crate) attributes: Vec<(String, String)>,
    pub(crate) style: InlineStyle,
    pub(crate) value: Option<String>,
    pub(crate) checked: Option<bool>,
    pub(crate) selected: Option<bool>,
    pub(crate) indeterminate: bool,
    pub(crate) scroll: (f32, f32),
}

impl ElementData {
    pub(crate) fn new(namespace: Namespace, name: String) -> ElementData {
        ElementData {
            namespace,
            name,
            attributes: Vec::new(),
            style: InlineStyle::default(),
            value: None,
            checked: None,
            selected: None,
            indeterminate: false,
            scroll: (0.0, 0.0),
        }
    }

    pub(crate) fn qualified_name(&self) -> String {
        if self.namespace == Namespace::Html {
            self.name.to_ascii_uppercase()
        } else {
            self.name.clone()
        }
    }

    pub(crate) fn clone_for_copy(&self) -> ElementData {
        ElementData {
            scroll: (0.0, 0.0),
            ..self.clone()
        }
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

fn valid_attribute_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            !character.is_whitespace() && !matches!(character, '"' | '\'' | '>' | '/' | '=' | '<')
        })
}

fn collapse_whitespace(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

impl Dom {
    pub(crate) fn element(&self, id: NodeId) -> Option<&ElementData> {
        match &self.node(id).kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    pub(crate) fn element_mut(&mut self, id: NodeId) -> Option<&mut ElementData> {
        match &mut self.node_mut(id).kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    /// The element's namespace.
    pub fn namespace(&self, id: NodeId) -> Option<Namespace> {
        self.element(id).map(|element| element.namespace)
    }

    fn attribute_key(&self, id: NodeId, name: &str) -> String {
        match self.namespace(id) {
            Some(Namespace::Html) => name.to_ascii_lowercase(),
            _ => name.to_owned(),
        }
    }

    /// `getAttribute`. Selector matching asks for attributes of every
    /// element on every restyle, so HTML names are lowercased in place.
    pub fn get_attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        let element = self.element(id)?;
        let html = element.namespace == Namespace::Html;
        element
            .attributes
            .iter()
            .find(|(candidate, _)| {
                if html {
                    candidate.len() == name.len()
                        && candidate
                            .bytes()
                            .zip(name.bytes())
                            .all(|(stored, asked)| stored == asked.to_ascii_lowercase())
                } else {
                    candidate == name
                }
            })
            .map(|(_, value)| value.as_str())
    }

    /// `hasAttribute`.
    pub fn has_attribute(&self, id: NodeId, name: &str) -> bool {
        self.get_attribute(id, name).is_some()
    }

    /// The attributes in order, borrowed.
    pub fn attribute_list(&self, id: NodeId) -> &[(String, String)] {
        self.element(id)
            .map_or(&[], |element| element.attributes.as_slice())
    }

    /// The attributes in order.
    pub fn attributes(&self, id: NodeId) -> Vec<(String, String)> {
        self.element(id)
            .map(|element| element.attributes.clone())
            .unwrap_or_default()
    }

    /// `setAttribute`.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<(), DomError> {
        if !valid_attribute_name(name) {
            return Err(DomError::InvalidCharacter(format!(
                "\"{name}\" is not a valid attribute name"
            )));
        }
        let key = self.attribute_key(id, name);
        let Some(element) = self.element_mut(id) else {
            return Ok(());
        };
        let old = element.attribute(&key).map(str::to_owned);
        let old_present = old.is_some();
        if old.as_deref() == Some(value)
            && !(element.name == "canvas" && matches!(key.as_str(), "width" | "height"))
        {
            // Repeated writes still notify observers, but cannot change CSS
            // or geometry. Canvas dimension assignments still invalidate.
            self.record_attribute(id, &key, old);
            return Ok(());
        }
        match element
            .attributes
            .iter_mut()
            .find(|(candidate, _)| *candidate == key)
        {
            Some(entry) => entry.1 = value.to_owned(),
            None => element.attributes.push((key.clone(), value.to_owned())),
        }
        if key == "style" {
            element.style = InlineStyle::parse(value);
        }
        if key == "id" {
            self.index_id(id, old.as_deref(), Some(value));
        }
        self.record_attribute(id, &key, old);
        self.details_attribute_changed(id, &key, old_present, true);
        self.touch();
        Ok(())
    }

    /// `removeAttribute`.
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) {
        let key = self.attribute_key(id, name);
        let Some(element) = self.element_mut(id) else {
            return;
        };
        let Some(position) = element
            .attributes
            .iter()
            .position(|(candidate, _)| *candidate == key)
        else {
            return;
        };
        let (_, old) = element.attributes.remove(position);
        if key == "style" {
            element.style = InlineStyle::default();
        }
        if key == "id" {
            self.index_id(id, Some(&old), None);
        }
        self.record_attribute(id, &key, Some(old));
        self.details_attribute_changed(id, &key, true, false);
        self.touch();
    }

    /// `toggleAttribute`; returns whether the attribute is now present.
    pub fn toggle_attribute(
        &mut self,
        id: NodeId,
        name: &str,
        force: Option<bool>,
    ) -> Result<bool, DomError> {
        let present = self.has_attribute(id, name);
        let wanted = force.unwrap_or(!present);
        if wanted && !present {
            self.set_attribute(id, name, "")?;
        } else if !wanted && present {
            self.remove_attribute(id, name);
        }
        Ok(wanted)
    }

    /// The `id` attribute.
    pub fn element_id(&self, id: NodeId) -> Option<&str> {
        self.get_attribute(id, "id")
    }

    /// The tokens of the `class` attribute.
    pub fn class_list(&self, id: NodeId) -> Vec<String> {
        let mut classes: Vec<String> = Vec::new();
        for class in self
            .get_attribute(id, "class")
            .unwrap_or("")
            .split_ascii_whitespace()
        {
            if !classes.iter().any(|existing| existing == class) {
                classes.push(class.to_owned());
            }
        }
        classes
    }

    /// Whether the class list contains `class`.
    pub fn has_class(&self, id: NodeId, class: &str) -> bool {
        self.get_attribute(id, "class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|candidate| candidate == class)
        })
    }

    fn check_token(token: &str) -> Result<(), DomError> {
        if token.is_empty() {
            return Err(DomError::Syntax("the token must not be empty".to_owned()));
        }
        if token
            .chars()
            .any(|character| character.is_ascii_whitespace())
        {
            return Err(DomError::InvalidCharacter(
                "the token must not contain whitespace".to_owned(),
            ));
        }
        Ok(())
    }

    fn set_classes(&mut self, id: NodeId, classes: &[String]) -> Result<(), DomError> {
        self.set_attribute(id, "class", &classes.join(" "))
    }

    /// `classList.add`.
    pub fn class_add(&mut self, id: NodeId, tokens: &[&str]) -> Result<(), DomError> {
        for token in tokens {
            Dom::check_token(token)?;
        }
        let mut classes = self.class_list(id);
        for token in tokens {
            if !classes.iter().any(|class| class == token) {
                classes.push((*token).to_owned());
            }
        }
        self.set_classes(id, &classes)
    }

    /// `classList.remove`.
    pub fn class_remove(&mut self, id: NodeId, tokens: &[&str]) -> Result<(), DomError> {
        for token in tokens {
            Dom::check_token(token)?;
        }
        if !self.has_attribute(id, "class") {
            return Ok(());
        }
        let classes: Vec<String> = self
            .class_list(id)
            .into_iter()
            .filter(|class| !tokens.contains(&class.as_str()))
            .collect();
        self.set_classes(id, &classes)
    }

    /// `classList.toggle`; returns whether the token is now present.
    pub fn class_toggle(
        &mut self,
        id: NodeId,
        token: &str,
        force: Option<bool>,
    ) -> Result<bool, DomError> {
        Dom::check_token(token)?;
        let present = self.has_class(id, token);
        let wanted = force.unwrap_or(!present);
        if wanted && !present {
            self.class_add(id, &[token])?;
        } else if !wanted && present {
            self.class_remove(id, &[token])?;
        }
        Ok(wanted)
    }

    /// `classList.replace`; returns whether `old` was present.
    pub fn class_replace(&mut self, id: NodeId, old: &str, new: &str) -> Result<bool, DomError> {
        Dom::check_token(old)?;
        Dom::check_token(new)?;
        if !self.has_class(id, old) {
            return Ok(false);
        }
        let mut classes: Vec<String> = Vec::new();
        for class in self.class_list(id) {
            let class = if class == old { new.to_owned() } else { class };
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
        self.set_classes(id, &classes)?;
        Ok(true)
    }

    /// The inline style.
    pub fn inline_style(&self, id: NodeId) -> InlineStyle {
        self.element(id)
            .map(|element| element.style.clone())
            .unwrap_or_default()
    }

    /// Modifies the inline style and updates the `style` attribute.
    pub fn update_inline_style(&mut self, id: NodeId, change: &mut dyn FnMut(&mut InlineStyle)) {
        let Some(element) = self.element_mut(id) else {
            return;
        };
        let mut style = element.style.clone();
        change(&mut style);
        if style == element.style {
            return;
        }
        let text = style.css_text();
        let _ = self.set_attribute(id, "style", &text);
    }

    /// The scroll offset of an element: `(scrollLeft, scrollTop)`.
    pub fn scroll(&self, id: NodeId) -> (f32, f32) {
        self.element(id)
            .map_or((0.0, 0.0), |element| element.scroll)
    }

    /// Sets the scroll offset of an element.
    pub fn set_scroll(&mut self, id: NodeId, left: f32, top: f32) {
        if let Some(element) = self.element_mut(id)
            && element.scroll != (left, top)
        {
            element.scroll = (left, top);
            if !self.scrolled.contains(&id) {
                self.scrolled.push(id);
            }
            self.touch();
        }
    }

    /// Drains the elements whose scroll offsets changed, in the order they
    /// first changed, for their `scroll` events.
    pub fn take_scrolled(&mut self) -> Vec<NodeId> {
        core::mem::take(&mut self.scrolled)
    }

    /// Whether scroll offsets changed since [`Dom::take_scrolled`].
    pub fn has_scrolled(&self) -> bool {
        !self.scrolled.is_empty()
    }

    pub(crate) fn input_type(&self, id: NodeId) -> String {
        self.get_attribute(id, "type")
            .unwrap_or("text")
            .trim()
            .to_ascii_lowercase()
    }

    /// Whether the element is a checkbox or radio input.
    pub fn is_checkable(&self, id: NodeId) -> bool {
        self.local_name(id) == Some("input")
            && matches!(self.input_type(id).as_str(), "checkbox" | "radio")
    }

    /// The `value` property of form controls and options.
    pub fn value(&self, id: NodeId) -> String {
        let Some(element) = self.element(id) else {
            return String::new();
        };
        match element.name.as_str() {
            "select" => self
                .selected_options(id)
                .first()
                .map(|option| self.value(*option))
                .unwrap_or_default(),
            "option" => element.attribute("value").map_or_else(
                || collapse_whitespace(&self.text_content(id).unwrap_or_default()),
                str::to_owned,
            ),
            "textarea" => element
                .value
                .clone()
                .unwrap_or_else(|| self.text_content(id).unwrap_or_default()),
            "input" if self.is_checkable(id) => {
                element.attribute("value").unwrap_or("on").to_owned()
            }
            _ => element
                .value
                .clone()
                .unwrap_or_else(|| element.attribute("value").unwrap_or("").to_owned()),
        }
    }

    /// Sets the `value` property.
    pub fn set_value(&mut self, id: NodeId, value: &str) {
        match self.local_name(id) {
            Some("select") => {
                let options = self.options(id);
                let chosen = options
                    .iter()
                    .copied()
                    .find(|option| self.value(*option) == value);
                for option in options {
                    self.set_selectedness(option, Some(option) == chosen);
                }
                self.touch();
            }
            Some("option") => {
                let _ = self.set_attribute(id, "value", value);
            }
            Some("input") if self.is_checkable(id) => {
                let _ = self.set_attribute(id, "value", value);
            }
            Some(_) => {
                let element = self.element_mut(id).expect("element");
                if element.value.as_deref() != Some(value) {
                    element.value = Some(value.to_owned());
                    self.touch();
                }
            }
            None => {}
        }
    }

    /// Whether the value was changed by the user or script.
    pub fn is_value_dirty(&self, id: NodeId) -> bool {
        self.element(id)
            .is_some_and(|element| element.value.is_some())
    }

    /// The `checked` property.
    pub fn checked(&self, id: NodeId) -> bool {
        self.element(id).is_some_and(|element| {
            element
                .checked
                .unwrap_or_else(|| element.attribute("checked").is_some())
        })
    }

    /// Sets the `checked` property; checking a radio button unchecks its group.
    pub fn set_checked(&mut self, id: NodeId, checked: bool) {
        let Some(element) = self.element_mut(id) else {
            return;
        };
        element.checked = Some(checked);
        self.touch();
        if !checked || self.input_type(id) != "radio" {
            return;
        }
        let Some(name) = self.get_attribute(id, "name").map(str::to_owned) else {
            return;
        };
        let owner = self.form_owner(id);
        let mut root = id;
        while let Some(parent) = self.parent(root) {
            root = parent;
        }
        for other in self.descendants(root) {
            if other != id
                && self.form_owner(other) == owner
                && self.local_name(other) == Some("input")
                && self.input_type(other) == "radio"
                && self.get_attribute(other, "name") == Some(name.as_str())
            {
                self.element_mut(other).expect("element").checked = Some(false);
            }
        }
    }

    /// The form a control belongs to: the `<form>` named by its `form`
    /// attribute, or else its nearest ancestor `<form>`.
    pub fn form_owner(&self, id: NodeId) -> Option<NodeId> {
        if let Some(form) = self.get_attribute(id, "form") {
            return self
                .get_element_by_id(Dom::DOCUMENT, form)
                .filter(|owner| self.local_name(*owner) == Some("form"));
        }
        let mut current = self.parent(id);
        while let Some(node) = current {
            if self.local_name(node) == Some("form") {
                return Some(node);
            }
            current = self.parent(node);
        }
        None
    }

    /// The `indeterminate` property.
    pub fn indeterminate(&self, id: NodeId) -> bool {
        self.element(id)
            .is_some_and(|element| element.indeterminate)
    }

    /// Sets the `indeterminate` property.
    pub fn set_indeterminate(&mut self, id: NodeId, indeterminate: bool) {
        if let Some(element) = self.element_mut(id) {
            element.indeterminate = indeterminate;
            self.touch();
        }
    }

    /// The `<option>` elements of a `<select>`, including those in groups.
    pub fn options(&self, select: NodeId) -> Vec<NodeId> {
        self.descendants(select)
            .into_iter()
            .filter(|node| self.local_name(*node) == Some("option"))
            .collect()
    }

    fn explicitly_selected(&self, option: NodeId) -> bool {
        self.element(option).is_some_and(|element| {
            element
                .selected
                .unwrap_or_else(|| element.attribute("selected").is_some())
        })
    }

    fn set_selectedness(&mut self, option: NodeId, selected: bool) {
        if let Some(element) = self.element_mut(option) {
            element.selected = Some(selected);
        }
    }

    /// The selected options. A single-selection list shows its first enabled
    /// option until script or the user changes the selection.
    pub fn selected_options(&self, select: NodeId) -> Vec<NodeId> {
        let options = self.options(select);
        let selected: Vec<NodeId> = options
            .iter()
            .copied()
            .filter(|option| self.explicitly_selected(*option))
            .collect();
        if self.has_attribute(select, "multiple") || self.size_attribute(select) > 1 {
            return selected;
        }
        if let Some(last) = selected.last() {
            return vec![*last];
        }
        let touched = options.iter().any(|option| {
            self.element(*option)
                .is_some_and(|element| element.selected == Some(false))
        });
        if touched {
            return Vec::new();
        }
        options
            .into_iter()
            .find(|option| !self.has_attribute(*option, "disabled"))
            .into_iter()
            .collect()
    }

    fn size_attribute(&self, select: NodeId) -> u32 {
        self.get_attribute(select, "size")
            .and_then(|size| size.trim().parse().ok())
            .unwrap_or(1)
    }

    /// Whether an option is selected.
    pub fn option_selected(&self, option: NodeId) -> bool {
        let mut select = self.parent(option);
        while let Some(candidate) = select {
            if self.local_name(candidate) == Some("select") {
                return self.selected_options(candidate).contains(&option);
            }
            select = self.parent(candidate);
        }
        self.explicitly_selected(option)
    }

    /// Selects or deselects an option; single-selection lists keep one option.
    pub fn set_option_selected(&mut self, option: NodeId, selected: bool) {
        let mut select = self.parent(option);
        while let Some(candidate) = select {
            if self.local_name(candidate) == Some("select") {
                break;
            }
            select = self.parent(candidate);
        }
        if selected
            && let Some(select) = select
            && !self.has_attribute(select, "multiple")
        {
            for other in self.options(select) {
                self.set_selectedness(other, false);
            }
        }
        self.set_selectedness(option, selected);
        self.touch();
    }

    /// `selectedIndex`.
    pub fn selected_index(&self, select: NodeId) -> i32 {
        let options = self.options(select);
        self.selected_options(select)
            .first()
            .and_then(|selected| options.iter().position(|option| option == selected))
            .map_or(-1, |index| index as i32)
    }

    /// Sets `selectedIndex`; out-of-range indexes deselect everything.
    pub fn set_selected_index(&mut self, select: NodeId, index: i32) {
        let options = self.options(select);
        for (position, option) in options.iter().enumerate() {
            self.set_selectedness(*option, position as i32 == index);
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests;
