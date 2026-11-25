//! Caret positions and the geometry of text ranges: the character boundary
//! of painted text nearest a point, as `caretPositionFromPoint` finds it, and
//! the rectangles part of a text node covers, as `Range.getClientRects`
//! reports them. Offsets count UTF-16 code units of a text node's data.

use crate::prelude::*;

use super::{Geometry, Placed, Scene};
use crate::styles::Styles;
use crate::text::TextMeasurer;
use crate::tree::{LayoutResult, TextRun};
use nativeboat_css::types::{Display, Float, PositionKind, Visibility};
use nativeboat_dom::{Dom, NodeId, NodeKind, node_type};
use nativeboat_ir::Rect;

/// What text geometry is computed from: the document, its styles and layout,
/// and the fonts that measured it.
#[allow(missing_debug_implementations)]
pub struct TextQuery<'a> {
    /// The document.
    pub dom: &'a Dom,
    /// Its computed styles.
    pub styles: &'a Styles,
    /// Its layout.
    pub layout: &'a LayoutResult,
    /// The text measurer of the layout.
    pub measurer: &'a dyn TextMeasurer,
}

/// Elements a caret is never placed inside.
const OPAQUE: &[&str] = &[
    "br", "img", "hr", "input", "wbr", "textarea", "select", "iframe", "canvas", "svg", "video",
    "audio", "object", "embed",
];

