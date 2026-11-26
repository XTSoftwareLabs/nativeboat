//! The layout tree: boxes for the DOM laid out with taffy.

use crate::prelude::*;

use crate::convert::{CalcStore, Mode, convert, mode_of};
use crate::inline::{
    Atom, AtomSize, Band, Edge, InlineBox, InlineContent, Segment, Symbol, WordBreaking,
    above_baseline, break_lines, break_lines_beside, layout_lines,
};
use crate::styles::Styles;
use crate::text::{TextMeasurer, font_of, line_height, pieces, transform};
use alloc::rc::Rc;
use nativeboat_css::types::{
    BackgroundImage, BorderCollapse, BorderStyle, Content, ContentItem, Direction, Display,
    LengthPercentageAuto as CssLengthAuto, LineHeight, ListStylePosition, ListStyleType,
    OverflowWrap, PositionKind, Px, TrackSize, VerticalAlign, WordBreak,
};
use nativeboat_css::{Color, ComputedStyle, PseudoElement, UsedBox};
use nativeboat_dom::{Dom, NodeId, NodeKind};
use nativeboat_ir::{ControlKind, Font, Rect, control_for};
use taffy::prelude::{
    AlignItems, AlignSelf, AvailableSpace, Display as TaffyDisplay, FlexWrap, GridPlacement,
    JustifyContent, Layout, LengthPercentage, LengthPercentageAuto, Line, Position,
    Rect as TaffyRect, Size, Style,
};
use taffy::style::GridTemplateComponent;
use taffy::style_helpers::{TaffyGridLine, TaffyGridSpan};
use taffy::util::{MaybeMath, MaybeResolve};
use taffy::{
    Baselines, BlockContext, Cache, CacheTree, LayoutBlockContainer, LayoutFlexboxContainer,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, RequestedAxis,
    ResolveOrZero, RoundTree, RunMode, SizingMode, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_leaf_layout, compute_root_layout,
};

/// What layout needs from the platform.
pub struct LayoutOptions<'a> {
    /// The viewport size in device-independent pixels.
    pub viewport: (f32, f32),
    /// Measures text.
    pub measurer: &'a dyn TextMeasurer,
    /// Returns the natural size of an image URL, if it is known.
    pub image_size: &'a dyn Fn(&str) -> Option<(f32, f32)>,
}

impl core::fmt::Debug for LayoutOptions<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LayoutOptions")
            .field("viewport", &self.viewport)
            .finish_non_exhaustive()
    }
}

/// The geometry of an element's box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoxGeometry {
    /// The border box in document coordinates, ignoring scrolling.
    pub rect: Rect,
    /// Padding widths: top, right, bottom, left.
    pub padding: [f32; 4],
    /// Border widths: top, right, bottom, left.
    pub border: [f32; 4],
    /// Used margins: top, right, bottom, left, including auto margins.
    pub margin: [f32; 4],
    /// Width of the scrollable content from the padding box origin, as
    /// `scrollWidth` reports it.
    pub content_width: f32,
    /// Height of the scrollable content, as `scrollHeight` reports it.
    pub content_height: f32,
    /// The room scrollbars take inside the border: the vertical one's
    /// width, then the horizontal one's height.
    pub scrollbars: [f32; 2],
}

impl BoxGeometry {
    /// The box `getComputedStyle` resolves an element's sizes, margins and
    /// paddings against: none for non-replaced inline elements, whose
    /// values stay as computed.
    pub fn used_box(&self, dom: &Dom, element: NodeId, style: &ComputedStyle) -> Option<UsedBox> {
        let replaced = matches!(
            dom.local_name(element),
            Some(
                "img"
                    | "svg"
                    | "canvas"
                    | "video"
                    | "iframe"
                    | "embed"
                    | "object"
                    | "input"
                    | "select"
                    | "textarea"
                    | "button"
            )
        );
        if style.display == Display::Inline && !replaced {
            return None;
        }
        Some(UsedBox {
            size: (self.rect.width, self.rect.height),
            border: self.border,
            padding: self.padding,
            margin: self.margin,
            scrollbars: self.scrollbars,
            sized: true,
        })
    }

    /// The padding box.
    pub fn padding_box(&self) -> Rect {
        Rect::new(
            self.rect.x + self.border[3],
            self.rect.y + self.border[0],
            (self.rect.width - self.border[1] - self.border[3]).max(0.0),
            (self.rect.height - self.border[0] - self.border[2]).max(0.0),
        )
    }

    /// The padding box without the room scrollbars take, as `clientWidth`
    /// and `clientHeight` measure it.
    pub fn client_box(&self) -> Rect {
        let padding = self.padding_box();
        Rect::new(
            padding.x,
            padding.y,
            (padding.width - self.scrollbars[0]).max(0.0),
            (padding.height - self.scrollbars[1]).max(0.0),
        )
    }

    /// The content box.
    pub fn content_box(&self) -> Rect {
        let padding = self.client_box();
        Rect::new(
            padding.x + self.padding[3],
            padding.y + self.padding[0],
            (padding.width - self.padding[1] - self.padding[3]).max(0.0),
            (padding.height - self.padding[0] - self.padding[2]).max(0.0),
        )
    }
}

/// A run of text placed on a line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Bounds in document coordinates: its line-height box.
    pub rect: Rect,
    /// The font's content area within the bounds, its ascent and descent:
    /// how far below the top it starts and how tall it is. Script geometry,
    /// as `getClientRects`, reports it, as Chromium does.
    pub content: (f32, f32),
    /// The line box holding the run: its top in document coordinates and
    /// its height. A point at that height falls to this line, as Chromium
    /// hit tests.
    pub line: (f32, f32),
    /// The text.
    pub text: String,
    /// Font.
    pub font: Font,
    /// Color.
    pub color: Color,
    /// Underline.
    pub underline: bool,
    /// How far below the baseline the underline begins, when
    /// `text-underline-offset` sets it.
    pub underline_offset: Option<f32>,
    /// Line-through.
    pub line_through: bool,
    /// Extra space after each character.
    pub letter_spacing: f32,
    /// The text node (or the element of generated content).
    pub node: NodeId,
    /// The element receiving pointer events.
    pub element: NodeId,
    /// The block container whose inline content holds the run.
    pub container: NodeId,
    /// A list marker's symbol, painted as a shape in a box relative to the
    /// run's origin instead of as text.
    pub symbol: Option<(Symbol, Rect)>,
    /// The pseudo-element of generated content with a box of its own that
    /// paints as a layer, which paints the run.
    pub pseudo: Option<PseudoElement>,
    /// Whether the text runs right to left: its first character is on the
    /// right.
    pub rtl: bool,
}

/// An empty positioned generated box, without a DOM node of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedBox {
    /// Whether this is the originating element's before or after decoration.
    pub pseudo: PseudoElement,
    /// Native geometry in document coordinates.
    pub geometry: BoxGeometry,
}

/// One line's part of an inline element's box.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineBoxRect {
    /// The block container whose inline content holds the element.
    pub container: NodeId,
    /// The element.
    pub node: NodeId,
    /// The border box of this part, in document coordinates.
    pub rect: Rect,
    /// Whether the element starts on this line, so its left edge shows.
    pub start: bool,
    /// Whether the element ends on this line, so its right edge shows.
    pub end: bool,
}

/// The laid-out document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutResult {
    /// Element boxes.
    pub boxes: HashMap<NodeId, BoxGeometry>,
    /// The boxes of inline elements whose boxes show, line by line, in
    /// document order.
    pub inline_boxes: Vec<InlineBoxRect>,
    /// Empty positioned generated boxes, indexed by their originating element.
    pub generated: HashMap<NodeId, Vec<GeneratedBox>>,
    /// Text runs in paint order.
    pub runs: Vec<TextRun>,
    /// Width of the document content.
    pub width: f32,
    /// Height of the document content.
    pub height: f32,
    /// The natural sizes of the background images and image elements'
    /// pictures, by URL.
    pub images: HashMap<String, (f32, f32)>,
    /// How scrollbars no author CSS styles look, and the size of the
    /// device pixels they fit (`0` when it is not known).
    pub scrollbars: (nativeboat_ir::scrollbar::Look, f32),
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Container(Mode),
    Inline(InlineContent),
    Leaf {
        width: f32,
        height: f32,
        ratio: Option<f32>,
        /// Contributes no min-content width (a percentage-sized control).
        compressible: bool,
    },
}

#[derive(Debug)]
struct Node {
    dom: Option<NodeId>,
    kind: Kind,
    style: Style,
    children: Vec<usize>,
    cache: Cache,
    unrounded: Layout,
    layout: Layout,
    /// The atoms of inline content as last laid out.
    atom_sizes: Vec<AtomSize>,
    /// The lines of inline content as they last broke, with the width and
    /// atom sizes they broke for.
    lines: core::cell::RefCell<Option<BrokenLines>>,
    /// The lines of inline content broken beside floats in its last layout.
    float_lines: Option<Rc<crate::inline::Lines>>,
    /// A table cell's own `width`, which sizes its column; the cell fills
    /// the column.
    cell_width: Option<taffy::Dimension>,
    /// Whether a table's columns take their widths from its first row
    /// (`table-layout: fixed` with a width).
    fixed_table: bool,
    /// A control showing a line of text, as a text field: the line's height
    /// and how far below its top the baseline lies. The line sits in the
    /// middle of the content box and gives the box its baseline.
    text_line: Option<(f32, f32)>,
    /// A replaced element's style with its percentage `width` and
    /// `max-width` resolved against zero, which its min-content
    /// contribution takes where they are cyclic.
    compressed: Option<Box<Style>>,
}

type BrokenLines = (f32, Vec<AtomSize>, Rc<crate::inline::Lines>);

struct Tree<'m> {
    nodes: Vec<Node>,
    measurer: &'m dyn TextMeasurer,
    /// Whether the container being laid out is finding its min-content
    /// width, against which its children's percentage widths are cyclic.
    compressing: bool,
}

impl Tree<'_> {
    /// A child's style as its container sees it: compressible replaced
    /// elements contribute as if their cyclic percentages were zero.
    fn item_style(&self, child: taffy::NodeId) -> &Style {
        let node = &self.nodes[usize::from(child)];
        match &node.compressed {
            Some(style) if self.compressing => style,
            _ => &node.style,
        }
    }
}

/// Iterates children of a layout node.
#[derive(Debug)]
pub struct Children<'a>(core::slice::Iter<'a, usize>);

impl Iterator for Children<'_> {
    type Item = taffy::NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|index| taffy::NodeId::from(*index))
    }
}

impl TraversePartialTree for Tree<'_> {
    type ChildIter<'a>
        = Children<'a>
    where
        Self: 'a;

    fn child_ids(&self, node: taffy::NodeId) -> Self::ChildIter<'_> {
        Children(self.nodes[usize::from(node)].children.iter())
    }

    fn child_count(&self, node: taffy::NodeId) -> usize {
        self.nodes[usize::from(node)].children.len()
    }

    fn get_child_id(&self, node: taffy::NodeId, index: usize) -> taffy::NodeId {
        taffy::NodeId::from(self.nodes[usize::from(node)].children[index])
    }
}

impl TraverseTree for Tree<'_> {}

/// [`measure_inline_beside`] without floats.
#[cfg(test)]
fn measure_inline(
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    sizes: &[AtomSize],
) -> (Size<f32>, crate::inline::Lines, Baselines) {
    measure_inline_beside(content, measurer, (known, available), sizes, None)
}

/// The size of inline content, its lines and the baselines of its first and
/// last lines, broken in the bands floats leave the lines when given.
fn measure_inline_beside(
    content: &InlineContent,
    measurer: &dyn TextMeasurer,
    (known, available): (Size<Option<f32>>, Size<AvailableSpace>),
    sizes: &[AtomSize],
    bands: Option<&dyn Fn(f32) -> Band>,
) -> (Size<f32>, crate::inline::Lines, Baselines) {
    let limit = known.width.or(match available.width {
        AvailableSpace::Definite(width) => Some(width),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    });
    let min_content = known.width.is_none() && available.width == AvailableSpace::MinContent;
    let lines = match (bands, limit) {
        // Bands reach no further than the content box.
        (Some(bands), Some(limit)) if !min_content => {
            let inside = |top: f32| {
                let band = bands(top);
                let left = band.left.clamp(0.0, limit);
                let right = (band.left + band.width).min(limit);
                Band {
                    left,
                    width: (right - left).max(0.0),
                    ..band
                }
            };
            break_lines_beside(content, measurer, sizes, &inside)
        }
        _ => break_lines(content, limit, min_content, measurer, sizes),
    };
    // Lines without text or boxes have no baseline.
    let mut shown = lines
        .lines
        .iter()
        .filter(|line| !line.fragments.is_empty() || !line.atoms.is_empty())
        .map(|line| line.top + line.baseline);
    let first = shown.next();
    let baselines = Baselines {
        first,
        last: shown.next_back().or(first),
    };
    // Shrink-to-fit: content wider than the space fills the space, not just
    // its widest wrapped line, as browsers size floats and flex items.
    let mut width = lines.width;
    if let (None, AvailableSpace::Definite(space)) = (known.width, available.width)
        && lines.lines.len() > 1
        && width < space
        && break_lines(content, None, false, measurer, sizes).width > space
    {
        width = space;
    }
    let size = Size {
        width: known.width.unwrap_or(width),
        height: known.height.unwrap_or(lines.height),
    };
    (size, lines, baselines)
}

/// Lays out inline content with atomic boxes: measures each box in the
/// space the lines have, breaks the lines around them and, for the final
/// layout, places them on their lines.
fn layout_inline(
    tree: &mut Tree<'_>,
    node: taffy::NodeId,
    inputs: LayoutInput,
    content: &InlineContent,
    bands: Option<&dyn Fn(f32) -> Band>,
) -> LayoutOutput {
    let index = usize::from(node);
    let known = inputs.known_dimensions;
    let space = known
        .width
        .map_or(inputs.available_space.width, AvailableSpace::Definite);
    // Percentages in the boxes resolve against the line width.
    let basis = known.width.or(inputs.parent_size.width);
    let mut sizes = Vec::with_capacity(content.atoms.len());
    let mut margins = Vec::with_capacity(content.atoms.len());
    let mut requests = Vec::with_capacity(content.atoms.len());
    for atom in &content.atoms {
        // Without a line width, the boxes' percentages are cyclic.
        let style = match basis {
            Some(_) => &tree.nodes[atom.node].style,
            None => tree.item_style(taffy::NodeId::from(atom.node)),
        };
        let margin: TaffyRect<f32> = style.margin.resolve_or_zero(basis, CalcStore::resolve);
        let room = match space {
            AvailableSpace::Definite(width) => {
                AvailableSpace::Definite((width - margin.left - margin.right).max(0.0))
            }
            other => other,
        };
        // Sizes the box's style gives, as border-box sizes, as block layout
        // resolves them; sizing keywords size it to its content.
        let room = if style.size.width == taffy::Dimension::max_content() {
            AvailableSpace::MaxContent
        } else if style.size.width == taffy::Dimension::min_content() {
            AvailableSpace::MinContent
        } else {
            room
        };
        // Percentage heights resolve against the block container's height,
        // which its inline content is given.
        let parent = Size {
            width: basis,
            height: inputs.parent_size.height,
        };
        let padding: TaffyRect<f32> = style.padding.resolve_or_zero(basis, CalcStore::resolve);
        let border: TaffyRect<f32> = style.border.resolve_or_zero(basis, CalcStore::resolve);
        let adjust = if style.box_sizing == taffy::BoxSizing::ContentBox {
            (padding + border).sum_axes()
        } else {
            Size::ZERO
        };
        let min = style
            .min_size
            .maybe_resolve(parent, CalcStore::resolve)
            .maybe_add(adjust);
        let max = style
            .max_size
            .maybe_resolve(parent, CalcStore::resolve)
            .maybe_add(adjust);
        let given = style
            .size
            .maybe_resolve(parent, CalcStore::resolve)
            .maybe_apply_aspect_ratio(style.aspect_ratio)
            .maybe_add(adjust)
            .maybe_clamp(min, max);
        let request = LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: given,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: parent,
            available_space: Size {
                width: room,
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let child = taffy::NodeId::from(atom.node);
        let measured = tree.compute_child_layout(child, request);
        // Only a full layout at that size finds the baselines of block
        // containers; placing the box later repeats it from the cache.
        let full = LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: measured.size.map(Some),
            ..request
        };
        let output = tree.compute_child_layout(child, full);
        let baseline = atom_baseline(&tree.nodes[atom.node], &output);
        sizes.push(AtomSize {
            width: output.size.width + margin.left + margin.right,
            height: output.size.height + margin.top + margin.bottom,
            baseline: baseline + margin.top,
        });
        margins.push(margin);
        requests.push(full);
    }
    let (size, lines, baselines) = measure_inline_beside(
        content,
        tree.measurer,
        (known, inputs.available_space),
        &sizes,
        bands,
    );
    if inputs.run_mode == RunMode::PerformLayout {
        tree.nodes[index].float_lines = bands.map(|_| Rc::new(lines.clone()));
        for placed in lines.lines.iter().flat_map(|line| &line.atoms) {
            let atom = &content.atoms[placed.atom];
            let margin = margins[placed.atom];
            let child = taffy::NodeId::from(atom.node);
            let output = tree.compute_child_layout(child, requests[placed.atom]);
            let style = &tree.nodes[atom.node].style;
            let basis = Some(size.width);
            let padding = style.padding.resolve_or_zero(basis, CalcStore::resolve);
            let border = style.border.resolve_or_zero(basis, CalcStore::resolve);
            let mut location = taffy::Point {
                x: placed.x + margin.left,
                y: placed.y + margin.top,
            };
            // A relatively positioned box moves from where the line put it.
            if style.position == Position::Relative {
                let inset = |value: LengthPercentageAuto, basis: f32| {
                    value.resolve_to_option(basis, CalcStore::resolve)
                };
                if let Some(left) = inset(style.inset.left, size.width) {
                    location.x += left;
                } else if let Some(right) = inset(style.inset.right, size.width) {
                    location.x -= right;
                }
                if let Some(top) = inset(style.inset.top, size.height) {
                    location.y += top;
                } else if let Some(bottom) = inset(style.inset.bottom, size.height) {
                    location.y -= bottom;
                }
            }
            tree.set_unrounded_layout(
                child,
                &Layout {
                    order: placed.atom as u32,
                    location,
                    size: output.size,
                    scrollable_overflow_rect: output.scrollable_overflow_rect,
                    scrollbar_size: Size::ZERO,
                    border,
                    padding,
                    margin,
                },
            );
        }
        tree.nodes[index].atom_sizes = sizes;
    }
    let overflow = TaffyRect {
        left: 0.0,
        top: 0.0,
        right: size.width.max(lines.width),
        bottom: size.height.max(lines.height),
    };
    LayoutOutput::from_sizes_and_baselines(size, overflow, baselines)
}

