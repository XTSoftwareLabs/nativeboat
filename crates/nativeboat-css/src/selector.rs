//! Selectors: parsing, specificity and matching.

use crate::prelude::*;

use crate::tokenizer::{Token, serialize_tokens, tokens};
use serde::{Deserialize, Serialize};

/// Selector specificity as (ids, classes, types).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize, Deserialize,
)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    fn add(self, other: Specificity) -> Specificity {
        Specificity(self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}

/// How two compound selectors relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Combinator {
    /// Whitespace: any ancestor.
    Descendant,
    /// `>`: the parent.
    Child,
    /// `+`: the previous element sibling.
    NextSibling,
    /// `~`: any previous element sibling.
    SubsequentSibling,
}

/// Attribute selector operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttributeOperator {
    /// `[a=v]`
    Equals,
    /// `[a~=v]`
    Includes,
    /// `[a|=v]`
    DashMatch,
    /// `[a^=v]`
    Prefix,
    /// `[a$=v]`
    Suffix,
    /// `[a*=v]`
    Substring,
}

/// An attribute selector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeSelector {
    /// Lowercased attribute name.
    pub name: String,
    /// Operator and value, or `None` for presence tests.
    pub matcher: Option<(AttributeOperator, String)>,
    /// Whether values compare ASCII case-insensitively (`i` flag).
    pub case_insensitive: bool,
}

/// The `An+B` argument of `:nth-*` pseudo-classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nth {
    /// Step.
    pub a: i32,
    /// Offset.
    pub b: i32,
}

impl Nth {
    /// Whether the 1-based `index` is selected.
    pub fn matches(self, index: i32) -> bool {
        if self.a == 0 {
            return index == self.b;
        }
        let distance = index - self.b;
        distance % self.a == 0 && distance / self.a >= 0
    }
}

/// Pseudo-classes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PseudoClass {
    /// `:hover`
    Hover,
    /// `:vertical`, on a scrollbar part.
    Vertical,
    /// `:horizontal`, on a scrollbar part.
    Horizontal,
    /// `:active`
    Active,
    /// `:focus`
    Focus,
    /// `:focus-visible`
    FocusVisible,
    /// `:focus-within`
    FocusWithin,
    /// `:disabled`
    Disabled,
    /// `:enabled`
    Enabled,
    /// `:checked`
    Checked,
    /// `:indeterminate`
    Indeterminate,
    /// `:required`
    Required,
    /// `:optional`
    Optional,
    /// `:read-only`
    ReadOnly,
    /// `:read-write`
    ReadWrite,
    /// `:placeholder-shown`
    PlaceholderShown,
    /// `:defined`: a built-in element, or a custom element its definition
    /// has upgraded.
    Defined,
    /// `:valid`
    Valid,
    /// `:invalid`
    Invalid,
    /// `:root`
    Root,
    /// `:scope`
    Scope,
    /// `:empty`
    Empty,
    /// `:first-child`
    FirstChild,
    /// `:last-child`
    LastChild,
    /// `:only-child`
    OnlyChild,
    /// `:first-of-type`
    FirstOfType,
    /// `:last-of-type`
    LastOfType,
    /// `:only-of-type`
    OnlyOfType,
    /// `:nth-child()`
    NthChild(Nth),
    /// `:nth-last-child()`
    NthLastChild(Nth),
    /// `:nth-child(An+B of S)`: counted among the siblings `S` matches.
    NthChildOf(Nth, SelectorList),
    /// `:nth-last-child(An+B of S)`.
    NthLastChildOf(Nth, SelectorList),
    /// `:nth-of-type()`
    NthOfType(Nth),
    /// `:nth-last-of-type()`
    NthLastOfType(Nth),
    /// `:not()`
    Not(SelectorList),
    /// `:is()` and its aliases.
    Is(SelectorList),
    /// `:where()`
    Where(SelectorList),
    /// `:has()`: an element related to this one matches.
    Has(Vec<Relative>),
    /// `:link` and `:any-link`
    Link,
    /// `:visited` and `:target`, which never match in an application.
    Never(String),
    /// `:host` and `:host()`: a shadow host, as the rules of its own shadow
    /// tree see it, when it matches the compound selectors given.
    Host(Option<SelectorList>),
    /// `:host-context()`: a shadow host that, or whose shadow-including
    /// ancestor, matches the compound selectors given.
    HostContext(SelectorList),
    /// `:lang()`: an element whose language one of the ranges matches.
    Lang(Vec<String>),
    /// `:dir()`: an element whose directionality is the lowercased value.
    Dir(String),
    /// A pseudo-class the native target does not support; never matches.
    Unsupported(String),
}

/// Pseudo-elements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PseudoElement {
    /// `::before`
    Before,
    /// `::after`
    After,
    /// `::placeholder`
    Placeholder,
    /// `::selection`
    Selection,
    /// `::first-line`
    FirstLine,
    /// `::first-letter`
    FirstLetter,
    /// `::marker`
    Marker,
    /// `::-webkit-scrollbar`
    Scrollbar,
    /// `::-webkit-scrollbar-track`
    ScrollbarTrack,
    /// `::-webkit-scrollbar-thumb`
    ScrollbarThumb,
    /// `::-webkit-scrollbar-corner`
    ScrollbarCorner,
    /// `::slotted()`: an element assigned to a slot, which the selector
    /// before it matches; [`Selector::slotted`] holds what the element must
    /// match.
    Slotted,
    /// Any other pseudo-element, such as `::part()`.
    Other(String),
}

impl PseudoElement {
    fn from_name(name: &str) -> PseudoElement {
        match name.to_ascii_lowercase().as_str() {
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            "placeholder" => PseudoElement::Placeholder,
            "selection" => PseudoElement::Selection,
            "first-line" => PseudoElement::FirstLine,
            "first-letter" => PseudoElement::FirstLetter,
            "marker" => PseudoElement::Marker,
            "-webkit-scrollbar" => PseudoElement::Scrollbar,
            "-webkit-scrollbar-track" => PseudoElement::ScrollbarTrack,
            "-webkit-scrollbar-thumb" => PseudoElement::ScrollbarThumb,
            "-webkit-scrollbar-corner" => PseudoElement::ScrollbarCorner,
            other => PseudoElement::Other(other.to_owned()),
        }
    }
}

/// A simple selector other than the type selector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SimpleSelector {
    /// `#id`
    Id(String),
    /// `.class`
    Class(String),
    /// `[attribute]`
    Attribute(AttributeSelector),
    /// `:pseudo-class`
    PseudoClass(PseudoClass),
    /// A state after a pseudo-element, evaluated on that part, not its host.
    PartState(PseudoClass),
}

/// A compound selector such as `button.primary:hover`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Compound {
    /// Lowercased type selector; `None` matches any element.
    pub tag: Option<String>,
    /// The other simple selectors.
    pub simple: Vec<SimpleSelector>,
}

/// A complex selector such as `.toolbar > button:hover`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selector {
    /// Compound selectors from left to right.
    pub compounds: Vec<Compound>,
    /// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
    /// The pseudo-element the selector targets, if any.
    pub pseudo_element: Option<PseudoElement>,
    /// The compound selectors of `::slotted()`, which the slotted element
    /// matches.
    #[serde(default)]
    pub slotted: Option<SelectorList>,
    /// Specificity.
    pub specificity: Specificity,
}

