//! Computed styles for every element of a DOM.

use crate::prelude::*;

use nativeboat_css::cascade::merge_matches;
use nativeboat_css::parser::{Declaration, parse_declaration_list};
use nativeboat_css::selector::{RelatedFeatures, Specificity, structure};
use nativeboat_css::types::{ContainerType, Display, Overflow};
use nativeboat_css::{ComputedStyle, PseudoElement, RuleMap, SheetRef, StyleEnvironment};
use nativeboat_dom::{Dom, ElementRef, Namespace, NodeId, NodeKind};

mod scrollbar;

/// SVG attributes that set the CSS property of the same name.
const PRESENTATION_ATTRIBUTES: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-dashoffset",
    "marker-start",
    "marker-mid",
    "marker-end",
    "opacity",
    "color",
    "display",
    "visibility",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "letter-spacing",
    "text-decoration",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
];

/// The declarations an element's attributes imply, below author styles:
/// an SVG element's presentation attributes, the `width` and `height` of
/// an outer `<svg>` and of HTML elements that map them to their size, and
/// the presentational attributes of tables.
fn presentation_hints(dom: &Dom, node: NodeId) -> Vec<Declaration> {
    let svg = dom.namespace(node) == Some(Namespace::Svg);
    let name = dom.local_name(node).unwrap_or("");
    let sized = if svg {
        name == "svg"
    } else {
        matches!(name, "img" | "video" | "iframe" | "embed" | "object")
    };
    // Table parts map their sizes too, as HTML's presentational hints do:
    // zero means no size.
    let table_part = !svg && matches!(name, "table" | "col" | "colgroup" | "td" | "th" | "tr");
    let mut hints = if svg {
        svg_presentation_hints(dom, node)
    } else {
        table_hints(dom, node, name)
            .iter()
            .chain(&direction_hint(dom, node, name))
            .flat_map(|hint| parse_declaration_list(hint))
            .filter(|declaration| !declaration.important)
            .collect()
    };
    if sized || table_part {
        for property in ["width", "height"] {
            let mapped = !matches!(
                (name, property),
                ("col" | "colgroup", "height") | ("tr", "width")
            );
            let Some(value) = dom
                .get_attribute(node, property)
                .map(str::trim)
                .filter(|value| mapped && !(table_part && value.parse::<f32>() == Ok(0.0)))
            else {
                continue;
            };
            let length = if value.parse::<f32>().is_ok_and(|number| number >= 0.0) {
                format!("{value}px")
            } else if value.ends_with('%') || value.ends_with("px") {
                value.to_owned()
            } else {
                continue;
            };
            hints.extend(
                parse_declaration_list(&format!("{property}: {length}"))
                    .into_iter()
                    .find(|declaration| declaration.name == property),
            );
        }
    }
    hints
}

/// The direction HTML's `dir` attribute gives an element, which also
/// isolates its text from the text around it. `dir="auto"`, and a `<bdi>`
/// without a direction, take the direction of their first strong
/// character.
fn direction_hint(dom: &Dom, node: NodeId, name: &str) -> Option<String> {
    let dir = dom
        .get_attribute(node, "dir")
        .map(|value| value.trim().to_ascii_lowercase());
    let direction = match dir.as_deref() {
        Some(direction @ ("ltr" | "rtl")) => direction,
        Some("auto") => auto_direction(dom, node),
        _ if name == "bdi" => auto_direction(dom, node),
        _ => return None,
    };
    let isolation = if name == "bdo" {
        "isolate-override"
    } else {
        "isolate"
    };
    Some(format!("direction: {direction}; unicode-bidi: {isolation}"))
}

/// The direction of the first strong character in an element's text,
/// leaving out text whose direction is its own: in `<bdi>` elements and in
/// elements with a `dir` attribute.
fn auto_direction(dom: &Dom, node: NodeId) -> &'static str {
    fn first(dom: &Dom, node: NodeId) -> Option<bool> {
        dom.child_nodes(node)
            .find_map(|child| match dom.kind(child) {
                NodeKind::Text(text) => crate::bidi::first_strong(text),
                NodeKind::Element(_)
                    if dom.has_attribute(child, "dir")
                        || dom.local_name(child).is_some_and(|name| {
                            matches!(name, "bdi" | "script" | "style" | "textarea")
                        }) =>
                {
                    None
                }
                NodeKind::Element(_) => first(dom, child),
                _ => None,
            })
    }
    if first(dom, node) == Some(true) {
        "rtl"
    } else {
        "ltr"
    }
}