/// Characters that `white-space` collapses or drops from laid-out text.
fn collapsible(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Aligns the texts laid out from a text node, in order, with its data: the
/// offset in `source` of each character boundary of each text. Laid-out text
/// collapses and drops white space; a preserved tab stays one character.
fn boundaries(source: &str, texts: &[&str]) -> Vec<Vec<u32>> {
    let characters: Vec<char> = source.chars().collect();
    let mut offsets = Vec::with_capacity(characters.len() + 1);
    let mut offset = 0;
    offsets.push(offset);
    for character in &characters {
        offset += character.len_utf16() as u32;
        offsets.push(offset);
    }
    let mut cursor = 0;
    texts
        .iter()
        .map(|text| {
            let mut bounds = Vec::with_capacity(text.len() + 1);
            for character in text.chars() {
                if character != ' ' {
                    while characters
                        .get(cursor)
                        .is_some_and(|source| collapsible(*source) && *source != character)
                    {
                        cursor += 1;
                    }
                }
                bounds.push(offsets[cursor]);
                if character != ' ' || characters.get(cursor).copied().is_some_and(collapsible) {
                    cursor = (cursor + 1).min(characters.len());
                }
            }
            bounds.push(offsets[cursor]);
            bounds
        })
        .collect()
}

/// The runs of some text nodes in layout order, with the offsets of their
/// character boundaries, by text node.
pub(super) fn run_bounds(
    dom: &Dom,
    layout: &LayoutResult,
    wanted: impl Fn(NodeId) -> bool,
) -> HashMap<NodeId, Vec<(usize, Vec<u32>)>> {
    let mut indexes: HashMap<NodeId, Vec<usize>> = HashMap::new();
    for (index, run) in layout.runs.iter().enumerate() {
        if wanted(run.node) {
            indexes.entry(run.node).or_default().push(index);
        }
    }
    indexes
        .into_iter()
        .map(|(node, indexes)| {
            let texts: Vec<&str> = indexes
                .iter()
                .map(|index| layout.runs[*index].text.as_str())
                .collect();
            let bounds = boundaries(dom.data(node).unwrap_or_default(), &texts);
            (node, indexes.into_iter().zip(bounds).collect())
        })
        .collect()
}

/// The runs of a text node in layout order, with the offsets of their
/// character boundaries.
fn node_runs(query: &TextQuery<'_>, node: NodeId) -> Vec<(usize, Vec<u32>)> {
    if query.dom.data(node).is_none() {
        return Vec::new();
    }
    run_bounds(query.dom, query.layout, |candidate| candidate == node)
        .remove(&node)
        .unwrap_or_default()
}

/// The advance of the first `count` characters of a run.
pub(super) fn advance(run: &TextRun, measurer: &dyn TextMeasurer, count: usize) -> f32 {
    // Tabs are as wide as their stops made them.
    if !run.text.is_empty() && run.text.chars().all(|character| character == '\t') {
        return run.rect.width * count as f32 / run.text.chars().count() as f32;
    }
    // Text at `font-size: 0` has no glyphs to advance by.
    if run.font.size <= 0.0 {
        return run.letter_spacing * count as f32;
    }
    let end = run
        .text
        .char_indices()
        .nth(count)
        .map_or(run.text.len(), |(index, _)| index);
    measurer.width(&run.text[..end], &run.font) + run.letter_spacing * count as f32
}

/// Where the characters from `from` to `to` of a run lie, from its left
/// edge: right to left, its first characters are on the right.
pub(super) fn extent(
    run: &TextRun,
    measurer: &dyn TextMeasurer,
    from: usize,
    to: usize,
) -> (f32, f32) {
    let (start, end) = (advance(run, measurer, from), advance(run, measurer, to));
    if run.rtl {
        (run.rect.width - end, run.rect.width - start)
    } else {
        (start, end)
    }
}

/// The character boundary of a run nearest a distance from its left edge.
fn nearest(run: &TextRun, measurer: &dyn TextMeasurer, x: f32) -> usize {
    let x = if run.rtl { run.rect.width - x } else { x };
    let count = run.text.chars().count();
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if advance(run, measurer, middle) <= x {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    if low < count && advance(run, measurer, low + 1) - x < x - advance(run, measurer, low) {
        low + 1
    } else {
        low
    }
}

/// The run nearest a point: on the point's line if any, else on the nearest
/// line; the topmost of equally near runs. Text without width, as Muya's
/// markers hidden at `font-size: 0`, takes the caret only when no other
/// text is near, as Chromium's hit testing passes over it.
fn closest<'p>(
    texts: impl Iterator<Item = &'p Placed>,
    runs: &[TextRun],
    x: f32,
    y: f32,
) -> Option<&'p Placed> {
    // Distances in Chromium's layout units of 1/64 pixel, so a point on the
    // edge between two lines is on neither's far side by a rounding error.
    let unit = |value: f32| crate::math::round(value * 64.0);
    let distance = |start: f32, size: f32, at: f32| {
        let (start, end, at) = (unit(start), unit(start + size), unit(at));
        (start - at).max(at - end).max(0.0)
    };
    let mut best: Option<((bool, f32, f32), &Placed)> = None;
    for placed in texts {
        let bounds = placed.region.bounds();
        // Vertically, a point belongs to the line box it is in, as in
        // Chromium, wherever the run's own text sits within the line.
        let run = &runs[placed.run];
        let line = Geometry {
            rect: Rect::new(
                placed.region.rect.x,
                placed.region.rect.y + (run.line.0 - run.rect.y),
                placed.region.rect.width,
                run.line.1,
            ),
            transform: placed.region.transform,
        }
        .bounds();
        let key = (
            bounds.width <= 0.0,
            distance(line.y, line.height, y),
            distance(bounds.x, bounds.width, x),
        );
        if best.is_none_or(|(nearest, _)| key <= nearest) {
            best = Some((key, placed));
        }
    }
    best.map(|(_, placed)| placed)
}

/// Whether a node holds nothing a caret could be placed in front of.
fn blank(dom: &Dom, node: NodeId) -> bool {
    match dom.kind(node) {
        NodeKind::Comment(_) => true,
        NodeKind::Text(text) => text.chars().all(collapsible),
        _ => false,
    }
}

/// The first caret position inside an element without text: in its first
/// rendered descendants that may hold text.
fn leading(query: &TextQuery<'_>, element: NodeId) -> NodeId {
    let dom = query.dom;
    let mut node = element;
    while let Some(child) = dom
        .children(node)
        .into_iter()
        .find(|child| !blank(dom, *child))
        && query.styles.get(child).is_some()
        && !OPAQUE.contains(&dom.local_name(child).unwrap_or_default())
    {
        node = child;
    }
    node
}