/// What selectors test of elements related to their subjects: the ids,
/// classes, attributes and states that their compounds name. Restyles
/// compare these parts of related elements rather than all their state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelatedFeatures {
    /// Whether some compound tests an id.
    pub ids: bool,
    /// The classes the compounds test.
    pub classes: alloc::collections::BTreeSet<String>,
    /// The lowercased attribute names the compounds test.
    pub attributes: alloc::collections::BTreeSet<String>,
    /// The [`state`] flags the compounds test.
    pub states: u32,
    /// Whether some compound tests `:empty`.
    pub empty: bool,
    /// The compounds that test states, with what else they require of an
    /// element: a state of an element that cannot match the rest of any of
    /// them matters to no selector.
    pub guarded: Vec<StateGuard>,
    /// [`RelatedFeatures::guarded`] by one key each guard requires
    /// (`.class`, `#id`, a tag or `[attribute]`); the states of guards with
    /// no key apply to every element.
    pub guard_keys: alloc::collections::BTreeMap<String, Vec<usize>>,
    /// The states compounds without anything else test.
    pub unguarded: u32,
}

/// A compound that tests states and the other parts it requires of an
/// element; attribute values are not compared, so it may hold for more
/// elements than the compound matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateGuard {
    /// The lowercased type it requires.
    pub tag: Option<String>,
    /// The ids it requires.
    pub ids: Vec<String>,
    /// The classes it requires.
    pub classes: Vec<String>,
    /// The lowercased attribute names it requires.
    pub attributes: Vec<String>,
    /// The [`state`] flags it tests.
    pub states: u32,
}

impl RelatedFeatures {
    /// The state flags that compounds test of an element with these
    /// features, as far as its type, id, classes and attributes tell.
    pub fn states_of(
        &self,
        tag: &str,
        id: Option<&str>,
        classes: &[&str],
        has_attribute: &dyn Fn(&str) -> bool,
    ) -> u32 {
        let mut states = self.unguarded;
        if states == self.states {
            return states;
        }
        let holds = |guard: &StateGuard| {
            guard
                .tag
                .as_deref()
                .is_none_or(|wanted| wanted.eq_ignore_ascii_case(tag))
                && guard.ids.iter().all(|wanted| id == Some(wanted.as_str()))
                && guard
                    .classes
                    .iter()
                    .all(|wanted| classes.contains(&wanted.as_str()))
                && guard.attributes.iter().all(|wanted| has_attribute(wanted))
        };
        let mut check = |key: String| {
            for index in self.guard_keys.get(&key).into_iter().flatten() {
                let guard = &self.guarded[*index];
                if guard.states & !states != 0 && holds(guard) {
                    states |= guard.states;
                }
            }
        };
        for class in classes {
            check(format!(".{class}"));
        }
        if let Some(id) = id {
            check(format!("#{id}"));
        }
        check(tag.to_ascii_lowercase());
        for key in self.guard_keys.keys().filter(|key| key.starts_with('[')) {
            if has_attribute(&key[1..key.len() - 1]) {
                check(key.clone());
            }
        }
        states
    }

    /// Records a compound's states with what else it requires.
    fn add_guard(&mut self, compound: &Compound) {
        let states = compound
            .simple
            .iter()
            .map(|simple| match simple {
                SimpleSelector::PseudoClass(pseudo) | SimpleSelector::PartState(pseudo) => {
                    state_flags(pseudo)
                }
                _ => 0,
            })
            .fold(0, |all, flags| all | flags);
        if states == 0 {
            return;
        }
        let mut guard = StateGuard {
            tag: compound.tag.clone(),
            states,
            ..StateGuard::default()
        };
        for simple in &compound.simple {
            match simple {
                SimpleSelector::Id(id) => guard.ids.push(id.clone()),
                SimpleSelector::Class(class) => guard.classes.push(class.clone()),
                SimpleSelector::Attribute(attribute) => {
                    guard.attributes.push(attribute.name.clone())
                }
                _ => {}
            }
        }
        let key = guard
            .classes
            .first()
            .map(|class| format!(".{class}"))
            .or_else(|| guard.ids.first().map(|id| format!("#{id}")))
            .or_else(|| guard.tag.clone())
            .or_else(|| guard.attributes.first().map(|name| format!("[{name}]")));
        match key {
            Some(key) => {
                self.guard_keys
                    .entry(key)
                    .or_default()
                    .push(self.guarded.len());
                self.guarded.push(guard);
            }
            None => self.unguarded |= states,
        }
    }

    fn add_simple(&mut self, simple: &SimpleSelector) {
        match simple {
            SimpleSelector::Id(_) => self.ids = true,
            SimpleSelector::Class(name) => {
                self.classes.insert(name.clone());
            }
            SimpleSelector::Attribute(attribute) => {
                self.attributes.insert(attribute.name.clone());
            }
            SimpleSelector::PseudoClass(pseudo) | SimpleSelector::PartState(pseudo) => {
                if *pseudo == PseudoClass::Empty {
                    self.empty = true;
                }
                self.states |= state_flags(pseudo);
            }
        }
    }
}

/// The [`state`] flags a pseudo-class tests. Positions are
/// `Relations::structure`; lists are added by their compounds and `:has()`
/// rules are matched every time.
fn state_flags(pseudo: &PseudoClass) -> u32 {
    match pseudo {
        PseudoClass::Hover => state::HOVER,
        PseudoClass::Vertical | PseudoClass::Horizontal => state::VERTICAL | state::HORIZONTAL,
        PseudoClass::Active => state::ACTIVE,
        PseudoClass::Focus => state::FOCUS,
        PseudoClass::FocusVisible => state::FOCUS_VISIBLE,
        PseudoClass::FocusWithin => state::FOCUS_WITHIN,
        PseudoClass::Disabled | PseudoClass::Enabled => state::DISABLED | state::FORM_CONTROL,
        PseudoClass::Checked => state::CHECKED,
        PseudoClass::Indeterminate => state::INDETERMINATE,
        PseudoClass::Required | PseudoClass::Optional => state::REQUIRED | state::FORM_CONTROL,
        PseudoClass::ReadOnly | PseudoClass::ReadWrite => state::EDITABLE,
        PseudoClass::PlaceholderShown => state::PLACEHOLDER_SHOWN,
        PseudoClass::Defined => state::UNDEFINED,
        PseudoClass::Valid | PseudoClass::Invalid => state::VALID | state::INVALID,
        PseudoClass::Link => state::LINK,
        _ => 0,
    }
}

/// What selectors test of the siblings and the ancestors of the elements
/// they match: the compounds left of `+` and `~`, and of descendant and
/// child combinators.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relations {
    /// What compounds test of siblings.
    pub siblings: RelatedFeatures,
    /// What compounds test of ancestors.
    pub ancestors: RelatedFeatures,
    /// The [`structure`] flags: what any compound tests of an element's
    /// place among its siblings.
    pub structure: u16,
}

/// What selectors test of an element's place among its siblings.
pub mod structure {
    /// Whether it is the first element child.
    pub const FIRST: u16 = 1;
    /// Whether it is the last element child.
    pub const LAST: u16 = 1 << 1;
    /// Its index among the element children.
    pub const INDEX: u16 = 1 << 2;
    /// Its index from the last element child.
    pub const INDEX_FROM_LAST: u16 = 1 << 3;
    /// Whether it is the first of its type.
    pub const FIRST_OF_TYPE: u16 = 1 << 4;
    /// Whether it is the last of its type.
    pub const LAST_OF_TYPE: u16 = 1 << 5;
    /// Its index among siblings of its type.
    pub const INDEX_OF_TYPE: u16 = 1 << 6;
    /// Its index from the last sibling of its type.
    pub const INDEX_OF_TYPE_FROM_LAST: u16 = 1 << 7;
    /// The siblings before it, which compounds left of `+` and `~` test.
    pub const PRECEDING: u16 = 1 << 8;
    /// The flags of the type-relative tests.
    pub const OF_TYPE: u16 = FIRST_OF_TYPE | LAST_OF_TYPE | INDEX_OF_TYPE | INDEX_OF_TYPE_FROM_LAST;
}

