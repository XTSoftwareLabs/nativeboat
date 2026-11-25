//! The selection and the text insertion caret, painted with the text they
//! cover: selected text on its `::selection` background in its
//! `::selection` color, and the caret where the selection collapses inside
//! the focused editing host, in its `caret-color`.

use crate::prelude::*;

use super::caret::{extent, run_bounds};
use crate::styles::Styles;
use crate::text::{TextMeasurer, font_of};
use crate::tree::LayoutResult;
use nativeboat_css::types::TextAlign;
use nativeboat_css::{Color, PseudoElement, RuleMap, SheetRef};
use nativeboat_dom::{Dom, ElementRef, NodeId, NodeKind};
use nativeboat_ir::Rect;

/// The selected part of a text node and the colors it is painted in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    /// Where the selected part starts, in UTF-16 code units.
    pub start: u32,
    /// Where it ends.
    pub end: u32,
    /// The color of the selected text, or `None` to keep the text's color.
    pub color: Option<Color>,
    /// The background behind the selected text.
    pub background: Color,
}

/// Where the caret is: a boundary point of the document, and its color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caret {
    /// The boundary's node.
    pub node: NodeId,
    /// The boundary's offset.
    pub offset: u32,
    /// The caret's color.
    pub color: Color,
}

/// The selection and the caret painted with a document's text.
#[allow(missing_debug_implementations)]
pub struct Highlight<'a> {
    /// The text measurer of the layout.
    pub measurer: &'a dyn TextMeasurer,
    /// The selected parts of text nodes.
    pub spans: HashMap<NodeId, Span>,
    /// The caret, while one shows.
    pub caret: Option<Caret>,
    /// The misspelled parts of text nodes, ascending, marked with a wavy
    /// underline.
    pub misspellings: HashMap<NodeId, Vec<(u32, u32)>>,
}

/// The run boundaries of highlighted text nodes, and where the caret is
/// painted: with the content of a box, at a rectangle in document
/// coordinates.
pub(super) type Placed = (
    HashMap<NodeId, Vec<(usize, Vec<u32>)>>,
    Option<(NodeId, Rect, Color)>,
);

impl<'a> Highlight<'a> {
    /// The highlight of a document's selection: its text in the colors of
    /// the nearest `::selection` style, or the system highlight colors, or,
    /// when the selection collapses inside the focused editing host, the
    /// caret.
    pub fn new(
        dom: &Dom,
        styles: &Styles,
        sheets: &[SheetRef<'_>],
        measurer: &'a dyn TextMeasurer,
    ) -> Highlight<'a> {
        let mut highlight = Highlight {
            measurer,
            spans: HashMap::new(),
            caret: None,
            misspellings: HashMap::new(),
        };
        let Some([start, end]) = dom.selection() else {
            return highlight;
        };
        if start == end {
            highlight.caret = caret(dom, styles, start);
            return highlight;
        }
        let rules = RuleMap::new(sheets, &styles.environment().media);
        let mut colors = HashMap::new();
        for (node, start, end) in dom.range_texts([start, end]) {
            let element = dom.parent_element(node);
            let (color, background) = *colors
                .entry(element)
                .or_insert_with(|| selection_colors(dom, styles, &rules, element));
            highlight.spans.insert(
                node,
                Span {
                    start,
                    end,
                    color,
                    background,
                },
            );
        }
        highlight
    }