/// Lays out inline content, beside floats in `bands` when given.
fn layout_text(
    tree: &mut Tree<'_>,
    node: taffy::NodeId,
    inputs: LayoutInput,
    content: &InlineContent,
    bands: Option<&dyn Fn(f32) -> Band>,
) -> LayoutOutput {
    if !content.atoms.is_empty() {
        return layout_inline(tree, node, inputs, content, bands);
    }
    let index = usize::from(node);
    let measurer = tree.measurer;
    let mut baselines = Baselines::NONE;
    let mut broken = None;
    let mut output = compute_leaf_layout(
        inputs,
        &tree.nodes[index].style,
        CalcStore::resolve,
        |known, available| {
            let (size, lines, first) =
                measure_inline_beside(content, measurer, (known, available), &[], bands);
            baselines = first;
            broken = Some(lines);
            size
        },
    );
    if inputs.run_mode == RunMode::PerformLayout {
        tree.nodes[index].float_lines = bands.and(broken).map(Rc::new);
    }
    // Baseline alignment lines up the first lines of text.
    output.baselines = baselines;
    output
}

/// Lays out inline content beside the floats of its block formatting
/// context: each line breaks in the room the floats leave at its top, as
/// browsers shorten line boxes next to floats.
fn layout_beside_floats(
    tree: &mut Tree<'_>,
    node: taffy::NodeId,
    inputs: LayoutInput,
    context: &BlockContext<'_>,
) -> LayoutOutput {
    use taffy::style::Clear;
    let index = usize::from(node);
    let Kind::Inline(content) = tree.nodes[index].kind.clone() else {
        return tree.compute_child_layout(node, inputs);
    };
    let style = &tree.nodes[index].style;
    let basis = inputs.parent_size.width;
    let padding: TaffyRect<f32> = style.padding.resolve_or_zero(basis, CalcStore::resolve);
    let border: TaffyRect<f32> = style.border.resolve_or_zero(basis, CalcStore::resolve);
    let (left, top) = (padding.left + border.left, padding.top + border.top);
    // The context measures from the box's border edges.
    let bands = |line: f32| {
        let y = top + line;
        let slot = context.find_content_slot(y, Clear::None, None);
        let bottom = slot.segment_id.map(|segment| {
            let next = context.find_content_slot(y, Clear::None, Some(segment));
            match next.segment_id {
                Some(_) => next.y,
                None => context.cleared_threshold(Clear::Both).unwrap_or(next.y),
            }
        });
        Band {
            top: slot.y - top,
            left: slot.x - left,
            width: slot.width,
            bottom: bottom.map(|bottom| bottom - top),
        }
    };
    layout_text(tree, node, inputs, &content, Some(&bands))
}

/// Where the baseline of an atomic box lies below its border box's top:
/// that of its first line, or the first row of a table; a text field's
/// line of text, in the middle of its content box, as Chromium places it; a
/// replaced element or a box without lines, or one that clips its content,
/// sits on its bottom edge.
fn atom_baseline(node: &Node, output: &LayoutOutput) -> f32 {
    let bottom = output.size.height;
    let clips = node.style.overflow.x != taffy::Overflow::Visible
        || node.style.overflow.y != taffy::Overflow::Visible;
    if let Some((line, above)) = node.text_line {
        let top = node
            .style
            .padding
            .top
            .resolve_or_zero(None, CalcStore::resolve)
            + node
                .style
                .border
                .top
                .resolve_or_zero(None, CalcStore::resolve);
        let below = node
            .style
            .padding
            .bottom
            .resolve_or_zero(None, CalcStore::resolve)
            + node
                .style
                .border
                .bottom
                .resolve_or_zero(None, CalcStore::resolve);
        let content = bottom - top - below;
        return top + (content - line) / 2.0 + above;
    }
    match node.kind {
        Kind::Leaf { .. } => bottom,
        Kind::Container(Mode::Block) if clips => bottom,
        _ => output.baselines.first.unwrap_or(bottom),
    }
}

fn measure_leaf(
    width: f32,
    height: f32,
    ratio: Option<f32>,
    known: Size<Option<f32>>,
    available: AvailableSpace,
) -> Size<f32> {
    match (known.width, known.height, ratio) {
        (Some(known_width), None, Some(ratio)) => Size {
            width: known_width,
            height: known_width / ratio,
        },
        (None, Some(known_height), Some(ratio)) => Size {
            width: known_height * ratio,
            height: known_height,
        },
        (None, None, Some(ratio)) if width == 0.0 && height == 0.0 => {
            // Ratio-only SVGs have no intrinsic dimensions. Definite
            // containing-block space sizes them without a fictitious
            // 300px minimum; intrinsic queries retain a bounded fallback.
            let width = match available {
                AvailableSpace::Definite(width) => width.max(0.0),
                AvailableSpace::MinContent => 0.0,
                AvailableSpace::MaxContent => 300.0,
            };
            Size {
                width,
                height: width / ratio,
            }
        }
        _ => Size {
            width: known.width.unwrap_or(width),
            height: known.height.unwrap_or(height),
        },
    }
}

impl LayoutPartialTree for Tree<'_> {
    type CoreContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    type CustomIdent = String;

    fn get_core_container_style(&self, node: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        &self.nodes[usize::from(node)].style
    }

    fn resolve_calc_value(&self, value: *const (), basis: f32) -> f32 {
        CalcStore::resolve(value, basis)
    }

    fn set_unrounded_layout(&mut self, node: taffy::NodeId, layout: &Layout) {
        self.nodes[usize::from(node)].unrounded = *layout;
    }

    fn compute_child_layout(&mut self, node: taffy::NodeId, inputs: LayoutInput) -> LayoutOutput {
        // Lines broken beside floats in an earlier layout do not hold
        // without them, even when the layout comes from the cache.
        if inputs.run_mode == RunMode::PerformLayout {
            self.nodes[usize::from(node)].float_lines = None;
        }
        compute_cached_layout(self, node, inputs, |tree, node, inputs| {
            // The children's percentage widths are cyclic while this box
            // finds its min-content width.
            let compressing = inputs.known_dimensions.width.is_none()
                && inputs.available_space.width == AvailableSpace::MinContent;
            let outer = core::mem::replace(&mut tree.compressing, compressing);
            let output = compute_node(tree, node, inputs);
            tree.compressing = outer;
            output
        })
    }
}

/// Lays out a node by its kind.
fn compute_node(tree: &mut Tree<'_>, node: taffy::NodeId, inputs: LayoutInput) -> LayoutOutput {
    let index = usize::from(node);
    match tree.nodes[index].kind.clone() {
        Kind::Container(Mode::Block) => compute_block_layout(tree, node, inputs, None),
        Kind::Container(Mode::Flex) => compute_flexbox_layout(tree, node, inputs),
        Kind::Container(Mode::Grid) => match table_columns(tree, index, &inputs) {
            // A table sizes its columns as tables do, then lays its
            // grid out at those widths.
            Some(widths) => {
                let tracks = widths
                    .into_iter()
                    .map(|width| GridTemplateComponent::Single(taffy::style_helpers::length(width)))
                    .collect();
                let automatic =
                    core::mem::replace(&mut tree.nodes[index].style.grid_template_columns, tracks);
                let output = compute_grid_layout(tree, node, inputs);
                tree.nodes[index].style.grid_template_columns = automatic;
                output
            }
            None => compute_grid_layout(tree, node, inputs),
        },
        Kind::Inline(content) => layout_text(tree, node, inputs, &content, None),
        Kind::Leaf {
            width,
            height,
            ratio,
            compressible,
        } => {
            // Without a containing block width, a replaced element's
            // percentages are cyclic here too.
            let style = match inputs.parent_size.width {
                Some(_) => &tree.nodes[index].style,
                None => tree.item_style(node),
            };
            let output =
                compute_leaf_layout(inputs, style, CalcStore::resolve, |known, available| {
                    let width = if compressible
                        && known.width.is_none()
                        && available.width == AvailableSpace::MinContent
                    {
                        0.0
                    } else {
                        width
                    };
                    measure_leaf(width, height, ratio, known, available.width)
                });
            if !tree.nodes[index].children.is_empty() {
                // Positioned author decorations use this control's
                // already measured box without changing its size.
                compute_block_layout(
                    tree,
                    node,
                    LayoutInput {
                        known_dimensions: output.size.map(Some),
                        ..inputs
                    },
                    None,
                );
            }
            output
        }
    }
}

impl CacheTree for Tree<'_> {
    fn cache_get(&mut self, node: taffy::NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.nodes[usize::from(node)].cache.get(inputs)
    }

    fn cache_store(&mut self, node: taffy::NodeId, inputs: &LayoutInput, output: LayoutOutput) {
        self.nodes[usize::from(node)].cache.store(inputs, output);
    }

    fn cache_clear(&mut self, node: taffy::NodeId) {
        self.nodes[usize::from(node)].cache.clear();
    }
}

impl LayoutFlexboxContainer for Tree<'_> {
    type FlexboxContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    type FlexboxItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node: taffy::NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.nodes[usize::from(node)].style
    }

    fn get_flexbox_child_style(&self, child: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
        self.item_style(child)
    }
}

impl LayoutGridContainer for Tree<'_> {
    type GridContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    type GridItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        &self.nodes[usize::from(node)].style
    }

    fn get_grid_child_style(&self, child: taffy::NodeId) -> Self::GridItemStyle<'_> {
        self.item_style(child)
    }
}

impl LayoutBlockContainer for Tree<'_> {
    type BlockContainerStyle<'a>
        = &'a Style
    where
        Self: 'a;

    type BlockItemStyle<'a>
        = &'a Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node: taffy::NodeId) -> Self::BlockContainerStyle<'_> {
        &self.nodes[usize::from(node)].style
    }

    fn get_block_child_style(&self, child: taffy::NodeId) -> Self::BlockItemStyle<'_> {
        self.item_style(child)
    }

    /// Block children lay out in their parent's block formatting context,
    /// unless they start their own: an empty block's margins collapse
    /// through it, and floats are the context's, not each block's.
    fn compute_block_child_layout(
        &mut self,
        node: taffy::NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        match block_ctx {
            Some(context)
                if matches!(
                    self.nodes[usize::from(node)].kind,
                    Kind::Container(Mode::Block)
                ) =>
            {
                compute_cached_layout(self, node, inputs, |tree, node, inputs| {
                    compute_block_layout(tree, node, inputs, Some(context))
                })
            }
            // Text beside floats breaks around them; the cache does not know
            // the floats.
            Some(context)
                if matches!(self.nodes[usize::from(node)].kind, Kind::Inline(_))
                    && context.has_active_floats(0.0) =>
            {
                layout_beside_floats(self, node, inputs, context)
            }
            _ => self.compute_child_layout(node, inputs),
        }
    }
}

impl RoundTree for Tree<'_> {
    fn get_unrounded_layout(&self, node: taffy::NodeId) -> Layout {
        self.nodes[usize::from(node)].unrounded
    }

    fn set_final_layout(&mut self, node: taffy::NodeId, layout: &Layout) {
        self.nodes[usize::from(node)].layout = *layout;
    }
}

/// How collected content flows: the parent's layout mode, the containing
/// blocks for absolute and fixed boxes, and inherited text decorations.
#[derive(Debug, Clone, Copy)]
struct Flow {
    mode: Mode,
    context: (usize, usize),
    lines: (bool, bool),
    /// The nearest positioned inline element around the content, which
    /// absolutely positioned descendants are placed against.
    anchor: Option<NodeId>,
}

/// An item collected from the children of a box.
enum Item {
    /// A block-level box.
    Block(usize),
    /// An atomic inline-level box such as a control or inline-block, with
    /// the styles of the text around it.
    Atomic(usize, Segment),
    /// Inline text.
    Text(Segment),
    /// Where an absolutely positioned box would be in the flow: its static
    /// position when its containing block is the box being built.
    Absolute(usize),
}

struct Builder<'a> {
    dom: &'a Dom,
    styles: &'a Styles,
    options: &'a LayoutOptions<'a>,
    calcs: CalcStore,
    nodes: Vec<Node>,
    deferred: Vec<(usize, usize)>,
    /// Absolutely positioned boxes and the positioned inline element they
    /// are placed against, instead of their containing block's box.
    anchored: HashMap<usize, NodeId>,
    /// Absolutely positioned boxes whose containing block is not their
    /// parent, with the empty box marking their static position in the
    /// parent's flow and whether it places them across and down.
    statics: HashMap<usize, (usize, bool, bool)>,
    /// Generated layout nodes and their originating elements, with the
    /// pseudo-element of empty boxes, whose decorations paint natively.
    generated: HashMap<usize, (NodeId, Option<PseudoElement>)>,
    /// Grid placements of table cells and captions: row line, row span,
    /// column span and column line, where a span of `0` covers every column.
    cells: HashMap<NodeId, (i16, u16, u16, i16)>,
    /// Boxes in table rows that are not cells, each in an anonymous cell
    /// that keeps it at its own height.
    anonymous_cells: HashSet<NodeId>,
    /// The inline boxes around the text being collected, outermost first.
    boxes: Vec<Rc<InlineBox>>,
    /// How far the baseline of the text being collected is raised by the
    /// `vertical-align` of the inline elements around it.
    shift: f32,
    /// The bidirectional embedding level of the text being collected, and
    /// whether `bidi-override` forces its direction.
    bidi: (u8, bool),
    /// The natural sizes of background images and image elements' pictures.
    images: HashMap<String, (f32, f32)>,
    /// The counters generated content shows, resolved when first needed.
    counters: core::cell::OnceCell<crate::counters::Resolved>,
}

/// A length that does not depend on a percentage basis, as inline boxes'
/// margins and padding mostly are; percentages count as zero.
pub(crate) fn fixed(length: &nativeboat_css::values::LengthPercentage) -> f32 {
    match length {
        nativeboat_css::values::LengthPercentage::Px(pixels) => *pixels,
        _ => 0.0,
    }
}

/// The box of an inline element when it shows: a background, border,
/// shadow, padding or horizontal margin.
fn inline_box(node: NodeId, style: &ComputedStyle, shift: f32) -> Option<InlineBox> {
    let border = |width: Px, kind: BorderStyle| {
        if matches!(kind, BorderStyle::None | BorderStyle::Hidden) {
            0.0
        } else {
            width.0
        }
    };
    let widths = &style.border_width;
    let kinds = &style.border_style;
    let border = [
        border(widths.top, kinds.top),
        border(widths.right, kinds.right),
        border(widths.bottom, kinds.bottom),
        border(widths.left, kinds.left),
    ];
    let padding = [
        fixed(&style.padding.top),
        fixed(&style.padding.right),
        fixed(&style.padding.bottom),
        fixed(&style.padding.left),
    ];
    let margin = |side: &CssLengthAuto| match side {
        CssLengthAuto::Length(length) => fixed(length),
        CssLengthAuto::Auto => 0.0,
    };
    let margin = [margin(&style.margin.left), margin(&style.margin.right)];
    // A positioned inline also places the absolute boxes inside it.
    let shows = style.position != PositionKind::Static
        || !style.background_color.is_transparent()
        || style.background_image != BackgroundImage::None
        || !style.box_shadow.0.is_empty()
        || border
            .iter()
            .chain(&padding)
            .chain(&margin)
            .any(|width| *width != 0.0);
    shows.then(|| InlineBox {
        shift,
        node,
        font: font_of(style),
        margin,
        above: border[0] + padding[0],
        below: border[2] + padding[2],
        rtl: style.direction == Direction::Rtl,
    })
}