/// The [`structure`] flags a pseudo-class tests.
fn structure_of(pseudo: &PseudoClass) -> u16 {
    use structure::*;
    match pseudo {
        PseudoClass::FirstChild => FIRST,
        PseudoClass::LastChild => LAST,
        PseudoClass::OnlyChild => FIRST | LAST,
        PseudoClass::FirstOfType => FIRST_OF_TYPE,
        PseudoClass::LastOfType => LAST_OF_TYPE,
        PseudoClass::OnlyOfType => FIRST_OF_TYPE | LAST_OF_TYPE,
        PseudoClass::NthChild(_) => INDEX,
        PseudoClass::NthLastChild(_) => INDEX_FROM_LAST,
        PseudoClass::NthChildOf(..) => INDEX,
        PseudoClass::NthLastChildOf(..) => INDEX_FROM_LAST,
        PseudoClass::NthOfType(_) => INDEX_OF_TYPE,
        PseudoClass::NthLastOfType(_) => INDEX_OF_TYPE_FROM_LAST,
        _ => 0,
    }
}

#[derive(Clone, Copy)]
enum Role {
    Subject,
    Sibling,
    Ancestor,
}

impl Relations {
    /// Adds what a selector tests of related elements.
    pub fn add(&mut self, selector: &Selector) {
        // The compounds before `::slotted()` test the slot, which is above
        // the element in the flat tree.
        let role = if selector.pseudo_element == Some(PseudoElement::Slotted) {
            Role::Ancestor
        } else {
            Role::Subject
        };
        self.add_selector(selector, role);
    }

    fn add_selector(&mut self, selector: &Selector, subject: Role) {
        let last = selector.compounds.len().saturating_sub(1);
        for (index, compound) in selector.compounds.iter().enumerate() {
            let role = match selector.combinators.get(index) {
                _ if index == last => subject,
                Some(Combinator::NextSibling | Combinator::SubsequentSibling) => Role::Sibling,
                _ => Role::Ancestor,
            };
            if matches!(role, Role::Sibling) {
                self.structure |= structure::PRECEDING;
            }
            match role {
                Role::Subject => {}
                Role::Sibling => self.siblings.add_guard(compound),
                Role::Ancestor => self.ancestors.add_guard(compound),
            }
            for simple in &compound.simple {
                if let SimpleSelector::PseudoClass(pseudo) | SimpleSelector::PartState(pseudo) =
                    simple
                {
                    self.structure |= structure_of(pseudo);
                }
                match simple {
                    // A nested selector's subject is the compound's element.
                    SimpleSelector::PseudoClass(
                        PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list),
                    )
                    | SimpleSelector::PartState(
                        PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list),
                    ) => list
                        .0
                        .iter()
                        .for_each(|nested| self.add_selector(nested, role)),
                    SimpleSelector::PseudoClass(
                        PseudoClass::NthChildOf(_, list) | PseudoClass::NthLastChildOf(_, list),
                    )
                    | SimpleSelector::PartState(
                        PseudoClass::NthChildOf(_, list) | PseudoClass::NthLastChildOf(_, list),
                    ) => list.0.iter().for_each(|nested| {
                        self.add_selector(nested, role);
                        self.add_selector(nested, Role::Sibling);
                    }),
                    // An element's language and direction come from its
                    // ancestors' attributes.
                    SimpleSelector::PseudoClass(PseudoClass::Lang(_))
                    | SimpleSelector::PartState(PseudoClass::Lang(_)) => {
                        self.ancestors.attributes.insert("lang".to_owned());
                        self.ancestors.attributes.insert("xml:lang".to_owned());
                    }
                    SimpleSelector::PseudoClass(PseudoClass::Dir(_))
                    | SimpleSelector::PartState(PseudoClass::Dir(_)) => {
                        self.ancestors.attributes.insert("dir".to_owned());
                    }
                    // What `:host()` and `:host-context()` test is of the
                    // host and its ancestors, outside the shadow tree.
                    SimpleSelector::PseudoClass(
                        PseudoClass::Host(Some(list)) | PseudoClass::HostContext(list),
                    ) => list
                        .0
                        .iter()
                        .for_each(|nested| self.add_selector(nested, Role::Ancestor)),
                    simple => match role {
                        Role::Subject => {}
                        Role::Sibling => self.siblings.add_simple(simple),
                        Role::Ancestor => self.ancestors.add_simple(simple),
                    },
                }
            }
        }
    }
}

/// A relative selector of `:has()`, such as `> img`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relative {
    /// How the matching element relates to the subject; `Descendant` when no
    /// combinator is written.
    pub combinator: Combinator,
    /// The selector.
    pub selector: Selector,
}

/// A comma-separated list of selectors.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SelectorList(pub Vec<Selector>);

impl Selector {
    /// Whether the selector looks at descendants with `:has()`.
    pub fn is_relational(&self) -> bool {
        self.compounds.iter().any(|compound| {
            compound.simple.iter().any(|simple| match simple {
                SimpleSelector::PseudoClass(PseudoClass::Has(_))
                | SimpleSelector::PartState(PseudoClass::Has(_)) => true,
                SimpleSelector::PseudoClass(
                    PseudoClass::Not(list)
                    | PseudoClass::Is(list)
                    | PseudoClass::Where(list)
                    | PseudoClass::NthChildOf(_, list)
                    | PseudoClass::NthLastChildOf(_, list),
                )
                | SimpleSelector::PartState(
                    PseudoClass::Not(list)
                    | PseudoClass::Is(list)
                    | PseudoClass::Where(list)
                    | PseudoClass::NthChildOf(_, list)
                    | PseudoClass::NthLastChildOf(_, list),
                ) => list.is_relational(),
                _ => false,
            })
        })
    }
}

impl SelectorList {
    /// Parses a selector list from text.
    pub fn parse(text: &str) -> Result<SelectorList, String> {
        parse_selector_list(&tokens(text))
    }

    /// Whether any selector in the list matches the element itself.
    pub fn matches<E: Element>(&self, element: &E) -> bool {
        self.0.iter().any(|selector| selector.matches(element))
    }

    fn max_specificity(&self) -> Specificity {
        self.0
            .iter()
            .map(|selector| selector.specificity)
            .max()
            .unwrap_or_default()
    }

    /// Whether a selector in the list looks at descendants with `:has()`,
    /// so matching an element depends on more than its own state, its
    /// siblings and its ancestors.
    pub fn is_relational(&self) -> bool {
        self.0.iter().any(Selector::is_relational)
    }

    /// Pseudo-classes in the list that the native target cannot evaluate.
    pub fn unsupported(&self) -> Vec<String> {
        let mut names = Vec::new();
        for selector in &self.0 {
            for compound in &selector.compounds {
                for simple in &compound.simple {
                    match simple {
                        SimpleSelector::PseudoClass(PseudoClass::Unsupported(name))
                        | SimpleSelector::PartState(PseudoClass::Unsupported(name)) => {
                            names.push(format!(":{name}"));
                        }
                        SimpleSelector::PseudoClass(
                            PseudoClass::Not(list)
                            | PseudoClass::Is(list)
                            | PseudoClass::Where(list)
                            | PseudoClass::NthChildOf(_, list)
                            | PseudoClass::NthLastChildOf(_, list),
                        )
                        | SimpleSelector::PartState(
                            PseudoClass::Not(list)
                            | PseudoClass::Is(list)
                            | PseudoClass::Where(list)
                            | PseudoClass::NthChildOf(_, list)
                            | PseudoClass::NthLastChildOf(_, list),
                        ) => names.extend(list.unsupported()),
                        SimpleSelector::PseudoClass(PseudoClass::Has(relative))
                        | SimpleSelector::PartState(PseudoClass::Has(relative)) => {
                            names.extend(relative_list(relative).unsupported());
                        }
                        _ => {}
                    }
                }
            }
        }
        names
    }
}