    /// Marks the misspelled parts of text nodes, except where their
    /// `::spelling-error` style removes the marks' decoration.
    pub fn mark_misspellings(
        &mut self,
        dom: &Dom,
        styles: &Styles,
        sheets: &[SheetRef<'_>],
        misspellings: impl IntoIterator<Item = (NodeId, Vec<(u32, u32)>)>,
    ) {
        // Indexing the sheets costs more than most renders: only text with
        // marks pays for it.
        let misspellings: Vec<_> = misspellings
            .into_iter()
            .filter(|(_, ranges)| !ranges.is_empty())
            .collect();
        if misspellings.is_empty() {
            return;
        }
        let rules = RuleMap::new(sheets, &styles.environment().media);
        let mut shown = HashMap::new();
        for (node, ranges) in misspellings {
            let element = dom.parent_element(node);
            let marked = *shown.entry(element).or_insert_with(|| {
                let spelling = PseudoElement::Other("spelling-error".into());
                core::iter::successors(element, |element| dom.parent_element(*element))
                    .find_map(|element| {
                        let style = styles.get(element)?;
                        rules
                            .compute_pseudo_style(
                                &ElementRef::new(dom, element),
                                &spelling,
                                style,
                                styles.environment(),
                            )
                            .map(|error| error.text_decoration_line != Default::default())
                    })
                    .unwrap_or(true)
            });
            if marked {
                self.misspellings.insert(node, ranges);
            }
        }
    }

    /// The run boundaries of the highlighted text and where the caret is.
    pub(super) fn place(&self, dom: &Dom, styles: &Styles, layout: &LayoutResult) -> Placed {
        let candidates = self
            .caret
            .map_or_else(Vec::new, |caret| caret_texts(dom, caret.node, caret.offset));
        let bounds = run_bounds(dom, layout, |node| {
            self.spans.contains_key(&node)
                || self.misspellings.contains_key(&node)
                || candidates.iter().any(|(text, _)| *text == node)
        });
        let caret = self.caret.and_then(|caret| {
            let (container, rect) = candidates
                .iter()
                .find_map(|(text, offset)| {
                    let runs = bounds.get(text)?;
                    let (index, stops) = runs
                        .iter()
                        .find(|(_, stops)| *offset < stops[stops.len() - 1])
                        .or(runs.last())?;
                    let run = &layout.runs[*index];
                    // A caret in text at `font-size: 0` has no height, as in
                    // browsers, and shows nothing.
                    if run.font.size <= 0.0 {
                        return None;
                    }
                    let column = stops
                        .iter()
                        .position(|stop| stop >= offset)
                        .unwrap_or(stops.len() - 1);
                    let x = run.rect.x + extent(run, self.measurer, column, column).0;
                    let metrics = self.measurer.metrics(&run.font);
                    let height = metrics.ascent + metrics.descent;
                    let y = run.rect.y + (run.rect.height - height) / 2.0;
                    Some((run.container, Rect::new(x, y, 1.0, height)))
                })
                .or_else(|| self.empty_line(dom, styles, layout, caret.node))?;
            Some((container, rect, caret.color))
        });
        (bounds, caret)
    }

    /// The caret at the start of the line of a box without text.
    fn empty_line(
        &self,
        dom: &Dom,
        styles: &Styles,
        layout: &LayoutResult,
        node: NodeId,
    ) -> Option<(NodeId, Rect)> {
        let element = if dom.is_element(node) {
            node
        } else {
            dom.parent_element(node)?
        };
        let style = styles.get(element)?;
        let container =
            core::iter::successors(Some(element), |element| dom.parent_element(*element))
                .find(|element| layout.boxes.contains_key(element))?;
        let font = font_of(style);
        // A caret in text at `font-size: 0` has no height, as in browsers.
        if font.size <= 0.0 {
            return None;
        }
        let content = layout.boxes[&container].content_box();
        let metrics = self.measurer.metrics(&font);
        let height = metrics.ascent + metrics.descent;
        let align = styles
            .get(container)
            .map_or(TextAlign::Start, |style| style.text_align);
        let x = match align {
            TextAlign::Center => content.x + content.width / 2.0,
            TextAlign::Right | TextAlign::End => content.x + content.width - 1.0,
            _ => content.x,
        };
        let y = content.y + (crate::text::line_height(style, self.measurer) - height) / 2.0;
        Some((container, Rect::new(x, y, 1.0, height)))
    }
}

/// The text nodes and offsets a caret may be painted at, in order of
/// preference: the boundary itself in a text node, or else the end of the
/// text before an element boundary and the start of the text after it.
fn caret_texts(dom: &Dom, node: NodeId, offset: u32) -> Vec<(NodeId, u32)> {
    let is_text = |node: &NodeId| matches!(dom.kind(*node), NodeKind::Text(_));
    if is_text(&node) {
        return vec![(node, offset)];
    }
    let children = dom.children(node);
    let subtree = |child: NodeId| core::iter::once(child).chain(dom.descendants(child));
    let before = offset
        .checked_sub(1)
        .and_then(|index| children.get(index as usize))
        .and_then(|child| subtree(*child).rfind(is_text))
        .map(|text| (text, u32::MAX));
    let after = children
        .get(offset as usize)
        .and_then(|child| subtree(*child).find(is_text))
        .map(|text| (text, 0));
    before.into_iter().chain(after).collect()
}

/// The caret at a collapsed selection inside the focused editing host.
fn caret(dom: &Dom, styles: &Styles, (node, offset): (NodeId, u32)) -> Option<Caret> {
    let host = dom.editing_host(node)?;
    if dom.focused() != Some(host) {
        return None;
    }
    let element = if dom.is_element(node) {
        node
    } else {
        dom.parent_element(node)?
    };
    let style = styles.get(element)?;
    let color = style.caret_color.0.unwrap_or(style.color);
    (!color.is_transparent()).then_some(Caret {
        node,
        offset,
        color,
    })
}

/// The colors of selected text in an element: those of the `::selection`
/// style of the element or of its nearest ancestor that has one, else the
/// system's highlight colors.
fn selection_colors(
    dom: &Dom,
    styles: &Styles,
    rules: &RuleMap<'_>,
    element: Option<NodeId>,
) -> (Option<Color>, Color) {
    let environment = styles.environment();
    core::iter::successors(element, |element| dom.parent_element(*element))
        .find_map(|element| {
            let style = styles.get(element)?;
            let selection = rules.compute_pseudo_style(
                &ElementRef::new(dom, element),
                &PseudoElement::Selection,
                style,
                environment,
            )?;
            // A color the `::selection` style inherits keeps the text's own.
            let color = (selection.color != style.color).then_some(selection.color);
            Some((color, selection.background_color))
        })
        .unwrap_or((
            Some(environment.system.highlight_text),
            environment.system.highlight,
        ))
}

#[cfg(test)]
mod tests;