/// The marker text of a list item with an ordinal.
fn marker_text(kind: ListStyleType, ordinal: i64) -> Option<String> {
    let text = crate::counters::counter_text(kind, ordinal);
    Some(match kind {
        ListStyleType::None => return None,
        ListStyleType::Disc
        | ListStyleType::Circle
        | ListStyleType::Square
        | ListStyleType::DisclosureClosed
        | ListStyleType::DisclosureOpen => text,
        _ => format!("{text}."),
    })
}

fn decoration(style: &ComputedStyle) -> (bool, bool) {
    (
        style.text_decoration_line.underline,
        style.text_decoration_line.line_through,
    )
}

impl Builder<'_> {
    fn add(&mut self, dom: Option<NodeId>, kind: Kind, style: Style) -> usize {
        self.nodes.push(Node {
            dom,
            kind,
            style,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded: Layout::with_order(0),
            layout: Layout::with_order(0),
            atom_sizes: Vec::new(),
            lines: core::cell::RefCell::new(None),
            float_lines: None,
            cell_width: None,
            fixed_table: false,
            text_line: None,
            compressed: None,
        });
        self.nodes.len() - 1
    }

    fn segment(
        &self,
        text: &str,
        style: &ComputedStyle,
        node: NodeId,
        element: NodeId,
        lines: (bool, bool),
        after_space: &mut bool,
    ) -> Segment {
        let text = transform(text, style.text_transform);
        // Tab stops are `tab-size` spaces apart, with letter spacing.
        let tab = if text.contains('\t') && !style.white_space.collapses_spaces() {
            let font = font_of(style);
            let space = if font.size > 0.0 {
                self.options.measurer.width(" ", &font)
            } else {
                0.0
            } + style.letter_spacing.0;
            match style.tab_size {
                nativeboat_css::types::TabSize::Spaces(count) => (count * space, space),
                nativeboat_css::types::TabSize::Length(length) => (length, space),
            }
        } else {
            (0.0, 0.0)
        };
        Segment {
            tab,
            pieces: pieces(&text, style.white_space, after_space),
            font: font_of(style),
            line_height: line_height(style, self.options.measurer),
            normal_line: style.line_height == LineHeight::Normal,
            // Glyphs fill in `-webkit-text-fill-color`; decorations, drawn
            // with them, take it too.
            color: style.text_fill_color.0.unwrap_or(style.color),
            underline: lines.0,
            underline_offset: style.text_underline_offset.0,
            line_through: lines.1,
            letter_spacing: style.letter_spacing.0,
            white_space: style.white_space,
            word_breaking: match (style.word_break, style.overflow_wrap) {
                (WordBreak::BreakAll, _) => WordBreaking::All,
                (WordBreak::BreakWord, _) | (_, OverflowWrap::Anywhere) => WordBreaking::Anywhere,
                (_, OverflowWrap::BreakWord) => WordBreaking::Overflowing,
                _ => WordBreaking::Normal,
            },
            node,
            element,
            boxes: Rc::from(self.boxes.as_slice()),
            edge: None,
            symbol: None,
            shift: self.shift,
            level: self.bidi.0,
            overridden: self.bidi.1,
        }
    }

    /// The space an inline box takes at an edge, in the current boxes.
    fn gap(
        &self,
        width: f32,
        edge: Edge,
        style: &ComputedStyle,
        node: NodeId,
        lines: (bool, bool),
    ) -> Segment {
        let mut after_space = false;
        Segment {
            pieces: vec![crate::text::Piece::Gap(width)],
            edge: Some(edge),
            ..self.segment("", style, node, node, lines, &mut after_space)
        }
    }

    /// The number of a list item among its siblings, following `start`,
    /// `reversed` and `value` attributes.
    fn ordinal(&self, item: NodeId) -> i64 {
        let parent = self.dom.parent(item).unwrap_or(item);
        let items: Vec<NodeId> = self
            .dom
            .child_elements(parent)
            .into_iter()
            .filter(|node| {
                self.styles
                    .get(*node)
                    .is_some_and(|style| style.display == Display::ListItem)
            })
            .collect();
        let number = |node: NodeId, name: &str| {
            self.dom
                .get_attribute(node, name)
                .and_then(|value| value.trim().parse::<i64>().ok())
        };
        let reversed = self.dom.get_attribute(parent, "reversed").is_some();
        let step = if reversed { -1 } else { 1 };
        let first = if reversed { items.len() as i64 } else { 1 };
        let mut value = number(parent, "start").unwrap_or(first);
        for node in items {
            value = number(node, "value").unwrap_or(value);
            if node == item {
                break;
            }
            value += step;
        }
        value
    }

    /// The marker of a list item, including CSS disclosure marker types.
    fn marker(
        &self,
        item: NodeId,
        style: &ComputedStyle,
        after_space: &mut bool,
    ) -> Option<Segment> {
        let text = marker_text(style.list_style_type, self.ordinal(item))?;
        // Markers end with a space, as browsers' counter styles do.
        let text = format!("{text}\u{a0}");
        // `::marker` rules restyle the marker's text: its color and font.
        let marker_style = self.styles.pseudo(item, &PseudoElement::Marker).cloned();
        let text_style = marker_style.as_deref().unwrap_or(style);
        let symbol = match style.list_style_type {
            ListStyleType::Disc => Some(Symbol::Disc),
            ListStyleType::Circle => Some(Symbol::Circle),
            ListStyleType::Square => Some(Symbol::Square),
            ListStyleType::DisclosureClosed => Some(Symbol::DisclosureClosed),
            ListStyleType::DisclosureOpen => Some(Symbol::DisclosureOpen),
            _ => None,
        };
        Some(Segment {
            symbol,
            ..self.segment(&text, text_style, item, item, (false, false), after_space)
        })
    }

    /// An outside marker: a box just left of the item's first line.
    fn outside_marker(&mut self, segment: Segment, style: &ComputedStyle) -> Option<usize> {
        // A symbol starts two thirds of the ascent and 8px before the
        // content, as Chromium places it; text ends where the content starts.
        let symbol_start = segment
            .symbol
            .filter(|symbol| matches!(symbol, Symbol::Disc | Symbol::Circle | Symbol::Square))
            .map(|_| {
                let ascent = self.options.measurer.metrics(&segment.font).ascent;
                (ascent * 2.0 / 3.0) as i32 as f32 + 8.0
            });
        let marker = self.inline_leaf(vec![segment], style)?;
        let node = &mut self.nodes[marker].style;
        node.position = Position::Absolute;
        node.inset = TaffyRect {
            left: symbol_start.map_or(LengthPercentageAuto::auto(), |start| {
                LengthPercentageAuto::length(-start)
            }),
            right: if symbol_start.is_some() {
                LengthPercentageAuto::auto()
            } else {
                LengthPercentageAuto::percent(1.0)
            },
            top: LengthPercentageAuto::length(0.0),
            bottom: LengthPercentageAuto::auto(),
        };
        Some(marker)
    }

    /// Gives an `<svg>` leaf a box for each of its `<foreignObject>`s, placed
    /// where the SVG's transforms put it, holding its HTML content. The
    /// SVG's own size is taken from its attributes or `viewBox`.
    fn foreign_objects(&mut self, svg: NodeId, index: usize, context: (usize, usize)) {
        let attribute = |name: &str| {
            self.dom
                .get_attribute(svg, name)
                .filter(|value| !value.trim().ends_with('%'))
                .and_then(|value| value.trim().trim_end_matches("px").parse::<f32>().ok())
        };
        let view_box = crate::svg::numbers(self.dom.get_attribute(svg, "viewBox").unwrap_or(""));
        let (view_width, view_height) = match *view_box.as_slice() {
            [_, _, width, height] if width > 0.0 && height > 0.0 => (Some(width), Some(height)),
            _ => (None, None),
        };
        let size = (
            attribute("width").or(view_width).unwrap_or(300.0),
            attribute("height").or(view_height).unwrap_or(150.0),
        );
        let mut children = Vec::new();
        for (object, area) in crate::svg::foreign_objects(self.dom, self.styles, svg, size) {
            let Some(style) = self.styles.get(object).cloned() else {
                continue;
            };
            let child = self.element(object, &style, true, (index, context.1));
            let placed = &mut self.nodes[child].style;
            placed.position = Position::Absolute;
            placed.inset = TaffyRect {
                left: LengthPercentageAuto::length(area.x),
                top: LengthPercentageAuto::length(area.y),
                right: LengthPercentageAuto::auto(),
                bottom: LengthPercentageAuto::auto(),
            };
            placed.size = Size {
                width: taffy::Dimension::length(area.width),
                height: taffy::Dimension::length(area.height),
            };
            children.push(child);
        }
        self.nodes[index].children = children;
    }

    /// Whether the children of an element are all table rows, row groups,
    /// cells or captions, which browsers wrap in an anonymous table.
    fn holds_table_parts(&self, node: NodeId) -> bool {
        let mut parts = false;
        for child in self.dom.child_elements(node) {
            match self.styles.get(child).map(|style| style.display) {
                None | Some(Display::None) => {}
                Some(
                    Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableRow
                    | Display::TableCaption,
                ) => parts = true,
                Some(_) => return false,
            }
        }
        parts
    }

    /// Lays a table out as a grid with one auto-sized column per cell
    /// position: records where each cell goes, and each caption when they
    /// sit inside the table's grid rather than beside it, and returns the
    /// number of columns.
    fn table(&mut self, table: NodeId, beside: bool) -> usize {
        let display = |node: NodeId| self.styles.get(node).map(|style| style.display);
        let rows = |group: NodeId| -> Vec<NodeId> {
            self.dom
                .child_elements(group)
                .into_iter()
                .filter(|row| display(*row) == Some(Display::TableRow))
                .collect()
        };
        let (mut captions, mut heads, mut bodies, mut feet) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for child in self.dom.child_elements(table) {
            match display(child) {
                Some(Display::TableCaption) => captions.push(child),
                Some(Display::TableHeaderGroup) => heads.extend(rows(child)),
                Some(Display::TableFooterGroup) => feet.extend(rows(child)),
                Some(Display::TableRowGroup) => bodies.extend(rows(child)),
                Some(Display::TableRow) => bodies.push(child),
                _ => {}
            }
        }
        if beside {
            captions.clear();
        }
        let first = if captions.is_empty() { 1 } else { 2 };
        let mut columns = 1;
        let mut placements = Vec::new();
        // The rows each column is covered until by row-spanning cells.
        let mut covered: Vec<usize> = Vec::new();
        for (index, row) in heads.iter().chain(&bodies).chain(&feet).enumerate() {
            let mut column = 0;
            for cell in self.dom.child_elements(*row) {
                match display(cell) {
                    Some(Display::TableCell) => {}
                    // Other boxes sit in anonymous cells.
                    Some(
                        Display::None
                        | Display::Contents
                        | Display::TableColumn
                        | Display::TableColumnGroup
                        | Display::TableRow
                        | Display::TableRowGroup
                        | Display::TableHeaderGroup
                        | Display::TableFooterGroup
                        | Display::TableCaption,
                    )
                    | None => continue,
                    Some(_) => {
                        self.anonymous_cells.insert(cell);
                    }
                }
                let span = |name: &str| {
                    self.dom
                        .get_attribute(cell, name)
                        .and_then(|value| value.trim().parse::<u16>().ok())
                        .unwrap_or(1)
                        .clamp(1, 1000)
                };
                let (down, across) = (span("rowspan"), span("colspan"));
                while covered.get(column).is_some_and(|until| *until > index) {
                    column += 1;
                }
                let end = column + usize::from(across);
                if covered.len() < end {
                    covered.resize(end, 0);
                }
                for until in &mut covered[column..end] {
                    *until = index + usize::from(down);
                }
                placements.push((
                    cell,
                    (first + index as i16, down, across, column as i16 + 1),
                ));
                column = end;
            }
            columns = columns.max(covered.len());
        }
        placements.extend(captions.into_iter().map(|caption| (caption, (1, 1, 0, 1))));
        self.cells.extend(placements);
        columns
    }

    /// The widths a table's `<col>` and `<colgroup>` elements give its
    /// columns, in order, each repeated by its `span`; `None` for columns
    /// they leave automatic.
    fn column_widths(
        &self,
        table: NodeId,
    ) -> Vec<Option<nativeboat_css::values::LengthPercentage>> {
        let span = |node: NodeId| {
            self.dom
                .get_attribute(node, "span")
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 1000)
        };
        let width = |node: NodeId| match self.styles.get(node).map(|style| &style.width) {
            Some(nativeboat_css::types::Size::Length(length)) => Some(length.clone()),
            _ => None,
        };
        let display = |node: NodeId| self.styles.get(node).map(|style| style.display);
        let mut widths = Vec::new();
        for child in self.dom.child_elements(table) {
            match display(child) {
                Some(Display::TableColumn) => {
                    widths.extend(core::iter::repeat_n(width(child), span(child)));
                }
                Some(Display::TableColumnGroup) => {
                    let columns: Vec<NodeId> = self
                        .dom
                        .child_elements(child)
                        .into_iter()
                        .filter(|column| display(*column) == Some(Display::TableColumn))
                        .collect();
                    if columns.is_empty() {
                        widths.extend(core::iter::repeat_n(width(child), span(child)));
                    }
                    for column in columns {
                        let own = width(column).or_else(|| width(child));
                        widths.extend(core::iter::repeat_n(own, span(column)));
                    }
                }
                _ => {}
            }
        }
        widths
    }

    fn generated(
        &self,
        element: NodeId,
        pseudo: &PseudoElement,
        lines: (bool, bool),
        after_space: &mut bool,
    ) -> Option<(Segment, alloc::rc::Rc<ComputedStyle>)> {
        let style = self.styles.pseudo(element, pseudo)?;
        if style.display == Display::None {
            return None;
        }
        let Content::Items(items) = &style.content else {
            return None;
        };
        let mut counters = crate::counters::shows_counters(items)
            .then(|| {
                let key = u8::from(*pseudo == PseudoElement::After);
                self.counters
                    .get_or_init(|| crate::counters::resolve(self.dom, self.styles))
                    .get(&(element, key))
            })
            .flatten()
            .into_iter()
            .flatten();
        let text: String = items
            .iter()
            .map(|item| match item {
                ContentItem::Text(text) => text.clone(),
                ContentItem::Attr(name) => self
                    .dom
                    .get_attribute(element, name)
                    .unwrap_or("")
                    .to_owned(),
                ContentItem::Counter(..) | ContentItem::Counters(..) => {
                    counters.next().cloned().unwrap_or_default()
                }
            })
            .collect();
        let own = decoration(style);
        Some((
            self.segment(
                &text,
                style,
                element,
                element,
                (lines.0 || own.0, lines.1 || own.1),
                after_space,
            ),
            style.clone(),
        ))
    }

    /// Collects generated text once, or gives empty positioned content native
    /// leaf geometry under the ordinary containing-block rules.
    fn collect_generated(
        &mut self,
        element: NodeId,
        pseudo: &PseudoElement,
        flow: Flow,
        after_space: &mut bool,
        items: &mut Vec<Item>,
    ) {
        let Some((segment, style)) = self.generated(element, pseudo, flow.lines, after_space)
        else {
            return;
        };
        if !matches!(style.position, PositionKind::Absolute | PositionKind::Fixed) {
            // Block-level generated content is a block of its own, with its
            // own alignment, padding and box, as Prism's line numbers are.
            if matches!(
                style.display,
                Display::Block | Display::ListItem | Display::Flex | Display::Grid
            ) {
                let mode = mode_of(style.display);
                let taffy = convert(&style, mode, &mut self.calcs);
                let block = self.add(None, Kind::Container(mode), taffy);
                self.nodes[block].children = self
                    .inline_leaf(vec![segment], &style)
                    .into_iter()
                    .collect();
                // The box paints its background and borders, its text with
                // it.
                self.generated
                    .insert(block, (element, Some(pseudo.clone())));
                items.push(Item::Block(block));
                return;
            }
            items.push(Item::Text(segment));
            return;
        }
        let taffy = convert(&style, Mode::Block, &mut self.calcs);
        let child = if segment.pieces.is_empty() {
            let leaf = self.add(
                None,
                Kind::Leaf {
                    width: 0.0,
                    height: 0.0,
                    ratio: None,
                    compressible: false,
                },
                taffy,
            );
            self.generated.insert(leaf, (element, Some(pseudo.clone())));
            leaf
        } else {
            // Positioned generated text is a block of its own, out of the
            // flow and placed by its insets, painted as a layer with its
            // text.
            let block = self.add(None, Kind::Container(Mode::Block), taffy);
            self.nodes[block].children = self
                .inline_leaf(vec![segment], &style)
                .into_iter()
                .collect();
            self.generated
                .insert(block, (element, Some(pseudo.clone())));
            block
        };
        let target = if style.position == PositionKind::Fixed {
            flow.context.1
        } else {
            if let Some(anchor) = flow.anchor {
                self.anchored.insert(child, anchor);
            }
            flow.context.0
        };
        self.deferred.push((target, child));
        items.push(Item::Absolute(child));
    }

    /// Whether an inline element contains block-level or atomic content.
    fn has_box_content(&self, element: NodeId) -> bool {
        self.child_element_nodes(element).any(|child| {
            let Some(style) = self.styles.get(child) else {
                return false;
            };
            // Absolutely positioned boxes are out of the inline's flow.
            if matches!(style.position, PositionKind::Absolute | PositionKind::Fixed) {
                return false;
            }
            match style.display {
                Display::None => false,
                Display::Inline | Display::Contents => {
                    self.is_replaced(child) || self.has_box_content(child)
                }
                _ => true,
            }
        })
    }

    /// Whether an element holds block-level boxes, directly or through
    /// inline elements: an inline element around them is split by them.
    fn has_block_content(&self, element: NodeId) -> bool {
        self.child_element_nodes(element).any(|child| {
            let Some(style) = self.styles.get(child) else {
                return false;
            };
            match style.display {
                Display::None => false,
                Display::Inline | Display::Contents => {
                    !self.is_replaced(child) && self.has_block_content(child)
                }
                // Out-of-flow boxes, floated or positioned, split nothing.
                display => {
                    !display.is_inline_level()
                        && style.float == nativeboat_css::types::Float::None
                        && !matches!(style.position, PositionKind::Absolute | PositionKind::Fixed)
                }
            }
        })
    }

    /// An element's child elements in the flat tree.
    fn child_element_nodes(&self, element: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.dom.flat_child_elements(element).into_iter()
    }

    fn is_replaced(&self, element: NodeId) -> bool {
        let name = self.dom.local_name(element).unwrap_or("");
        matches!(
            name,
            "img" | "svg" | "canvas" | "video" | "iframe" | "input" | "select"
        ) || native_control(
            self.dom,
            element,
            self.styles.get(element).map(alloc::rc::Rc::as_ref),
        )
        .is_some()
    }

    /// A `<select>` the page draws: its selected option, then room for the
    /// arrow, as wide as its widest option needs.
    fn drawn_select(&mut self, node: NodeId, style: &ComputedStyle) -> usize {
        let taffy = convert(style, Mode::Block, &mut self.calcs);
        let index = self.add(Some(node), Kind::Container(Mode::Block), taffy);
        let font = font_of(style);
        let measurer = self.options.measurer;
        let labels: Vec<String> = self
            .dom
            .options(node)
            .into_iter()
            .map(|option| collapse(&self.dom.text_content(option).unwrap_or_default()))
            .collect();
        let label = select_label(self.dom, node);
        let widest = labels
            .iter()
            .map(|label| measurer.width(label, &font))
            .fold(0.0, f32::max);
        let mut after_space = true;
        let segment = self.segment(
            &label,
            style,
            node,
            node,
            decoration(style),
            &mut after_space,
        );
        // At `line-height: normal` the label's line is the primary font's
        // ascent and descent without its line gap, as Blink's menu list
        // inner element sets it.
        let line = if style.line_height == LineHeight::Normal {
            let metrics = measurer.metrics(&font);
            metrics.ascent + metrics.descent
        } else {
            style.used_line_height()
        };
        let text = match self.inline_leaf(vec![segment], style) {
            Some(leaf) => leaf,
            None => self.add(
                None,
                Kind::Leaf {
                    width: 0.0,
                    height: line,
                    ratio: None,
                    compressible: false,
                },
                Style::DEFAULT,
            ),
        };
        self.nodes[text].style.flex_grow = 1.0;
        let room = SELECT_ARROW + (widest - measurer.width(&label, &font)).max(0.0);
        let arrow = self.add(
            None,
            Kind::Leaf {
                width: room,
                height: 0.0,
                ratio: None,
                compressible: false,
            },
            Style {
                flex_shrink: 0.0,
                ..Style::DEFAULT
            },
        );
        // The label's line is as tall as the primary font's, whatever fonts
        // its text falls back to, inside a device pixel per zoom step above
        // and below while the select keeps its appearance, as Blink sizes a
        // menu list's inner box.
        let padding = if style.appearance_none {
            0.0
        } else {
            match measurer.device_pixel() {
                pixel if pixel > 0.0 => crate::math::floor(1.0 / pixel + 1e-3) * pixel,
                _ => 1.0,
            }
        };
        let row = self.add(
            None,
            Kind::Container(Mode::Flex),
            Style {
                display: TaffyDisplay::Flex,
                align_items: Some(AlignItems::CENTER),
                size: taffy::Size {
                    width: taffy::Dimension::auto(),
                    height: taffy::Dimension::length(line + 2.0 * padding),
                },
                ..Style::DEFAULT
            },
        );
        self.nodes[row].children = vec![text, arrow];
        let inner = self.center_button(index, vec![row]);
        self.nodes[index].children = vec![inner];
        index
    }

    /// Puts a button's content into an anonymous block centered vertically,
    /// as browsers lay out buttons.
    fn center_button(&mut self, index: usize, children: Vec<usize>) -> usize {
        self.align_content(index, children, JustifyContent::CENTER)
    }

    /// Puts a box's content into an anonymous block placed vertically as
    /// `justify` says, for buttons and the `vertical-align` of table cells.
    fn align_content(
        &mut self,
        index: usize,
        children: Vec<usize>,
        justify: JustifyContent,
    ) -> usize {
        let style = Style {
            display: TaffyDisplay::Block,
            ..Style::DEFAULT
        };
        let inner = self.add(None, Kind::Container(Mode::Block), style);
        self.nodes[inner].children = children;
        let node = &mut self.nodes[index];
        node.style.display = TaffyDisplay::Flex;
        node.style.flex_direction = taffy::FlexDirection::Column;
        node.style.flex_wrap = FlexWrap::NoWrap;
        node.style.justify_content = Some(justify);
        node.style.align_items = Some(AlignItems::STRETCH);
        node.kind = Kind::Container(Mode::Flex);
        inner
    }

    fn collect(
        &mut self,
        node: NodeId,
        parent_style: &ComputedStyle,
        flow: Flow,
        after_space: &mut bool,
        items: &mut Vec<Item>,
    ) {
        if let Some(parent) = self.dom.parent_element(node)
            && self.dom.local_name(parent) == Some("details")
            && !self.dom.has_attribute(parent, "open")
            && !self.dom.is_disclosure_summary(node)
        {
            return;
        }
        let Flow {
            mode: parent_mode,
            context,
            lines,
            anchor,
        } = flow;
        if let NodeKind::Text(text) = self.dom.kind(node) {
            let element = self.dom.flat_parent(node).unwrap_or(node);
            let segment = self.segment(text, parent_style, node, element, lines, after_space);
            items.push(Item::Text(segment));
            return;
        }
        let Some(style) = self.styles.get(node).cloned() else {
            return;
        };
        if style.display == Display::Inline {
            self.request_background(&style);
        }
        match style.display {
            Display::None | Display::TableColumn | Display::TableColumnGroup => return,
            // Tables lay out as grids of their cells.
            Display::Contents
            | Display::TableRowGroup
            | Display::TableHeaderGroup
            | Display::TableFooterGroup
            | Display::TableRow => {
                for child in self.dom.flat_children(node) {
                    self.collect(child, &style, flow, after_space, items);
                }
                return;
            }
            _ => {}
        }
        if matches!(style.position, PositionKind::Absolute | PositionKind::Fixed) {
            let target = if style.position == PositionKind::Fixed {
                context.1
            } else {
                context.0
            };
            let child = self.element(node, &style, true, context);
            if style.position == PositionKind::Absolute
                && let Some(anchor) = anchor
            {
                self.anchored.insert(child, anchor);
            }
            self.deferred.push((target, child));
            items.push(Item::Absolute(child));
            return;
        }
        let replaced = self.is_replaced(node);
        if parent_mode == Mode::Block
            && style.display == Display::Inline
            && !replaced
            && !self.has_box_content(node)
        {
            let own = decoration(&style);
            let lines = (lines.0 || own.0, lines.1 || own.1);
            let outer_shift = self.shift;
            self.shift += baseline_shift(&style, parent_style, self.options.measurer);
            // An embedding or isolate opens a level in its direction.
            let outer_bidi = self.bidi;
            {
                use nativeboat_css::types::UnicodeBidi;
                let rtl = style.direction == Direction::Rtl;
                match style.unicode_bidi {
                    UnicodeBidi::Normal => {}
                    UnicodeBidi::BidiOverride | UnicodeBidi::IsolateOverride => {
                        self.bidi = (crate::bidi::embed(self.bidi.0, rtl), true);
                    }
                    UnicodeBidi::Embed | UnicodeBidi::Isolate | UnicodeBidi::Plaintext => {
                        self.bidi = (crate::bidi::embed(self.bidi.0, rtl), false);
                    }
                }
            }
            // An element whose box shows takes its margin, border and
            // padding at its start and end, and its text is in its box.
            let boxed = inline_box(node, &style, self.shift);
            let edges = boxed.as_ref().map(|boxed| {
                let widths = &style.border_width;
                let shows =
                    |kind: BorderStyle| !matches!(kind, BorderStyle::None | BorderStyle::Hidden);
                let left = boxed.margin[0]
                    + fixed(&style.padding.left)
                    + if shows(style.border_style.left) {
                        widths.left.0
                    } else {
                        0.0
                    };
                let right = boxed.margin[1]
                    + fixed(&style.padding.right)
                    + if shows(style.border_style.right) {
                        widths.right.0
                    } else {
                        0.0
                    };
                // Right to left, the box starts with its right side.
                if style.direction == Direction::Rtl {
                    (right, left)
                } else {
                    (left, right)
                }
            });
            if let Some(boxed) = boxed {
                self.boxes.push(Rc::new(boxed));
            }
            if let Some((left, _)) = edges {
                items.push(Item::Text(self.gap(left, Edge::Start, &style, node, lines)));
            }
            let anchor = if style.position == PositionKind::Static {
                anchor
            } else {
                Some(node)
            };
            let inner = Flow {
                lines,
                anchor,
                ..flow
            };
            let collected = items.len();
            self.collect_generated(node, &PseudoElement::Before, inner, after_space, items);
            for child in self.dom.flat_children(node) {
                self.collect(child, &style, inner, after_space, items);
            }
            self.collect_generated(node, &PseudoElement::After, inner, after_space, items);
            if self.dom.local_name(node) == Some("br") {
                items.push(Item::Text(Segment {
                    pieces: vec![crate::text::Piece::Break],
                    ..self.segment("", &style, node, node, lines, after_space)
                }));
                *after_space = true;
            }
            // An empty element still has a place on its line: an empty
            // rectangle as tall as its font, as Chromium reports it.
            if items.len() == collected && edges.is_none() {
                let mut after = *after_space;
                items.push(Item::Text(Segment {
                    pieces: vec![crate::text::Piece::Gap(0.0)],
                    ..self.segment("", &style, node, node, lines, &mut after)
                }));
            }
            if let Some((_, right)) = edges {
                items.push(Item::Text(self.gap(right, Edge::End, &style, node, lines)));
                self.boxes.pop();
            }
            self.shift = outer_shift;
            self.bidi = outer_bidi;
            return;
        }
        // An inline element around block-level boxes lays out as a block, as
        // Chromium splits it around them: they take the containing block's
        // width, and the element's box spans them.
        let split = style.display == Display::Inline && !replaced && self.has_block_content(node);
        let child = self.element(node, &style, parent_mode != Mode::Block || split, context);
        *after_space = false;
        if !split
            && (style.display.is_inline_level() || replaced && style.display == Display::Inline)
        {
            let segment = self.segment("", parent_style, node, node, lines, after_space);
            items.push(Item::Atomic(child, segment));
        } else {
            items.push(Item::Block(child));
        }
    }

    fn inline_leaf(&mut self, segments: Vec<Segment>, style: &ComputedStyle) -> Option<usize> {
        self.inline_content(segments, Vec::new(), style)
    }

    /// Inline content of text and atomic boxes, which become its children.
    fn inline_content(
        &mut self,
        segments: Vec<Segment>,
        atoms: Vec<Atom>,
        style: &ComputedStyle,
    ) -> Option<usize> {
        let children = atoms.iter().map(|atom| atom.node).collect();
        let content = InlineContent {
            segments,
            text_align: style.text_align,
            direction: style.direction,
            indent: 0.0,
            strut: line_height(style, self.options.measurer),
            strut_font: font_of(style),
            atoms,
            ellipsis: style.text_overflow == nativeboat_css::types::TextOverflow::Ellipsis
                && style.overflow_x != nativeboat_css::types::Overflow::Visible,
            clamp: matches!(style.display, Display::WebkitBox | Display::WebkitInlineBox)
                .then_some(style.line_clamp.0)
                .flatten()
                .filter(|_| style.box_orient == nativeboat_css::types::BoxOrient::Vertical),
        };
        if content.is_blank() {
            return None;
        }
        // The lines are an anonymous block in their container's block
        // formatting context, beside its floats.
        let style = Style {
            display: TaffyDisplay::Block,
            ..Style::DEFAULT
        };
        let index = self.add(None, Kind::Inline(content), style);
        self.nodes[index].children = children;
        Some(index)
    }

    /// The computed style of a box: its element's, or its generated
    /// content's.
    fn box_style(&self, child: usize) -> Option<&Rc<ComputedStyle>> {
        match self.nodes[child].dom {
            Some(element) => self.styles.get(element),
            None => {
                let (element, pseudo) = self.generated.get(&child)?;
                self.styles.pseudo(*element, pseudo.as_ref()?)
            }
        }
    }

    /// The axes along which an absolutely positioned box's insets are both
    /// automatic, so it sits at its static position along them; `None` when
    /// it has no such axis.
    fn static_axes(&self, child: usize) -> Option<(bool, bool)> {
        let style = self.box_style(child)?;
        let auto = |edge: &CssLengthAuto| matches!(edge, CssLengthAuto::Auto);
        let axes = (
            auto(&style.inset.left) && auto(&style.inset.right),
            auto(&style.inset.top) && auto(&style.inset.bottom),
        );
        (axes.0 || axes.1).then_some(axes)
    }

    /// Whether an absolutely positioned box was inline-level before its
    /// positioning made it block-level.
    fn originally_inline(&self, child: usize) -> bool {
        self.box_style(child).is_some_and(|style| {
            matches!(
                style.original_display,
                Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid
            )
        })
    }

    /// Turns items into layout children: blocks, and runs of text and
    /// atomic boxes between them as inline content. The absolute boxes of
    /// `owner`, a block container, stay in order between its blocks, so
    /// they are statically positioned where they would have been.
    fn group(
        &mut self,
        items: Vec<Item>,
        mode: Mode,
        style: &ComputedStyle,
        owner: Option<usize>,
    ) -> Vec<usize> {
        let mut children = Vec::new();
        let mut run: Vec<Segment> = Vec::new();
        let mut atoms: Vec<Atom> = Vec::new();
        for item in items {
            match item {
                Item::Text(segment) => run.push(segment),
                Item::Atomic(child, mut segment) if mode == Mode::Block => {
                    segment.pieces = vec![crate::text::Piece::Atom(atoms.len())];
                    let align = self.nodes[child]
                        .dom
                        .and_then(|element| self.styles.get(element))
                        .map(|style| style.vertical_align.clone())
                        .unwrap_or_default();
                    atoms.push(Atom { node: child, align });
                    run.push(segment);
                }
                Item::Atomic(child, _) | Item::Block(child) => {
                    let text = core::mem::take(&mut run);
                    let boxes = core::mem::take(&mut atoms);
                    children.extend(self.inline_content(text, boxes, style));
                    children.push(child);
                }
                Item::Absolute(child) => {
                    if mode != Mode::Block {
                        continue;
                    }
                    let between_blocks = run
                        .iter()
                        .flat_map(|segment| &segment.pieces)
                        .all(|piece| matches!(piece, crate::text::Piece::Space(_)))
                        && run.iter().all(|segment| segment.boxes.is_empty());
                    // A box that was inline before it was positioned is
                    // placed where it would have been in its line: after
                    // the text before it, at the line's top. An empty atom
                    // there marks the place without breaking the line.
                    if !between_blocks
                        && self.originally_inline(child)
                        && let Some(last) = run.last()
                        && let Some((across, down)) = self.static_axes(child)
                    {
                        let marker = self.add(
                            None,
                            Kind::Leaf {
                                width: 0.0,
                                height: 0.0,
                                ratio: None,
                                compressible: false,
                            },
                            Style::DEFAULT,
                        );
                        let mut segment = last.clone();
                        segment.pieces = vec![crate::text::Piece::Atom(atoms.len())];
                        segment.edge = None;
                        segment.symbol = None;
                        atoms.push(Atom {
                            node: marker,
                            align: VerticalAlign::Top,
                        });
                        run.push(segment);
                        self.statics.insert(child, (marker, across, down));
                        continue;
                    }
                    if !atoms.is_empty() {
                        continue;
                    }
                    let in_flow =
                        owner.is_some_and(|owner| self.deferred.contains(&(owner, child)));
                    if !between_blocks && in_flow {
                        continue;
                    }
                    if in_flow && let Some(owner) = owner {
                        run.clear();
                        self.deferred.retain(|deferred| *deferred != (owner, child));
                        children.push(child);
                        continue;
                    }
                    // A box whose containing block is further out is placed
                    // where it would have been in this flow, along the axes
                    // its insets leave automatic, as CSS's static position is.
                    let Some((across, down)) = self.static_axes(child) else {
                        continue;
                    };
                    // The box is block-level: it would start below the text
                    // before it.
                    if between_blocks {
                        run.clear();
                    } else {
                        let text = core::mem::take(&mut run);
                        children.extend(self.inline_content(text, Vec::new(), style));
                    }
                    // An empty block, which margins collapse through as
                    // they would without it.
                    let marker = self.add(
                        None,
                        Kind::Leaf {
                            width: 0.0,
                            height: 0.0,
                            ratio: None,
                            compressible: false,
                        },
                        Style {
                            display: TaffyDisplay::Block,
                            ..Style::DEFAULT
                        },
                    );
                    children.push(marker);
                    self.statics.insert(child, (marker, across, down));
                }
            }
        }
        children.extend(self.inline_content(run, atoms, style));
        children
    }

    fn control_size(
        &self,
        kind: ControlKind,
        element: NodeId,
        style: &ComputedStyle,
    ) -> (f32, f32) {
        let font = font_of(style);
        let measurer = self.options.measurer;
        let line = line_height(style, measurer);
        let digit = measurer.width("0", &font);
        let number = |name: &str, default: f32| {
            self.dom
                .get_attribute(element, name)
                .and_then(|value| value.trim().parse::<f32>().ok())
                .filter(|value| *value > 0.0)
                .unwrap_or(default)
        };
        let widest_option = || {
            self.dom
                .options(element)
                .into_iter()
                .map(|option| {
                    measurer.width(
                        self.dom.text_content(option).unwrap_or_default().trim(),
                        &font,
                    )
                })
                .fold(0.0, f32::max)
        };
        match kind {
            ControlKind::Button => {
                let caption = button_caption(self.dom, element);
                (measurer.width(&caption, &font), line)
            }
            ControlKind::TextBox | ControlKind::Password => (number("size", 20.0) * digit, line),
            ControlKind::TextArea => (
                number("cols", 20.0) * digit + 17.0,
                number("rows", 2.0) * line,
            ),
            ControlKind::CheckBox | ControlKind::Radio => (13.0, 13.0),
            ControlKind::ComboBox => (widest_option() + 24.0, line),
            ControlKind::ListBox => (widest_option() + 17.0, number("size", 4.0) * line),
            ControlKind::Slider => (129.0, 16.0),
            ControlKind::ProgressBar => (160.0, 16.0),
            // Room for the value as the picker shows it and its button.
            ControlKind::Date => (measurer.width("00/00/0000", &font) + 30.0, line),
            ControlKind::Time => (measurer.width("00:00 AM", &font) + 24.0, line),
            ControlKind::DateTime => (measurer.width("00/00/0000, 00:00 AM", &font) + 30.0, line),
            ControlKind::Month => (measurer.width("September 0000", &font) + 30.0, line),
        }
    }

    /// The `alt` text of an image that did not load and has no size of its
    /// own.
    fn alternative(&self, element: NodeId) -> Option<String> {
        if self.dom.local_name(element) != Some("img")
            || self.dom.get_attribute(element, "width").is_some()
            || self.dom.get_attribute(element, "height").is_some()
        {
            return None;
        }
        let loaded = self
            .dom
            .get_attribute(element, "src")
            .and_then(|src| (self.options.image_size)(src))
            .is_some();
        let alt = self.dom.get_attribute(element, "alt")?.trim();
        (!loaded && !alt.is_empty()).then(|| alt.to_owned())
    }

    fn image_size(&mut self, element: NodeId) -> (f32, f32, Option<f32>) {
        let attribute = |name: &str| {
            self.dom
                .get_attribute(element, name)
                .and_then(|value| value.trim().trim_end_matches("px").parse::<f32>().ok())
        };
        if self.dom.local_name(element) == Some("svg")
            && attribute("width").is_none()
            && attribute("height").is_none()
            && let [_, _, width, height] =
                *crate::svg::numbers(self.dom.get_attribute(element, "viewBox").unwrap_or(""))
                    .as_slice()
            && width > 0.0
            && height > 0.0
        {
            return (0.0, 0.0, Some(width / height));
        }
        // A frame shows another document, never its `src` as a picture: it
        // is 300×150 unless its attributes size it, with no ratio to keep,
        // as in Chromium.
        if self.dom.local_name(element) == Some("iframe") {
            return (
                attribute("width").unwrap_or(300.0),
                attribute("height").unwrap_or(150.0),
                None,
            );
        }
        let video = self.dom.local_name(element) == Some("video");
        let picture = video
            .then(|| video_picture(self.dom, element))
            .flatten()
            .and_then(|key| Some(((self.options.image_size)(&key)?, key)));
        if video && picture.is_none() {
            // Without a picture yet, a video is 300×150, with no ratio to
            // keep, as in Chromium.
            return (
                attribute("width").unwrap_or(300.0),
                attribute("height").unwrap_or(150.0),
                None,
            );
        }
        let natural = if let Some((size, key)) = picture {
            // Painting fits the picture into the box by its natural size.
            self.images.insert(key, size);
            Some(size)
        } else if self.dom.local_name(element) == Some("svg") {
            // Without a size, an SVG is as wide as a default replaced element
            // and has the proportions of its view box.
            let view_box = self.dom.get_attribute(element, "viewBox").unwrap_or("");
            let ratio = match *crate::svg::numbers(view_box).as_slice() {
                [_, _, width, height] if width > 0.0 && height > 0.0 => width / height,
                _ => 2.0,
            };
            Some((300.0, 300.0 / ratio))
        } else {
            let src = self.dom.get_attribute(element, "src");
            let natural = src.and_then(|src| (self.options.image_size)(src));
            // Painting fits the picture into the box by its natural size.
            if let (Some(src), Some(size)) = (src, natural) {
                self.images.insert(src.to_owned(), size);
            }
            natural
        };
        let ratio = natural
            .filter(|(width, height)| *width > 0.0 && *height > 0.0)
            .map(|(width, height)| width / height);
        let (natural_width, natural_height) = natural.unwrap_or((0.0, 0.0));
        match (attribute("width"), attribute("height"), ratio) {
            (Some(width), Some(height), _) => (width, height, Some(width / height.max(0.001))),
            (Some(width), None, Some(ratio)) => (width, width / ratio, Some(ratio)),
            (None, Some(height), Some(ratio)) => (height * ratio, height, Some(ratio)),
            (width, height, ratio) => (
                width.unwrap_or(natural_width),
                height.unwrap_or(natural_height),
                ratio,
            ),
        }
    }

    /// Builds the box of an element; `blockify` forces block-level display.
    /// Asks for an element's background image, which loads it for painting,
    /// as browsers fetch the images of styled elements.
    fn request_background(&mut self, style: &ComputedStyle) {
        let urls = style.background_image.urls();
        for url in urls.into_iter().chain(style.mask_image.urls()) {
            if !self.images.contains_key(url)
                && let Some(size) = (self.options.image_size)(url)
            {
                self.images.insert(url.to_owned(), size);
            }
        }
    }

    fn element(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        blockify: bool,
        context: (usize, usize),
    ) -> usize {
        self.request_background(style);
        let display = if blockify {
            style.display.blockified()
        } else {
            style.display
        };
        let name = self.dom.local_name(node).unwrap_or("");
        if name == "input" && native_control(self.dom, node, Some(style)).is_none() {
            // A button-like input drawn as a box shows its caption.
            let taffy = convert(style, Mode::Block, &mut self.calcs);
            let index = self.add(Some(node), Kind::Container(Mode::Block), taffy);
            let mut after_space = true;
            let caption = button_caption(self.dom, node);
            let segment = self.segment(
                &caption,
                style,
                node,
                node,
                decoration(style),
                &mut after_space,
            );
            // An empty caption still takes a line.
            let line = Kind::Leaf {
                width: 0.0,
                height: line_height(style, self.options.measurer),
                ratio: None,
                compressible: false,
            };
            let text = match self.inline_leaf(vec![segment], style) {
                Some(leaf) => leaf,
                None => self.add(None, line, Style::DEFAULT),
            };
            let inner = self.center_button(index, vec![text]);
            self.nodes[index].children = vec![inner];
            return index;
        }
        if name == "select"
            && native_control(self.dom, node, Some(style)).is_none()
            && control_for(name, |attribute| self.dom.get_attribute(node, attribute))
                == Some(ControlKind::ComboBox)
        {
            return self.drawn_select(node, style);
        }
        if let Some(kind) = native_control(self.dom, node, Some(style)) {
            let (width, height) = self.control_size(kind, node, style);
            let mut taffy = convert(style, Mode::Block, &mut self.calcs);
            taffy.item_is_replaced = true;
            // Form controls sized by percentages contribute no minimum
            // width, so their containers may shrink them (CSS Sizing's
            // compressible replaced elements).
            let compressible = percentage_width(style);
            let index = self.add(
                Some(node),
                Kind::Leaf {
                    width,
                    height,
                    ratio: None,
                    compressible,
                },
                taffy,
            );
            self.nodes[index].compressed = compressed(&self.nodes[index].style);
            if matches!(
                kind,
                ControlKind::TextBox
                    | ControlKind::Password
                    | ControlKind::ComboBox
                    | ControlKind::Button
            ) {
                let measurer = self.options.measurer;
                let line = line_height(style, measurer);
                let above = above_baseline(measurer, &font_of(style), line);
                self.nodes[index].text_line = Some((line, above));
            }
            if matches!(kind, ControlKind::CheckBox | ControlKind::Radio) && style.author_appearance
            {
                let context = (
                    if style.position != PositionKind::Static {
                        index
                    } else {
                        context.0
                    },
                    context.1,
                );
                let flow = Flow {
                    mode: Mode::Block,
                    context,
                    lines: decoration(style),
                    anchor: None,
                };
                let mut after_space = true;
                let mut items = Vec::new();
                for pseudo in [PseudoElement::Before, PseudoElement::After] {
                    self.collect_generated(node, &pseudo, flow, &mut after_space, &mut items);
                }
                self.nodes[index].children.extend(
                    self.deferred
                        .iter()
                        .filter(|(target, _)| *target == index)
                        .map(|(_, child)| *child),
                );
                self.deferred.retain(|(target, _)| *target != index);
            }
            return index;
        }
        if let Some(alt) = self.alternative(node) {
            // A broken image shows its alternative text, as browsers do.
            let taffy = convert(style, Mode::Block, &mut self.calcs);
            let index = self.add(Some(node), Kind::Container(Mode::Block), taffy);
            let mut after_space = true;
            let segment =
                self.segment(&alt, style, node, node, decoration(style), &mut after_space);
            let text = self.inline_leaf(vec![segment], style);
            self.nodes[index].children.extend(text);
            return index;
        }
        if matches!(name, "img" | "svg" | "canvas" | "video" | "iframe") {
            let (width, height, ratio) = self.image_size(node);
            let mut taffy = convert(style, Mode::Block, &mut self.calcs);
            taffy.item_is_replaced = true;
            let index = self.add(
                Some(node),
                Kind::Leaf {
                    width,
                    height,
                    ratio,
                    compressible: false,
                },
                taffy,
            );
            self.nodes[index].compressed = compressed(&self.nodes[index].style);
            if name == "svg" {
                self.foreign_objects(node, index, context);
            }
            return index;
        }
        let table = display == Display::Table;
        // A vertical `-webkit-box` lays its content out in lines, which
        // `-webkit-line-clamp` counts, as Chromium does.
        let vertical_box = matches!(display, Display::WebkitBox | Display::WebkitInlineBox)
            && style.box_orient == nativeboat_css::types::BoxOrient::Vertical;
        let display_mode = if vertical_box {
            Mode::Block
        } else {
            mode_of(display)
        };
        // Rows and cells in another box sit in an anonymous table inside it,
        // as when a stylesheet makes a `<table>` a block.
        let anonymous_table = !table && display_mode == Mode::Block && self.holds_table_parts(node);
        let mode = if table || anonymous_table {
            Mode::Grid
        } else {
            display_mode
        };
        let own_mode = if anonymous_table { Mode::Block } else { mode };
        let mut taffy = convert(style, own_mode, &mut self.calcs);
        // A button in block flow is as wide as its content when its width
        // is `auto`, whatever its display, as HTML's rendering rules and
        // Blink size buttons; flex and grid items keep their alignment.
        if name == "button"
            && !blockify
            && !display.is_inline_level()
            && !matches!(style.position, PositionKind::Absolute | PositionKind::Fixed)
            && taffy.size.width.is_auto()
        {
            taffy.size.width = taffy::Dimension::fit_content();
        }
        let mut grid = Style::DEFAULT;
        let fixed_table = (table || anonymous_table)
            && style.table_layout == nativeboat_css::types::TableLayout::Fixed
            && !matches!(style.width, nativeboat_css::types::Size::Auto);
        // An in-flow table's captions sit above or below it, outside its
        // border, in a box around it that takes its margins, as in
        // browsers. Floated and absolutely positioned tables keep them in
        // their grid.
        let captions_beside = table
            && style.float == nativeboat_css::types::Float::None
            && matches!(
                style.position,
                PositionKind::Static | PositionKind::Relative
            )
            && self.dom.child_elements(node).into_iter().any(|child| {
                self.styles.get(child).map(|style| style.display) == Some(Display::TableCaption)
            });
        if table || anonymous_table {
            // A table with an automatic width fits its content, in block
            // layout too, instead of stretching.
            let target = if table { &mut taffy } else { &mut grid };
            target.display = TaffyDisplay::Grid;
            target.item_is_table = true;
            let columns = self.table(node, captions_beside);
            // Columns that `<col>` and `<colgroup>` elements size take
            // their widths, as Element Plus sizes its tables' columns.
            let widths = self.column_widths(node);
            target.grid_template_columns = (0..columns.max(widths.len()))
                .map(|index| {
                    let size = widths
                        .get(index)
                        .cloned()
                        .flatten()
                        .map_or(TrackSize::Auto, TrackSize::Length);
                    GridTemplateComponent::Single(self.calcs.track(&size))
                })
                .collect();
            let spacing = match style.border_collapse {
                BorderCollapse::Separate => style.border_spacing,
                BorderCollapse::Collapse => Default::default(),
            };
            target.gap = Size {
                width: LengthPercentage::length(spacing.horizontal.0),
                height: LengthPercentage::length(spacing.vertical.0),
            };
            // The spacing also lies between the table's border and its
            // outer cells.
            if spacing.horizontal.0 > 0.0 || spacing.vertical.0 > 0.0 {
                use nativeboat_css::values::LengthPercentage as CssLength;
                let own = |side: &CssLength| match side {
                    CssLength::Px(px) if table => Some(*px),
                    _ if table => None,
                    _ => Some(0.0),
                };
                let padding = &style.padding;
                for (slot, side, extra) in [
                    (&mut target.padding.top, &padding.top, spacing.vertical.0),
                    (
                        &mut target.padding.bottom,
                        &padding.bottom,
                        spacing.vertical.0,
                    ),
                    (
                        &mut target.padding.left,
                        &padding.left,
                        spacing.horizontal.0,
                    ),
                    (
                        &mut target.padding.right,
                        &padding.right,
                        spacing.horizontal.0,
                    ),
                ] {
                    if let Some(px) = own(side) {
                        *slot = LengthPercentage::length(px + extra);
                    }
                }
            }
        }
        if self.anonymous_cells.contains(&node) {
            taffy.align_self = Some(AlignSelf::START);
        }
        // A cell outside a table row sits in an anonymous table, which fits
        // its content.
        if display == Display::TableCell
            && !self.cells.contains_key(&node)
            && taffy.size.width.is_auto()
        {
            taffy.size.width = taffy::Dimension::fit_content();
        }
        let mut cell_width = None;
        if let Some(&(row, down, across, column)) = self.cells.get(&node) {
            // A cell's width sizes its column, which the cell fills.
            if across != 0 && !taffy.size.width.is_auto() {
                cell_width = Some(core::mem::replace(
                    &mut taffy.size.width,
                    taffy::Dimension::auto(),
                ));
            }
            taffy.grid_row = Line {
                start: GridPlacement::from_line_index(row),
                end: GridPlacement::from_span(down),
            };
            taffy.grid_column = if across == 0 {
                Line {
                    start: GridPlacement::from_line_index(1),
                    end: GridPlacement::from_line_index(-1),
                }
            } else {
                Line {
                    start: GridPlacement::from_line_index(column),
                    end: GridPlacement::from_span(across),
                }
            };
        }
        let index = self.add(Some(node), Kind::Container(own_mode), taffy);
        self.nodes[index].cell_width = cell_width;
        self.nodes[index].fixed_table = table && fixed_table;
        let positioned = style.position != PositionKind::Static;
        let context = (if positioned { index } else { context.0 }, context.1);
        let lines = decoration(style);
        let mut after_space = true;
        let mut items = Vec::new();
        let mut marker = None;
        // The container's text starts at the level of its direction.
        let outer_bidi = core::mem::replace(
            &mut self.bidi,
            (u8::from(style.direction == Direction::Rtl), false),
        );
        if display == Display::ListItem
            && let Some(segment) = self.marker(node, style, &mut after_space)
        {
            match style.list_style_position {
                ListStylePosition::Inside => items.push(Item::Text(segment)),
                ListStylePosition::Outside => marker = self.outside_marker(segment, style),
            }
            after_space = true;
        }
        self.collect_generated(
            node,
            &PseudoElement::Before,
            Flow {
                mode,
                context,
                lines,
                anchor: None,
            },
            &mut after_space,
            &mut items,
        );
        for child in self.dom.flat_children(node) {
            self.collect(
                child,
                style,
                Flow {
                    mode,
                    context,
                    lines,
                    anchor: None,
                },
                &mut after_space,
                &mut items,
            );
        }
        self.collect_generated(
            node,
            &PseudoElement::After,
            Flow {
                mode,
                context,
                lines,
                anchor: None,
            },
            &mut after_space,
            &mut items,
        );
        self.bidi = outer_bidi;
        let items = if mode == Mode::Block {
            items
        } else {
            items
                .into_iter()
                .map(|item| match item {
                    Item::Atomic(child, _) => Item::Block(child),
                    other => other,
                })
                .collect()
        };
        // Wrapped content would give absolute boxes another parent.
        let wrapped = name == "button"
            || anonymous_table
            || display == Display::TableCell
                && matches!(
                    style.vertical_align,
                    VerticalAlign::Middle | VerticalAlign::Bottom
                );
        let owner = (!wrapped).then_some(index);
        let mut all = self.group(items, mode, style, owner);
        all.extend(marker);
        if name == "button" && mode == Mode::Block {
            all = vec![self.center_button(index, all)];
        }
        // Cells place their content as their `vertical-align` says;
        // baseline alignment is approximated by the top.
        let cell = display == Display::TableCell && mode == Mode::Block;
        match style.vertical_align {
            VerticalAlign::Middle if cell => {
                all = vec![self.align_content(index, all, JustifyContent::CENTER)];
            }
            VerticalAlign::Bottom if cell => {
                all = vec![self.align_content(index, all, JustifyContent::END)];
            }
            _ => {}
        }
        if anonymous_table {
            let inner = self.add(None, Kind::Container(Mode::Grid), grid);
            self.nodes[inner].fixed_table = fixed_table;
            self.nodes[inner].children = all;
            all = vec![inner];
        }
        all.extend(
            self.deferred
                .iter()
                .filter(|(target, _)| *target == index)
                .map(|(_, child)| *child),
        );
        self.deferred.retain(|(target, _)| *target != index);
        if captions_beside {
            return self.caption_box(index, all);
        }
        self.nodes[index].children = all;
        index
    }

    /// Puts a table's captions beside it, in a box around them that takes
    /// the table's margins and width: those whose `caption-side` is `top`
    /// above it, the others below. Returns the box.
    fn caption_box(&mut self, table: usize, children: Vec<usize>) -> usize {
        let side = |builder: &Self, child: usize| {
            builder.nodes[child]
                .dom
                .and_then(|node| builder.styles.get(node))
                .filter(|style| style.display == Display::TableCaption)
                .map(|style| style.caption_side)
        };
        let (mut above, mut below, mut rest) = (Vec::new(), Vec::new(), Vec::new());
        for child in children {
            match side(self, child) {
                Some(nativeboat_css::types::CaptionSide::Top) => above.push(child),
                Some(nativeboat_css::types::CaptionSide::Bottom) => below.push(child),
                None => rest.push(child),
            }
        }
        self.nodes[table].children = rest;
        let inner = &mut self.nodes[table].style;
        let mut outer = Style {
            display: TaffyDisplay::Grid,
            item_is_table: true,
            margin: core::mem::replace(&mut inner.margin, TaffyRect::zero()),
            align_self: inner.align_self,
            justify_self: inner.justify_self,
            flex_grow: inner.flex_grow,
            flex_shrink: inner.flex_shrink,
            flex_basis: inner.flex_basis,
            grid_row: core::mem::take(&mut inner.grid_row),
            grid_column: core::mem::take(&mut inner.grid_column),
            ..Style::DEFAULT
        };
        // A table's width is its border box's; it fills the box around it.
        if !inner.size.width.is_auto() {
            outer.size.width =
                core::mem::replace(&mut inner.size.width, taffy::Dimension::percent(1.0));
            inner.box_sizing = taffy::BoxSizing::BorderBox;
        }
        let index = self.add(None, Kind::Container(Mode::Grid), outer);
        above.push(table);
        above.extend(below);
        self.nodes[index].children = above;
        index
    }
}