/// Bit flags describing dynamic element state used by pseudo-classes.
pub mod state {
    /// The pointer is over the element or a descendant.
    pub const HOVER: u32 = 1;
    /// The element is being activated.
    pub const ACTIVE: u32 = 1 << 1;
    /// The element has focus.
    pub const FOCUS: u32 = 1 << 2;
    /// The element has keyboard-visible focus.
    pub const FOCUS_VISIBLE: u32 = 1 << 3;
    /// The element or a descendant has focus.
    pub const FOCUS_WITHIN: u32 = 1 << 4;
    /// The element is a disabled form control.
    pub const DISABLED: u32 = 1 << 5;
    /// The element is a checked checkbox, radio or selected option.
    pub const CHECKED: u32 = 1 << 6;
    /// The element is an indeterminate checkbox.
    pub const INDETERMINATE: u32 = 1 << 7;
    /// The element is a required form control.
    pub const REQUIRED: u32 = 1 << 8;
    /// The element accepts text editing.
    pub const EDITABLE: u32 = 1 << 9;
    /// The element shows its placeholder.
    pub const PLACEHOLDER_SHOWN: u32 = 1 << 10;
    /// The element is a hyperlink with an `href`.
    pub const LINK: u32 = 1 << 11;
    /// The element is a form control that can be enabled or disabled.
    pub const FORM_CONTROL: u32 = 1 << 12;
    /// The part belongs to a vertical scrollbar.
    pub const VERTICAL: u32 = 1 << 13;
    /// The part belongs to a horizontal scrollbar.
    pub const HORIZONTAL: u32 = 1 << 14;
    /// The control, or every control in the form or fieldset, satisfies
    /// its constraints.
    pub const VALID: u32 = 1 << 15;
    /// The control, or a control in the form or fieldset, fails a
    /// constraint.
    pub const INVALID: u32 = 1 << 16;
    /// The element has a custom element name and no definition has
    /// upgraded it yet.
    pub const UNDEFINED: u32 = 1 << 17;
}

/// The view of an element that selector matching needs.
/// A Bloom filter of the tags, ids and classes of an element's ancestors:
/// a descendant selector whose ancestor part names something no ancestor
/// has fails without walking them, as Chromium's selector filter does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AncestorFilter([u64; 4]);

impl AncestorFilter {
    /// Adds a feature's hash.
    pub fn insert(&mut self, hash: u32) {
        for bit in [hash & 0xff, hash >> 8 & 0xff] {
            self.0[(bit >> 6) as usize] |= 1 << (bit & 63);
        }
    }

    /// Whether a feature may be among the ancestors.
    pub fn may_contain(&self, hash: u32) -> bool {
        [hash & 0xff, hash >> 8 & 0xff]
            .iter()
            .all(|bit| self.0[(bit >> 6) as usize] & 1 << (bit & 63) != 0)
    }

    /// Adds an element's tag, id and classes.
    pub fn insert_element(&mut self, tag: &str, id: Option<&str>, classes: Option<&str>) {
        self.insert(feature_hash(b't', tag, true));
        if let Some(id) = id {
            self.insert(feature_hash(b'#', id, false));
        }
        for class in classes.into_iter().flat_map(str::split_ascii_whitespace) {
            self.insert(feature_hash(b'.', class, false));
        }
    }
}

/// A 32-bit FNV-1a hash of a kind of feature and its name; tags compare
/// without case.
fn feature_hash(kind: u8, name: &str, fold: bool) -> u32 {
    let step = |hash: u32, byte: u8| (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    let start = step(0x811c_9dc5, kind);
    name.bytes().fold(start, |hash, byte| {
        step(
            hash,
            if fold {
                byte.to_ascii_lowercase()
            } else {
                byte
            },
        )
    })
}

/// Whether some ancestor could match a compound by the features it names.
fn ancestors_may_match(compound: &Compound, filter: &AncestorFilter) -> bool {
    if let Some(tag) = &compound.tag
        && tag != "*"
        && !filter.may_contain(feature_hash(b't', tag, true))
    {
        return false;
    }
    compound.simple.iter().all(|simple| match simple {
        SimpleSelector::Id(id) => filter.may_contain(feature_hash(b'#', id, false)),
        SimpleSelector::Class(class) => filter.may_contain(feature_hash(b'.', class, false)),
        _ => true,
    })
}

pub trait Element: Sized {
    /// Lowercased local name.
    fn local_name(&self) -> &str;
    /// The `id` attribute.
    fn id(&self) -> Option<&str>;
    /// Whether the class list contains `class`.
    fn has_class(&self, class: &str) -> bool;
    /// An attribute value by lowercased name.
    fn attribute(&self, name: &str) -> Option<&str>;
    /// The parent element.
    fn parent_element(&self) -> Option<Self>;
    /// The previous element sibling.
    fn previous_element_sibling(&self) -> Option<Self>;
    /// The next element sibling.
    fn next_element_sibling(&self) -> Option<Self>;
    /// The first element child.
    fn first_element_child(&self) -> Option<Self>;
    /// Whether this is the same element as `other`.
    fn is_same(&self, other: &Self) -> bool;
    /// Whether this is the document element.
    fn is_root(&self) -> bool;
    /// Whether this is the scoping root of a query (`:scope`); stylesheets
    /// scope to the document element.
    fn is_scope(&self) -> bool {
        self.is_root()
    }
    /// Whether the element has no element or text children.
    fn is_empty(&self) -> bool;
    /// Dynamic state flags from [`state`].
    fn state(&self) -> u32;
    /// The filter of the element's ancestors' features, when the element
    /// keeps one; without it descendant selectors walk the ancestors.
    fn ancestor_filter(&self) -> Option<AncestorFilter> {
        None
    }
    /// The tree the element is in, as stylesheets name their scope: 0 for
    /// the document, else its shadow root's.
    fn tree_scope(&self) -> u32 {
        0
    }
    /// The scope of the shadow tree the element hosts.
    fn shadow_scope(&self) -> Option<u32> {
        None
    }
    /// The element as its own shadow tree's rules see it: featureless but
    /// for `:host`, `:host()` and `:host-context()`.
    fn as_featureless_host(&self) -> Option<Self> {
        None
    }
    /// Whether this is a host seen from its own shadow tree.
    fn is_featureless_host(&self) -> bool {
        false
    }
    /// A featureless host with its features, which `:host()` tests.
    fn featured(&self) -> Option<Self> {
        None
    }
    /// The parent element, or for the top of a shadow tree its host, which
    /// `:host-context()` walks.
    fn host_context_parent(&self) -> Option<Self> {
        self.parent_element()
    }
    /// The slot the element is assigned to.
    fn assigned_slot(&self) -> Option<Self> {
        None
    }
    /// The element's language: the `lang` attribute of the element or its
    /// nearest ancestor that has one.
    fn language(&self) -> Option<String> {
        None
    }
    /// Whether the element's directionality is right-to-left.
    fn is_rtl(&self) -> bool {
        false
    }
}

/// Whether a language range of `:lang()` matches a language: the same tag,
/// or a tag that starts with the range and a hyphen, ignoring case; `*`
/// matches every tag.
fn language_matches(language: &str, range: &str) -> bool {
    if range == "*" {
        return !language.is_empty();
    }
    language.eq_ignore_ascii_case(range)
        || (language.len() > range.len()
            && language.is_char_boundary(range.len())
            && language[..range.len()].eq_ignore_ascii_case(range)
            && language.as_bytes()[range.len()] == b'-')
}

impl Selector {
    /// Whether the selector matches the element itself (not a pseudo-element).
    pub fn matches<E: Element>(&self, element: &E) -> bool {
        self.pseudo_element.is_none() && self.matches_at(self.compounds.len() - 1, element, None)
    }

    /// Whether a `::slotted()` selector matches an element assigned to
    /// `slot`: the selector matches the slot and its argument the element.
    pub fn matches_slotted<E: Element>(&self, slot: &E, element: &E) -> bool {
        self.pseudo_element == Some(PseudoElement::Slotted)
            && self.matches_at(self.compounds.len() - 1, slot, None)
            && self
                .slotted
                .as_ref()
                .is_some_and(|list| list.matches(element))
    }

    /// Whether the selector matches the given pseudo-element of the element.
    pub fn matches_pseudo<E: Element>(&self, element: &E, pseudo: &PseudoElement) -> bool {
        self.matches_pseudo_state(element, pseudo, 0)
    }

    /// Matches a pseudo-element with its own hover, active and axis state.
    pub fn matches_pseudo_state<E: Element>(
        &self,
        element: &E,
        pseudo: &PseudoElement,
        flags: u32,
    ) -> bool {
        self.pseudo_element.as_ref() == Some(pseudo)
            && self.matches_at(self.compounds.len() - 1, element, None)
            && self
                .compounds
                .last()
                .unwrap()
                .simple
                .iter()
                .all(|simple| match simple {
                    SimpleSelector::PartState(class) => matches_part(class, flags),
                    _ => true,
                })
    }

    /// Matches compounds from `index` leftwards. With an anchor, the element
    /// matching the leftmost compound must relate to the anchor element by
    /// the anchor's combinator, as a relative selector of `:has()` requires.
    fn matches_at<E: Element>(
        &self,
        index: usize,
        element: &E,
        anchor: Option<(&E, Combinator)>,
    ) -> bool {
        if !matches_compound(&self.compounds[index], element) {
            return false;
        }
        if index == 0 {
            return anchor.is_none_or(|(scope, combinator)| related(scope, element, combinator));
        }
        let previous = index - 1;
        let matches = |candidate: &E| self.matches_at(previous, candidate, anchor);
        match self.combinators[previous] {
            Combinator::Child => element
                .parent_element()
                .is_some_and(|parent| matches(&parent)),
            Combinator::NextSibling => element
                .previous_element_sibling()
                .is_some_and(|sibling| matches(&sibling)),
            Combinator::Descendant => {
                if element
                    .ancestor_filter()
                    .is_some_and(|filter| !ancestors_may_match(&self.compounds[previous], &filter))
                {
                    return false;
                }
                let mut current = element.parent_element();
                while let Some(ancestor) = current {
                    if matches(&ancestor) {
                        return true;
                    }
                    current = ancestor.parent_element();
                }
                false
            }
            Combinator::SubsequentSibling => {
                let mut current = element.previous_element_sibling();
                while let Some(sibling) = current {
                    if matches(&sibling) {
                        return true;
                    }
                    current = sibling.previous_element_sibling();
                }
                false
            }
        }
    }
}

/// Whether `element` relates to `scope` by `combinator`: as a descendant,
/// a child, the next sibling or a later sibling.
fn related<E: Element>(scope: &E, element: &E, combinator: Combinator) -> bool {
    let (step, once): (fn(&E) -> Option<E>, bool) = match combinator {
        Combinator::Child => (E::parent_element, true),
        Combinator::Descendant => (E::parent_element, false),
        Combinator::NextSibling => (E::previous_element_sibling, true),
        Combinator::SubsequentSibling => (E::previous_element_sibling, false),
    };
    let mut current = step(element);
    while let Some(candidate) = current {
        if candidate.is_same(scope) {
            return true;
        }
        if once {
            return false;
        }
        current = step(&candidate);
    }
    false
}

/// Whether a descendant of `root` passes `test`.
fn any_descendant<E: Element>(root: &E, test: &dyn Fn(&E) -> bool) -> bool {
    let mut child = root.first_element_child();
    while let Some(current) = child {
        if test(&current) || any_descendant(&current, test) {
            return true;
        }
        child = current.next_element_sibling();
    }
    false
}

/// Whether an element matching a relative selector relates to `scope`:
/// candidates are the scope's descendants, or its later siblings and their
/// descendants.
fn has_relative<E: Element>(scope: &E, relative: &Relative) -> bool {
    let selector = &relative.selector;
    let last = selector.compounds.len() - 1;
    let anchored =
        |candidate: &E| selector.matches_at(last, candidate, Some((scope, relative.combinator)));
    match relative.combinator {
        Combinator::Descendant | Combinator::Child => any_descendant(scope, &anchored),
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            let mut sibling = scope.next_element_sibling();
            while let Some(current) = sibling {
                if anchored(&current) || any_descendant(&current, &anchored) {
                    return true;
                }
                sibling = current.next_element_sibling();
            }
            false
        }
    }
}

