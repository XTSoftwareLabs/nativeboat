//! The cascade: matching rules, ordering declarations and resolving `var()`.

use crate::prelude::*;

use crate::computed::{ComputedStyle, PropertyId, Specified, SpecifiedValues, StyleEnvironment};
use crate::container::{ContainerQuery, QueryContainer};
use crate::media::MediaEnvironment;
use crate::parser::{Declaration, Rule, StyleRule, Stylesheet};
use crate::selector::{Element, PseudoElement, Relations, Selector, SimpleSelector, Specificity};
use crate::shorthand::{alias, expand, longhands_of, unprefixed};
use crate::tokenizer::{Token, serialize_tokens, tokens};
use crate::values::{ComponentValue, parse_component_values, without_whitespace};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::RefCell;

/// Where a stylesheet comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The built-in user agent stylesheet.
    UserAgent,
    /// The application's stylesheets.
    Author,
}

/// A stylesheet taking part in the cascade.
#[derive(Debug, Clone, Copy)]
pub struct SheetRef<'a> {
    /// The stylesheet.
    pub sheet: &'a Stylesheet,
    /// Its origin.
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    rank: u8,
    /// Whether the declaration wins by shadow-tree context: an element's
    /// own tree's normal declarations win over the `:host` and
    /// `::slotted()` rules of the shadow tree inside it, whose important
    /// declarations win the other way round.
    context: bool,
    inline: bool,
    /// The cascade layer's precedence: later layers win, and rules outside
    /// any layer win over layered ones, the other way round for important
    /// declarations.
    layer: u32,
    specificity: Specificity,
    order: usize,
}

fn rank(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::Author, false) => 1,
        (Origin::Author, true) => 2,
        (Origin::UserAgent, true) => 3,
    }
}

/// A style rule that applies in the media environment.
#[derive(Debug)]
struct IndexedRule<'a> {
    rule: &'a StyleRule,
    origin: Origin,
    /// The tree its sheet applies in, as [`Stylesheet::scope`] names it.
    scope: u32,
    /// The precedence of the rule's cascade layer among the sheets' layers;
    /// rules outside any layer have the highest.
    layer: u32,
    /// The position of its first declaration among the declarations of all
    /// such rules, in cascade order.
    first: usize,
    /// The queries of the `@container` rules it is in, which an element's
    /// query containers must all satisfy.
    containers: Vec<&'a ContainerQuery>,
}

/// Selectors by what the compound of their subject requires: an id, a
/// class, a type, or nothing an element can be looked up by.
#[derive(Debug, Default)]
struct Buckets<'a> {
    ids: HashMap<&'a str, Vec<usize>>,
    classes: HashMap<&'a str, Vec<usize>>,
    tags: HashMap<&'a str, Vec<usize>>,
    any: Vec<usize>,
}

impl<'a> Buckets<'a> {
    /// Whether any selector is indexed.
    fn is_some(&self) -> bool {
        !self.any.is_empty()
            || !self.ids.is_empty()
            || !self.classes.is_empty()
            || !self.tags.is_empty()
    }

    fn insert(&mut self, selector: &'a Selector, position: usize) {
        let subject = &selector.compounds[selector.compounds.len() - 1];
        let id = subject.simple.iter().find_map(|simple| match simple {
            SimpleSelector::Id(id) => Some(id.as_str()),
            _ => None,
        });
        let class = subject.simple.iter().find_map(|simple| match simple {
            SimpleSelector::Class(class) => Some(class.as_str()),
            _ => None,
        });
        let list = match (id, class, subject.tag.as_deref()) {
            (Some(id), _, _) => self.ids.entry(id).or_default(),
            (None, Some(class), _) => self.classes.entry(class).or_default(),
            (None, None, Some(tag)) => self.tags.entry(tag).or_default(),
            (None, None, None) => &mut self.any,
        };
        list.push(position);
    }