/// HTML's "rules for parsing non-negative integers": leading digits after
/// white space, ignoring what follows them.
fn non_negative_integer(value: &str) -> Option<u32> {
    let digits: String = value
        .trim_start_matches([' ', '\t', '\n', '\x0c', '\r'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// The `cellpadding` of a cell's table: its nearest table ancestor.
fn cell_padding(dom: &Dom, cell: NodeId) -> Option<&str> {
    let table = core::iter::successors(dom.parent(cell), |node| dom.parent(*node))
        .find(|node| dom.local_name(*node) == Some("table"))?;
    dom.get_attribute(table, "cellpadding")
}

/// The declarations HTML's rendering section gives table attributes:
/// `cellspacing` and `cellpadding` (on the table's own cells), `align`,
/// `valign`, `bgcolor` and `nowrap`.
fn table_hints(dom: &Dom, node: NodeId, name: &str) -> Vec<String> {
    let attribute = |name: &str| dom.get_attribute(node, name);
    let keyword = |name: &str| attribute(name).map(|value| value.trim().to_ascii_lowercase());
    let mut hints = Vec::new();
    if name == "table" {
        if let Some(spacing) = attribute("cellspacing").and_then(non_negative_integer) {
            hints.push(format!("border-spacing: {spacing}px"));
        }
        match keyword("align").as_deref() {
            Some("left") => hints.push("float: left".to_owned()),
            Some("right") => hints.push("float: right".to_owned()),
            Some("center") => hints.push("margin-left: auto; margin-right: auto".to_owned()),
            _ => {}
        }
    }
    if matches!(name, "td" | "th") {
        if let Some(padding) = cell_padding(dom, node).and_then(non_negative_integer) {
            hints.push(format!("padding: {padding}px"));
        }
        if attribute("nowrap").is_some() {
            hints.push("white-space: nowrap".to_owned());
        }
    }
    if matches!(name, "td" | "th" | "tr" | "thead" | "tbody" | "tfoot") {
        if let Some(align @ ("left" | "right" | "center" | "justify")) = keyword("align").as_deref()
        {
            hints.push(format!("text-align: {align}"));
        }
        if let Some(valign @ ("top" | "middle" | "bottom" | "baseline")) =
            keyword("valign").as_deref()
        {
            hints.push(format!("vertical-align: {valign}"));
        }
    }
    if matches!(
        name,
        "body" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
    ) && let Some(color) = attribute("bgcolor")
    {
        hints.push(format!("background-color: {}", color.trim()));
    }
    hints
}

/// The presentation attributes of an SVG element as declarations.
fn svg_presentation_hints(dom: &Dom, node: NodeId) -> Vec<Declaration> {
    PRESENTATION_ATTRIBUTES
        .iter()
        .filter_map(|name| {
            let value = dom.get_attribute(node, name)?;
            // Lengths may be bare numbers in attributes: user units, pixels.
            let value = match *name {
                "font-size" | "letter-spacing" if value.trim().parse::<f32>().is_ok() => {
                    format!("{}px", value.trim())
                }
                _ => value.to_owned(),
            };
            parse_declaration_list(&format!("{name}: {value}"))
                .into_iter()
                .find(|declaration| declaration.name == *name && !declaration.important)
        })
        .collect()
}

/// Computed styles of the elements of a DOM.
#[derive(Debug, Clone, Default)]
pub struct Styles {
    elements: HashMap<NodeId, Rc<ComputedStyle>>,
    pseudo: HashMap<(NodeId, u8), Rc<ComputedStyle>>,
    scrollbars: HashMap<NodeId, nativeboat_ir::Scrollbars>,
    /// The rules each scroll container's scrollbar parts match.
    scrollbar_rules: HashMap<NodeId, Vec<MatchedRules>>,
    environment: StyleEnvironment,
    retained: HashMap<NodeId, Rc<StyleKey>>,
    source_environment: StyleEnvironment,
    /// A hash of every element's selector inputs, so an unchanged
    /// document reuses all styles at once.
    selector_state: Option<u64>,
    /// Each styled element's selector context: its inputs and what
    /// selectors test of its siblings and ancestors.
    contexts: HashMap<NodeId, u64>,
    /// How the last computation went: elements styled, those whose
    /// matched rules were reused and those whose computed style was.
    stats: [u32; 3],
}

/// A 64-bit FNV-1a hash of selector inputs.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(self, bytes: &[u8]) -> Fnv {
        Fnv(bytes.iter().fold(self.0, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        }))
    }

    fn word(self, word: u64) -> Fnv {
        self.bytes(&word.to_le_bytes())
    }

    fn text(self, text: &str) -> Fnv {
        self.word(text.len() as u64).bytes(text.as_bytes())
    }
}