/// The selectors of relative selectors, as a list.
fn relative_list(relative: &[Relative]) -> SelectorList {
    SelectorList(
        relative
            .iter()
            .map(|relative| relative.selector.clone())
            .collect(),
    )
}

fn matches_compound<E: Element>(compound: &Compound, element: &E) -> bool {
    // A host seen from its own shadow tree matches only compounds of
    // `:host`, `:host()` and `:host-context()`.
    if element.is_featureless_host() {
        return compound.tag.as_deref().is_none_or(|tag| tag == "*")
            && !compound.simple.is_empty()
            && compound.simple.iter().all(|simple| {
                matches!(
                    simple,
                    SimpleSelector::PseudoClass(PseudoClass::Host(_) | PseudoClass::HostContext(_))
                ) && matches_simple(simple, element)
            });
    }
    if let Some(tag) = &compound.tag
        && !element.local_name().eq_ignore_ascii_case(tag)
    {
        return false;
    }
    compound
        .simple
        .iter()
        .all(|simple| matches_simple(simple, element))
}

fn matches_simple<E: Element>(simple: &SimpleSelector, element: &E) -> bool {
    match simple {
        SimpleSelector::Id(id) => element.id() == Some(id.as_str()),
        SimpleSelector::Class(class) => element.has_class(class),
        SimpleSelector::Attribute(attribute) => matches_attribute(attribute, element),
        SimpleSelector::PseudoClass(pseudo) => matches_pseudo_class(pseudo, element),
        SimpleSelector::PartState(_) => true,
    }
}

fn matches_attribute<E: Element>(selector: &AttributeSelector, element: &E) -> bool {
    let Some(actual) = element.attribute(&selector.name) else {
        return false;
    };
    let Some((operator, expected)) = &selector.matcher else {
        return true;
    };
    // Only case-insensitive matches need lowered copies.
    let lowered;
    let (actual, expected): (&str, &str) = if selector.case_insensitive {
        lowered = (actual.to_ascii_lowercase(), expected.to_ascii_lowercase());
        (&lowered.0, &lowered.1)
    } else {
        (actual, expected)
    };
    match operator {
        AttributeOperator::Equals => actual == expected,
        AttributeOperator::Includes => actual.split_ascii_whitespace().any(|word| word == expected),
        AttributeOperator::DashMatch => {
            actual == expected
                || actual
                    .strip_prefix(expected)
                    .is_some_and(|rest| rest.starts_with('-'))
        }
        AttributeOperator::Prefix => !expected.is_empty() && actual.starts_with(expected),
        AttributeOperator::Suffix => !expected.is_empty() && actual.ends_with(expected),
        AttributeOperator::Substring => !expected.is_empty() && actual.contains(expected),
    }
}

fn is_form_control(element: &impl Element) -> bool {
    element.state() & state::FORM_CONTROL != 0
}

/// Counts siblings in one direction, optionally only those of the same type.
/// The 1-based position of an element `S` matches among its siblings `S`
/// matches, for `:nth-child(An+B of S)`, or `None` when `S` does not match it.
fn position_among<E: Element>(element: &E, forward: bool, list: &SelectorList) -> Option<i32> {
    if !list.matches(element) {
        return None;
    }
    let mut position = 1;
    let mut current = if forward {
        element.next_element_sibling()
    } else {
        element.previous_element_sibling()
    };
    while let Some(sibling) = current {
        if list.matches(&sibling) {
            position += 1;
        }
        current = if forward {
            sibling.next_element_sibling()
        } else {
            sibling.previous_element_sibling()
        };
    }
    Some(position)
}