impl Scene {
    /// The caret position nearest a window point, as `caretPositionFromPoint`
    /// finds it: a text node and offset in the element at the point, or else
    /// in the box around it, or the start of a box without text.
    pub fn caret_from_point(&self, query: &TextQuery<'_>, x: f32, y: f32) -> Option<(NodeId, u32)> {
        let (dom, runs) = (query.dom, &query.layout.runs);
        let hit = self.hit_test(x, y)?;
        let texts: Vec<&Placed> = self
            .texts
            .iter()
            .filter(|placed| {
                dom.node_type(runs[placed.run].node) == node_type::TEXT
                    && self.unclipped(placed.clip, x, y)
            })
            .collect();
        let mut scope = hit;
        // A point between an element's block children belongs to the first
        // child that ends below it, or the last, as Chromium positions it.
        while let Some(child) = self.block_below(query, scope, y) {
            scope = child;
        }
        loop {
            let inside = texts
                .iter()
                .copied()
                .filter(|placed| dom.contains(scope, runs[placed.run].node));
            if let Some(placed) = closest(inside, runs, x, y) {
                let run = &runs[placed.run];
                let (_, bounds) = node_runs(query, run.node)
                    .into_iter()
                    .find(|(index, _)| *index == placed.run)
                    .expect("a painted run belongs to its text node");
                let local = placed
                    .region
                    .local(x, y)
                    .map_or(0.0, |(local, _)| local - placed.region.rect.x);
                return Some((run.node, bounds[nearest(run, query.measurer, local)]));
            }
            match dom.parent_element(scope) {
                Some(parent) if !query.layout.boxes.contains_key(&scope) => scope = parent,
                _ => return Some((leading(query, scope), 0)),
            }
        }
    }

    /// The block child of `element` a point at height `y` falls to when it
    /// lies in no child's box: the first child whose box ends below the
    /// point, or else the last. `None` when the point is inside a child's
    /// box or the element has no rendered block children.
    fn block_below(&self, query: &TextQuery<'_>, element: NodeId, y: f32) -> Option<NodeId> {
        let dom = query.dom;
        let mut children = Vec::new();
        for child in dom.children(element) {
            if blank(dom, child) {
                continue;
            }
            let Some(style) = query.styles.get(child) else {
                // Text beside blocks sits in an anonymous block, which the
                // nearest text stands for.
                if dom.node_type(child) == node_type::TEXT {
                    return None;
                }
                continue;
            };
            if style.display == Display::None {
                continue;
            }
            // Out-of-flow and floating boxes are no candidates, nor are
            // hidden or empty ones, as in Chromium's hit testing.
            if matches!(style.position, PositionKind::Absolute | PositionKind::Fixed)
                || style.float != Float::None
                || style.visibility != Visibility::Visible
            {
                continue;
            }
            if style.display.is_inline_level() {
                return None;
            }
            if let Some(rect) = self.client_rect(child)
                && rect.height > 0.0
            {
                children.push((child, rect));
            }
        }
        if children.is_empty()
            || children
                .iter()
                .any(|(_, rect)| rect.y <= y && y < rect.y + rect.height)
        {
            return None;
        }
        children
            .iter()
            .find(|(_, rect)| rect.y + rect.height > y)
            .or(children.last())
            .map(|(child, _)| *child)
    }

    /// The window rectangles that a range of a text node's data covers, one
    /// per painted run, as `Range.getClientRects` reports them; a collapsed
    /// range gives the caret's zero-width rectangle where text is beside it,
    /// and none between line breaks, as Chromium gives none.
    pub fn text_rects(
        &self,
        query: &TextQuery<'_>,
        node: NodeId,
        start: u32,
        end: u32,
    ) -> Vec<Rect> {
        let (start, end) = (start.min(end), start.max(end));
        let mut rects = Vec::new();
        for (index, bounds) in node_runs(query, node) {
            let (first, last) = (bounds[0], bounds[bounds.len() - 1]);
            let covered = if start == end {
                first <= start && start <= last
            } else {
                start < last && end > first
            };
            if !covered {
                continue;
            }
            let Some(placed) = self
                .texts
                .iter()
                .chain(&self.unshown)
                .find(|placed| placed.run == index)
            else {
                continue;
            };
            let run = &query.layout.runs[index];
            let column = |offset: u32| {
                bounds
                    .iter()
                    .position(|bound| *bound >= offset)
                    .unwrap_or(bounds.len() - 1)
            };
            let (from, to) = extent(run, query.measurer, column(start), column(end));
            let rect = placed.region.rect;
            let (top, height) = run.content;
            let region = Geometry {
                rect: Rect::new(rect.x + from, rect.y + top, to - from, height),
                transform: placed.region.transform,
            };
            rects.push(region.bounds());
            if start == end {
                break;
            }
        }
        rects
    }
}

#[cfg(test)]
mod tests;