/// Every DOM input of an element the supported selector matcher consumes:
/// its name, namespace, attributes, dynamic state and emptiness. Text
/// contents matter through emptiness and form state.
fn element_state(dom: &Dom, node: NodeId) -> u64 {
    use nativeboat_css::selector::Element;
    let element = ElementRef::new(dom, node);
    let hash = dom.attribute_list(node).iter().fold(
        Fnv::new()
            .text(element.local_name())
            .word(namespace_code(dom, node)),
        |hash, (name, value)| hash.text(name).text(value),
    );
    // A cell's padding hint comes from its table's attribute.
    let hash = match element.local_name() {
        "td" | "th" => hash.text(cell_padding(dom, node).unwrap_or("")),
        _ => hash,
    };
    hash.word(u64::from(element.state()))
        .word(u64::from(element.is_empty()))
        .0
}

fn namespace_code(dom: &Dom, node: NodeId) -> u64 {
    match dom.namespace(node) {
        None => 0,
        Some(Namespace::Html) => 1,
        Some(Namespace::Svg) => 2,
        Some(Namespace::MathMl) => 3,
    }
}

/// The parts of an element's state that selectors test from the elements
/// related to it: a class, attribute or state that no compound on the left
/// of a combinator names changes no other element's style.
fn related_state(dom: &Dom, node: NodeId, features: &RelatedFeatures) -> u64 {
    use nativeboat_css::selector::Element;
    let element = ElementRef::new(dom, node);
    let mut hash = Fnv::new()
        .text(element.local_name())
        .word(namespace_code(dom, node));
    for (name, value) in dom.attribute_list(node).iter() {
        let tested = features
            .attributes
            .iter()
            .any(|attribute| attribute.eq_ignore_ascii_case(name))
            || (features.ids && name == "id");
        if tested {
            hash = hash.text(name).text(value);
        } else if name == "class" && !features.classes.is_empty() {
            for class in value
                .split_ascii_whitespace()
                .filter(|class| features.classes.contains(*class))
            {
                hash = hash.text(class);
            }
        }
    }
    if features.states != 0 {
        // Only the states compounds this element can match test.
        let class = dom.get_attribute(node, "class").unwrap_or("");
        let classes: Vec<&str> = class.split_ascii_whitespace().collect();
        let states = features.states_of(
            element.local_name(),
            dom.get_attribute(node, "id"),
            &classes,
            &|name| dom.has_attribute(node, name),
        );
        hash = hash.word(u64::from(element.state() & states));
    }
    if features.empty {
        hash = hash.word(u64::from(element.is_empty()));
    }
    hash.0
}