    /// The selectors an element may match, in cascade order.
    fn candidates<E: Element>(&self, element: &E) -> Vec<usize> {
        let mut found = self.any.clone();
        let mut add = |list: Option<&Vec<usize>>| found.extend(list.into_iter().flatten());
        // Type selectors ignore case, as SVG's `clipPath` shows; most names
        // are lowercase already.
        let name = element.local_name();
        let lowercase;
        let name = if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            lowercase = name.to_ascii_lowercase();
            lowercase.as_str()
        } else {
            name
        };
        add(self.tags.get(name));
        add(element.id().and_then(|id| self.ids.get(id)));
        for class in element
            .attribute("class")
            .unwrap_or_default()
            .split_ascii_whitespace()
        {
            add(self.classes.get(class));
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// What a declaration of a style sheet sets, worked out once.
#[derive(Debug, Clone)]
enum Parsed {
    /// The value uses `var()`, which depends on the element.
    Variable,
    /// The longhand values it sets on every element.
    Fixed(Rc<Vec<(PropertyId, Specified)>>),
}

/// The style rules of a cascade that apply in a media environment, with
/// their selectors indexed so that an element is tested only against the
/// selectors it may match, and the values of their declarations parsed
/// once.
#[derive(Debug)]
pub struct RuleMap<'a> {
    rules: Vec<IndexedRule<'a>>,
    /// Every selector with its rule, in cascade order.
    selectors: Vec<(usize, &'a Selector)>,
    elements: Buckets<'a>,
    pseudo_elements: Buckets<'a>,
    /// Selectors with `:has()`, which depend on an element's descendants;
    /// apart, so that the others can be matched once per selector context.
    relational_elements: Buckets<'a>,
    relational_pseudo_elements: Buckets<'a>,
    /// Selectors of rules in `@container` rules, which depend on the sizes
    /// of elements' query containers.
    conditional_elements: Buckets<'a>,
    conditional_pseudo_elements: Buckets<'a>,
    /// Declarations by position, as far as they were parsed.
    parsed: RefCell<Vec<Option<Parsed>>>,
    /// What the selectors test of elements' siblings and ancestors.
    relations: Relations,
    /// Custom properties that `@property` registers: whether they inherit
    /// and their initial values. Later registrations win.
    registered: BTreeMap<String, (bool, Option<String>)>,
    /// The highest layer precedence, which rules outside any layer have.
    unlayered: u32,
    /// The selectors with `::slotted()`, which match the elements assigned
    /// to the slots their compounds match.
    slotted: Vec<usize>,
}

/// Cascade layers in the order they are first declared: each layer's place
/// among its siblings, by its full dotted name.
#[derive(Default)]
struct Layers {
    places: BTreeMap<String, Vec<u32>>,
    /// The number of sublayers declared in each layer, the outermost by "".
    children: BTreeMap<String, u32>,
    anonymous: u32,
}

impl Layers {
    /// Declares a layer inside `parent`, by a possibly dotted name, and
    /// returns its full name. An empty name declares a new anonymous layer.
    fn declare(&mut self, parent: &str, name: &str) -> String {
        let anonymous;
        let parts: Vec<&str> = if name.is_empty() {
            self.anonymous += 1;
            anonymous = format!("\u{0}{}", self.anonymous);
            vec![anonymous.as_str()]
        } else {
            name.split('.').map(str::trim).collect()
        };
        let mut full = parent.to_owned();
        for part in parts {
            let outer = full.clone();
            full = if outer.is_empty() {
                part.to_owned()
            } else {
                format!("{outer}.{part}")
            };
            if !self.places.contains_key(&full) {
                let mut place = self.places.get(&outer).cloned().unwrap_or_default();
                let next = self.children.entry(outer).or_default();
                place.push(*next);
                *next += 1;
                self.places.insert(full.clone(), place);
            }
        }
        full
    }

    /// The sort key of rules directly in a layer: its place, then above its
    /// sublayers.
    fn key(&self, layer: &str) -> Vec<u32> {
        let mut key = self.places.get(layer).cloned().unwrap_or_default();
        key.push(u32::MAX);
        key
    }
}

impl<'a> RuleMap<'a> {
    /// Indexes the rules of the sheets that apply in the media environment.
    pub fn new(sheets: &[SheetRef<'a>], media: &MediaEnvironment) -> RuleMap<'a> {
        let mut map = RuleMap {
            rules: Vec::new(),
            selectors: Vec::new(),
            elements: Buckets::default(),
            pseudo_elements: Buckets::default(),
            relational_elements: Buckets::default(),
            relational_pseudo_elements: Buckets::default(),
            conditional_elements: Buckets::default(),
            conditional_pseudo_elements: Buckets::default(),
            parsed: RefCell::default(),
            relations: Relations::default(),
            registered: BTreeMap::new(),
            unlayered: 0,
            slotted: Vec::new(),
        };
        let mut declarations = 0;
        let mut layers = Layers::default();
        let mut keys = Vec::new();
        for sheet in sheets {
            map.add(
                &sheet.sheet.rules,
                (sheet.origin, sheet.sheet.scope, &[]),
                media,
                &mut declarations,
                (&mut layers, ""),
                &mut keys,
            );
        }
        // Layers rank by their keys; a sheet without layers keeps one rank.
        let mut ranks: Vec<&Vec<u32>> = keys.iter().collect();
        ranks.sort();
        ranks.dedup();
        for (rule, key) in map.rules.iter_mut().zip(&keys) {
            rule.layer = ranks.binary_search(&key).unwrap_or_default() as u32;
        }
        map.unlayered = ranks
            .binary_search(&&vec![u32::MAX])
            .map_or(ranks.len(), |rank| rank) as u32;
        map.parsed = RefCell::new(vec![None; declarations]);
        map
    }

    fn add(
        &mut self,
        rules: &'a [Rule],
        (origin, scope, containers): (Origin, u32, &[&'a ContainerQuery]),
        media: &MediaEnvironment,
        declarations: &mut usize,
        (layers, layer): (&mut Layers, &str),
        keys: &mut Vec<Vec<u32>>,
    ) {
        for rule in rules {
            match rule {
                Rule::Style(style) => {
                    let index = self.rules.len();
                    self.rules.push(IndexedRule {
                        rule: style,
                        origin,
                        scope,
                        layer: 0,
                        first: *declarations,
                        containers: containers.to_vec(),
                    });
                    keys.push(layers.key(layer));
                    *declarations += style.declarations.len();
                    for selector in &style.selectors.0 {
                        let position = self.selectors.len();
                        self.selectors.push((index, selector));
                        self.relations.add(selector);
                        if selector.pseudo_element == Some(PseudoElement::Slotted) {
                            self.slotted.push(position);
                            continue;
                        }
                        let pseudo = selector.pseudo_element.is_some();
                        let buckets = match (pseudo, selector.is_relational()) {
                            _ if !containers.is_empty() && pseudo => {
                                &mut self.conditional_pseudo_elements
                            }
                            _ if !containers.is_empty() => &mut self.conditional_elements,
                            (true, false) => &mut self.pseudo_elements,
                            (false, false) => &mut self.elements,
                            (true, true) => &mut self.relational_pseudo_elements,
                            (false, true) => &mut self.relational_elements,
                        };
                        buckets.insert(selector, position);
                    }
                }
                Rule::Media { query, rules } if query.matches(media) => self.add(
                    rules,
                    (origin, scope, containers),
                    media,
                    declarations,
                    (layers, layer),
                    keys,
                ),
                Rule::Supports { condition, rules } if supports(condition) => self.add(
                    rules,
                    (origin, scope, containers),
                    media,
                    declarations,
                    (layers, layer),
                    keys,
                ),
                Rule::Container { query, rules } => {
                    let mut inner = containers.to_vec();
                    inner.push(query);
                    self.add(
                        rules,
                        (origin, scope, &inner),
                        media,
                        declarations,
                        (layers, layer),
                        keys,
                    );
                }
                Rule::Layer { name, rules } => {
                    let inner = layers.declare(layer, name);
                    self.add(
                        rules,
                        (origin, scope, containers),
                        media,
                        declarations,
                        (layers, &inner),
                        keys,
                    );
                }
                Rule::LayerOrder(names) => {
                    for name in names {
                        layers.declare(layer, name);
                    }
                }
                Rule::Property {
                    name,
                    inherits,
                    initial,
                    ..
                } => {
                    self.registered
                        .insert(name.clone(), (*inherits, initial.clone()));
                }
                _ => {}
            }
        }
    }

    /// Whether a rule's selector looks at descendants with `:has()`.
    pub fn is_relational(&self) -> bool {
        self.relational_elements.is_some() || self.relational_pseudo_elements.is_some()
    }

    /// Whether any rule is in an `@container` rule.
    pub fn has_container_queries(&self) -> bool {
        self.conditional_elements.is_some() || self.conditional_pseudo_elements.is_some()
    }

    /// The rules in `@container` rules that match, as
    /// [`RuleMap::matching`]: `container` gives the size of the element's
    /// nearest query container with a name, or with any name for `None`,
    /// if it has one.
    pub fn matching_conditional<E: Element>(
        &self,
        element: &E,
        pseudo: Option<(&PseudoElement, u32)>,
        container: &dyn Fn(Option<&str>) -> Option<QueryContainer>,
    ) -> Vec<(usize, Specificity)> {
        let buckets = match pseudo {
            Some(_) => &self.conditional_pseudo_elements,
            None => &self.conditional_elements,
        };
        if !buckets.is_some() {
            return Vec::new();
        }
        let mut matched = self.matching_in(buckets, element, pseudo);
        matched.retain(|(rule, _)| {
            self.rules[*rule].containers.iter().all(|query| {
                container(query.name.as_deref()).is_some_and(|size| query.matches(&size))
            })
        });
        matched
    }

    /// What the rules' selectors test of elements' siblings and ancestors.
    pub fn relations(&self) -> &Relations {
        &self.relations
    }

    /// Whether any rule targets a pseudo-element.
    pub fn has_pseudo_elements(&self) -> bool {
        !self.pseudo_elements.any.is_empty()
            || !self.pseudo_elements.ids.is_empty()
            || !self.pseudo_elements.classes.is_empty()
            || !self.pseudo_elements.tags.is_empty()
            || self.relational_pseudo_elements.is_some()
    }

    /// The rules matching an element, or one of its pseudo-elements, with
    /// the highest specificity of their matching selectors, in cascade
    /// order.
    /// Rule positions are comparable only within the same ordered, unchanged
    /// sheets and media environment. Matching still observes current DOM state.
    pub fn matching<E: Element>(
        &self,
        element: &E,
        pseudo: Option<(&PseudoElement, u32)>,
    ) -> Vec<(usize, Specificity)> {
        merge_matches(
            &self.matching_local(element, pseudo),
            &self.matching_relational(element, pseudo),
        )
    }

    /// The rules without `:has()` that match, as [`RuleMap::matching`]: they
    /// depend only on the element, its siblings and its ancestors.
    pub fn matching_local<E: Element>(
        &self,
        element: &E,
        pseudo: Option<(&PseudoElement, u32)>,
    ) -> Vec<(usize, Specificity)> {
        let buckets = match pseudo {
            Some(_) => &self.pseudo_elements,
            None => &self.elements,
        };
        let mut matched = self.matching_in(buckets, element, pseudo);
        // A slotted element also matches the `::slotted()` rules of its
        // slot's shadow tree.
        if pseudo.is_none()
            && !self.slotted.is_empty()
            && let Some(slot) = element.assigned_slot()
        {
            let scope = slot.tree_scope();
            let before = matched.len();
            for &position in &self.slotted {
                let (rule, selector) = self.selectors[position];
                if self.rules[rule].scope == scope && selector.matches_slotted(&slot, element) {
                    matched.push((rule, selector.specificity));
                }
            }
            if matched.len() > before {
                matched.sort_by_key(|(rule, _)| *rule);
                matched.dedup_by(|later, earlier| {
                    let same = later.0 == earlier.0;
                    if same {
                        earlier.1 = earlier.1.max(later.1);
                    }
                    same
                });
            }
        }
        matched
    }

    /// The rules with `:has()` that match, as [`RuleMap::matching`].
    pub fn matching_relational<E: Element>(
        &self,
        element: &E,
        pseudo: Option<(&PseudoElement, u32)>,
    ) -> Vec<(usize, Specificity)> {
        let buckets = match pseudo {
            Some(_) => &self.relational_pseudo_elements,
            None => &self.relational_elements,
        };
        if !buckets.is_some() {
            return Vec::new();
        }
        self.matching_in(buckets, element, pseudo)
    }

    fn matching_in<E: Element>(
        &self,
        buckets: &Buckets<'a>,
        element: &E,
        pseudo: Option<(&PseudoElement, u32)>,
    ) -> Vec<(usize, Specificity)> {
        let mut matched: Vec<(usize, Specificity)> = Vec::new();
        // An element matches the rules of its own tree, and a host the
        // `:host` rules of the shadow tree it hosts; the user agent's rules
        // apply in every tree.
        let tree = element.tree_scope();
        let shadow = element.shadow_scope();
        let host = shadow.and_then(|_| element.as_featureless_host());
        for position in buckets.candidates(element) {
            let (rule, selector) = self.selectors[position];
            let indexed = &self.rules[rule];
            let subject = if indexed.origin == Origin::UserAgent || indexed.scope == tree {
                Some(element)
            } else if shadow == Some(indexed.scope) {
                host.as_ref()
            } else {
                None
            };
            let Some(subject) = subject else {
                continue;
            };
            let matches = match pseudo {
                Some((pseudo, flags)) => selector.matches_pseudo_state(subject, pseudo, flags),
                None => selector.matches(subject),
            };
            if !matches {
                continue;
            }
            // A rule's selectors are next to each other.
            match matched.last_mut() {
                Some((last, specificity)) if *last == rule => {
                    *specificity = (*specificity).max(selector.specificity);
                }
                _ => matched.push((rule, selector.specificity)),
            }
        }
        matched
    }

    /// The longhand values a declaration of a style sheet sets, unless its
    /// value depends on custom properties.
    fn fixed(
        &self,
        position: usize,
        declaration: &Declaration,
    ) -> Option<Rc<Vec<(PropertyId, Specified)>>> {
        let mut parsed = self.parsed.borrow_mut();
        let entry = parsed[position].get_or_insert_with(|| {
            if uses_variables(&declaration.value) {
                return Parsed::Variable;
            }
            let name = unprefixed(&declaration.name);
            let values = property_ids(name)
                .and_then(|ids| declared_values(name, &ids, &declaration.value))
                .unwrap_or_default();
            Parsed::Fixed(Rc::new(values))
        });
        match entry {
            Parsed::Fixed(values) => Some(values.clone()),
            Parsed::Variable => None,
        }
    }

    /// Computes an element's style from the rules, its presentational
    /// hints and its inline style.
    pub fn compute_style<E: Element>(
        &self,
        element: &E,
        hints: &[Declaration],
        inline: &[Declaration],
        parent: Option<&ComputedStyle>,
        environment: &StyleEnvironment,
    ) -> ComputedStyle {
        let inherited = parent
            .map(|parent| parent.custom.clone())
            .unwrap_or_default();
        let specified = cascade(element, self, hints, inline, None, &inherited);
        ComputedStyle::compute(&specified, parent, environment)
    }

    /// [`RuleMap::compute_style`] with the rules the element matches, as
    /// [`RuleMap::matching`] gives them, already known.
    pub fn compute_matched_style<E: Element>(
        &self,
        element: &E,
        matched: &[(usize, Specificity)],
        hints: &[Declaration],
        inline: &[Declaration],
        parent: Option<&ComputedStyle>,
        environment: &StyleEnvironment,
    ) -> ComputedStyle {
        let inherited = parent
            .map(|parent| parent.custom.clone())
            .unwrap_or_default();
        let specified = cascade_with(
            element,
            self,
            Some(matched),
            hints,
            inline,
            None,
            &inherited,
        );
        ComputedStyle::compute(&specified, parent, environment)
    }

    /// Computes the style of a pseudo-element, or `None` when no rule
    /// targets it.
    ///
    /// `::before` and `::after` also need `content` other than `normal` or
    /// `none`.
    pub fn compute_pseudo_style<E: Element>(
        &self,
        element: &E,
        pseudo: &PseudoElement,
        element_style: &ComputedStyle,
        environment: &StyleEnvironment,
    ) -> Option<ComputedStyle> {
        self.compute_pseudo_style_state(element, pseudo, 0, element_style, environment)
    }

    /// [`RuleMap::compute_pseudo_style`] with the rules the pseudo-element
    /// matches, as [`RuleMap::matching`] and
    /// [`RuleMap::matching_conditional`] give them, already known.
    pub fn compute_pseudo_style_matched<E: Element>(
        &self,
        element: &E,
        pseudo: &PseudoElement,
        matched: &[(usize, Specificity)],
        element_style: &ComputedStyle,
        environment: &StyleEnvironment,
    ) -> Option<ComputedStyle> {
        let specified = cascade_state(
            element,
            self,
            Some(matched),
            &[],
            &[],
            Some((pseudo, 0)),
            &element_style.custom,
        );
        if specified.values.is_empty() {
            return None;
        }
        let style = ComputedStyle::compute(&specified, Some(element_style), environment);
        let generated = matches!(pseudo, PseudoElement::Before | PseudoElement::After);
        if generated && !matches!(style.content, crate::types::Content::Items(_)) {
            return None;
        }
        Some(style)
    }

    /// Computes a generated part's style with its own interaction and axis state.
    pub fn compute_pseudo_style_state<E: Element>(
        &self,
        element: &E,
        pseudo: &PseudoElement,
        flags: u32,
        element_style: &ComputedStyle,
        environment: &StyleEnvironment,
    ) -> Option<ComputedStyle> {
        let specified = cascade_state(
            element,
            self,
            None,
            &[],
            &[],
            Some((pseudo, flags)),
            &element_style.custom,
        );
        if specified.values.is_empty() {
            return None;
        }
        let style = ComputedStyle::compute(&specified, Some(element_style), environment);
        let generated = matches!(pseudo, PseudoElement::Before | PseudoElement::After);
        if generated && !matches!(style.content, crate::types::Content::Items(_)) {
            return None;
        }
        Some(style)
    }
}

/// Whether a value refers to custom properties.
fn uses_variables(value: &str) -> bool {
    value.to_ascii_lowercase().contains("var(")
}

/// The properties a declaration sets: its own, its longhands, or every
/// property for `all`.
fn property_ids(name: &str) -> Option<Vec<PropertyId>> {
    match PropertyId::from_name(name).or_else(|| alias(name)) {
        Some(id) => Some(vec![id]),
        None if name == "all" => Some(PropertyId::ALL.to_vec()),
        None => longhands_of(name),
    }
}

/// The longhand values a declaration's value sets, or `None` when the value
/// is invalid for the property.
fn declared_values(
    name: &str,
    ids: &[PropertyId],
    text: &str,
) -> Option<Vec<(PropertyId, Specified)>> {
    if let Some(keyword) = css_wide_keyword(text) {
        return Some(ids.iter().map(|id| (*id, keyword.clone())).collect());
    }
    let parsed = parse_component_values(text);
    let longhands = if PropertyId::from_name(name).is_some() || alias(name).is_some() {
        Some(vec![(ids[0], parsed)])
    } else if name == "all" {
        None
    } else {
        expand(name, &parsed)
    };
    longhands
        .filter(|longhands| {
            longhands
                .iter()
                .all(|(id, value)| ComputedStyle::is_valid(*id, value))
        })
        .map(|longhands| {
            longhands
                .into_iter()
                .map(|(id, value)| (id, Specified::Value(value)))
                .collect()
        })
}

/// The style an `@keyframes` keyframe gives an element: its declarations
/// over the element's computed style, with `var()` taken from that style
/// and lengths relative to it. As in keyframes, `!important` declarations,
/// custom properties and animation properties are ignored.
pub fn keyframe_style(
    style: &ComputedStyle,
    declarations: &[Declaration],
    environment: &StyleEnvironment,
) -> ComputedStyle {
    let mut values = BTreeMap::new();
    for declaration in declarations {
        if declaration.important || declaration.name.starts_with("--") {
            continue;
        }
        let name = unprefixed(&declaration.name);
        if name.starts_with("animation") {
            continue;
        }
        let Some(ids) = property_ids(name) else {
            continue;
        };
        let text = if uses_variables(&declaration.value) {
            match substitute(&declaration.value, &style.custom) {
                Some(text) => text,
                None => continue,
            }
        } else {
            declaration.value.clone()
        };
        if let Some(longhands) = declared_values(name, &ids, &text) {
            values.extend(longhands);
        }
    }
    style.with_specified(&values, environment)
}

/// A declaration that applies to an element: whether an author wrote it,
/// and its position in the rule map when it comes from a style sheet.
type Applied<'a> = (&'a Declaration, bool, Option<usize>);

/// Collects the declarations that apply to an element, lowest priority first.
///
/// `hints` are presentational hints such as SVG presentation attributes:
/// author declarations that every author rule overrides.
pub fn matching_declarations<'a, E: Element>(
    element: &E,
    sheets: &[SheetRef<'a>],
    hints: &'a [Declaration],
    inline: &'a [Declaration],
    pseudo: Option<&PseudoElement>,
    media: &MediaEnvironment,
) -> Vec<&'a Declaration> {
    let map = RuleMap::new(sheets, media);
    authored_declarations(element, &map, None, hints, inline, pseudo.zip(Some(0)))
        .into_iter()
        .map(|(declaration, _, _)| declaration)
        .collect()
}

/// Like [`matching_declarations`], with whether an author wrote each
/// declaration and where a style sheet's declarations are in the map.
fn authored_declarations<'a, E: Element>(
    element: &E,
    map: &RuleMap<'a>,
    matched: Option<&[(usize, Specificity)]>,
    hints: &'a [Declaration],
    inline: &'a [Declaration],
    pseudo: Option<(&PseudoElement, u32)>,
) -> Vec<Applied<'a>> {
    let mut found: Vec<(Priority, Applied<'a>)> = Vec::new();
    let tree = element.tree_scope();
    for (order, declaration) in hints.iter().enumerate() {
        // Presentational hints precede every author layer.
        let priority = Priority {
            rank: rank(Origin::Author, false),
            context: true,
            inline: false,
            layer: 0,
            specificity: Specificity::default(),
            order,
        };
        found.push((priority, (declaration, false, None)));
    }
    let base = hints.len();
    let matched = matched.map_or_else(|| map.matching(element, pseudo), <[_]>::to_vec);
    for (index, specificity) in matched {
        let rule = &map.rules[index];
        let declarations: &'a [Declaration] = &rule.rule.declarations;
        for (offset, declaration) in declarations.iter().enumerate() {
            let position = rule.first + offset;
            let layer = if declaration.important {
                map.unlayered - rule.layer
            } else {
                rule.layer
            };
            // Rules of a shadow tree that reach its host or the elements
            // assigned to its slots come from an inner context.
            let inner = rule.origin == Origin::Author && rule.scope != tree;
            let priority = Priority {
                rank: rank(rule.origin, declaration.important),
                context: inner == declaration.important,
                inline: false,
                layer,
                specificity,
                order: base + position,
            };
            let author = rule.origin == Origin::Author;
            found.push((priority, (declaration, author, Some(position))));
        }
    }
    if pseudo.is_none() {
        let base = base + map.parsed.borrow().len();
        for (order, declaration) in inline.iter().enumerate() {
            let priority = Priority {
                rank: rank(Origin::Author, declaration.important),
                context: !declaration.important,
                inline: true,
                layer: map.unlayered,
                specificity: Specificity::default(),
                order: base + order,
            };
            found.push((priority, (declaration, true, None)));
        }
    }
    found.sort_by_key(|(priority, _)| *priority);
    found.into_iter().map(|(_, applied)| applied).collect()
}

/// Whether an author declaration takes the native look away from form
/// controls, as browsers do when authors style a control's background or
/// border, or set `appearance: none`.
fn disables_appearance(declaration: &Declaration) -> bool {
    // Chromium knows `-webkit-appearance`, not `-moz-appearance`.
    let name = declaration
        .name
        .strip_prefix("-webkit-")
        .unwrap_or(&declaration.name);
    match name {
        "appearance" => declaration.value.trim().eq_ignore_ascii_case("none"),
        "background" | "background-color" | "background-image" => true,
        "border-collapse" | "border-spacing" => false,
        _ => name.starts_with("border"),
    }
}

fn css_wide_keyword(value: &str) -> Option<Specified> {
    match value.trim().to_ascii_lowercase().as_str() {
        "inherit" => Some(Specified::Inherit),
        "initial" => Some(Specified::Initial),
        "unset" | "revert" | "revert-layer" => Some(Specified::Unset),
        _ => None,
    }
}

/// Substitutes `var()` references; `None` means a reference could not be resolved.
pub fn substitute(value: &str, custom: &BTreeMap<String, String>) -> Option<String> {
    substitute_depth(value, custom, 0)
}

fn substitute_depth(
    value: &str,
    custom: &BTreeMap<String, String>,
    depth: usize,
) -> Option<String> {
    if depth > 32 {
        return None;
    }
    let list = tokens(value);
    let mut output = String::new();
    let mut index = 0;
    while index < list.len() {
        let token = &list[index];
        index += 1;
        let Token::Function(name) = token else {
            output.push_str(&token.to_css());
            continue;
        };
        if !name.eq_ignore_ascii_case("var") {
            output.push_str(&token.to_css());
            continue;
        }
        let start = index;
        let mut nesting = 0;
        while index < list.len() {
            match &list[index] {
                Token::Function(_) | Token::LeftParen => nesting += 1,
                Token::RightParen if nesting == 0 => break,
                Token::RightParen => nesting -= 1,
                _ => {}
            }
            index += 1;
        }
        let arguments = &list[start..index.min(list.len())];
        index += 1;
        let comma = arguments.iter().position(|token| *token == Token::Comma);
        let name_tokens = &arguments[..comma.unwrap_or(arguments.len())];
        let name = name_tokens.iter().find_map(|token| match token {
            Token::Ident(name) if name.starts_with("--") => Some(name.clone()),
            _ => None,
        })?;
        let replacement = match (custom.get(&name), comma) {
            (Some(value), _) => substitute_depth(value, custom, depth + 1)?,
            (None, Some(comma)) => substitute_depth(
                &serialize_tokens(&arguments[comma + 1..]),
                custom,
                depth + 1,
            )?,
            (None, None) => return None,
        };
        output.push_str(replacement.trim());
    }
    Some(output)
}

/// Evaluates an `@supports` condition.
pub fn supports(condition: &str) -> bool {
    let values = parse_component_values(condition);
    supports_condition(&without_whitespace(&values)).unwrap_or(false)
}

fn supports_condition(values: &[ComponentValue]) -> Option<bool> {
    match values {
        [not, group] if not.is_ident("not") => supports_group(group).map(|result| !result),
        [first, rest @ ..] => {
            let mut result = supports_group(first)?;
            for pair in rest.chunks(2) {
                let [operator, group] = pair else {
                    return None;
                };
                let value = supports_group(group)?;
                match operator.ident()?.as_str() {
                    "and" => result = result && value,
                    "or" => result = result || value,
                    _ => return None,
                }
            }
            Some(result)
        }
        [] => None,
    }
}

fn supports_group(value: &ComponentValue) -> Option<bool> {
    match value {
        ComponentValue::Parens(inner) => {
            let colon = inner
                .iter()
                .position(|value| matches!(value, ComponentValue::Token(Token::Colon)));
            match colon {
                Some(colon) => {
                    let name = without_whitespace(&inner[..colon]);
                    let [name] = name.as_slice() else {
                        return Some(false);
                    };
                    let Some(name) = name.ident() else {
                        return Some(false);
                    };
                    Some(declaration_supported(&name, &inner[colon + 1..]))
                }
                None => supports_condition(&without_whitespace(inner)),
            }
        }
        ComponentValue::Function(name, arguments) if name == "selector" => {
            Some(crate::selector::SelectorList::parse(&component_text(arguments)).is_ok())
        }
        _ => Some(false),
    }
}

fn component_text(values: &[ComponentValue]) -> String {
    let mut text = String::new();
    for value in values {
        match value {
            ComponentValue::Token(token) => text.push_str(&token.to_css()),
            ComponentValue::Function(name, arguments) => {
                text.push_str(name);
                text.push('(');
                text.push_str(&component_text(arguments));
                text.push(')');
            }
            ComponentValue::Parens(inner) => {
                text.push('(');
                text.push_str(&component_text(inner));
                text.push(')');
            }
            ComponentValue::Brackets(inner) => {
                text.push('[');
                text.push_str(&component_text(inner));
                text.push(']');
            }
        }
    }
    text
}

fn declaration_supported(name: &str, value: &[ComponentValue]) -> bool {
    if name.starts_with("--") {
        return true;
    }
    let name = unprefixed(name);
    let known = PropertyId::from_name(name).or_else(|| alias(name));
    // A value with `var()` is valid until substitution, so a property the
    // engine knows supports it, as in Chromium.
    if references_variables(value) {
        return known.is_some() || longhands_of(name).is_some();
    }
    if let Some(id) = known {
        return ComputedStyle::is_valid(id, value);
    }
    longhands_of(name).is_some() && expand(name, value).is_some()
}

/// Whether component values reference a custom property with `var()`.
fn references_variables(values: &[ComponentValue]) -> bool {
    values.iter().any(|value| match value {
        ComponentValue::Function(name, arguments) => {
            name.eq_ignore_ascii_case("var") || references_variables(arguments)
        }
        ComponentValue::Parens(inner) | ComponentValue::Brackets(inner) => {
            references_variables(inner)
        }
        ComponentValue::Token(_) => false,
    })
}

fn assign(values: &mut BTreeMap<PropertyId, Specified>, ids: &[PropertyId], value: &Specified) {
    for id in ids {
        values.insert(*id, value.clone());
    }
}

/// Resolves the cascaded value of every property for an element.
pub fn cascade<E: Element>(
    element: &E,
    map: &RuleMap<'_>,
    hints: &[Declaration],
    inline: &[Declaration],
    pseudo: Option<&PseudoElement>,
    inherited_custom: &Rc<BTreeMap<String, String>>,
) -> SpecifiedValues {
    cascade_with(element, map, None, hints, inline, pseudo, inherited_custom)
}

/// [`cascade`] with the rules the element matches, when already known.
fn cascade_with<E: Element>(
    element: &E,
    map: &RuleMap<'_>,
    matched: Option<&[(usize, Specificity)]>,
    hints: &[Declaration],
    inline: &[Declaration],
    pseudo: Option<&PseudoElement>,
    inherited_custom: &Rc<BTreeMap<String, String>>,
) -> SpecifiedValues {
    cascade_state(
        element,
        map,
        matched,
        hints,
        inline,
        pseudo.zip(Some(0)),
        inherited_custom,
    )
}

/// Two lists of matched rules in cascade order as one, each rule once with
/// its highest specificity.
pub fn merge_matches(
    left: &[(usize, Specificity)],
    right: &[(usize, Specificity)],
) -> Vec<(usize, Specificity)> {
    if right.is_empty() {
        return left.to_vec();
    }
    let mut merged: Vec<(usize, Specificity)> = left.iter().chain(right).copied().collect();
    merged.sort_by_key(|(rule, _)| *rule);
    merged.dedup_by(|(rule, specificity), (kept, highest)| {
        let same = rule == kept;
        if same {
            *highest = (*highest).max(*specificity);
        }
        same
    });
    merged
}

fn cascade_state<E: Element>(
    element: &E,
    map: &RuleMap<'_>,
    matched: Option<&[(usize, Specificity)]>,
    hints: &[Declaration],
    inline: &[Declaration],
    pseudo: Option<(&PseudoElement, u32)>,
    inherited_custom: &Rc<BTreeMap<String, String>>,
) -> SpecifiedValues {
    let declarations = authored_declarations(element, map, matched, hints, inline, pseudo);
    let author_appearance = declarations
        .iter()
        .any(|(declaration, author, _)| *author && disables_appearance(declaration));
    let appearance_none = declarations.iter().any(|(declaration, author, _)| {
        *author
            && matches!(
                declaration.name.as_str(),
                "appearance" | "-webkit-appearance"
            )
            && declaration.value.trim().eq_ignore_ascii_case("none")
    });
    let mut custom = inherited_custom.clone();
    // Registered properties that do not inherit start from their initial
    // value on every element, and the others from it where no ancestor set
    // them.
    for (name, (inherits, initial)) in &map.registered {
        let current = custom.get(name);
        let wanted = if *inherits {
            current.or(initial.as_ref())
        } else {
            initial.as_ref()
        };
        if current != wanted {
            let wanted = wanted.cloned();
            let map = Rc::make_mut(&mut custom);
            match wanted {
                Some(value) => map.insert(name.clone(), value),
                None => map.remove(name),
            };
        }
    }
    for (declaration, _, _) in &declarations {
        if !declaration.name.starts_with("--") {
            continue;
        }
        let registered = map.registered.get(&declaration.name);
        let initial = registered.and_then(|(_, initial)| initial.clone());
        let inherited = inherited_custom.get(&declaration.name).cloned();
        let value = match css_wide_keyword(&declaration.value) {
            Some(Specified::Inherit) => inherited,
            Some(Specified::Unset) if registered.is_none_or(|(inherits, _)| *inherits) => {
                inherited.or(initial)
            }
            Some(_) => initial,
            None => Some(declaration.value.clone()),
        };
        let map = Rc::make_mut(&mut custom);
        match value {
            Some(value) => map.insert(declaration.name.clone(), value),
            None => map.remove(&declaration.name),
        };
    }
    let mut values = BTreeMap::new();
    for (declaration, _, position) in &declarations {
        if declaration.name.starts_with("--") {
            continue;
        }
        // Style sheet values without `var()` are the same for every element.
        if let Some(fixed) = position.and_then(|position| map.fixed(position, declaration)) {
            for (id, value) in fixed.iter() {
                values.insert(*id, value.clone());
            }
            continue;
        }
        let name = unprefixed(&declaration.name);
        let Some(ids) = property_ids(name) else {
            continue;
        };
        let has_var = uses_variables(&declaration.value);
        let text = if has_var {
            match substitute(&declaration.value, &custom) {
                Some(text) => text,
                None => {
                    assign(&mut values, &ids, &Specified::Unset);
                    continue;
                }
            }
        } else {
            declaration.value.clone()
        };
        match declared_values(name, &ids, &text) {
            Some(longhands) => values.extend(longhands),
            None if has_var => assign(&mut values, &ids, &Specified::Unset),
            None => {}
        }
    }
    SpecifiedValues {
        values,
        custom,
        author_appearance,
        appearance_none,
    }
}

/// Computes an element's style from the style sheets, its presentational
/// hints and its inline style. To style many elements, compute them with one
/// [`RuleMap`].
pub fn compute_style<E: Element>(
    element: &E,
    sheets: &[SheetRef<'_>],
    hints: &[Declaration],
    inline: &[Declaration],
    parent: Option<&ComputedStyle>,
    environment: &StyleEnvironment,
) -> ComputedStyle {
    RuleMap::new(sheets, &environment.media).compute_style(
        element,
        hints,
        inline,
        parent,
        environment,
    )
}

/// Computes the style of a pseudo-element from the style sheets, as
/// [`RuleMap::compute_pseudo_style`] does.
pub fn compute_pseudo_style<E: Element>(
    element: &E,
    pseudo: &PseudoElement,
    sheets: &[SheetRef<'_>],
    element_style: &ComputedStyle,
    environment: &StyleEnvironment,
) -> Option<ComputedStyle> {
    RuleMap::new(sheets, &environment.media).compute_pseudo_style(
        element,
        pseudo,
        element_style,
        environment,
    )
}

#[cfg(test)]
mod tests;