fn sibling_position<E: Element>(element: &E, forward: bool, same_type: bool) -> i32 {
    let step = |current: &E| {
        if forward {
            current.next_element_sibling()
        } else {
            current.previous_element_sibling()
        }
    };
    let mut position = 1;
    let mut current = step(element);
    while let Some(sibling) = current {
        if !same_type || sibling.local_name() == element.local_name() {
            position += 1;
        }
        current = step(&sibling);
    }
    position
}

fn matches_pseudo_class<E: Element>(pseudo: &PseudoClass, element: &E) -> bool {
    let has = |flag: u32| element.state() & flag != 0;
    match pseudo {
        PseudoClass::Hover => has(state::HOVER),
        PseudoClass::Vertical | PseudoClass::Horizontal => false,
        PseudoClass::Active => has(state::ACTIVE),
        PseudoClass::Focus => has(state::FOCUS),
        PseudoClass::FocusVisible => has(state::FOCUS_VISIBLE),
        PseudoClass::FocusWithin => has(state::FOCUS_WITHIN),
        PseudoClass::Disabled => has(state::DISABLED),
        PseudoClass::Enabled => is_form_control(element) && !has(state::DISABLED),
        PseudoClass::Checked => has(state::CHECKED),
        PseudoClass::Indeterminate => has(state::INDETERMINATE),
        PseudoClass::Required => has(state::REQUIRED),
        PseudoClass::Optional => is_form_control(element) && !has(state::REQUIRED),
        PseudoClass::ReadOnly => !has(state::EDITABLE),
        PseudoClass::ReadWrite => has(state::EDITABLE),
        PseudoClass::PlaceholderShown => has(state::PLACEHOLDER_SHOWN),
        PseudoClass::Defined => !has(state::UNDEFINED),
        PseudoClass::Valid => has(state::VALID),
        PseudoClass::Invalid => has(state::INVALID),
        PseudoClass::Link => has(state::LINK),
        PseudoClass::Root => element.is_root(),
        PseudoClass::Scope => element.is_scope(),
        PseudoClass::Empty => element.is_empty(),
        PseudoClass::FirstChild => element.previous_element_sibling().is_none(),
        PseudoClass::LastChild => element.next_element_sibling().is_none(),
        PseudoClass::OnlyChild => {
            element.previous_element_sibling().is_none() && element.next_element_sibling().is_none()
        }
        PseudoClass::FirstOfType => sibling_position(element, false, true) == 1,
        PseudoClass::LastOfType => sibling_position(element, true, true) == 1,
        PseudoClass::OnlyOfType => {
            sibling_position(element, false, true) == 1
                && sibling_position(element, true, true) == 1
        }
        PseudoClass::NthChild(nth) => nth.matches(sibling_position(element, false, false)),
        PseudoClass::NthLastChild(nth) => nth.matches(sibling_position(element, true, false)),
        PseudoClass::NthChildOf(nth, list) => {
            position_among(element, false, list).is_some_and(|position| nth.matches(position))
        }
        PseudoClass::NthLastChildOf(nth, list) => {
            position_among(element, true, list).is_some_and(|position| nth.matches(position))
        }
        PseudoClass::NthOfType(nth) => nth.matches(sibling_position(element, false, true)),
        PseudoClass::NthLastOfType(nth) => nth.matches(sibling_position(element, true, true)),
        PseudoClass::Not(list) => !list.matches(element),
        PseudoClass::Is(list) | PseudoClass::Where(list) => list.matches(element),
        PseudoClass::Has(relative) => relative
            .iter()
            .any(|relative| has_relative(element, relative)),
        PseudoClass::Host(list) => {
            element.is_featureless_host()
                && list
                    .as_ref()
                    .is_none_or(|list| element.featured().is_some_and(|host| list.matches(&host)))
        }
        PseudoClass::HostContext(list) => {
            if !element.is_featureless_host() {
                return false;
            }
            let mut current = element.featured();
            while let Some(candidate) = current {
                if list.matches(&candidate) {
                    return true;
                }
                current = candidate.host_context_parent();
            }
            false
        }
        PseudoClass::Lang(ranges) => element.language().is_some_and(|language| {
            ranges
                .iter()
                .any(|range| language_matches(&language, range))
        }),
        PseudoClass::Dir(direction) => match direction.as_str() {
            "rtl" => element.is_rtl(),
            "ltr" => !element.is_rtl(),
            _ => false,
        },
        PseudoClass::Never(_) | PseudoClass::Unsupported(_) => false,
    }
}