/// The native control that renders an element. Buttons whose background or
/// border the author styles are drawn like other boxes, as browsers draw
/// them.
pub fn native_control(
    dom: &Dom,
    element: NodeId,
    style: Option<&ComputedStyle>,
) -> Option<ControlKind> {
    let name = dom.local_name(element)?;
    let kind = control_for(name, |attribute| dom.get_attribute(element, attribute))?;
    let drawn = matches!(kind, ControlKind::Button | ControlKind::ComboBox)
        && style.is_some_and(|style| style.author_appearance);
    (!drawn).then_some(kind)
}

/// The prefix of the image key under which a page keeps a video's first
/// frame: the frame is not the image its URL names, so it has a key of its
/// own.
pub const VIDEO_FRAME: &str = "nativeboat-video-frame:";

/// The media a `<video>` or `<audio>` element plays: its `src`, else its
/// first `<source>` child's.
pub fn media_url(dom: &Dom, element: NodeId) -> Option<&str> {
    dom.get_attribute(element, "src").or_else(|| {
        dom.child_nodes(element)
            .filter(|child| dom.local_name(*child) == Some("source"))
            .find_map(|source| dom.get_attribute(source, "src"))
    })
}

/// The image a `<video>` shows before it plays, as an image key: its
/// poster when it has one, as Chromium shows it, else its first frame.
pub fn video_picture(dom: &Dom, element: NodeId) -> Option<String> {
    match dom.get_attribute(element, "poster").map(str::trim) {
        Some(poster) if !poster.is_empty() => Some(poster.to_owned()),
        _ => media_url(dom, element).map(|url| format!("{VIDEO_FRAME}{url}")),
    }
}