/// What selectors with the [`structure`] flags test of each child's place
/// among its siblings, as one word per child.
fn places(dom: &Dom, children: &[NodeId], flags: u16) -> Vec<u64> {
    let count = children.len();
    let types: Vec<(&str, u64)> = if flags & structure::OF_TYPE != 0 {
        children
            .iter()
            .map(|child| {
                (
                    dom.local_name(*child).unwrap_or_default(),
                    namespace_code(dom, *child),
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    children
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut hash = Fnv::new();
            let mut add = |flag: u16, value: usize| {
                if flags & flag != 0 {
                    hash = hash.word(value as u64);
                }
            };
            add(structure::FIRST, usize::from(index == 0));
            add(structure::LAST, usize::from(index + 1 == count));
            add(structure::INDEX, index);
            add(structure::INDEX_FROM_LAST, count - 1 - index);
            if let Some(kind) = types.get(index) {
                let before = types[..index].iter().filter(|other| *other == kind).count();
                let after = types[index + 1..]
                    .iter()
                    .filter(|other| *other == kind)
                    .count();
                add(structure::FIRST_OF_TYPE, usize::from(before == 0));
                add(structure::LAST_OF_TYPE, usize::from(after == 0));
                add(structure::INDEX_OF_TYPE, before);
                add(structure::INDEX_OF_TYPE_FROM_LAST, after);
            }
            hash.0
        })
        .collect()
}

/// The selector inputs of the whole document and its shadow trees, with
/// element order and parents, which sibling and relational selectors depend
/// on.
fn selector_state(dom: &Dom) -> u64 {
    let elements = if dom.has_shadow_roots() {
        let mut nodes = dom.shadow_including_descendants(Dom::DOCUMENT);
        nodes.retain(|node| dom.is_element(*node));
        nodes
    } else {
        dom.get_elements_by_tag_name(Dom::DOCUMENT, "*")
    };
    elements
        .into_iter()
        .fold(Fnv::new(), |hash, node| {
            hash.word(u64::from(node.0))
                .word(
                    dom.parent_or_host(node)
                        .map_or(u64::MAX, |parent| u64::from(parent.0)),
                )
                .word(element_state(dom, node))
        })
        .0
}

/// What selectors test of the place of each element a slot shows among
/// its own siblings, the host's children, which are not the slot's.
fn slotted_places(dom: &Dom, children: &[NodeId], flags: u16) -> Vec<u64> {
    children
        .iter()
        .map(|child| {
            let siblings = dom
                .parent(*child)
                .map(|parent| dom.child_elements(parent))
                .unwrap_or_default();
            let index = siblings.iter().position(|sibling| sibling == child);
            index.map_or(0, |index| places(dom, &siblings, flags)[index])
        })
        .collect()
}

/// Selector results are refreshed even when computed values can be retained.
type MatchedRules = Vec<(usize, Specificity)>;

/// What an element's style depends on in its selector context: the rules
/// without `:has()` it and its pseudo-elements match, and its own
/// declarations.
#[derive(Debug, PartialEq)]
struct Matched {
    rules: MatchedRules,
    pseudo: Vec<(u8, MatchedRules)>,
    inline: Vec<Declaration>,
    hints: Vec<Declaration>,
}

#[derive(Debug)]
struct StyleKey {
    parent: Option<NodeId>,
    matched: Rc<Matched>,
    /// The rules with `:has()` the element and then each of its
    /// pseudo-elements match, which descendants change.
    relational: Vec<MatchedRules>,
    environment: StyleEnvironment,
}

impl PartialEq for StyleKey {
    fn eq(&self, other: &StyleKey) -> bool {
        self.parent == other.parent
            && self.environment == other.environment
            && self.relational == other.relational
            && (Rc::ptr_eq(&self.matched, &other.matched) || self.matched == other.matched)
    }
}

/// The rules the scrollbar parts of an element match, in each state and
/// direction scrollbars are drawn in.
fn scrollbar_rules(element: &ElementRef<'_>, rules: &RuleMap<'_>) -> Vec<MatchedRules> {
    use nativeboat_css::selector::state::{ACTIVE, HORIZONTAL, HOVER, VERTICAL};
    let mut matched = Vec::new();
    for part in [
        PseudoElement::Scrollbar,
        PseudoElement::ScrollbarTrack,
        PseudoElement::ScrollbarThumb,
        PseudoElement::ScrollbarCorner,
    ] {
        for axis in [0, HORIZONTAL, VERTICAL] {
            for flags in [0, HOVER, HOVER | ACTIVE] {
                matched.push(rules.matching(element, Some((&part, axis | flags))));
            }
        }
    }
    matched
}

fn pseudo_key(pseudo: &PseudoElement) -> u8 {
    match pseudo {
        PseudoElement::Before => 0,
        PseudoElement::After => 1,
        PseudoElement::Marker => 3,
        _ => 2,
    }
}

/// Computes an element's own style.
fn element_style(
    dom: &Dom,
    rules: &RuleMap<'_>,
    node: NodeId,
    parent: Option<&ComputedStyle>,
    environment: &StyleEnvironment,
) -> ComputedStyle {
    let inline = dom.inline_style(node).declarations();
    let hints = presentation_hints(dom, node);
    rules.compute_style(
        &ElementRef::new(dom, node),
        &hints,
        &inline,
        parent,
        environment,
    )
}

/// The elements `<use>` elements show, with their ancestors: they are
/// styled even inside hidden elements.
fn shown_by_use(dom: &Dom) -> (HashSet<NodeId>, HashSet<NodeId>) {
    let targets: HashSet<NodeId> = dom
        .get_elements_by_tag_name(Dom::DOCUMENT, "use")
        .into_iter()
        .filter_map(|element| {
            let href = dom
                .get_attribute(element, "href")
                .or_else(|| dom.get_attribute(element, "xlink:href"))?;
            dom.get_element_by_id(Dom::DOCUMENT, href.strip_prefix('#')?)
        })
        .collect();
    let paths = targets
        .iter()
        .flat_map(|target| core::iter::successors(Some(*target), |node| dom.parent_element(*node)))
        .collect();
    (targets, paths)
}

impl Styles {
    /// Computes the style of every element that is rendered. Descendants of
    /// `display: none` elements are left out, except what `<use>` elements
    /// show; [`Styles::style_of`] computes their styles when asked.
    pub fn compute(dom: &Dom, sheets: &[SheetRef<'_>], environment: &StyleEnvironment) -> Styles {
        Self::compute_reusing(dom, sheets, environment, None)
    }

    /// Retains computed values whose selector matches, declarations, inherited
    /// style and environment are unchanged. The caller must supply previous
    /// styles only when the ordered stylesheet contents are unchanged.
    pub fn compute_reusing(
        dom: &Dom,
        sheets: &[SheetRef<'_>],
        environment: &StyleEnvironment,
        previous: Option<&Styles>,
    ) -> Styles {
        Self::compute_from(dom, sheets, None, environment, previous, None)
    }

    /// Whether these styles still hold for a document in an environment:
    /// nothing a selector or style reads has changed since, as when only
    /// text changed.
    pub fn reusable(&self, dom: &Dom, environment: &StyleEnvironment) -> bool {
        self.source_environment == *environment
            && dom.document_element().is_some()
            && self.selector_state == Some(selector_state(dom))
    }

    /// [`Styles::compute_reusing`] with the rules of the style sheets
    /// already indexed for the environment's media, as a caller that keeps
    /// the index between restyles has them.
    pub fn compute_with_rules(
        dom: &Dom,
        rules: &RuleMap<'_>,
        environment: &StyleEnvironment,
        previous: Option<&Styles>,
    ) -> Styles {
        Self::compute_from(dom, &[], Some(rules), environment, previous, None)
    }

    /// [`Styles::compute_with_rules`] for pages whose style sheets use
    /// container query units: `containers` gives the content box size each
    /// element last laid out at, which query containers pass to their
    /// descendants.
    pub fn compute_with_containers(
        dom: &Dom,
        rules: &RuleMap<'_>,
        environment: &StyleEnvironment,
        previous: Option<&Styles>,
        containers: &dyn Fn(NodeId) -> Option<(f32, f32)>,
    ) -> Styles {
        Self::compute_from(
            dom,
            &[],
            Some(rules),
            environment,
            previous,
            Some(containers),
        )
    }

    fn compute_from(
        dom: &Dom,
        sheets: &[SheetRef<'_>],
        indexed: Option<&RuleMap<'_>>,
        environment: &StyleEnvironment,
        previous: Option<&Styles>,
        containers: Option<&dyn Fn(NodeId) -> Option<(f32, f32)>>,
    ) -> Styles {
        let mut styles = Styles {
            environment: *environment,
            source_environment: *environment,
            ..Styles::default()
        };
        let Some(root) = dom.document_element() else {
            return styles;
        };
        let state = selector_state(dom);
        // Previous results hold for the same sheets, which the caller
        // checks, and the same environment.
        let previous = previous.filter(|previous| previous.source_environment == *environment);
        // Query containers that moved restyle their descendants.
        let sizes_hold = |previous: &Styles| {
            containers.is_none_or(|containers| {
                previous.elements.iter().all(|(node, style)| {
                    style.container_type == ContainerType::Normal
                        || containers(*node).is_none_or(|(width, height)| {
                            style.query_size.is_some_and(|(known_width, known_height)| {
                                known_width == width
                                    && (style.container_type != ContainerType::Size
                                        || known_height == height)
                            })
                        })
                })
            })
        };
        if let Some(previous) = previous
            .filter(|previous| previous.selector_state == Some(state) && sizes_hold(previous))
        {
            return previous.clone();
        }
        let containers = containers.unwrap_or(&|_| None);
        styles.selector_state = Some(state);
        let built;
        let rules = match indexed {
            Some(rules) => rules,
            None => {
                built = RuleMap::new(sheets, &environment.media);
                &built
            }
        };
        let with_pseudo = rules.has_pseudo_elements();
        let relations = rules.relations();
        // An element in the same selector context matches the same rules
        // without `:has()`: its context covers its own inputs and what
        // selectors test of its siblings, its ancestors and theirs, which
        // its parent's lineage holds. Rules with `:has()` are matched every
        // time.
        let relational = rules.is_relational();
        let (targets, paths) = shown_by_use(dom);
        let mut environment = *environment;
        // Elements with their parent's style, whether they are inside an
        // element a `<use>` shows, their context and their lineage.
        type Pending = (NodeId, Option<Rc<ComputedStyle>>, bool, u64, u64);
        let mut pending: Vec<Pending> = vec![(
            root,
            None,
            false,
            Fnv::new()
                .word(u64::from(root.0))
                .word(element_state(dom, root))
                .0,
            Fnv::new()
                .word(u64::from(root.0))
                .word(related_state(dom, root, &relations.ancestors))
                .0,
        )];
        while let Some((node, parent, shown, context, lineage)) = pending.pop() {
            let element = ElementRef::new(dom, node);
            let fields = matches!(dom.local_name(node), Some("input" | "textarea"));
            // List items and summaries show markers that `::marker` styles.
            let marked = matches!(dom.local_name(node), Some("li" | "summary"));
            let parts: Vec<_> = [
                Some(PseudoElement::Before),
                Some(PseudoElement::After),
                fields.then_some(PseudoElement::Placeholder),
                marked.then_some(PseudoElement::Marker),
            ]
            .into_iter()
            .flatten()
            .filter(|_| with_pseudo)
            .collect();
            let unchanged =
                previous.filter(|previous| previous.contexts.get(&node) == Some(&context));
            styles.stats[0] += 1;
            styles.stats[1] += u32::from(unchanged.is_some());
            let matched = unchanged
                .and_then(|previous| previous.retained.get(&node))
                .map(|key| key.matched.clone())
                .unwrap_or_else(|| {
                    Rc::new(Matched {
                        rules: rules.matching_local(&element, None),
                        pseudo: parts
                            .iter()
                            .map(|part| {
                                let matched = rules.matching_local(&element, Some((part, 0)));
                                (pseudo_key(part), matched)
                            })
                            .collect(),
                        inline: dom.inline_style(node).declarations(),
                        hints: presentation_hints(dom, node),
                    })
                });
            styles.contexts.insert(node, context);
            let relational = if relational {
                core::iter::once(rules.matching_relational(&element, None))
                    .chain(
                        parts
                            .iter()
                            .map(|part| rules.matching_relational(&element, Some((part, 0)))),
                    )
                    .collect()
            } else {
                Vec::new()
            };
            let key = StyleKey {
                parent: dom.flat_parent_element(node),
                matched,
                relational,
                environment,
            };
            // A parent style that changed in nothing its children take from
            // it keeps their styles, unless one inherits explicitly.
            let inherited = parent.as_ref().is_none_or(|parent| {
                previous
                    .and_then(|previous| key.parent.and_then(|id| previous.get(id)))
                    .is_some_and(|old| {
                        Rc::ptr_eq(old, parent)
                            || (old.inherits_same(parent)
                                && previous
                                    .and_then(|previous| previous.get(node))
                                    .is_some_and(|own| !own.inherits_explicitly))
                    })
            });
            let retained = previous.filter(|previous| {
                inherited && previous.retained.get(&node).is_some_and(|old| **old == key)
            });
            styles.stats[2] += u32::from(retained.is_some());
            let style = retained
                .and_then(|previous| previous.get(node))
                .cloned()
                .unwrap_or_else(|| {
                    let matched = merge_matches(
                        &key.matched.rules,
                        key.relational.first().map_or(&[], Vec::as_slice),
                    );
                    Rc::new(rules.compute_matched_style(
                        &element,
                        &matched,
                        &key.matched.hints,
                        &key.matched.inline,
                        parent.as_deref(),
                        &environment,
                    ))
                });
            // A query container gives its descendants its own size, as
            // last laid out; a new size restyles them.
            let style = match (style.container_type, containers(node)) {
                (ContainerType::Normal, _) | (_, None) => style,
                (kind, Some((width, height))) => {
                    let height = if kind == ContainerType::Size {
                        height
                    } else {
                        parent
                            .as_ref()
                            .and_then(|parent| parent.query_size)
                            .map_or(environment.media.height as f32, |(_, height)| height)
                    };
                    if style.query_size == Some((width, height)) {
                        style
                    } else {
                        Rc::new(ComputedStyle {
                            query_size: Some((width, height)),
                            ..(*style).clone()
                        })
                    }
                }
            };
            styles.retained.insert(node, Rc::new(key));
            if node == root {
                environment.root_font_size = style.font_size.0;
                styles.environment = environment;
            }
            if with_pseudo {
                for pseudo in parts {
                    let computed = if let Some(previous) = retained {
                        previous.pseudo(node, &pseudo).cloned()
                    } else {
                        rules
                            .compute_pseudo_style(&element, &pseudo, &style, &environment)
                            .map(Rc::new)
                    };
                    if let Some(computed) = computed {
                        styles.pseudo.insert((node, pseudo_key(&pseudo)), computed);
                    }
                }
            }
            if (with_pseudo
                || style.scrollbar_width != nativeboat_css::types::ScrollbarWidth::Auto
                || style.scrollbar_color.0.is_some())
                && (node == root
                    || [style.overflow_x, style.overflow_y]
                        .iter()
                        .any(|overflow| matches!(overflow, Overflow::Auto | Overflow::Scroll)))
            {
                // Only boxes that may show scrollbars style them: boxes with
                // `overflow: hidden`, as table cells often are, never do.
                // An element with the same style whose scrollbar parts match
                // the same rules keeps its scrollbars.
                let matched = unchanged
                    .and_then(|previous| previous.scrollbar_rules.get(&node).cloned())
                    .unwrap_or_else(|| scrollbar_rules(&element, rules));
                let scrollbars = match retained {
                    Some(previous) if previous.scrollbar_rules.get(&node) == Some(&matched) => {
                        previous.scrollbars.get(&node).cloned()
                    }
                    _ => scrollbar::compute(&element, rules, &style, &environment, node == root),
                };
                styles.scrollbar_rules.insert(node, matched);
                if let Some(scrollbars) = scrollbars {
                    styles.scrollbars.insert(node, scrollbars);
                }
            }
            let shown = shown || targets.contains(&node);
            let rendered = shown || style.display != Display::None;
            // Rendering follows the flat tree: a host's shadow tree, and
            // the elements a slot shows.
            let children = dom.flat_child_elements(node);
            let slot = dom.is_assigning_slot(node);
            let places = if slot {
                slotted_places(dom, &children, relations.structure)
            } else {
                places(dom, &children, relations.structure)
            };
            // A child's context holds what selectors test of its siblings:
            // its place among them, and the siblings before it when
            // compounds left of `+` or `~` test them; an element inserted
            // among them changes no other child's context otherwise. The
            // elements a slot shows have the host's children as siblings,
            // all of which their contexts hold.
            let mut preceding = Fnv::new().word(lineage);
            if slot {
                for sibling in children
                    .first()
                    .and_then(|child| dom.parent(*child))
                    .map(|host| dom.child_elements(host))
                    .unwrap_or_default()
                {
                    preceding = preceding.word(u64::from(sibling.0)).word(related_state(
                        dom,
                        sibling,
                        &relations.siblings,
                    ));
                }
            }
            let mut contexts = Vec::with_capacity(children.len());
            for (child, place) in children.into_iter().zip(places) {
                let within = preceding.word(u64::from(child.0)).word(place);
                if rendered || paths.contains(&child) {
                    let context = within.word(element_state(dom, child)).0;
                    let lineage = within
                        .word(related_state(dom, child, &relations.ancestors))
                        .0;
                    contexts.push((child, context, lineage));
                }
                if relations.structure & structure::PRECEDING != 0 {
                    preceding = preceding
                        .word(related_state(dom, child, &relations.siblings))
                        .word(place);
                }
            }
            for (child, context, lineage) in contexts.into_iter().rev() {
                pending.push((child, Some(style.clone()), shown, context, lineage));
            }
            styles.elements.insert(node, style);
        }
        styles
    }

    /// The style of an element, computed on demand for one inside a hidden
    /// element from the same style sheets; `None` for an element outside
    /// the document.
    pub fn style_of(
        &self,
        dom: &Dom,
        sheets: &[SheetRef<'_>],
        node: NodeId,
    ) -> Option<Rc<ComputedStyle>> {
        let mut unstyled = Vec::new();
        let mut current = node;
        let styled = loop {
            if let Some(style) = self.elements.get(&current) {
                break style.clone();
            }
            unstyled.push(current);
            current = dom.flat_parent_element(current)?;
        };
        if unstyled.is_empty() {
            return Some(styled);
        }
        let rules = RuleMap::new(sheets, &self.environment.media);
        Some(unstyled.into_iter().rev().fold(styled, |parent, element| {
            Rc::new(element_style(
                dom,
                &rules,
                element,
                Some(&parent),
                &self.environment,
            ))
        }))
    }

    /// The computed style of an element.
    pub fn get(&self, node: NodeId) -> Option<&Rc<ComputedStyle>> {
        self.elements.get(&node)
    }

    /// Iterates over the computed element styles.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &ComputedStyle)> {
        self.elements
            .iter()
            .map(|(node, style)| (*node, style.as_ref()))
    }

    /// Changes a presentation style without changing snapshots that share it.
    pub fn get_mut(&mut self, node: NodeId) -> Option<&mut ComputedStyle> {
        let style = self.elements.get_mut(&node)?;
        self.selector_state = None;
        self.retained.remove(&node);
        Some(Rc::make_mut(style))
    }

    /// The style of an element (slot 0) or of its `::before` (1),
    /// `::after` (2) or `::placeholder` (3) pseudo-element, as animations
    /// address them.
    pub fn target(&self, (node, slot): (NodeId, u8)) -> Option<&Rc<ComputedStyle>> {
        match slot {
            0 => self.elements.get(&node),
            slot => self.pseudo.get(&(node, slot - 1)),
        }
    }

    /// Changes the presentation style of a [`Styles::target`].
    pub fn target_mut(&mut self, (node, slot): (NodeId, u8)) -> Option<&mut ComputedStyle> {
        if slot == 0 {
            return self.get_mut(node);
        }
        let style = self.pseudo.get_mut(&(node, slot - 1))?;
        Some(Rc::make_mut(style))
    }

    /// Every element's and pseudo-element's style, by [`Styles::target`].
    pub fn targets(&self) -> impl Iterator<Item = ((NodeId, u8), &ComputedStyle)> {
        self.elements
            .iter()
            .map(|(node, style)| ((*node, 0), style.as_ref()))
            .chain(
                self.pseudo
                    .iter()
                    .map(|((node, key), style)| ((*node, key + 1), style.as_ref())),
            )
    }

    /// The pseudo-element a [`Styles::target`] slot names, as events name
    /// it, or an empty string for the element.
    pub fn target_pseudo(slot: u8) -> &'static str {
        match slot {
            0 => "",
            1 => "::before",
            2 => "::after",
            _ => "::placeholder",
        }
    }

    /// The computed style of a `::before`, `::after` or `::placeholder` pseudo-element.
    pub fn pseudo(&self, node: NodeId, pseudo: &PseudoElement) -> Option<&Rc<ComputedStyle>> {
        self.pseudo.get(&(node, pseudo_key(pseudo)))
    }

    /// The environment the styles were computed in, with the root font size.
    pub fn environment(&self) -> &StyleEnvironment {
        &self.environment
    }

    /// The author-styled scrollbars of a viewport or scroll container.
    pub fn scrollbars(&self, node: NodeId) -> Option<&nativeboat_ir::Scrollbars> {
        self.scrollbars.get(&node)
    }

    /// Elements styled by the computation that made these styles, those
    /// whose matched rules and those whose computed styles were reused.
    pub fn stats(&self) -> [u32; 3] {
        self.stats
    }

    /// The number of styled elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether no element is styled.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
}

#[cfg(test)]
mod tests;