/// Parses a selector list from tokens.
pub fn parse_selector_list(tokens: &[Token]) -> Result<SelectorList, String> {
    let mut parser = Parser {
        tokens,
        index: 0,
        slotted: None,
    };
    let list = parser.list()?;
    Ok(list)
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    /// The argument of the `::slotted()` the current selector ends with.
    slotted: Option<SelectorList>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.index;
        while self.peek() == Some(&Token::Whitespace) {
            self.index += 1;
        }
        self.index > start
    }

    fn list(&mut self) -> Result<SelectorList, String> {
        let mut selectors = Vec::new();
        loop {
            self.skip_whitespace();
            selectors.push(self.selector()?);
            match self.peek() {
                None => return Ok(SelectorList(selectors)),
                Some(_) => self.index += 1,
            }
        }
    }

    fn selector(&mut self) -> Result<Selector, String> {
        let mut pseudo_element = None;
        let mut compounds = vec![self.compound(&mut pseudo_element)?];
        let mut combinators = Vec::new();
        loop {
            let had_whitespace = self.skip_whitespace();
            let combinator = match self.peek() {
                None | Some(Token::Comma) => break,
                Some(Token::Delim('>')) => Combinator::Child,
                Some(Token::Delim('+')) => Combinator::NextSibling,
                Some(Token::Delim('~')) => Combinator::SubsequentSibling,
                Some(_) if had_whitespace => Combinator::Descendant,
                Some(token) => return Err(format!("unexpected `{}` in selector", token.to_css())),
            };
            if combinator != Combinator::Descendant {
                self.index += 1;
                self.skip_whitespace();
            }
            if pseudo_element.is_some() {
                return Err("a pseudo-element must be last in a selector".to_owned());
            }
            combinators.push(combinator);
            compounds.push(self.compound(&mut pseudo_element)?);
        }
        let mut specificity = Specificity::default();
        for compound in &compounds {
            specificity = specificity.add(compound_specificity(compound));
        }
        if pseudo_element.is_some() {
            specificity = specificity.add(Specificity(0, 0, 1));
        }
        let slotted = self.slotted.take();
        if let Some(list) = &slotted {
            specificity = specificity.add(list.max_specificity());
        }
        Ok(Selector {
            compounds,
            combinators,
            pseudo_element,
            slotted,
            specificity,
        })
    }

    fn compound(&mut self, pseudo_element: &mut Option<PseudoElement>) -> Result<Compound, String> {
        let mut compound = Compound::default();
        let start = self.index;
        match self.peek() {
            Some(Token::Ident(name)) => {
                compound.tag = Some(name.to_ascii_lowercase());
                self.index += 1;
            }
            Some(Token::Delim('*')) => self.index += 1,
            _ => {}
        }
        loop {
            let simple = match self.peek() {
                Some(Token::Hash { value, id: true }) => {
                    let id = value.clone();
                    self.index += 1;
                    SimpleSelector::Id(id)
                }
                Some(Token::Delim('.')) => match self.tokens.get(self.index + 1) {
                    Some(Token::Ident(class)) => {
                        let class = class.clone();
                        self.index += 2;
                        SimpleSelector::Class(class)
                    }
                    _ => return Err("expected a class name after `.`".to_owned()),
                },
                Some(Token::LeftBracket) => {
                    self.index += 1;
                    SimpleSelector::Attribute(self.attribute()?)
                }
                Some(Token::Colon) => {
                    self.index += 1;
                    match self.pseudo()? {
                        Pseudo::Class(class) => {
                            if pseudo_element.is_some() {
                                SimpleSelector::PartState(class)
                            } else {
                                SimpleSelector::PseudoClass(class)
                            }
                        }
                        Pseudo::Element(element) => {
                            if pseudo_element.is_some() {
                                return Err(
                                    "a selector may target only one pseudo-element".to_owned()
                                );
                            }
                            *pseudo_element = Some(element);
                            continue;
                        }
                    }
                }
                _ => break,
            };
            if pseudo_element.is_some() && !matches!(simple, SimpleSelector::PartState(_)) {
                return Err("only state pseudo-classes may follow a pseudo-element".to_owned());
            }
            compound.simple.push(simple);
        }
        if self.index == start {
            return Err(match self.peek() {
                Some(token) => format!("unexpected `{}` in selector", token.to_css()),
                None => "expected a selector".to_owned(),
            });
        }
        Ok(compound)
    }

    fn attribute(&mut self) -> Result<AttributeSelector, String> {
        self.skip_whitespace();
        let name = match self.peek() {
            Some(Token::Ident(name)) => name.to_ascii_lowercase(),
            _ => return Err("expected an attribute name".to_owned()),
        };
        self.index += 1;
        self.skip_whitespace();
        let mut selector = AttributeSelector {
            name,
            matcher: None,
            case_insensitive: false,
        };
        let operator = match self.peek() {
            Some(Token::RightBracket) => {
                self.index += 1;
                return Ok(selector);
            }
            Some(Token::Delim('=')) => {
                self.index += 1;
                AttributeOperator::Equals
            }
            Some(Token::Delim(prefix @ ('~' | '|' | '^' | '$' | '*')))
                if self.tokens.get(self.index + 1) == Some(&Token::Delim('=')) =>
            {
                let operator = match prefix {
                    '~' => AttributeOperator::Includes,
                    '|' => AttributeOperator::DashMatch,
                    '^' => AttributeOperator::Prefix,
                    '$' => AttributeOperator::Suffix,
                    _ => AttributeOperator::Substring,
                };
                self.index += 2;
                operator
            }
            _ => return Err("invalid attribute selector".to_owned()),
        };
        self.skip_whitespace();
        let value = match self.peek() {
            Some(Token::Ident(value) | Token::String(value)) => value.clone(),
            Some(Token::Number { value, .. }) => crate::tokenizer::format_number(*value),
            _ => return Err("expected an attribute value".to_owned()),
        };
        self.index += 1;
        self.skip_whitespace();
        if let Some(Token::Ident(flag)) = self.peek() {
            selector.case_insensitive = match flag.to_ascii_lowercase().as_str() {
                "i" => true,
                "s" => false,
                _ => return Err("invalid attribute selector flag".to_owned()),
            };
            self.index += 1;
            self.skip_whitespace();
        }
        if self.peek() != Some(&Token::RightBracket) {
            return Err("expected `]`".to_owned());
        }
        self.index += 1;
        selector.matcher = Some((operator, value));
        Ok(selector)
    }

    fn pseudo(&mut self) -> Result<Pseudo, String> {
        if self.peek() == Some(&Token::Colon) {
            self.index += 1;
            return match self.peek() {
                Some(Token::Ident(name)) => {
                    let element = PseudoElement::from_name(name);
                    self.index += 1;
                    Ok(Pseudo::Element(element))
                }
                Some(Token::Function(name)) if name.eq_ignore_ascii_case("slotted") => {
                    self.index += 1;
                    let arguments = self.arguments();
                    self.slotted = Some(compound_list(&arguments, "::slotted()")?);
                    Ok(Pseudo::Element(PseudoElement::Slotted))
                }
                Some(Token::Function(name)) => {
                    let element = PseudoElement::Other(name.to_ascii_lowercase());
                    self.index += 1;
                    self.arguments();
                    Ok(Pseudo::Element(element))
                }
                _ => Err("expected a pseudo-element name".to_owned()),
            };
        }
        match self.peek() {
            Some(Token::Ident(name)) => {
                let name = name.to_ascii_lowercase();
                self.index += 1;
                Ok(match name.as_str() {
                    "before" | "after" | "first-line" | "first-letter" => {
                        Pseudo::Element(PseudoElement::from_name(&name))
                    }
                    _ => Pseudo::Class(simple_pseudo_class(name)),
                })
            }
            Some(Token::Function(name)) => {
                let name = name.to_ascii_lowercase();
                self.index += 1;
                let arguments = self.arguments();
                functional_pseudo_class(&name, &arguments).map(Pseudo::Class)
            }
            _ => Err("expected a pseudo-class name".to_owned()),
        }
    }

    /// Collects the tokens of a function's arguments and consumes `)`.
    fn arguments(&mut self) -> Vec<Token> {
        let mut depth = 0;
        let mut arguments = Vec::new();
        while let Some(token) = self.peek() {
            let token = token.clone();
            self.index += 1;
            match token {
                Token::RightParen if depth == 0 => break,
                Token::RightParen => depth -= 1,
                Token::Function(_) | Token::LeftParen => depth += 1,
                _ => {}
            }
            arguments.push(token);
        }
        arguments
    }
}

enum Pseudo {
    Class(PseudoClass),
    Element(PseudoElement),
}

fn simple_pseudo_class(name: String) -> PseudoClass {
    match name.as_str() {
        "hover" => PseudoClass::Hover,
        "vertical" => PseudoClass::Vertical,
        "horizontal" => PseudoClass::Horizontal,
        "active" => PseudoClass::Active,
        "focus" => PseudoClass::Focus,
        "focus-visible" => PseudoClass::FocusVisible,
        "focus-within" => PseudoClass::FocusWithin,
        "disabled" => PseudoClass::Disabled,
        "enabled" => PseudoClass::Enabled,
        "checked" => PseudoClass::Checked,
        "indeterminate" => PseudoClass::Indeterminate,
        "required" => PseudoClass::Required,
        "optional" => PseudoClass::Optional,
        "read-only" => PseudoClass::ReadOnly,
        "read-write" => PseudoClass::ReadWrite,
        "placeholder-shown" => PseudoClass::PlaceholderShown,
        "host" => PseudoClass::Host(None),
        "defined" => PseudoClass::Defined,
        "valid" => PseudoClass::Valid,
        "invalid" => PseudoClass::Invalid,
        "root" => PseudoClass::Root,
        "scope" => PseudoClass::Scope,
        "empty" => PseudoClass::Empty,
        "first-child" => PseudoClass::FirstChild,
        "last-child" => PseudoClass::LastChild,
        "only-child" => PseudoClass::OnlyChild,
        "first-of-type" => PseudoClass::FirstOfType,
        "last-of-type" => PseudoClass::LastOfType,
        "only-of-type" => PseudoClass::OnlyOfType,
        "link" | "any-link" => PseudoClass::Link,
        "visited" | "target" => PseudoClass::Never(name),
        _ => PseudoClass::Unsupported(name),
    }
}

/// The parts of a list between its top-level commas: commas inside nested
/// functions, parentheses and brackets belong to their part.
fn top_level_parts(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Function(_) | Token::LeftParen | Token::LeftBracket => depth += 1,
            Token::RightParen | Token::RightBracket => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                parts.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&tokens[start..]);
    parts
}