/// The width the arrow of a drawn `<select>` takes, in CSS pixels.
pub const SELECT_ARROW: f32 = 20.0;

/// The box Chromium centers a drawn `<select>`'s arrow in, at the inside
/// of its right border: the width of a scroll bar arrow, in CSS pixels.
pub const SELECT_ARROW_BOX: f32 = 17.0;

/// The label of a `<select>`: its selected option, else its first.
pub fn select_label(dom: &Dom, select: NodeId) -> String {
    let options = dom.options(select);
    let selected = dom.selected_options(select);
    selected
        .first()
        .or(options.first())
        .map(|option| collapse(&dom.text_content(*option).unwrap_or_default()))
        .unwrap_or_default()
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The caption of a button element or button-like input.
pub fn button_caption(dom: &Dom, element: NodeId) -> String {
    if dom.local_name(element) == Some("button") {
        let text = dom.text_content(element).unwrap_or_default();
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    if let Some(value) = dom.get_attribute(element, "value") {
        return value.to_owned();
    }
    match dom
        .get_attribute(element, "type")
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "submit" => "Submit".to_owned(),
        "reset" => "Reset".to_owned(),
        "file" => "Choose File".to_owned(),
        _ => String::new(),
    }
}

/// Where layout nodes came from: generated content's originating elements,
/// and absolute boxes' anchoring inline elements.
type Origins<'a> = (
    &'a HashMap<usize, (NodeId, Option<PseudoElement>)>,
    &'a HashMap<usize, NodeId>,
    &'a HashMap<usize, (usize, bool, bool)>,
);

fn collect_results(
    tree: &Tree<'_>,
    index: usize,
    origin: (f32, f32),
    (container, pseudo): (NodeId, Option<&PseudoElement>),
    result: &mut LayoutResult,
    (generated, anchored, statics): Origins<'_>,
    marks: &mut HashMap<usize, (f32, f32)>,
) {
    let node = &tree.nodes[index];
    // Generated text belongs to its originating element.
    let container = node
        .dom
        .or_else(|| generated.get(&index).map(|(element, _)| *element))
        .unwrap_or(container);
    let pseudo = match generated.get(&index) {
        Some((_, Some(own))) => Some(own),
        _ if node.dom.is_some() => None,
        _ => pseudo,
    };
    let layout = node.layout;
    let x = origin.0 + layout.location.x;
    let y = origin.1 + layout.location.y;
    marks.insert(index, (x, y));
    let geometry = BoxGeometry {
        margin: [
            layout.margin.top,
            layout.margin.right,
            layout.margin.bottom,
            layout.margin.left,
        ],
        rect: Rect::new(x, y, layout.size.width, layout.size.height),
        padding: [
            layout.padding.top,
            layout.padding.right,
            layout.padding.bottom,
            layout.padding.left,
        ],
        border: [
            layout.border.top,
            layout.border.right,
            layout.border.bottom,
            layout.border.left,
        ],
        // What scrolls is at least the client area: the padding box less
        // its scrollbars, as `scrollWidth` reports it.
        content_width: layout.scrollable_overflow_rect.right.max(
            layout.size.width
                - layout.border.left
                - layout.border.right
                - layout.scrollbar_size.width,
        ),
        content_height: layout.scrollable_overflow_rect.bottom.max(
            layout.size.height
                - layout.border.top
                - layout.border.bottom
                - layout.scrollbar_size.height,
        ),
        scrollbars: [layout.scrollbar_size.width, layout.scrollbar_size.height],
    };
    if let Some(dom) = node.dom {
        result.boxes.insert(dom, geometry);
    }
    if let Some((element, Some(pseudo))) = generated.get(&index) {
        result
            .generated
            .entry(*element)
            .or_default()
            .push(GeneratedBox {
                pseudo: pseudo.clone(),
                geometry,
            });
    }
    if let Kind::Inline(content) = &node.kind {
        // Lines break at the width the content was sized to; rounding the
        // box to pixels may have made it a fraction narrower.
        let width = node.unrounded.size.width.max(layout.size.width);
        let known = node
            .lines
            .borrow()
            .as_ref()
            .filter(|(at, sizes, _)| *at == width && *sizes == node.atom_sizes)
            .map(|(_, _, lines)| lines.clone());
        let lines = node.float_lines.clone().or(known).unwrap_or_else(|| {
            let lines = Rc::new(layout_lines(
                content,
                Some(width),
                tree.measurer,
                &node.atom_sizes,
            ));
            *node.lines.borrow_mut() = Some((width, node.atom_sizes.clone(), lines.clone()));
            lines
        });
        let count = lines.lines.len();
        for (index, line) in lines.lines.iter().enumerate() {
            // Text that overflows a clipping container, or the last of
            // clamped lines, ends in an ellipsis.
            // Right-to-left lines overflow on the left.
            let (start, end) = line.extent(&node.atom_sizes);
            let overflows = if content.direction == Direction::Rtl {
                start < -0.01
            } else {
                end > width + 0.01
            };
            let ellipsized = ((lines.clamped && index + 1 == count)
                || (content.ellipsis && overflows))
                .then(|| crate::inline::ellipsize(content, &line.fragments, width, tree.measurer));
            let (fragments, mark) = match &ellipsized {
                Some((kept, mark)) => (kept.as_slice(), mark.as_ref()),
                None => (line.fragments.as_slice(), None),
            };
            inline_boxes(
                tree,
                content,
                &node.atom_sizes,
                line,
                (x, y),
                container,
                result,
            );
            let (top, baseline) = (line.top, line.baseline);
            for (fragment, generated) in fragments
                .iter()
                .map(|fragment| (fragment, false))
                .chain(mark.map(|mark| (mark, true)))
            {
                let segment = &content.segments[fragment.segment];
                // Gaps show no glyphs; an empty element's place is a run
                // without text, for its rectangle. Text at `font-size: 0`
                // has no glyphs either, but its place is an empty rectangle
                // on the baseline, as Chromium reports it.
                let empty = segment.edge.is_none()
                    && matches!(segment.pieces.as_slice(), [crate::text::Piece::Gap(_)]);
                if fragment.text.is_empty() && !empty {
                    continue;
                }
                let content = if segment.font.size > 0.0 {
                    let metrics = tree.measurer.metrics(&segment.font);
                    let height = metrics.ascent + metrics.descent;
                    let above =
                        crate::text::half_leading(tree.measurer, segment.line_height, height);
                    (above, height)
                } else {
                    (segment.line_height / 2.0, 0.0)
                };
                result.runs.push(TextRun {
                    content,
                    line: (y + top, line.height),
                    rect: Rect::new(
                        x + fragment.x,
                        y + top
                            + (baseline
                                - segment.shift
                                - above_baseline(
                                    tree.measurer,
                                    &segment.font,
                                    segment.line_height,
                                )),
                        fragment.width,
                        segment.line_height,
                    ),
                    text: fragment.text.clone(),
                    font: segment.font.clone(),
                    color: segment.color,
                    underline: segment.underline,
                    underline_offset: segment.underline_offset,
                    line_through: segment.line_through,
                    letter_spacing: segment.letter_spacing,
                    // The ellipsis is no text of the node's: like generated
                    // content, it belongs to the element.
                    node: if generated {
                        segment.element
                    } else {
                        segment.node
                    },
                    element: segment.element,
                    container,
                    symbol: segment
                        .symbol
                        .map(|symbol| (symbol, symbol_rect(tree.measurer, segment, symbol))),
                    pseudo: pseudo.cloned(),
                    rtl: fragment.level % 2 == 1,
                });
            }
        }
    }
    for child in &node.children {
        // A box placed against a positioned inline element moves from this
        // box's padding edge to the start of the inline's first fragment,
        // and its insets measure the inline's box instead of this one.
        let origin = anchored
            .get(child)
            .and_then(|anchor| inline_containing_block(&result.inline_boxes, *anchor))
            .map_or((x, y), |block| {
                let placed = &tree.nodes[*child];
                let start = (block.x - layout.border.left, block.y - layout.border.top);
                let (across, down) = anchored_offsets(&placed.style, &placed.layout, block);
                (
                    across.map_or(start.0, |at| at - placed.layout.location.x),
                    down.map_or(start.1, |at| at - placed.layout.location.y),
                )
            });
        // A box at its static position starts, margin and all, where its
        // marker in its parent's flow is.
        let origin = statics
            .get(child)
            .and_then(|(marker, across, down)| {
                let (mark_x, mark_y) = *marks.get(marker)?;
                let placed = &tree.nodes[*child].layout;
                Some((
                    if *across {
                        mark_x + placed.margin.left - placed.location.x
                    } else {
                        origin.0
                    },
                    if *down {
                        mark_y + placed.margin.top - placed.location.y
                    } else {
                        origin.1
                    },
                ))
            })
            .unwrap_or(origin);
        collect_results(
            tree,
            *child,
            origin,
            (container, pseudo),
            result,
            (generated, anchored, statics),
            marks,
        );
    }
}

/// How far an inline element's `vertical-align` raises its baseline above
/// its parent's, as Chromium shifts inline boxes: `sub` and `super` by a
/// fraction of the parent's font size, lengths as given and percentages of
/// the element's line height.
fn baseline_shift(
    style: &ComputedStyle,
    parent: &ComputedStyle,
    measurer: &dyn TextMeasurer,
) -> f32 {
    let size = parent.font_size.0;
    match &style.vertical_align {
        VerticalAlign::Sub => -(size / 5.0 + 1.0),
        VerticalAlign::Super => size / 3.0 + 1.0,
        VerticalAlign::Length(nativeboat_css::LengthPercentage::Px(px)) => *px,
        VerticalAlign::Length(nativeboat_css::LengthPercentage::Percent(fraction)) => {
            fraction * line_height(style, measurer)
        }
        _ => 0.0,
    }
}

/// The containing block an inline element forms for the absolutely
/// positioned boxes in it: from the start of its first fragment to the end
/// of its last (CSS 2.1 section 10.1), in document coordinates.
fn inline_containing_block(parts: &[InlineBoxRect], anchor: NodeId) -> Option<Rect> {
    let first = parts
        .iter()
        .find(|part| part.node == anchor && part.start)?;
    let last = parts
        .iter()
        .rev()
        .find(|part| part.node == anchor && part.end)
        .unwrap_or(first);
    let right = (last.rect.x + last.rect.width).max(first.rect.x);
    let bottom = (last.rect.y + last.rect.height).max(first.rect.y);
    Some(Rect::new(
        first.rect.x,
        first.rect.y,
        right - first.rect.x,
        bottom - first.rect.y,
    ))
}

/// Where an absolutely positioned box's border box goes along each axis
/// whose insets are not both `auto`, against an inline containing block.
fn anchored_offsets(style: &Style, placed: &Layout, block: Rect) -> (Option<f32>, Option<f32>) {
    let inset = |value: LengthPercentageAuto, basis: f32| {
        value.resolve_to_option(basis, CalcStore::resolve)
    };
    let across = inset(style.inset.left, block.width)
        .map(|left| block.x + left + placed.margin.left)
        .or_else(|| {
            inset(style.inset.right, block.width).map(|right| {
                block.x + block.width - right - placed.margin.right - placed.size.width
            })
        });
    let down = inset(style.inset.top, block.height)
        .map(|top| block.y + top + placed.margin.top)
        .or_else(|| {
            inset(style.inset.bottom, block.height).map(|bottom| {
                block.y + block.height - bottom - placed.margin.bottom - placed.size.height
            })
        });
    (across, down)
}