/// `An+B of S`: the `An+B` tokens and the selector list after a top-level
/// `of`, when there is one.
fn nth_of(arguments: &[Token]) -> Option<(&[Token], &[Token])> {
    let mut depth = 0usize;
    for (index, token) in arguments.iter().enumerate() {
        match token {
            Token::Function(_) | Token::LeftParen | Token::LeftBracket => depth += 1,
            Token::RightParen | Token::RightBracket => depth = depth.saturating_sub(1),
            Token::Ident(word) if depth == 0 && word.eq_ignore_ascii_case("of") => {
                return Some((&arguments[..index], &arguments[index + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn functional_pseudo_class(name: &str, arguments: &[Token]) -> Result<PseudoClass, String> {
    let nth = || parse_nth(arguments).ok_or_else(|| format!("invalid :{name}() argument"));
    if matches!(name, "nth-child" | "nth-last-child")
        && let Some((step, selectors)) = nth_of(arguments)
    {
        let nth = parse_nth(step).ok_or_else(|| format!("invalid :{name}() argument"))?;
        let list = parse_selector_list(selectors)?;
        return Ok(if name == "nth-child" {
            PseudoClass::NthChildOf(nth, list)
        } else {
            PseudoClass::NthLastChildOf(nth, list)
        });
    }
    Ok(match name {
        "host" => PseudoClass::Host(Some(compound_list(arguments, ":host()")?)),
        "lang" => PseudoClass::Lang(language_ranges(arguments)?),
        "dir" => match arguments
            .iter()
            .filter(|token| **token != Token::Whitespace)
            .collect::<Vec<_>>()[..]
        {
            [Token::Ident(value)] => PseudoClass::Dir(value.to_ascii_lowercase()),
            _ => return Err("invalid :dir() argument".to_owned()),
        },
        "host-context" => PseudoClass::HostContext(compound_list(arguments, ":host-context()")?),
        "not" => PseudoClass::Not(parse_selector_list(arguments)?),
        "is" | "matches" | "-webkit-any" | "-moz-any" => PseudoClass::Is(forgiving(arguments)),
        "where" => PseudoClass::Where(forgiving(arguments)),
        "has" => PseudoClass::Has(relative_selectors(arguments)?),
        "nth-child" => PseudoClass::NthChild(nth()?),
        "nth-last-child" => PseudoClass::NthLastChild(nth()?),
        "nth-of-type" => PseudoClass::NthOfType(nth()?),
        "nth-last-of-type" => PseudoClass::NthLastOfType(nth()?),
        other => PseudoClass::Unsupported(other.to_owned()),
    })
}

/// Parses the language ranges of `:lang()`: identifiers or strings,
/// separated by commas.
fn language_ranges(arguments: &[Token]) -> Result<Vec<String>, String> {
    let mut ranges = Vec::new();
    for part in top_level_parts(arguments) {
        let tokens: Vec<&Token> = part
            .iter()
            .filter(|token| **token != Token::Whitespace)
            .collect();
        match tokens[..] {
            [Token::Ident(range) | Token::String(range)] => ranges.push(range.clone()),
            [Token::Delim('*')] => ranges.push("*".to_owned()),
            _ => return Err("invalid :lang() argument".to_owned()),
        }
    }
    if ranges.is_empty() {
        return Err("invalid :lang() argument".to_owned());
    }
    Ok(ranges)
}

/// Parses the argument of `:host()`, `:host-context()` or `::slotted()`:
/// compound selectors without pseudo-elements.
fn compound_list(arguments: &[Token], name: &str) -> Result<SelectorList, String> {
    let list = parse_selector_list(arguments)?;
    if list
        .0
        .iter()
        .any(|selector| selector.compounds.len() != 1 || selector.pseudo_element.is_some())
    {
        return Err(format!("{name} takes compound selectors"));
    }
    Ok(list)
}

/// Parses the relative selectors of `:has()`, each with an optional leading
/// combinator.
fn relative_selectors(arguments: &[Token]) -> Result<Vec<Relative>, String> {
    let mut relative = Vec::new();
    for part in top_level_parts(arguments) {
        let start = part
            .iter()
            .position(|token| *token != Token::Whitespace)
            .ok_or("empty :has() argument")?;
        let (combinator, rest) = match part[start] {
            Token::Delim('>') => (Combinator::Child, &part[start + 1..]),
            Token::Delim('+') => (Combinator::NextSibling, &part[start + 1..]),
            Token::Delim('~') => (Combinator::SubsequentSibling, &part[start + 1..]),
            _ => (Combinator::Descendant, &part[start..]),
        };
        for selector in parse_selector_list(rest)?.0 {
            relative.push(Relative {
                combinator,
                selector,
            });
        }
    }
    Ok(relative)
}

/// Parses a forgiving selector list, dropping invalid entries.
fn forgiving(arguments: &[Token]) -> SelectorList {
    let mut selectors = Vec::new();
    for part in top_level_parts(arguments) {
        if let Ok(list) = parse_selector_list(part) {
            selectors.extend(list.0);
        }
    }
    SelectorList(selectors)
}

/// Parses `An+B`, `odd` and `even`.
pub fn parse_nth(arguments: &[Token]) -> Option<Nth> {
    let text: String = serialize_tokens(arguments)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    match text.as_str() {
        "odd" => return Some(Nth { a: 2, b: 1 }),
        "even" => return Some(Nth { a: 2, b: 0 }),
        _ => {}
    }
    let Some(n) = text.find('n') else {
        return Some(Nth {
            a: 0,
            b: text.parse().ok()?,
        });
    };
    let a = match &text[..n] {
        "" | "+" => 1,
        "-" => -1,
        number => number.parse().ok()?,
    };
    let b = match &text[n + 1..] {
        "" => 0,
        rest => rest.strip_prefix('+').unwrap_or(rest).parse().ok()?,
    };
    Some(Nth { a, b })
}

/// States of generated UI parts have no DOM ancestry or structural state.
fn matches_part(class: &PseudoClass, flags: u32) -> bool {
    let list_matches = |list: &SelectorList| {
        list.0.iter().any(|selector| {
            selector.pseudo_element.is_none()
            && selector.compounds.len() == 1
            && selector.compounds[0].tag.is_none()
            && selector.compounds[0].simple.iter().all(|simple| {
                matches!(simple, SimpleSelector::PseudoClass(class) if matches_part(class, flags))
            })
        })
    };
    match class {
        PseudoClass::Hover => flags & state::HOVER != 0,
        PseudoClass::Active => flags & state::ACTIVE != 0,
        PseudoClass::Vertical => flags & state::VERTICAL != 0,
        PseudoClass::Horizontal => flags & state::HORIZONTAL != 0,
        PseudoClass::Not(list) => !list_matches(list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => list_matches(list),
        _ => false,
    }
}

fn compound_specificity(compound: &Compound) -> Specificity {
    let mut specificity = Specificity(0, 0, u32::from(compound.tag.is_some()));
    for simple in &compound.simple {
        specificity = specificity.add(match simple {
            SimpleSelector::Id(_) => Specificity(1, 0, 0),
            SimpleSelector::PseudoClass(PseudoClass::Where(_))
            | SimpleSelector::PartState(PseudoClass::Where(_)) => Specificity::default(),
            SimpleSelector::PseudoClass(PseudoClass::Not(list) | PseudoClass::Is(list))
            | SimpleSelector::PartState(PseudoClass::Not(list) | PseudoClass::Is(list)) => {
                list.max_specificity()
            }
            SimpleSelector::PseudoClass(PseudoClass::Has(relative))
            | SimpleSelector::PartState(PseudoClass::Has(relative)) => {
                relative_list(relative).max_specificity()
            }
            SimpleSelector::PseudoClass(
                PseudoClass::NthChildOf(_, list) | PseudoClass::NthLastChildOf(_, list),
            )
            | SimpleSelector::PartState(
                PseudoClass::NthChildOf(_, list) | PseudoClass::NthLastChildOf(_, list),
            ) => Specificity(0, 1, 0).add(list.max_specificity()),
            SimpleSelector::PseudoClass(
                PseudoClass::Host(Some(list)) | PseudoClass::HostContext(list),
            ) => Specificity(0, 1, 0).add(list.max_specificity()),
            _ => Specificity(0, 1, 0),
        });
    }
    specificity
}

#[cfg(test)]
mod tests;