/// Where a marker symbol shows in its run, sized from the font's ascent as
/// Chromium sizes it (`ListMarker::RelativeSymbolMarkerRect`).
fn symbol_rect(measurer: &dyn TextMeasurer, segment: &Segment, symbol: Symbol) -> Rect {
    let metrics = measurer.metrics(&segment.font);
    // Text is centered in the run's line box.
    let top = (segment.line_height - (metrics.ascent + metrics.descent)) / 2.0;
    if matches!(symbol, Symbol::DisclosureClosed | Symbol::DisclosureOpen) {
        // A triangle about half the ascent high, its middle a third of the
        // ascent above the baseline, as Chromium draws it.
        let size = ((metrics.ascent * 0.53 + 0.5) as i32) as f32;
        return Rect::new(
            0.0,
            top + metrics.ascent * 2.0 / 3.0 - size / 2.0,
            size,
            size,
        );
    }
    let ascent = (metrics.ascent + 0.5) as i32;
    let size = ((ascent * 2 / 3 + 1) / 2) as f32;
    Rect::new(
        1.0,
        top + (3 * (ascent - ascent * 2 / 3) / 2) as f32,
        size,
        size,
    )
}

/// The parts on one line of the inline boxes its text is in: each box
/// covers its text's fragments, its content height centered in the line as
/// the text is, with its border and padding above and below.
fn inline_boxes(
    tree: &Tree<'_>,
    content: &InlineContent,
    sizes: &[AtomSize],
    line: &crate::inline::Line,
    (x, y): (f32, f32),
    container: NodeId,
    result: &mut LayoutResult,
) {
    let mut parts: Vec<(Rc<InlineBox>, f32, f32, bool, bool)> = Vec::new();
    // What each box covers: its text and the atomic boxes in it.
    let spans = line
        .fragments
        .iter()
        .map(|fragment| (fragment.segment, fragment.x, fragment.width))
        .chain(line.atoms.iter().map(|placed| {
            let width = sizes.get(placed.atom).map_or(0.0, |size| size.width);
            (placed.segment, placed.x, width)
        }));
    for (segment, from, width) in spans {
        let segment = &content.segments[segment];
        let innermost = segment.boxes.len().saturating_sub(1);
        for (depth, boxed) in segment.boxes.iter().enumerate() {
            let (mut left, mut right) = (from, from + width);
            let edge = if depth == innermost {
                segment.edge
            } else {
                None
            };
            // A box's margin is outside its border box; right to left,
            // its start is its right side.
            match (edge, boxed.rtl) {
                (Some(Edge::Start), false) | (Some(Edge::End), true) => left += boxed.margin[0],
                (Some(Edge::End), false) | (Some(Edge::Start), true) => right -= boxed.margin[1],
                (None, _) => {}
            }
            match parts.iter_mut().find(|part| Rc::ptr_eq(&part.0, boxed)) {
                Some(part) => {
                    part.1 = part.1.min(left);
                    part.2 = part.2.max(right);
                    part.3 |= edge == Some(Edge::Start);
                    part.4 |= edge == Some(Edge::End);
                }
                None => parts.push((
                    boxed.clone(),
                    left,
                    right,
                    edge == Some(Edge::Start),
                    edge == Some(Edge::End),
                )),
            }
        }
    }
    let baseline = y + line.top + line.baseline;
    for (boxed, left, right, start, end) in parts {
        // A box's content area is its font's ascent and descent around the
        // baseline.
        let metrics = tree.measurer.metrics(&boxed.font);
        let top = baseline - boxed.shift - metrics.ascent - boxed.above;
        result.inline_boxes.push(InlineBoxRect {
            container,
            node: boxed.node,
            rect: Rect::new(
                x + left,
                top,
                (right - left).max(0.0),
                metrics.ascent + metrics.descent + boxed.above + boxed.below,
            ),
            start: if boxed.rtl { end } else { start },
            end: if boxed.rtl { start } else { end },
        });
    }
}

/// The layout tree of a document's last layout. Subtrees of the next
/// layout that are built the same, as all but the edited paragraph are
/// while typing, keep their measurements instead of being laid out again.
#[derive(Default)]
pub struct LayoutCache {
    nodes: Vec<Node>,
    root: usize,
    /// Keeps the last layout's `calc()` values alive, so a new one cannot
    /// take an old one's address and pass for it.
    _calcs: Option<CalcStore>,
    /// The measurer's generation the measurements were made in.
    generation: u64,
    /// How many boxes the layout reused.
    reused: usize,
    /// The `overflow: auto` boxes whose content overflowed, with the axis
    /// (0 across, 1 down) their scrollbar shows on.
    scrollbars: HashSet<(NodeId, u8)>,
}

impl LayoutCache {
    /// How many boxes the last layout took from the one before.
    pub fn reused(&self) -> usize {
        self.reused
    }
}

impl core::fmt::Debug for LayoutCache {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LayoutCache")
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

/// Moves the measurements of the subtrees of `new` built the same as in
/// `old` over to them, and returns whether the whole subtree was.
fn reuse(
    old: &mut [Node],
    old_index: usize,
    new: &mut [Node],
    new_index: usize,
    reused: &mut usize,
) -> bool {
    let same = {
        let (before, after) = (&old[old_index], &new[new_index]);
        before.dom == after.dom
            && before.kind == after.kind
            && before.style == after.style
            && before.cell_width == after.cell_width
            && before.fixed_table == after.fixed_table
    };
    // The child lists are borrowed out and put back, rather than copied for
    // every box on every layout.
    let old_children = core::mem::take(&mut old[old_index].children);
    let new_children = core::mem::take(&mut new[new_index].children);
    let mut identical = same && old_children.len() == new_children.len();
    if old_children.len() == new_children.len() {
        for (&before, &after) in old_children.iter().zip(&new_children) {
            identical &= reuse(old, before, new, after, reused);
        }
    } else {
        // Children came or went: those of the same elements may still be
        // unchanged.
        for &after in &new_children {
            let Some(dom) = new[after].dom else {
                continue;
            };
            if let Some(&before) = old_children
                .iter()
                .find(|&&before| old[before].dom == Some(dom))
            {
                reuse(old, before, new, after, reused);
            }
        }
    }
    old[old_index].children = old_children;
    new[new_index].children = new_children;
    if identical {
        let before = &mut old[old_index];
        let after = &mut new[new_index];
        after.cache = core::mem::replace(&mut before.cache, Cache::new());
        after.unrounded = before.unrounded;
        after.atom_sizes = core::mem::take(&mut before.atom_sizes);
        after.lines = core::mem::take(&mut before.lines);
        after.float_lines = before.float_lines.take();
        *reused += 1;
    }
    identical
}

/// Lays out the document.
pub fn layout(dom: &Dom, styles: &Styles, options: &LayoutOptions<'_>) -> LayoutResult {
    layout_cached(dom, styles, options, &mut LayoutCache::default())
}

/// Lays out the document again, reusing what has not changed since the
/// layout `cache` holds, and keeps this layout there for the next one.
pub fn layout_cached(
    dom: &Dom,
    styles: &Styles,
    options: &LayoutOptions<'_>,
    cache: &mut LayoutCache,
) -> LayoutResult {
    // Scrollbars take room from their boxes, as Windows' classic ones do in
    // Chromium: a box whose content comes to overflow, or no longer does,
    // is laid out again with or without its scrollbar, starting from those
    // the last layout showed.
    let mut shown = core::mem::take(&mut cache.scrollbars);
    let mut passes = 0;
    loop {
        let (result, next) = layout_pass(dom, styles, options, cache, &shown);
        passes += 1;
        if next == shown || passes == 3 {
            cache.scrollbars = next;
            return result;
        }
        shown = next;
    }
}

/// How thick an element's scrollbar on an axis (0 across, 1 down) is: its
/// author style's size, else the thickness `platform` the browser's own
/// scrollbars have.
pub(crate) fn scrollbar_thickness(
    styles: &Styles,
    node: NodeId,
    axis: usize,
    cross: f32,
    platform: f32,
) -> f32 {
    let Some(bars) = styles.scrollbars(node) else {
        return platform;
    };
    let bar = &bars.axes[axis];
    if !bar.enabled {
        return 0.0;
    }
    match &bar.thickness {
        nativeboat_css::types::Size::Length(value) => value.resolve(cross).max(0.0),
        _ => platform,
    }
}

/// Gives scroll containers room for the scrollbars they show: always with
/// `overflow: scroll`, and with `auto` on the axes `shown` names; those no
/// author CSS styles are `platform` thick.
fn reserve_scrollbars(
    nodes: &mut [Node],
    styles: &Styles,
    shown: &HashSet<(NodeId, u8)>,
    platform: f32,
) {
    use nativeboat_css::types::Overflow as CssOverflow;
    for node in nodes.iter_mut() {
        let Some(dom) = node.dom else {
            continue;
        };
        if !matches!(node.kind, Kind::Container(_) | Kind::Inline(_)) {
            continue;
        }
        let Some(style) = styles.get(dom) else {
            continue;
        };
        let bar = |axis: u8, overflow: CssOverflow| match overflow {
            CssOverflow::Scroll => true,
            CssOverflow::Auto => shown.contains(&(dom, axis)),
            _ => false,
        };
        let across = bar(0, style.overflow_x);
        let down = bar(1, style.overflow_y);
        if !across && !down {
            continue;
        }
        let thickness = |axis: usize| scrollbar_thickness(styles, dom, axis, 0.0, platform);
        let (height, width) = (
            if across { thickness(0) } else { 0.0 },
            if down { thickness(1) } else { 0.0 },
        );
        let axis = |show: bool, size: f32| {
            if show && size > 0.0 {
                taffy::Overflow::Scroll
            } else {
                taffy::Overflow::Hidden
            }
        };
        node.style.overflow = taffy::Point {
            x: axis(across, height),
            y: axis(down, width),
        };
        node.style.scrollbar_width = width.max(height);
    }
}

/// The `overflow: auto` boxes whose content overflows them, by axis.
fn overflowing(tree: &Tree<'_>, styles: &Styles, result: &LayoutResult) -> HashSet<(NodeId, u8)> {
    use nativeboat_css::types::Overflow as CssOverflow;
    let mut found = HashSet::new();
    for node in &tree.nodes {
        let Some(dom) = node.dom else {
            continue;
        };
        if !matches!(node.kind, Kind::Container(_) | Kind::Inline(_)) {
            continue;
        }
        let Some(style) = styles.get(dom) else {
            continue;
        };
        let layout = &node.unrounded;
        let width = layout.size.width - layout.border.left - layout.border.right;
        let height = layout.size.height - layout.border.top - layout.border.bottom;
        let mut overflow = layout.scrollable_overflow_rect;
        // Translations move what overflows, as an active marker centered by
        // `top: 50%; transform: translateY(-50%)` that layout sees hang
        // below its item: the content size corrected for them tells.
        if let Some(geometry) = result.boxes.get(&dom) {
            let rounded = &node.layout;
            let laid = (
                rounded.scrollable_overflow_rect.right.max(
                    rounded.size.width
                        - rounded.border.left
                        - rounded.border.right
                        - rounded.scrollbar_size.width,
                ),
                rounded.scrollable_overflow_rect.bottom.max(
                    rounded.size.height
                        - rounded.border.top
                        - rounded.border.bottom
                        - rounded.scrollbar_size.height,
                ),
            );
            overflow.right += geometry.content_width - laid.0;
            overflow.bottom += geometry.content_height - laid.1;
        }
        if style.overflow_x == CssOverflow::Auto
            && overflow.right > width - layout.scrollbar_size.width + 0.5
        {
            found.insert((dom, 0));
        }
        if style.overflow_y == CssOverflow::Auto
            && overflow.bottom > height - layout.scrollbar_size.height + 0.5
        {
            found.insert((dom, 1));
        }
    }
    found
}

/// Rounds a laid out tree to multiples of `unit` as taffy's `round_layout`
/// rounds to whole pixels: positions on their own, and sizes, padding and
/// overflow as the difference of their rounded edges, so that adjacent boxes
/// meet. Chromium lays boxes out in sixty-fourths of a device pixel and
/// moves their edges to whole pixels only to paint them. Border widths stay
/// as computed: styles already snap them to whole device pixels, as Chromium
/// does, and rounding their edges would make equal sides differ, or vanish,
/// by where the box falls. Taffy's own rounding without `std` takes a
/// floating-point remainder for every value, which costs more than the rest
/// of the pass.
fn round_layout(
    tree: &mut Tree<'_>,
    node: taffy::NodeId,
    (cumulative_x, cumulative_y): (f32, f32),
    unit: f32,
) {
    let round = |value: f32| crate::math::round(value / unit) * unit;
    let unrounded = tree.get_unrounded_layout(node);
    let mut layout = unrounded;
    let x = cumulative_x + unrounded.location.x;
    let y = cumulative_y + unrounded.location.y;
    // An edge `offset` from the box's rounded start, as a rounded length.
    let across = |offset: f32| round(x + offset) - round(x);
    let down = |offset: f32| round(y + offset) - round(y);
    let (width, height) = (unrounded.size.width, unrounded.size.height);
    layout.location.x = round(unrounded.location.x);
    layout.location.y = round(unrounded.location.y);
    layout.size.width = across(width);
    layout.size.height = down(height);
    layout.scrollbar_size.width = round(unrounded.scrollbar_size.width);
    layout.scrollbar_size.height = round(unrounded.scrollbar_size.height);
    layout.padding.left = across(unrounded.padding.left);
    layout.padding.right = round(x + width) - round(x + width - unrounded.padding.right);
    layout.padding.top = down(unrounded.padding.top);
    layout.padding.bottom = round(y + height) - round(y + height - unrounded.padding.bottom);
    let overflow = unrounded.scrollable_overflow_rect;
    layout.scrollable_overflow_rect.left = across(overflow.left);
    layout.scrollable_overflow_rect.right = across(overflow.right);
    layout.scrollable_overflow_rect.top = down(overflow.top);
    layout.scrollable_overflow_rect.bottom = down(overflow.bottom);
    tree.set_final_layout(node, &layout);
    for index in 0..tree.child_count(node) {
        let child = tree.get_child_id(node, index);
        round_layout(tree, child, (x, y), unit);
    }
}

fn layout_pass(
    dom: &Dom,
    styles: &Styles,
    options: &LayoutOptions<'_>,
    cache: &mut LayoutCache,
    shown: &HashSet<(NodeId, u8)>,
) -> (LayoutResult, HashSet<(NodeId, u8)>) {
    let mut builder = Builder {
        dom,
        styles,
        options,
        calcs: CalcStore::default(),
        // About as many boxes as last time: growing moves every box.
        nodes: Vec::with_capacity(cache.nodes.len() + cache.nodes.len() / 8),
        deferred: Vec::new(),
        cells: HashMap::new(),
        anonymous_cells: HashSet::new(),
        generated: HashMap::new(),
        anchored: HashMap::new(),
        statics: HashMap::new(),
        boxes: Vec::new(),
        shift: 0.0,
        bidi: (0, false),
        images: HashMap::new(),
        counters: core::cell::OnceCell::new(),
    };
    let (width, height) = options.viewport;
    let viewport = builder.add(
        None,
        Kind::Container(Mode::Flex),
        Style {
            display: TaffyDisplay::Flex,
            flex_direction: taffy::FlexDirection::Column,
            size: Size {
                width: taffy::Dimension::length(width),
                height: taffy::Dimension::length(height),
            },
            ..Style::DEFAULT
        },
    );
    let root = dom
        .document_element()
        .and_then(|root| styles.get(root).cloned().map(|style| (root, style)));
    if let Some((root, style)) = root
        && style.display != Display::None
    {
        let child = builder.element(root, &style, true, (viewport, viewport));
        let mut children = vec![child];
        children.extend(builder.deferred.drain(..).map(|(_, child)| child));
        builder.nodes[viewport].children = children;
    }
    let scrollbars = (
        options.measurer.scrollbar_look(),
        options.measurer.device_pixel(),
    );
    reserve_scrollbars(
        &mut builder.nodes,
        styles,
        shown,
        scrollbars.0.thickness(scrollbars.1),
    );
    let generation = options.measurer.generation();
    let mut reused = 0;
    if cache.generation == generation && !cache.nodes.is_empty() {
        let mut old = core::mem::take(&mut cache.nodes);
        reuse(
            &mut old,
            cache.root,
            &mut builder.nodes,
            viewport,
            &mut reused,
        );
    }
    let mut tree = Tree {
        nodes: builder.nodes,
        measurer: options.measurer,
        compressing: false,
    };
    let calcs = builder.calcs;
    compute_root_layout(
        &mut tree,
        taffy::NodeId::from(viewport),
        Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        },
    );
    // Whole CSS pixels where the device's are unknown.
    let pixel = options.measurer.device_pixel();
    let unit = if pixel > 0.0 { pixel / 64.0 } else { 1.0 };
    round_layout(&mut tree, taffy::NodeId::from(viewport), (0.0, 0.0), unit);
    let mut result = LayoutResult {
        images: core::mem::take(&mut builder.images),
        scrollbars,
        ..LayoutResult::default()
    };
    collect_results(
        &tree,
        viewport,
        (0.0, 0.0),
        (Dom::DOCUMENT, None),
        &mut result,
        (&builder.generated, &builder.anchored, &builder.statics),
        &mut HashMap::new(),
    );
    let (right, bottom) = viewport_overflow(&tree, styles, viewport);
    result.width = right.max(width);
    result.height = bottom.max(height);
    translate_overflow(&tree, styles, &builder.generated, viewport, &mut result);
    let overflowing = overflowing(&tree, styles, &result);
    *cache = LayoutCache {
        nodes: tree.nodes,
        root: viewport,
        _calcs: Some(calcs),
        generation,
        reused,
        scrollbars: HashSet::new(),
    };
    (result, overflowing)
}

/// How far the document reaches right and down. Fixed-position boxes stay
/// with the viewport, so neither they nor what they hold make it scroll,
/// as MarkText's fixed `body` holds menus parked at `right: -9999px`.
fn viewport_overflow(tree: &Tree<'_>, styles: &Styles, viewport: usize) -> (f32, f32) {
    let mut extent = (0.0f32, 0.0f32);
    for &child in &tree.nodes[viewport].children {
        let node = &tree.nodes[child];
        let fixed = node
            .dom
            .and_then(|dom| styles.get(dom))
            .is_some_and(|style| style.position == PositionKind::Fixed);
        if fixed {
            continue;
        }
        let layout = node.layout;
        let overflow = node.style.overflow;
        let clips =
            overflow.x != taffy::Overflow::Visible || overflow.y != taffy::Overflow::Visible;
        let (width, height) = if clips {
            (layout.size.width, layout.size.height)
        } else {
            (
                layout.size.width.max(layout.scrollable_overflow_rect.right),
                layout
                    .size
                    .height
                    .max(layout.scrollable_overflow_rect.bottom),
            )
        };
        extent.0 = extent.0.max(layout.location.x + width);
        extent.1 = extent.1.max(layout.location.y + height);
    }
    extent
}

/// Corrects scrollable overflow for CSS translations, which layout does not
/// see: a box centered by `left: 50%; transform: translateX(-50%)` makes no
/// container scroll.
fn translate_overflow(
    tree: &Tree<'_>,
    styles: &Styles,
    generated: &HashMap<usize, (NodeId, Option<PseudoElement>)>,
    viewport: usize,
    result: &mut LayoutResult,
) {
    let translation = |index: usize| {
        let node = &tree.nodes[index];
        // Generated boxes have the styles of their pseudo-elements.
        let pseudo = || match generated.get(&index) {
            Some((element, Some(pseudo))) => styles.pseudo(*element, pseudo),
            _ => None,
        };
        node.dom
            .and_then(|dom| styles.get(dom))
            .or_else(pseudo)
            .map_or([0.0; 2], |style| {
                let matrix = style
                    .full_transform()
                    .matrix(node.layout.size.width, node.layout.size.height);
                [matrix[4], matrix[5]]
            })
    };
    // Each box's translation, read once: most boxes have none.
    let translations: Vec<[f32; 2]> = (0..tree.nodes.len()).map(translation).collect();
    if translations.iter().all(|shift| *shift == [0.0; 2]) {
        return;
    }
    let translation = |index: usize| translations[index];
    let clips = |index: usize| {
        let overflow = tree.nodes[index].style.overflow;
        overflow.x != taffy::Overflow::Visible || overflow.y != taffy::Overflow::Visible
    };
    // The right and bottom edges descendants reach from each box's border
    // box origin, with and without their translations; boxes that clip
    // their overflow hide their descendants'.
    type Extents = ([f32; 2], [f32; 2]);
    fn extents(
        tree: &Tree<'_>,
        index: usize,
        translation: &dyn Fn(usize) -> [f32; 2],
        clips: &dyn Fn(usize) -> bool,
        found: &mut [Extents],
    ) -> Extents {
        let (mut moved, mut kept) = ([0.0f32; 2], [0.0f32; 2]);
        for &child in &tree.nodes[index].children {
            let layout = tree.nodes[child].layout;
            let inner = extents(tree, child, translation, clips, found);
            let (inner_moved, inner_kept) = if clips(child) {
                ([0.0; 2], [0.0; 2])
            } else {
                inner
            };
            let shift = translation(child);
            let origin = [layout.location.x, layout.location.y];
            let size = [layout.size.width, layout.size.height];
            for axis in 0..2 {
                kept[axis] = kept[axis].max(origin[axis] + size[axis].max(inner_kept[axis]));
                moved[axis] =
                    moved[axis].max(origin[axis] + shift[axis] + size[axis].max(inner_moved[axis]));
            }
        }
        found[index] = (moved, kept);
        (moved, kept)
    }
    let mut found = vec![([0.0; 2], [0.0; 2]); tree.nodes.len()];
    let (moved, kept) = extents(tree, viewport, &translation, &clips, &mut found);
    // What overflows the padding box changes by what the translations
    // change of the descendants' overflow.
    let corrected = |content: f32, own: f32, start: f32, moved: f32, kept: f32| {
        let (before, after) = (
            (kept - start - own).max(0.0),
            (moved - start - own).max(0.0),
        );
        if before == after {
            content
        } else if after == 0.0 {
            // Translated, every box fits: the margins layout counted
            // beyond the farthest one moved with it.
            own
        } else {
            own + ((content - own).max(0.0) - before + after).max(0.0)
        }
    };
    for (index, (node, &(moved, kept))) in tree.nodes.iter().zip(&found).enumerate() {
        let Some(dom) = node.dom.filter(|_| clips(index)) else {
            continue;
        };
        let Some(geometry) = result.boxes.get_mut(&dom) else {
            continue;
        };
        let border = node.layout.border;
        let bars = node.layout.scrollbar_size;
        let own = [
            node.layout.size.width - border.left - border.right - bars.width,
            node.layout.size.height - border.top - border.bottom - bars.height,
        ];
        geometry.content_width = corrected(
            geometry.content_width,
            own[0],
            border.left,
            moved[0],
            kept[0],
        );
        geometry.content_height = corrected(
            geometry.content_height,
            own[1],
            border.top,
            moved[1],
            kept[1],
        );
    }
    let size = tree.nodes[viewport].layout.size;
    result.width = corrected(result.width, size.width, 0.0, moved[0], kept[0]);
    result.height = corrected(result.height, size.height, 0.0, moved[1], kept[1]);
}

/// The column widths of a table with automatic columns, sized as CSS 2.1's
/// table layout and Chromium size them. With `table-layout: fixed` and a
/// width, the first row's cells give their columns widths and the others
/// share the rest. Otherwise each column takes its single-column cells'
/// minimum and maximum content widths, widened to lengths the cells give,
/// a spanning cell's shortfall goes to its columns in proportion to their
/// maximum widths, columns given percentages take those shares of the
/// table, and the other columns fit the rest, room to spare going to
/// automatic columns before those given lengths. `None` for other boxes,
/// which lay out as grids.
fn table_columns(tree: &mut Tree<'_>, index: usize, inputs: &LayoutInput) -> Option<Vec<f32>> {
    let style = &tree.nodes[index].style;
    let automatic = |track: &GridTemplateComponent<_>| matches!(track, GridTemplateComponent::Single(single) if *single == <taffy::TrackSizingFunction as taffy::prelude::TaffyAuto>::AUTO);
    if !style.item_is_table
        || style.grid_template_columns.is_empty()
        || !style.grid_template_columns.iter().all(automatic)
    {
        return None;
    }
    let count = style.grid_template_columns.len();
    let placed = |child: usize| match tree.nodes[child].style.grid_column {
        Line {
            start: GridPlacement::Line(line),
            end: GridPlacement::Span(span),
        } if line.as_i16() >= 1 => Some(((line.as_i16() - 1) as usize, usize::from(span))),
        _ => None,
    };
    let cells: Vec<(usize, usize, usize)> = tree.nodes[index]
        .children
        .iter()
        .filter_map(|child| placed(*child).map(|(column, span)| (*child, column, span)))
        .filter(|(_, column, span)| column + span <= count && *span >= 1)
        .collect();
    let fixed_table = tree.nodes[index].fixed_table;
    let gap = style.gap.width.resolve_or_zero(None, CalcStore::resolve);
    let basis = inputs.parent_size.width;
    let padding = style.padding.resolve_or_zero(basis, CalcStore::resolve);
    let border = style.border.resolve_or_zero(basis, CalcStore::resolve);
    let edges = padding.left + padding.right + border.left + border.right;
    let measure = |tree: &mut Tree<'_>, child: usize, width: AvailableSpace| {
        tree.compute_child_layout(
            taffy::NodeId::from(child),
            LayoutInput {
                run_mode: RunMode::ComputeSize,
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Horizontal,
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: false,
                    height: false,
                },
                parent_size: Size::NONE,
                available_space: Size {
                    width,
                    height: AvailableSpace::MaxContent,
                },
                vertical_margins_are_collapsible: Line::FALSE,
            },
        )
        .size
        .width
    };
    let gaps = gap * (count - 1) as f32;
    // The table's own width, which its grid resolves only inside layout.
    let own = tree.nodes[index]
        .style
        .size
        .width
        .maybe_resolve(inputs.parent_size.width, CalcStore::resolve)
        .map(|width| {
            if tree.nodes[index].style.box_sizing == taffy::BoxSizing::ContentBox {
                width + edges
            } else {
                width
            }
        });
    let known = inputs.known_dimensions.width.or(own);
    let room_for = |total: f32| match (known, inputs.available_space.width) {
        (Some(width), _) => Some((width - edges).max(0.0)),
        (None, AvailableSpace::Definite(width)) => Some((width - edges).max(0.0).min(total)),
        _ => None,
    };
    // A cell's own width: a length of its border box, or a share of the
    // table's.
    enum Wanted {
        Length(f32),
        Share(f32),
    }
    let wanted = |tree: &Tree<'_>, child: usize| {
        let width = tree.nodes[child].cell_width?;
        let at = |basis: f32| width.maybe_resolve(Some(basis), CalcStore::resolve);
        let (one, two) = (at(100.0)?, at(200.0)?);
        if (two - one).abs() > 1e-3 {
            return Some(Wanted::Share(one / 100.0));
        }
        let style = &tree.nodes[child].style;
        let padding = style.padding.resolve_or_zero(None, CalcStore::resolve);
        let border = style.border.resolve_or_zero(None, CalcStore::resolve);
        let edges = if style.box_sizing == taffy::BoxSizing::ContentBox {
            padding.left + padding.right + border.left + border.right
        } else {
            0.0
        };
        Some(Wanted::Length(one + edges))
    };
    // `table-layout: fixed`: the first row's cells give their columns
    // widths, which the others share the rest of the room beside.
    if fixed_table && let Some(room) = room_for(f32::INFINITY) {
        let row = |child: usize| match &tree.nodes[child].style.grid_row.start {
            GridPlacement::Line(line) => line.as_i16(),
            _ => i16::MAX,
        };
        let first = cells.iter().map(|(child, ..)| row(*child)).min();
        let mut widths: Vec<Option<f32>> = vec![None; count];
        for (child, column, span) in &cells {
            if Some(row(*child)) != first {
                continue;
            }
            let width = match wanted(tree, *child) {
                Some(Wanted::Length(width)) => width,
                Some(Wanted::Share(share)) => share * (room - gaps),
                None => continue,
            };
            let room = gap * (*span - 1) as f32;
            for slot in &mut widths[*column..column + span] {
                *slot = Some((width - room) / *span as f32);
            }
        }
        let taken: f32 = widths.iter().flatten().sum();
        let open = widths.iter().filter(|width| width.is_none()).count();
        let each = if open > 0 {
            (room - gaps - taken).max(0.0) / open as f32
        } else {
            0.0
        };
        return Some(
            widths
                .into_iter()
                .map(|width| width.unwrap_or(each))
                .collect(),
        );
    }
    let mut cells: Vec<(usize, usize, f32, f32, Option<Wanted>)> = cells
        .into_iter()
        .map(|(child, column, span)| {
            let wanted = wanted(tree, child);
            let (min, max) = (
                measure(tree, child, AvailableSpace::MinContent),
                measure(tree, child, AvailableSpace::MaxContent),
            );
            // A length widens the cell to it.
            match wanted {
                Some(Wanted::Length(width)) => {
                    (column, span, min.max(width), min.max(width), wanted)
                }
                _ => (column, span, min, max, wanted),
            }
        })
        .collect();
    let (mut least, mut most) = (vec![0.0f32; count], vec![0.0f32; count]);
    // Columns whose cells give lengths keep them; those given shares of
    // the table take them.
    let (mut fixed, mut shares) = (vec![false; count], vec![0.0f32; count]);
    for (column, span, min, max, wanted) in &cells {
        if *span == 1 {
            least[*column] = least[*column].max(*min);
            most[*column] = most[*column].max(*max);
            match wanted {
                Some(Wanted::Length(_)) => fixed[*column] = true,
                Some(Wanted::Share(share)) => shares[*column] = shares[*column].max(*share),
                None => {}
            }
        }
    }
    // Narrower spans first, as the algorithm takes them.
    cells.sort_by_key(|(_, span, ..)| *span);
    for (column, span, min, max, _) in cells.into_iter().filter(|(_, span, ..)| *span > 1) {
        let columns = column..column + span;
        let room = gap * (span - 1) as f32;
        let weights: Vec<f32> = most[columns.clone()].to_vec();
        for (widths, wanted) in [(&mut most, max - room), (&mut least, min - room)] {
            let have: f32 = widths[columns.clone()].iter().sum();
            if wanted > have {
                share(&mut widths[columns.clone()], &weights, wanted - have);
            }
        }
    }
    for (min, max) in least.iter_mut().zip(&mut most) {
        *max = max.max(*min);
    }
    let total_min = least.iter().sum::<f32>() + gaps;
    // Columns given shares of the table make it as wide as their shares
    // ask, when its width is automatic.
    let shared: f32 = shares.iter().sum::<f32>().min(0.99);
    let others: f32 = (0..count)
        .filter(|column| shares[*column] == 0.0)
        .map(|column| most[column])
        .sum();
    let total_max = (0..count)
        .filter(|column| shares[*column] > 0.0)
        .map(|column| most[column] / shares[column])
        .fold(others / (1.0 - shared), f32::max)
        .max(most.iter().sum::<f32>())
        + gaps;
    let room = match (known, inputs.available_space.width) {
        (None, AvailableSpace::MinContent) => total_min,
        (None, AvailableSpace::MaxContent) => total_max,
        _ => room_for(total_max).unwrap_or(total_max),
    };
    if room <= total_min {
        return Some(least);
    }
    // Shares come first, then the other columns as tables lay them out.
    let mut widths = least.clone();
    let mut left = room - gaps;
    for column in 0..count {
        if shares[column] > 0.0 {
            widths[column] = (shares[column] * (room - gaps)).max(least[column]);
            left -= widths[column];
        }
    }
    let rest: Vec<usize> = (0..count).filter(|column| shares[*column] == 0.0).collect();
    let (rest_min, rest_max) = (
        rest.iter().map(|column| least[*column]).sum::<f32>(),
        rest.iter().map(|column| most[*column]).sum::<f32>(),
    );
    if left >= rest_max {
        // Room to spare goes to the automatic columns as their maximum
        // widths are, then to those cells give lengths, then to the shared.
        for column in &rest {
            widths[*column] = most[*column];
        }
        let extra = left - rest_max;
        let automatic: Vec<usize> = rest
            .iter()
            .copied()
            .filter(|column| !fixed[*column])
            .collect();
        let takers = if !automatic.is_empty() {
            automatic
        } else if !rest.is_empty() {
            rest.clone()
        } else {
            (0..count).collect()
        };
        let mut taken: Vec<f32> = takers.iter().map(|column| widths[*column]).collect();
        let weights: Vec<f32> = takers.iter().map(|column| most[*column]).collect();
        share(&mut taken, &weights, extra);
        for (column, width) in takers.into_iter().zip(taken) {
            widths[column] = width;
        }
    } else if left > rest_min {
        let part = (left - rest_min) / (rest_max - rest_min);
        for column in rest {
            widths[column] = least[column] + (most[column] - least[column]) * part;
        }
    }
    Some(widths)
}

/// Adds `extra` to `widths` in proportion to `weights`, or evenly when
/// they are all zero.
fn share(widths: &mut [f32], weights: &[f32], extra: f32) {
    let total: f32 = weights.iter().sum();
    let count = widths.len().max(1) as f32;
    for (width, weight) in widths.iter_mut().zip(weights) {
        *width += if total > 0.0 {
            extra * weight / total
        } else {
            extra / count
        };
    }
}

/// A replaced element's style for its min-content contribution where its
/// percentages are cyclic: a percentage or `calc()` in its `width` or
/// `max-width` makes the contribution zero, its padding and border, as
/// Chromium compresses replaced elements; none without either.
fn compressed(style: &Style) -> Option<Box<Style>> {
    let relative = |length: taffy::style::CompactLength| {
        length.tag() == taffy::style::CompactLength::PERCENT_TAG || length.is_calc()
    };
    if !relative(style.size.width.into_raw()) && !relative(style.max_size.width.into_raw()) {
        return None;
    }
    let mut compressed = style.clone();
    compressed.max_size.width = taffy::style::LengthPercentageAuto::length(0.0);
    Some(Box::new(compressed))
}

/// Whether `width` or `max-width` depends on the containing block's width.
fn percentage_width(style: &ComputedStyle) -> bool {
    use nativeboat_css::types::{MaxSize, Size};
    use nativeboat_css::values::LengthPercentage;
    let relative = |length: &LengthPercentage| !matches!(length, LengthPercentage::Px(_));
    matches!(&style.width, Size::Length(length) if relative(length))
        || matches!(&style.max_width, MaxSize::Length(length) if relative(length))
}

#[cfg(test)]
mod tests;
