//! The native UI of a laid-out document: surfaces for the window and scroll
//! containers, display lists of painted content and native controls.
//!
//! Painting follows CSS stacking in a simplified form: every positioned,
//! translucent or transformed element is a stacking context whose positioned
//! descendants are painted after its in-flow content, ordered by `z-index`.
//! A positioned element with `z-index: auto` that nothing else makes a
//! stacking context leaves its descendants with a positive `z-index` to the
//! enclosing stacking context, as CSS orders them.
//! Scroll containers become child surfaces that stack their own content.

use crate::prelude::*;

use crate::inline::Symbol;
use crate::styles::Styles;
use crate::text::font_of;
use crate::tree::{BoxGeometry, InlineBoxRect, LayoutResult, TextRun, button_caption};
use nativeboat_css::types::{
    BackfaceVisibility, BackgroundImage, BorderStyle, ClipPath, ColorStop, Display, Isolation,
    LengthPercentageAuto, Overflow, PointerEvents, PositionKind, Px, Radius, Resize, Visibility,
};
use nativeboat_css::types::{FillRule, Filter, LineCap, LineJoin};
use nativeboat_css::{Color, ComputedStyle, PseudoElement};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::scrollbar::{self, Hit as Part};
use nativeboat_ir::{
    Control, ControlKind, ControlState, Corner, DisplayItem, NativeChild, NativeTree, PathSegment,
    Radii, Rect, Stroke, Surface, control_for,
};

mod caret;
mod clip_path;
mod geometry;
mod highlight;
mod toggle;
pub use caret::TextQuery;
use clip_path::ClipShape;
use geometry::{Geometry, IDENTITY};
pub use highlight::{Caret, Highlight, Span};

/// The native UI of a document and what the runtime needs to route input.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    /// The native UI tree.
    pub tree: NativeTree,
    rects: HashMap<NodeId, Rect>,
    hits: Vec<Hit>,
    clips: Vec<Clip>,
    /// The outlines of clips to paths, in window coordinates before any
    /// transform, which clips name by index.
    clip_paths: Vec<(Vec<PathSegment>, FillRule)>,
    texts: Vec<Placed>,
    /// Text laid out but not shown, `visibility: hidden` or at
    /// `font-size: 0`: script geometry reports it, as Chromium does, but
    /// carets from points and the pointer pass it by.
    unshown: Vec<Placed>,
    /// The resize grips of boxes the user can resize, in paint order.
    resizers: Vec<Hit>,
    /// Scroll containers painted in place, in paint order.
    scrollers: Vec<InPlace>,
}

/// A scroll container painted in place, under a transform that scales or
/// rotates it: where its padding box and scrollbars are, and how far it
/// scrolls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InPlace {
    /// The scroll container.
    pub node: NodeId,
    region: Geometry,
    clip: Option<usize>,
    /// Its scrollbars, relative to its padding box.
    pub bars: scrollbar::Geometry,
    /// The size of the area its content scrolls in.
    pub view: (f32, f32),
    /// Its largest scroll offsets.
    pub maximum: (f32, f32),
}

impl InPlace {
    /// A window point relative to the padding box, before the transform.
    pub fn local(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (x, y) = self.region.local(x, y)?;
        Some((x - self.region.rect.x, y - self.region.rect.y))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Hit {
    region: Geometry,
    clip: Option<usize>,
    node: NodeId,
}

/// A painted text run: its index in the layout's runs and where it shows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    run: usize,
    region: Geometry,
    clip: Option<usize>,
}

/// A persistent clip chain shared by descendants. Each clip retains the
/// transform active when it was established.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clip {
    region: Geometry,
    parent: Option<usize>,
    /// The window position of the content origin it was set in.
    window: (f32, f32),
    /// The index of the path it clips to within `region`, if any.
    path: Option<usize>,
}

impl Scene {
    /// The border box of a rendered element in window coordinates, as
    /// `getBoundingClientRect` reports it; inline elements report the union
    /// of their text.
    pub fn client_rect(&self, node: NodeId) -> Option<Rect> {
        self.rects.get(&node).copied()
    }

    /// The topmost element painted at a point in window coordinates.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.region.contains(x, y) && self.unclipped(hit.clip, x, y))
            .map(|hit| hit.node)
    }

    /// The box whose resize grip is painted at a point in window
    /// coordinates, the topmost first.
    pub fn resizer_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.resizers
            .iter()
            .rev()
            .find(|hit| hit.region.contains(x, y) && self.unclipped(hit.clip, x, y))
            .map(|hit| hit.node)
    }

    /// The scroll container painted in place whose scrollbar is at a point
    /// in window coordinates, the topmost first, and the part there.
    pub fn scrollbar_at(&self, x: f32, y: f32) -> Option<(&InPlace, Part)> {
        self.scrollers.iter().rev().find_map(|scroller| {
            let (local_x, local_y) = scroller.local(x, y)?;
            let part = scroller.bars.hit(local_x, local_y)?;
            self.unclipped(scroller.clip, x, y)
                .then_some((scroller, part))
        })
    }

    /// A scroll container painted in place.
    pub fn in_place(&self, node: NodeId) -> Option<&InPlace> {
        self.scrollers.iter().find(|scroller| scroller.node == node)
    }

    /// Whether an element is a native control kept out of sight, in a
    /// rotated or skewed box, which takes the pointer where its image
    /// shows; selects open the page's own pop-up instead.
    pub fn takes_pointer(&self, node: NodeId) -> bool {
        let id = node.index();
        let mut pending = vec![&self.tree.root];
        while let Some(surface) = pending.pop() {
            if let Some(control) = surface.controls.iter().find(|control| control.id == id) {
                return control.state.transform.is_some() && control.kind != ControlKind::ComboBox;
            }
            pending.extend(&surface.children);
        }
        false
    }

    /// Whether a scroll container scrolls on a native surface of its own.
    pub fn has_surface(&self, node: NodeId) -> bool {
        let id = node.index();
        let mut pending = vec![&self.tree.root];
        while let Some(surface) = pending.pop() {
            if surface.id == id {
                return true;
            }
            pending.extend(&surface.children);
        }
        false
    }

    /// Whether a point lies inside a clip chain.
    fn unclipped(&self, mut clip: Option<usize>, x: f32, y: f32) -> bool {
        while let Some(index) = clip {
            let area = self.clips[index];
            let Some((local_x, local_y)) = area.region.contains_local(x, y) else {
                return false;
            };
            if let Some((segments, rule)) = area.path.map(|path| &self.clip_paths[path])
                && !geometry::path_contains(segments, *rule, local_x, local_y)
            {
                return false;
            }
            clip = area.parent;
        }
        true
    }
}

/// Where a surface's content sits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    /// The document position drawn at the content origin.
    origin: (f32, f32),
    /// The window position of the content origin.
    window: (f32, f32),
    /// The visible area in window coordinates.
    clip: Option<Rect>,
    /// Maps untransformed window coordinates to painted coordinates.
    transform: [f32; 6],
    hit_clip: Option<usize>,
    /// The nearest scrollport in document coordinates, including its scroll.
    scrollport: Rect,
    /// The opacity of the translucent boxes around the content.
    opacity: f32,
    /// Where fixed-position descendants' containing block starts.
    fixed: Fixed,
    /// The innermost clip at the content of the containing block of
    /// absolutely positioned descendants, and the visible area there: clips
    /// between it and them do not clip them.
    positioned: (Option<usize>, Option<Rect>),
    /// Whether text is smoothed in grayscale: Chromium gives up ClearType
    /// in a composited layer, unless all its text lies on what it paints
    /// opaque.
    grayscale: bool,
    /// Whether text is smoothed in grayscale whatever lies beneath: Chromium
    /// gives up ClearType under effects that could fringe it.
    no_lcd: bool,
}

/// The containing block of fixed-position boxes: the window, or a
/// transformed or filtered box. Boxes inside it that clip their overflow do
/// not clip fixed-position descendants, as a dialog's overlay inside a
/// clipping editor still covers the window.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fixed {
    /// The display list length where the containing block's content starts:
    /// clips opened after it do not clip fixed-position boxes.
    mark: usize,
    /// The visible area in window coordinates there.
    clip: Option<Rect>,
    hit_clip: Option<usize>,
}

impl Frame {
    /// Converts document coordinates to surface content coordinates.
    fn local(&self, rect: Rect) -> Rect {
        rect.offset(-self.origin.0, -self.origin.1)
    }

    /// Converts surface content coordinates to window coordinates.
    fn window(&self, rect: Rect) -> Rect {
        rect.offset(self.window.0, self.window.1)
    }

    fn region(&self, rect: Rect) -> Geometry {
        Geometry {
            rect: self.window(rect),
            transform: self.transform,
        }
    }

    fn hit(&self, rect: Rect, node: NodeId) -> Hit {
        Hit {
            region: self.region(rect),
            clip: self.hit_clip,
            node,
        }
    }
}

/// An element painted as a unit.
#[derive(Debug, Clone)]
struct Layer {
    generated: bool,
    /// The pseudo-element of a generated box.
    pseudo: Option<PseudoElement>,
    node: NodeId,
    z: i32,
    /// Whether `z-index` is `auto`.
    auto: bool,
    style: Rc<ComputedStyle>,
    geometry: BoxGeometry,
    frame: Frame,
}

/// How a box treats overflowing content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overflowing {
    Visible,
    Clip,
    Scroll,
}

/// Whether a transform only moves what it maps.
fn translates_only(transform: [f32; 6]) -> bool {
    let near = |value: f32, expected: f32| (value - expected).abs() < 1e-6;
    near(transform[0], 1.0)
        && near(transform[1], 0.0)
        && near(transform[2], 0.0)
        && near(transform[3], 1.0)
}

fn overflow_of(style: &ComputedStyle) -> Overflowing {
    let (x, y) = (style.overflow_x, style.overflow_y);
    let scrolls = |overflow: Overflow| matches!(overflow, Overflow::Auto | Overflow::Scroll);
    // `visible` next to `hidden` computes to `auto`.
    let promoted = (x == Overflow::Visible) != (y == Overflow::Visible)
        && (x == Overflow::Hidden || y == Overflow::Hidden);
    if scrolls(x) || scrolls(y) || promoted {
        Overflowing::Scroll
    } else if x != Overflow::Visible || y != Overflow::Visible {
        Overflowing::Clip
    } else {
        Overflowing::Visible
    }
}

fn clamp(value: f32, max: f32) -> f32 {
    value.min(max).max(0.0)
}

/// Sticky movement is bounded by both the scrollport and containing block.
#[allow(clippy::too_many_arguments)]
fn sticky_offset(
    start: f32,
    size: f32,
    view: f32,
    extent: f32,
    parent: f32,
    parent_extent: f32,
    before: Option<f32>,
    after: Option<f32>,
) -> f32 {
    if before.is_none() && after.is_none() {
        return 0.0;
    }
    let leading = before.unwrap_or(0.0);
    // When the box is larger than the sticky view, reduce the end inset.
    let trailing = after.unwrap_or(0.0).min(extent - leading - size);
    let mut position = start;
    if after.is_some() {
        position = position.min(view + extent - trailing - size);
    }
    if before.is_some() {
        position = position.max(view + leading);
    }
    position.clamp(
        parent.min(start),
        (parent + parent_extent - size).max(start),
    ) - start
}

/// Whether a display item is a shadow that shows over part of `rect`: an
/// outer one outside its box, or an inset one inside it.
fn shadow_falls_on(item: &DisplayItem, rect: Rect) -> bool {
    let DisplayItem::Shadow {
        rect: shape,
        blur,
        inset,
        clip,
        ..
    } = item
    else {
        return false;
    };
    let overlaps = |a: Rect, b: Rect| {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    };
    let reach = Rect::new(
        shape.x - blur,
        shape.y - blur,
        shape.width + 2.0 * blur,
        shape.height + 2.0 * blur,
    );
    let covers = |outer: Rect, inner: Rect| {
        outer.x <= inner.x
            && outer.y <= inner.y
            && outer.x + outer.width >= inner.x + inner.width
            && outer.y + outer.height >= inner.y + inner.height
    };
    if *inset {
        overlaps(*clip, rect)
    } else {
        // An outer shadow is not drawn inside its own box, which may hold the
        // surface.
        overlaps(reach, rect) && !covers(*clip, rect)
    }
}

/// Copies onto child surfaces the outer shadows painted after them that
/// fall on them, as a positioned tab strip's shadow on the editor below it:
/// Chromium paints such shadows over the editor, but a child surface's
/// window covers what its parent paints there. Shadows inside clips,
/// transforms, opacity groups or filters stay with the parent.
fn shadows_over_children(surface: &mut Surface) {
    let mut depth = 0usize;
    let mut painted: Vec<u64> = Vec::new();
    let mut copies: Vec<(usize, DisplayItem)> = Vec::new();
    for item in &surface.items {
        match item {
            DisplayItem::PushTransform(_)
            | DisplayItem::PushClip { .. }
            | DisplayItem::PushClipPath { .. }
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => depth += 1,
            DisplayItem::PopTransform
            | DisplayItem::PopClip
            | DisplayItem::PopOpacity
            | DisplayItem::PopFilter
            | DisplayItem::PopMask => depth = depth.saturating_sub(1),
            DisplayItem::Child(NativeChild::Surface(id)) if depth == 0 => painted.push(*id),
            DisplayItem::Shadow {
                rect,
                clip,
                inset: false,
                ..
            } if depth == 0 => {
                for (index, child) in surface.children.iter().enumerate() {
                    if !painted.contains(&child.id) || !shadow_falls_on(item, child.rect) {
                        continue;
                    }
                    // Into the child's content, which its scroll moves.
                    let (dx, dy) = (child.scroll_x - child.rect.x, child.scroll_y - child.rect.y);
                    let mut copy = item.clone();
                    if let DisplayItem::Shadow {
                        rect: shape,
                        clip: area,
                        ..
                    } = &mut copy
                    {
                        *shape = rect.offset(dx, dy);
                        *area = clip.offset(dx, dy);
                    }
                    copies.push((index, copy));
                }
            }
            _ => {}
        }
    }
    for (index, copy) in copies {
        surface.children[index].items.push(copy);
    }
}

/// Closes the clips opened in the display list since `mark` that no other
/// group encloses, innermost first, and returns them, outermost first, to
/// be opened again.
fn escape_clips(mark: usize, surface: &mut Surface) -> Vec<DisplayItem> {
    let mut open: Vec<Option<DisplayItem>> = Vec::new();
    for item in &surface.items[mark.min(surface.items.len())..] {
        match item {
            DisplayItem::PushClip { .. } | DisplayItem::PushClipPath { .. } => {
                open.push(Some(item.clone()));
            }
            DisplayItem::PushTransform(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => open.push(None),
            DisplayItem::PopClip
            | DisplayItem::PopTransform
            | DisplayItem::PopOpacity
            | DisplayItem::PopFilter
            | DisplayItem::PopMask => {
                open.pop();
            }
            _ => {}
        }
    }
    let mut closed = Vec::new();
    while let Some(Some(clip)) = open.last().cloned() {
        open.pop();
        surface.items.push(DisplayItem::PopClip);
        closed.push(clip);
    }
    closed.reverse();
    closed
}

fn intersect(clip: Option<Rect>, rect: Rect) -> Rect {
    clip.map_or(rect, |clip| {
        let x = clip.x.max(rect.x);
        let y = clip.y.max(rect.y);
        let right = (clip.x + clip.width).min(rect.x + rect.width);
        let bottom = (clip.y + clip.height).min(rect.y + rect.height);
        Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
    })
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect::new(x, y, right - x, bottom - y)
}

/// The largest rectangle two rectangles known to be opaque cover together,
/// as Chromium's `gfx::MaximumCoveredRect` finds it: the larger of the two,
/// or their overlap stretched across both, along either axis.
fn maximum_covered(a: Rect, b: Rect) -> Rect {
    let area = |rect: Rect| rect.width * rect.height;
    let mut best = if area(b) > area(a) { b } else { a };
    // Rectangles that touch count as overlapping along their shared edge.
    let touch = a.x <= b.x + b.width
        && b.x <= a.x + a.width
        && a.y <= b.y + b.height
        && b.y <= a.y + a.height;
    let overlap = intersect(Some(a), b);
    if touch && (overlap.width > 0.0 || overlap.height > 0.0) {
        let top = a.y.min(b.y);
        let bottom = (a.y + a.height).max(b.y + b.height);
        let tall = Rect::new(overlap.x, top, overlap.width, bottom - top);
        let left = a.x.min(b.x);
        let right = (a.x + a.width).max(b.x + b.width);
        let wide = Rect::new(left, overlap.y, right - left, overlap.height);
        for candidate in [tall, wide] {
            if area(candidate) > area(best) {
                best = candidate;
            }
        }
    }
    best
}

/// Whether filters keep ClearType, as Chromium's
/// `FilterOperations::AllowsLCDText`: none, or from Chromium 135 on, one
/// that does not pull the colors of ClearType's fringes apart.
fn allows_lcd_text(filters: &[Filter], chromium: u32) -> bool {
    match filters {
        [] => true,
        _ if chromium < 135 => false,
        [filter] => match filter {
            Filter::Grayscale(_)
            | Filter::Sepia(_)
            | Filter::Opacity(_)
            | Filter::Brightness(_)
            | Filter::Blur(_)
            | Filter::DropShadow { .. } => true,
            Filter::Contrast(amount) | Filter::Saturate(amount) => *amount <= 1.0,
            Filter::Invert(amount) => *amount <= 0.5,
            Filter::HueRotate(_) => false,
        },
        _ => false,
    }
}

/// The last element with a box, in tree order, whose backdrop filter
/// could fringe ClearType.
fn lcd_backdrop(
    dom: &Dom,
    styles: &Styles,
    layout: &LayoutResult,
    chromium: u32,
) -> Option<NodeId> {
    let fringes = |node: &NodeId| {
        styles
            .get(*node)
            .is_some_and(|style| !allows_lcd_text(&style.backdrop_filter.0, chromium))
    };
    if !layout.boxes.keys().any(fringes) {
        return None;
    }
    dom.descendants(Dom::DOCUMENT)
        .into_iter()
        .filter(|node| layout.boxes.contains_key(node))
        .rfind(fringes)
}

/// Whether a style makes a stacking context.
fn stacks(style: &ComputedStyle) -> bool {
    (style.position != PositionKind::Static && style.z_index.0.is_some())
        || matches!(style.position, PositionKind::Fixed | PositionKind::Sticky)
        || style.opacity < 1.0
        || style.has_transform()
        || !style.filter.0.is_empty()
        || !style.backdrop_filter.0.is_empty()
        || style.clip_path != ClipPath::None
        || style.isolation == Isolation::Isolate
}

/// Corner radii resolved against a box and scaled down to fit it.
fn radii(style: &ComputedStyle, rect: Rect) -> Radii {
    let corners = &style.border_radius;
    fit_radii(
        [
            &corners.top_left,
            &corners.top_right,
            &corners.bottom_right,
            &corners.bottom_left,
        ],
        rect,
    )
}

/// The corners of an edge inset from a box's by `insets`, top, right,
/// bottom and left: each corner's radii less the insets beside it, across
/// and down.
fn inset_radii(radii: Radii, insets: [f32; 4]) -> Radii {
    // The sides beside each corner, clockwise from top left: left or right
    // across, top or bottom down.
    let sides = [(3, 0), (1, 0), (1, 2), (3, 2)];
    core::array::from_fn(|corner| {
        let (across, down) = sides[corner];
        radii[corner].less(insets[across], insets[down])
    })
}

/// The corners of a box's content edge: its border radii less the border
/// and padding beside each corner.
fn content_radii(geometry: &BoxGeometry, radii: Radii) -> Radii {
    inset_radii(
        radii,
        core::array::from_fn(|side| geometry.border[side] + geometry.padding[side]),
    )
}

/// Corners' radii, top left first, resolved against a box and scaled down
/// to fit it as CSS fits them: all by the one factor that keeps every
/// side's two radii within its length.
fn fit_radii(corners: [&Radius; 4], rect: Rect) -> Radii {
    let radii = corners.map(|radius| {
        let corner = Corner {
            x: radius.horizontal.resolve(rect.width).max(0.0),
            y: radius.vertical.resolve(rect.height).max(0.0),
        };
        if corner.is_square() {
            Corner::SQUARE
        } else {
            corner
        }
    });
    let fit = |length: f32, a: f32, b: f32| {
        if a + b > length {
            length / (a + b)
        } else {
            1.0
        }
    };
    let scale = fit(rect.width, radii[0].x, radii[1].x)
        .min(fit(rect.width, radii[3].x, radii[2].x))
        .min(fit(rect.height, radii[0].y, radii[3].y))
        .min(fit(rect.height, radii[1].y, radii[2].y));
    radii.map(|radius| radius.scaled(scale))
}

/// Spread preserves sharp corners and grows small radii continuously.
fn spread_radius(radius: f32, grow: f32) -> f32 {
    let grow = if radius < grow {
        grow * (1.0 + crate::math::pow(radius / grow - 1.0, 3.0))
    } else {
        grow
    };
    (radius + grow).max(0.0)
}

/// A corner spread by `grow`, each radius as `spread_radius` grows it.
fn spread_corner(corner: Corner, grow: f32) -> Corner {
    Corner {
        x: spread_radius(corner.x, grow),
        y: spread_radius(corner.y, grow),
    }
}

/// Gradient stops with positions; unpositioned stops are spread evenly.
fn gradient_stops(stops: &[ColorStop]) -> Vec<(Color, f32)> {
    let last = stops.len().saturating_sub(1);
    let fixed: Vec<Option<f32>> = stops
        .iter()
        .enumerate()
        .map(|(index, stop)| {
            stop.position.or(match index {
                0 => Some(0.0),
                index if index == last => Some(1.0),
                _ => None,
            })
        })
        .collect();
    let mut previous = f32::NEG_INFINITY;
    let mut result = Vec::with_capacity(stops.len());
    for (index, stop) in stops.iter().enumerate() {
        let position = fixed[index].unwrap_or_else(|| {
            let (end, next) = fixed[index..]
                .iter()
                .enumerate()
                .find_map(|(offset, position)| position.map(|position| (index + offset, position)))
                .unwrap_or((last, 1.0));
            previous + (next.max(previous) - previous) / (end - index + 1) as f32
        });
        previous = position.max(previous);
        result.push((stop.color, previous));
    }
    result
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Painter<'a> {
    dom: &'a Dom,
    styles: &'a Styles,
    layout: &'a LayoutResult,
    /// Text runs by container, with their indexes in the layout.
    runs: HashMap<NodeId, Vec<(usize, &'a TextRun)>>,
    /// The runs of generated boxes painted as layers, by originating element.
    pseudo_runs: HashMap<NodeId, Vec<(usize, &'a TextRun)>>,
    /// The line parts of inline boxes by container.
    inline_boxes: HashMap<NodeId, Vec<&'a InlineBoxRect>>,
    /// The element whose background paints the whole canvas.
    canvas: Option<NodeId>,
    canvas_color: Color,
    /// The element whose `overflow` applies to the window.
    viewport_overflow: Option<NodeId>,
    root_scroll: (f32, f32),
    /// The selection and caret painted with the text.
    highlight: Option<&'a Highlight<'a>>,
    /// The character boundaries of the runs of highlighted text nodes.
    bounds: HashMap<NodeId, Vec<(usize, Vec<u32>)>>,
    /// The caret: the box it is painted with, where, and its color.
    caret: Option<(NodeId, Rect, Color)>,
    /// The last element, in tree order, whose backdrop filter keeps
    /// ClearType from what paints before it.
    lcd_backdrop: Option<NodeId>,
    /// The Chromium release whose rules for ClearType text apply.
    chromium: u32,
    scene: Scene,
}

/// Where a box's content starts in the display list and hit list, and the
/// clip its content is painted in.
struct Painted {
    items: usize,
    hits: usize,
    clip: Option<(Rect, Radii)>,
}

impl Painter<'_> {
    fn layer(&self, node: NodeId, frame: Frame) -> Option<Layer> {
        let style = self.styles.get(node)?;
        let geometry = self.layout.boxes.get(&node)?;
        Some(Layer {
            generated: false,
            pseudo: None,
            node,
            z: style.z_index.0.unwrap_or(0),
            auto: style.z_index.0.is_none(),
            style: style.clone(),
            geometry: *geometry,
            frame,
        })
    }

    fn generated_layer(&self, node: NodeId, pseudo: &PseudoElement, frame: Frame) -> Option<Layer> {
        let geometry = self
            .layout
            .generated
            .get(&node)?
            .iter()
            .find(|value| value.pseudo == *pseudo)?
            .geometry;
        let style = self.styles.pseudo(node, pseudo)?.clone();
        Some(Layer {
            generated: true,
            pseudo: Some(pseudo.clone()),
            node,
            z: style.z_index.0.unwrap_or(0),
            auto: style.z_index.0.is_none(),
            style,
            geometry,
            frame,
        })
    }

    /// Paints the box of an element's generated content: where it lies in
    /// the flow, or with the layers when it is positioned, transformed or
    /// translucent.
    fn generated_box(
        &mut self,
        node: NodeId,
        pseudo: &PseudoElement,
        frame: Frame,
        surface: &mut Surface,
        layers: &mut Vec<Layer>,
    ) {
        let Some(layer) = self.generated_layer(node, pseudo, frame) else {
            return;
        };
        let style = &layer.style;
        // Masks and clip paths, as progress badges mask a box to an icon,
        // apply in the stacking context the box makes, as the element's do.
        if style.position != PositionKind::Static
            || style.has_transform()
            || style.opacity < 1.0
            || !style.filter.0.is_empty()
            || !style.backdrop_filter.0.is_empty()
            || style.clip_path != ClipPath::None
            || style.mask_image != BackgroundImage::None
        {
            layers.push(layer);
        } else {
            self.paint_box(&layer, surface, layers);
        }
    }

    fn element(
        &mut self,
        node: NodeId,
        frame: Frame,
        surface: &mut Surface,
        layers: &mut Vec<Layer>,
    ) {
        match self.layer(node, frame) {
            Some(layer) if layer.style.position != PositionKind::Static => layers.push(layer),
            Some(layer)
                if layer.style.opacity < 1.0
                    || layer.style.has_transform()
                    || !layer.style.filter.0.is_empty()
                    || !layer.style.backdrop_filter.0.is_empty()
                    || layer.style.clip_path != ClipPath::None
                    || layer.style.mask_image != BackgroundImage::None
                    || layer.style.isolation == Isolation::Isolate =>
            {
                // Opacity, transforms, filters, clip paths and isolation
                // make stacking contexts, which keep their layers.
                let escaped = self.context(layer, surface);
                debug_assert!(escaped.is_empty());
            }
            Some(layer) => {
                self.paint_box(&layer, surface, layers);
            }
            None => {
                // Inline and `display: contents` elements have no box of
                // their own but may hold boxes.
                if self
                    .styles
                    .get(node)
                    .is_some_and(|style| style.display != Display::None)
                {
                    layers.extend(self.generated_layer(node, &PseudoElement::Before, frame));
                    // Positioned generated text placed against the element.
                    for (index, run) in self.runs.remove(&node).unwrap_or_default() {
                        self.text(index, run, node, frame, surface);
                    }
                    for child in self.dom.flat_child_elements(node) {
                        self.element(child, frame, surface, layers);
                    }
                    layers.extend(self.generated_layer(node, &PseudoElement::After, frame));
                }
            }
        }
    }

    /// Paints an element as a stacking context. Returns the layers of
    /// descendants with a `z-index` that belong to the enclosing stacking
    /// context, when the element makes none of its own.
    fn context(&mut self, layer: Layer, surface: &mut Surface) -> Vec<Layer> {
        let mut layer = layer;
        let style = layer.style.clone();
        // An element turned away from the viewer, with its descendants, is
        // not drawn when its back is hidden.
        if style.backface_visibility == BackfaceVisibility::Hidden
            && style
                .full_transform()
                .faces_back(layer.geometry.rect.width, layer.geometry.rect.height)
        {
            return Vec::new();
        }
        if style.position == PositionKind::Fixed {
            layer.frame.origin.0 -= self.root_scroll.0;
            layer.frame.origin.1 -= self.root_scroll.1;
            layer.frame.clip = layer.frame.fixed.clip;
            layer.frame.hit_clip = layer.frame.fixed.hit_clip;
        }
        // An absolutely positioned box shows and takes the pointer outside
        // the clips between it and its containing block, in the same
        // surface.
        if style.position == PositionKind::Absolute
            && let (hit_clip, clip) = layer.frame.positioned
            && hit_clip
                .and_then(|index| self.scene.clips.get(index))
                .is_some_and(|found| found.window == layer.frame.window)
        {
            layer.frame.clip = clip;
            layer.frame.hit_clip = hit_clip;
        }
        if style.position == PositionKind::Sticky {
            let port = layer.frame.scrollport;
            let rect = layer.geometry.rect;
            let parent = core::iter::successors(self.dom.parent_element(layer.node), |node| {
                self.dom.parent_element(*node)
            })
            .filter(|node| {
                self.styles.get(*node).is_some_and(|style| {
                    !matches!(style.display, Display::Inline | Display::Contents)
                })
            })
            .find_map(|node| self.layout.boxes.get(&node))
            .map_or(rect, BoxGeometry::content_box);
            // Insets constrain the border box; containing-block bounds
            // constrain the margin box. A margin extending beyond its
            // containing block is limited to the original available gap.
            let margin = layer.geometry.margin;
            let left = margin[3].min(rect.x - parent.x);
            let right = margin[1].min(parent.x + parent.width - rect.x - rect.width);
            let top = margin[0].min(rect.y - parent.y);
            let bottom = margin[2].min(parent.y + parent.height - rect.y - rect.height);
            let inset = |edge: &LengthPercentageAuto, size: f32| match edge {
                LengthPercentageAuto::Auto => None,
                LengthPercentageAuto::Length(value) => Some(value.resolve(size)),
            };
            let x = sticky_offset(
                rect.x,
                rect.width,
                port.x,
                port.width,
                parent.x + left,
                parent.width - left - right,
                inset(&style.inset.left, port.width),
                inset(&style.inset.right, port.width),
            );
            let y = sticky_offset(
                rect.y,
                rect.height,
                port.y,
                port.height,
                parent.y + top,
                parent.height - top - bottom,
                inset(&style.inset.top, port.height),
                inset(&style.inset.bottom, port.height),
            );
            layer.frame.origin.0 -= x;
            layer.frame.origin.1 -= y;
        }
        // The transform's translation moves the layer; the rest of it
        // turns, scales and skews it about its origin.
        let [a, b, c, d, moved_x, moved_y] = style
            .full_transform()
            .matrix(layer.geometry.rect.width, layer.geometry.rect.height);
        layer.frame.origin.0 -= moved_x;
        layer.frame.origin.1 -= moved_y;
        let translucent = style.opacity < 1.0;
        if translucent {
            surface
                .items
                .push(DisplayItem::PushOpacity(style.opacity.max(0.0)));
            // Surfaces inside show through it too.
            layer.frame.opacity *= style.opacity.max(0.0);
        }
        let transformed = [a, b, c, d] != [1.0, 0.0, 0.0, 1.0];
        let clip = if matches!(style.position, PositionKind::Absolute | PositionKind::Fixed) {
            style.clip.map(|clip| {
                let rect = layer.frame.local(layer.geometry.rect);
                let left = clip.left.unwrap_or(0.0);
                let top = clip.top.unwrap_or(0.0);
                Rect::new(
                    rect.x + left,
                    rect.y + top,
                    (clip.right.unwrap_or(rect.width) - left).max(0.0),
                    (clip.bottom.unwrap_or(rect.height) - top).max(0.0),
                )
            })
        } else {
            None
        };
        if transformed {
            // CSS origins resolve against the border box, including
            // percentages.
            let rect = layer.frame.local(layer.geometry.rect);
            let (cx, cy) = (
                rect.x + style.transform_origin.x.resolve(rect.width),
                rect.y + style.transform_origin.y.resolve(rect.height),
            );
            let transform = [a, b, c, d, cx - (a * cx + c * cy), cy - (b * cx + d * cy)];
            surface.items.push(DisplayItem::PushTransform(transform));
            // The display list uses surface coordinates. Move its matrix
            // into window coordinates before composing ancestor transforms.
            let (x, y) = layer.frame.window;
            let window = [
                a,
                b,
                c,
                d,
                transform[4] + x - a * x - c * y,
                transform[5] + y - b * x - d * y,
            ];
            layer.frame.transform = crate::svg::multiply(layer.frame.transform, window);
        }
        // `clip` and `clip-path` clip the element and its descendants, in
        // its transformed coordinates.
        let border = layer.frame.local(layer.geometry.rect);
        let clips: Vec<ClipShape> = clip
            .map(|rect| ClipShape::Rounded(rect, [Corner::SQUARE; 4]))
            .into_iter()
            .chain(clip_path::clip_path(
                &style,
                &layer.geometry,
                border,
                self.dom,
            ))
            .collect();
        for shape in &clips {
            match shape {
                ClipShape::Rounded(rect, radii) => {
                    let window = layer.frame.window(*rect);
                    self.clip_frame(&mut layer.frame, window, None);
                    surface.items.push(DisplayItem::PushClip {
                        rect: *rect,
                        radii: *radii,
                    });
                }
                ClipShape::Path(segments, rule) => {
                    let bounds = clip_path::path_bounds(segments);
                    let window = layer.frame.window(bounds);
                    let moved = crate::svg::transform_segments(
                        [1.0, 0.0, 0.0, 1.0, window.x - bounds.x, window.y - bounds.y],
                        segments,
                    );
                    self.clip_frame(&mut layer.frame, window, Some((moved, *rule)));
                    surface.items.push(DisplayItem::PushClipPath {
                        segments: segments.clone(),
                        fill_rule: *rule,
                    });
                }
            }
        }
        // A mask keeps what its image's alpha covers, over the border box.
        let masked = style.mask_image != BackgroundImage::None;
        if masked {
            let rect = layer.frame.local(layer.geometry.rect);
            let mut mask = Vec::new();
            self.background_image(
                &style.mask_image,
                &ImageLayer::mask(&style),
                (rect, rect, [Corner::SQUARE; 4]),
                &mut mask,
            );
            surface.items.push(DisplayItem::PushMask(mask));
        }
        // Chromium composites an element with a backdrop filter into a layer
        // of its own, whose text loses ClearType unless all of it lies on
        // what the layer paints opaque.
        if !style.backdrop_filter.0.is_empty() {
            layer.frame.grayscale = !self.text_on_opaque(layer.node);
            // What paints after the last backdrop filter that could fringe
            // ClearType, in the element's own effect, keeps it, unless a
            // filter around the element still could.
            if !layer.generated && self.lcd_backdrop == Some(layer.node) {
                layer.frame.no_lcd =
                    core::iter::successors(self.dom.parent_element(layer.node), |node| {
                        self.dom.parent_element(*node)
                    })
                    .filter_map(|node| self.styles.get(node))
                    .any(|style| !allows_lcd_text(&style.filter.0, self.chromium));
            }
        }
        // What is behind the element shows filtered through its border box.
        if !style.backdrop_filter.0.is_empty() && style.visibility == Visibility::Visible {
            let rect = layer.frame.local(layer.geometry.rect);
            surface.items.push(DisplayItem::Backdrop {
                rect,
                radii: radii(&style, rect),
                filters: style.backdrop_filter.0.clone(),
            });
        }
        // Filters apply to the element and its descendants before clipping
        // and opacity.
        let filtered = !style.filter.0.is_empty();
        if !allows_lcd_text(&style.filter.0, self.chromium) {
            layer.frame.no_lcd = true;
        }
        if filtered {
            surface
                .items
                .push(DisplayItem::PushFilter(style.filter.0.clone()));
        }
        // Transformed and filtered boxes contain fixed-position descendants,
        // translated ones too.
        let contains_fixed = style.has_transform() || filtered;
        if contains_fixed {
            layer.frame.fixed = Fixed {
                mark: surface.items.len(),
                clip: layer.frame.clip,
                hit_clip: layer.frame.hit_clip,
            };
        }
        let mut layers = Vec::new();
        // Clips open around this box stay open around its layers.
        let outer = layer.frame.hit_clip;
        let painted = self.paint_box(&layer, surface, &mut layers);
        let stacks = !layer.auto
            || translucent
            || transformed
            || filtered
            || !style.backdrop_filter.0.is_empty()
            || style.isolation == Isolation::Isolate
            || matches!(style.position, PositionKind::Fixed | PositionKind::Sticky)
            || style.clip_path != ClipPath::None
            || masked
            || !clips.is_empty()
            || painted.clip.is_some();
        let mut escaped = Vec::new();
        if !stacks {
            // Layers with a z-index other than zero belong to the enclosing
            // stacking context, which paints negative ones beneath all its
            // content and positive ones after everything at zero.
            let (inside, outside) = layers
                .into_iter()
                .partition(|layer: &Layer| layer.auto || layer.z == 0);
            layers = inside;
            escaped = outside;
        }
        layers.sort_by_key(|layer| layer.z);
        let positive = layers.split_off(layers.partition_point(|layer| layer.z < 0));
        let mut negative = layers;
        // The layers at zero and above paint after the content; boxes among
        // them that make no stacking context hand back negative layers.
        negative.extend(self.layers(positive, (painted.clip, outer), contains_fixed, surface));
        if stacks {
            self.paint_beneath(
                negative,
                (painted.items, painted.hits),
                (painted.clip, outer),
                contains_fixed,
                surface,
            );
        } else {
            escaped.extend(negative);
        }
        if filtered {
            surface.items.push(DisplayItem::PopFilter);
        }
        if masked {
            surface.items.push(DisplayItem::PopMask);
        }
        for _ in &clips {
            surface.items.push(DisplayItem::PopClip);
        }
        if transformed {
            surface.items.push(DisplayItem::PopTransform);
        }
        if translucent {
            surface.items.push(DisplayItem::PopOpacity);
        }
        escaped
    }

    /// Paints negative layers in `z-index` order beneath the content that
    /// starts at `mark`, the display item and hit where it starts: after
    /// the box's own background.
    fn paint_beneath(
        &mut self,
        mut negative: Vec<Layer>,
        mark: (usize, usize),
        clips: (Option<(Rect, Radii)>, Option<usize>),
        contains_fixed: bool,
        surface: &mut Surface,
    ) {
        if negative.is_empty() {
            return;
        }
        negative.sort_by_key(|layer| layer.z);
        let items = surface.items.len();
        let hits = self.scene.hits.len();
        let deeper = self.layers(negative, clips, contains_fixed, surface);
        debug_assert!(deeper.is_empty());
        let beneath: Vec<DisplayItem> = surface.items.drain(items..).collect();
        surface.items.splice(mark.0..mark.0, beneath);
        let beneath: Vec<Hit> = self.scene.hits.drain(hits..).collect();
        self.scene.hits.splice(mark.1..mark.1, beneath);
    }

    /// Clips a frame to `rect`, in window coordinates, or to a path within
    /// it, which takes the pointer only where it covers.
    fn clip_frame(
        &mut self,
        frame: &mut Frame,
        rect: Rect,
        path: Option<(Vec<PathSegment>, FillRule)>,
    ) {
        frame.clip = Some(intersect(frame.clip, rect));
        let index = self.scene.clips.len();
        let path = path.map(|path| {
            self.scene.clip_paths.push(path);
            self.scene.clip_paths.len() - 1
        });
        self.scene.clips.push(Clip {
            region: Geometry {
                rect,
                transform: frame.transform,
            },
            parent: frame.hit_clip,
            window: frame.window,
            path,
        });
        frame.hit_clip = Some(index);
    }

    /// The clips of the boxes around a positioned box that clip it, in its
    /// content coordinates, outermost first: those of every box around a
    /// relatively positioned one, and around an absolutely positioned one
    /// those from its containing block out, in the same surface and
    /// transform. Its stacking context paints it outside them otherwise.
    fn clips_around(&self, layer: &Layer, outer: Option<usize>) -> Vec<Rect> {
        let mut at = match layer.style.position {
            PositionKind::Fixed => return Vec::new(),
            PositionKind::Absolute => layer.frame.positioned.0,
            _ => layer.frame.hit_clip,
        };
        let mut rects = Vec::new();
        // Clips already open where the stacking context paints stay so.
        while let Some(clip) = at
            .filter(|index| Some(*index) != outer)
            .and_then(|index| self.scene.clips.get(index))
        {
            if clip.window != layer.frame.window || clip.region.transform != layer.frame.transform {
                break;
            }
            rects.push(
                clip.region
                    .rect
                    .offset(-layer.frame.window.0, -layer.frame.window.1),
            );
            at = clip.parent;
        }
        rects.reverse();
        rects
    }

    /// Paints layers in `clip`; `contains_fixed` when the box they belong
    /// to is the containing block of fixed-position ones, which its clip
    /// then clips too.
    fn layers(
        &mut self,
        layers: Vec<Layer>,
        (clip, outer): (Option<(Rect, Radii)>, Option<usize>),
        contains_fixed: bool,
        surface: &mut Surface,
    ) -> Vec<Layer> {
        let mut negative = Vec::new();
        if layers.is_empty() {
            return negative;
        }
        if let Some((rect, radii)) = clip {
            surface.items.push(DisplayItem::PushClip { rect, radii });
        }
        // Layers that descendants hand back take their place by `z-index`,
        // after the layers before them in document order.
        let mut pending: alloc::collections::VecDeque<Layer> = layers.into();
        while let Some(mut layer) = pending.pop_front() {
            let fixed = layer.style.position == PositionKind::Fixed;
            if fixed && contains_fixed {
                layer.frame.fixed.mark = surface.items.len();
            }
            // A fixed-position box paints outside the clips of the boxes
            // between it and its containing block, in its place in the
            // stacking order.
            let reopened = if fixed {
                escape_clips(layer.frame.fixed.mark, surface)
            } else {
                Vec::new()
            };
            let mut clips = self.clips_around(&layer, outer);
            // The box's own clip, around all its layers, is open already.
            if let Some((own, _)) = clip
                && clips.first() == Some(&own)
            {
                clips.remove(0);
            }
            for rect in &clips {
                surface.items.push(DisplayItem::PushClip {
                    rect: *rect,
                    radii: [Corner::SQUARE; 4],
                });
            }
            for escaped in self.context(layer, surface) {
                // Negative ones paint beneath the content painted already,
                // where the stacking context puts them.
                if escaped.z < 0 {
                    negative.push(escaped);
                    continue;
                }
                let at = pending.partition_point(|layer| layer.z <= escaped.z);
                pending.insert(at, escaped);
            }
            for _ in &clips {
                surface.items.push(DisplayItem::PopClip);
            }
            surface.items.extend(reopened);
        }
        if clip.is_some() {
            surface.items.push(DisplayItem::PopClip);
        }
        negative
    }

    /// Paints a box and its in-flow content.
    fn paint_box(
        &mut self,
        layer: &Layer,
        surface: &mut Surface,
        layers: &mut Vec<Layer>,
    ) -> Painted {
        let Layer {
            node,
            style,
            geometry,
            frame,
            ..
        } = layer;
        let node = *node;
        let rect = frame.local(geometry.rect);
        if !layer.generated {
            self.scene.rects.insert(node, frame.region(rect).bounds());
        }
        let visible = style.visibility == Visibility::Visible;
        let radii = radii(style, rect);
        if visible {
            self.decorations(layer, rect, radii, &mut surface.items);
            if style.pointer_events == PointerEvents::Auto {
                self.scene.hits.push(frame.hit(rect, node));
            }
        }
        let mut painted = Painted {
            items: surface.items.len(),
            hits: self.scene.hits.len(),
            clip: None,
        };
        if layer.generated {
            // Generated text with a box of its own paints with the box.
            let runs = self.pseudo_runs.remove(&node).unwrap_or_default();
            let (own, rest): (Vec<_>, Vec<_>) = runs
                .into_iter()
                .partition(|(_, run)| run.pseudo == layer.pseudo);
            if !rest.is_empty() {
                self.pseudo_runs.insert(node, rest);
            }
            for (index, run) in own {
                self.text(index, run, node, *frame, surface);
            }
            return painted;
        }
        if let Some(kind) = crate::native_control(self.dom, node, Some(style)) {
            let mut state = self.control_state(node, kind, style);
            // Text fields that author CSS styles edit inside the box the
            // page draws.
            let editable = matches!(
                kind,
                ControlKind::TextBox
                    | ControlKind::Password
                    | ControlKind::TextArea
                    | ControlKind::Date
                    | ControlKind::Time
                    | ControlKind::DateTime
                    | ControlKind::Month
            );
            let toggle = matches!(kind, ControlKind::CheckBox | ControlKind::Radio);
            // A toggle made fully transparent, as switches lay one over
            // their track, shows the page there and still takes the clicks.
            let transparent = frame.opacity <= 0.0 || style.opacity <= 0.0;
            state.bare = (editable && style.author_appearance) || toggle;
            // Checkboxes and radio buttons look as Chromium draws them: the
            // page paints them and the control, which it shows through,
            // takes their input.
            if toggle && !style.author_appearance && !transparent && visible {
                use nativeboat_css::selector::state::{ACTIVE, FOCUS_VISIBLE, HOVER};
                let flags = self.dom.element_state(node);
                toggle::paint(
                    &mut surface.items,
                    rect,
                    &toggle::Look {
                        radio: kind == ControlKind::Radio,
                        checked: state.checked,
                        indeterminate: state.indeterminate && kind == ControlKind::CheckBox,
                        disabled: state.disabled,
                        hovered: flags & HOVER != 0,
                        pressed: flags & ACTIVE != 0 && flags & HOVER != 0,
                        focus_visible: flags & FOCUS_VISIBLE != 0,
                        accent: style.accent_color.resolve(style.color),
                    },
                );
            }
            // Its text is smoothed as the page's text there is.
            state.grayscale = frame.grayscale || frame.no_lcd;
            // A native control cannot rotate or skew: under a transform that
            // does, it is kept out of sight and its image paints through the
            // transform, which input maps back to it. Scaled, it stays where
            // its box shows.
            let [a, b, c, d, e, f] = frame.transform;
            let scales = b.abs() < 1e-6 && c.abs() < 1e-6 && a > 0.0 && d > 0.0;
            let turned = !translates_only(frame.transform) && !scales;
            // Fields an author styles edit in their content box. Text fields
            // show their text in its middle, whatever their line height, as
            // Chromium does: the platform centers it by the font's height.
            // Date and time pickers, which draw their text at their top, get
            // the line in the middle of the box.
            let rect = if state.bare && editable {
                let content = frame.local(geometry.content_box());
                if matches!(
                    kind,
                    ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
                ) {
                    content
                } else {
                    let line = self
                        .highlight
                        .map_or(style.used_line_height(), |highlight| {
                            crate::text::line_height(style, highlight.measurer)
                        })
                        .min(content.height);
                    Rect::new(
                        content.x,
                        content.y + (content.height - line) / 2.0,
                        content.width,
                        line,
                    )
                }
            } else {
                rect
            };
            // A scaled control sits where its box shows, its text scaled
            // with it, so that it can still be typed in.
            let rect = if translates_only(frame.transform) || turned {
                rect
            } else {
                let shown = frame.region(rect).bounds();
                state.font.size *= a.min(d);
                shown.offset(-frame.window.0, -frame.window.1)
            };
            let window = frame.window(rect);
            if turned {
                let (x, y) = (window.x, window.y);
                state.transform = Some([a, b, c, d, a * x + c * y + e, b * x + d * y + f]);
            } else {
                state.clip = frame.clip.and_then(|clip| {
                    let area = intersect(Some(clip), window);
                    (area != window).then_some(area.offset(-window.x, -window.y))
                });
            }
            surface.controls.push(Control {
                id: node.index(),
                kind,
                rect,
                state,
            });
            surface
                .items
                .push(DisplayItem::Child(NativeChild::Control(node.index())));
            if kind == ControlKind::TextArea {
                self.resizer(layer, surface);
            }
            if toggle && style.author_appearance {
                layers.extend(self.generated_layer(node, &PseudoElement::Before, *frame));
                layers.extend(self.generated_layer(node, &PseudoElement::After, *frame));
            }
            return painted;
        }
        if self.dom.local_name(node) == Some("select")
            && control_for("select", |name| self.dom.get_attribute(node, name))
                == Some(ControlKind::ComboBox)
            && visible
        {
            // The arrow of a select the page draws, centered in the box
            // Chromium keeps for it inside the right border, whatever the
            // padding.
            let inner = frame.local(geometry.padding_box());
            let (x, y) = (
                inner.x + inner.width - crate::tree::SELECT_ARROW_BOX / 2.0,
                inner.y + inner.height / 2.0,
            );
            surface.items.push(DisplayItem::Path {
                segments: vec![
                    PathSegment::Move(x - 4.0, y - 2.0),
                    PathSegment::Line(x, y + 2.0),
                    PathSegment::Line(x + 4.0, y - 2.0),
                ],
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: None,
                fill_rule: FillRule::NonZero,
                stroke: Some(Stroke {
                    color: style.color,
                    width: 1.5,
                    cap: LineCap::Round,
                    join: LineJoin::Round,
                    dashes: Vec::new(),
                    dash_offset: 0.0,
                }),
            });
        }
        let overflow = if self.viewport_overflow == Some(node) {
            Overflowing::Visible
        } else {
            overflow_of(style)
        };
        // A native surface cannot scale or rotate: under a transform that
        // does, a scroll container's content paints in place, clipped and
        // scrolled, so that it follows the transform as in Chromium.
        let in_place = overflow == Overflowing::Scroll && !translates_only(frame.transform);
        let overflow = if in_place {
            Overflowing::Clip
        } else {
            overflow
        };
        let scrolls =
            |axis: Overflow| axis == Overflow::Hidden || (in_place && axis.is_scroll_container());
        match overflow {
            Overflowing::Visible => self.content(node, *frame, surface, layers),
            Overflowing::Clip => {
                let clip = frame.local(geometry.padding_box());
                surface
                    .items
                    .push(DisplayItem::PushClip { rect: clip, radii });
                let (left, top) = self.dom.scroll(node);
                // Content scrolls within the room its scrollbars leave.
                let client = geometry.client_box();
                let left = if scrolls(style.overflow_x) {
                    clamp(left, geometry.content_width - client.width)
                } else {
                    0.0
                };
                let top = if scrolls(style.overflow_y) {
                    clamp(top, geometry.content_height - client.height)
                } else {
                    0.0
                };
                let port = geometry.padding_box();
                let mut inner = Frame {
                    origin: (frame.origin.0 + left, frame.origin.1 + top),
                    scrollport: if scrolls(style.overflow_x) || scrolls(style.overflow_y) {
                        Rect::new(port.x + left, port.y + top, port.width, port.height)
                    } else {
                        frame.scrollport
                    },
                    ..*frame
                };
                self.clip_frame(&mut inner, frame.window(clip), None);
                if style.has_transform() || !style.filter.0.is_empty() {
                    // The containing block of fixed-position descendants
                    // clips them.
                    inner.fixed = Fixed {
                        mark: surface.items.len(),
                        clip: inner.clip,
                        hit_clip: inner.hit_clip,
                    };
                }
                self.content(node, inner, surface, layers);
                surface.items.push(DisplayItem::PopClip);
                painted.clip = Some((clip, radii));
                if in_place {
                    self.scrollbars_in_place(layer, surface, (left, top));
                }
            }
            Overflowing::Scroll => self.scroller(layer, surface),
        }
        // Scroll containers can be resized; boxes that clip without
        // scrolling cannot.
        if overflow == Overflowing::Scroll
            || (overflow == Overflowing::Clip
                && (style.overflow_x == Overflow::Hidden || style.overflow_y == Overflow::Hidden))
        {
            self.resizer(layer, surface);
        }
        let outline = style.outline_width.0;
        if visible
            && outline > 0.0
            && !matches!(style.outline_style, BorderStyle::None | BorderStyle::Hidden)
        {
            let grow = outline + style.outline_offset.0;
            surface.items.push(DisplayItem::Border {
                rect: Rect::new(
                    rect.x - grow,
                    rect.y - grow,
                    rect.width + 2.0 * grow,
                    rect.height + 2.0 * grow,
                ),
                widths: [outline; 4],
                colors: [style.outline_color.0.unwrap_or(style.color); 4],
                styles: [style.outline_style; 4],
                radii,
            });
        }
        painted
    }

    /// The scrollbars of a scroll container painted in place, in the room
    /// layout left them, as author CSS styles them or as Chromium draws
    /// them; recorded for the pointer and the wheel.
    fn scrollbars_in_place(&mut self, layer: &Layer, surface: &mut Surface, scroll: (f32, f32)) {
        let Layer {
            node,
            geometry,
            frame,
            ..
        } = layer;
        let padding = frame.local(geometry.padding_box());
        let client = geometry.client_box();
        let view = (padding.width, padding.height);
        let content = (geometry.content_width, geometry.content_height);
        // The bars take the room layout reserved: a vertical bar's width,
        // a horizontal bar's height.
        let axes = core::array::from_fn(|axis| {
            let thickness = geometry.scrollbars[1 - axis];
            scrollbar::Axis {
                enabled: thickness > 0.0,
                force: true,
                thickness,
            }
        });
        let styled = self.styles.scrollbars(*node);
        let (look, pixel) = self.layout.scrollbars;
        let parts = if styled.is_some() {
            (0.0, scrollbar::MINIMUM_THUMB)
        } else {
            (look.button_length(pixel), look.minimum_thumb(pixel))
        };
        let visible = (client.width, client.height);
        let bars = scrollbar::Geometry::with(axes, (view, visible), content, scroll, parts);
        let part = |parts: Option<(NodeId, u8)>| {
            parts
                .filter(|(owner, _)| owner == node)
                .and_then(|(_, code)| crate::scrollbars::part(code))
        };
        let (hover, pressed) = self.dom.scrollbar_parts();
        let (hover, pressed) = (part(hover), part(pressed));
        let mut items = vec![DisplayItem::PushTransform([
            1.0, 0.0, 0.0, 1.0, padding.x, padding.y,
        ])];
        match styled {
            Some(style) => scrollbar::styled_items(style, &bars, hover, pressed, &mut items),
            None => {
                scrollbar::platform_items(look, &bars, scroll, (hover, pressed), pixel, &mut items)
            }
        }
        items.push(DisplayItem::PopTransform);
        if items.len() > 2 {
            surface.items.extend(items);
        }
        // As far as the bars scroll, whether or not they show.
        let maximum = |content: f32, extent: f32| {
            if scrollbar::overflows(content, extent) {
                content - extent
            } else {
                0.0
            }
        };
        self.scene.scrollers.push(InPlace {
            node: *node,
            region: frame.region(padding),
            clip: frame.hit_clip,
            bars,
            view: visible,
            maximum: (
                maximum(content.0, client.width),
                maximum(content.1, client.height),
            ),
        });
    }

    /// The grip of a box its `resize` lets the user resize, in the
    /// bottom-right corner of its padding box: as wide as its vertical
    /// scrollbar and as tall as its horizontal one, shown or not. Chromium
    /// draws two diagonal lines there, dark with a light edge below them so
    /// that they show on any background.
    fn resizer(&mut self, layer: &Layer, surface: &mut Surface) {
        let Layer {
            node,
            style,
            geometry,
            frame,
            ..
        } = layer;
        if style.resize == Resize::None || style.visibility != Visibility::Visible {
            return;
        }
        let (look, pixel) = self.layout.scrollbars;
        let thickness = |axis: usize| {
            crate::tree::scrollbar_thickness(self.styles, *node, axis, 0.0, look.thickness(pixel))
        };
        let [down, across] = geometry.scrollbars;
        let (width, height) = match (down > 0.0, across > 0.0) {
            (false, false) => (thickness(1), thickness(0)),
            (true, false) => (down, down),
            (false, true) => (across, across),
            (true, true) => (down, across),
        };
        let padding = frame.local(geometry.padding_box());
        if width <= 0.0 || height <= 0.0 || padding.width < width || padding.height < height {
            return;
        }
        let corner = Rect::new(
            padding.x + padding.width - width,
            padding.y + padding.height - height,
            width,
            height,
        );
        let (right, bottom) = (corner.x + width, corner.y + height);
        let ends = [
            (
                (right - 1.0, corner.y + height / 2.0),
                (corner.x + width / 2.0, bottom - 1.0),
            ),
            (
                (right - 1.0, corner.y + height * 0.75),
                (corner.x + width * 0.75, bottom - 1.0),
            ),
        ];
        for (color, shift) in [
            (Color::rgba(0, 0, 0, 0.6), 0.0),
            (Color::rgba(255, 255, 255, 0.6), 1.0),
        ] {
            surface.items.push(DisplayItem::Path {
                segments: ends
                    .iter()
                    .flat_map(|((x0, y0), (x1, y1))| {
                        [
                            PathSegment::Move(*x0, y0 + shift),
                            PathSegment::Line(x1 + shift, *y1),
                        ]
                    })
                    .collect(),
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: None,
                fill_rule: FillRule::NonZero,
                stroke: Some(Stroke {
                    color,
                    width: 1.0,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    dashes: Vec::new(),
                    dash_offset: 0.0,
                }),
            });
        }
        self.scene.resizers.push(frame.hit(corner, *node));
    }

    /// Paints a background or mask image over `rect` with tiles placed in
    /// `area`. A cross-fade paints the image it fades out, then the one it
    /// fades in at the fade's opacity above it.
    fn background_image(
        &self,
        image: &BackgroundImage,
        layer: &ImageLayer<'_>,
        (rect, area, radii): (Rect, Rect, Radii),
        items: &mut Vec<DisplayItem>,
    ) {
        match image {
            BackgroundImage::None => {}
            BackgroundImage::Url(url) => match self.layout.images.get(url) {
                Some(&natural) => {
                    let tiles = layer_tiles(layer, area, rect, natural);
                    items.push(DisplayItem::PushClip { rect, radii });
                    items.extend(tiles.into_iter().map(|tile| DisplayItem::Image {
                        rect: tile,
                        url: url.clone(),
                    }));
                    items.push(DisplayItem::PopClip);
                }
                // An image of unknown size fills the box.
                None => items.push(DisplayItem::Image {
                    rect: area,
                    url: url.clone(),
                }),
            },
            BackgroundImage::LinearGradient { angle, stops } => items.push(DisplayItem::Gradient {
                rect,
                angle: *angle,
                stops: gradient_stops(stops),
                radii,
            }),
            // Radial gradients are approximated by a vertical gradient.
            BackgroundImage::RadialGradient { stops } => items.push(DisplayItem::Gradient {
                rect,
                angle: 180.0,
                stops: gradient_stops(stops),
                radii,
            }),
            BackgroundImage::CrossFade { from, to, progress } => {
                self.background_image(from, layer, (rect, area, radii), items);
                items.push(DisplayItem::PushOpacity(progress.clamp(0.0, 1.0)));
                self.background_image(to, layer, (rect, area, radii), items);
                items.push(DisplayItem::PopOpacity);
            }
        }
    }

    fn decorations(&self, layer: &Layer, rect: Rect, radii: Radii, items: &mut Vec<DisplayItem>) {
        // A shadow shows unless it is transparent, or sharp and no bigger
        // than its box: an outer one then lies under the box, and an inset
        // one leaves nothing of the box to cover.
        let shows = |shadow: &nativeboat_css::types::Shadow| {
            !shadow.color.is_transparent()
                && (shadow.blur > 0.0
                    || shadow.x.abs() > -shadow.spread
                    || shadow.y.abs() > -shadow.spread)
        };
        let Layer {
            node,
            style,
            geometry,
            frame,
            ..
        } = layer;
        // Earlier shadows are drawn on top of later ones. Shadows that
        // cannot show are left out, as Tailwind's zero rings are: their
        // edges would otherwise show where the box sits between pixels.
        for shadow in style
            .box_shadow
            .0
            .iter()
            .rev()
            .filter(|shadow| !shadow.inset && shows(shadow))
        {
            let grow = shadow.spread;
            items.push(DisplayItem::Shadow {
                rect: Rect::new(
                    rect.x + shadow.x - grow,
                    rect.y + shadow.y - grow,
                    (rect.width + 2.0 * grow).max(0.0),
                    (rect.height + 2.0 * grow).max(0.0),
                ),
                radii: radii.map(|corner| spread_corner(corner, grow)),
                color: shadow.color,
                blur: shadow.blur,
                inset: false,
                clip: rect,
                clip_radii: radii,
            });
        }
        if (layer.generated || self.canvas != Some(*node))
            && !style.background_color.is_transparent()
        {
            items.push(DisplayItem::Fill {
                rect,
                color: style.background_color,
                radii,
            });
        }
        let area = frame.local(geometry.padding_box());
        self.background_image(
            &style.background_image,
            &ImageLayer::background(style),
            (rect, area, radii),
            items,
        );
        let padding = frame.local(geometry.padding_box());
        let inner = inset_radii(radii, geometry.border);
        for shadow in style
            .box_shadow
            .0
            .iter()
            .rev()
            .filter(|shadow| shadow.inset && shows(shadow))
        {
            let shrink = shadow.spread;
            items.push(DisplayItem::Shadow {
                rect: Rect::new(
                    padding.x + shadow.x + shrink,
                    padding.y + shadow.y + shrink,
                    (padding.width - 2.0 * shrink).max(0.0),
                    (padding.height - 2.0 * shrink).max(0.0),
                ),
                radii: inner.map(|corner| spread_corner(corner, -shrink)),
                color: shadow.color,
                blur: shadow.blur,
                inset: true,
                clip: padding,
                clip_radii: inner,
            });
        }
        if geometry.border.iter().any(|width| *width > 0.0) {
            let sides = &style.border_color;
            let kinds = &style.border_style;
            items.push(DisplayItem::Border {
                rect,
                widths: geometry.border,
                colors: [sides.top, sides.right, sides.bottom, sides.left],
                styles: [kinds.top, kinds.right, kinds.bottom, kinds.left],
                radii,
            });
        }
        if self.dom.local_name(*node) == Some("img")
            && let Some(src) = self.dom.get_attribute(*node, "src")
        {
            let content = frame.local(geometry.content_box());
            self.picture(src, style, content, content_radii(geometry, radii), items);
        }
        if self.dom.local_name(*node) == Some("video")
            && let Some(picture) = crate::tree::video_picture(self.dom, *node)
        {
            let content = frame.local(geometry.content_box());
            self.picture(
                &picture,
                style,
                content,
                content_radii(geometry, radii),
                items,
            );
        }
        if self.dom.local_name(*node) == Some("svg") {
            let content = frame.local(geometry.content_box());
            let measurer = self.highlight.map(|highlight| highlight.measurer);
            crate::svg::paint(self.dom, self.styles, *node, content, items, measurer);
        }
        if let Some((key, version)) = self.dom.canvas(*node) {
            items.push(DisplayItem::Canvas {
                rect: frame.local(geometry.content_box()),
                key,
                version,
            });
        }
    }

    /// Paints an image element's picture into its content box: sized from
    /// its natural size as `object-fit` says, placed by `object-position`
    /// and clipped to the box's rounded content edge, as Chromium paints
    /// replaced content.
    fn picture(
        &self,
        url: &str,
        style: &ComputedStyle,
        content: Rect,
        radii: Radii,
        items: &mut Vec<DisplayItem>,
    ) {
        use nativeboat_css::types::{BackgroundRepeat, BackgroundSize, ObjectFit, Repeat};
        let rect = match (style.object_fit, self.layout.images.get(url)) {
            (ObjectFit::Fill, _) | (_, None) => content,
            (fit, Some(&natural)) => {
                let fits = natural.0 <= content.width && natural.1 <= content.height;
                let size = match fit {
                    ObjectFit::Cover => BackgroundSize::Cover,
                    ObjectFit::None => BackgroundSize::Auto,
                    ObjectFit::ScaleDown if fits => BackgroundSize::Auto,
                    _ => BackgroundSize::Contain,
                };
                let layer = ImageLayer {
                    size: &size,
                    position: &style.object_position,
                    repeat: BackgroundRepeat {
                        x: Repeat::NoRepeat,
                        y: Repeat::NoRepeat,
                    },
                };
                match layer_tiles(&layer, content, content, natural).first() {
                    Some(&tile) => tile,
                    None => return,
                }
            }
        };
        // A picture that overflows the box, or a box with rounded corners,
        // clips the picture to the content edge.
        let overflows = rect.x < content.x
            || rect.y < content.y
            || rect.x + rect.width > content.x + content.width
            || rect.y + rect.height > content.y + content.height;
        let clipped = overflows || radii.iter().any(|corner| !corner.is_square());
        if clipped {
            items.push(DisplayItem::PushClip {
                rect: content,
                radii,
            });
        }
        items.push(DisplayItem::Image {
            rect,
            url: url.to_owned(),
        });
        if clipped {
            items.push(DisplayItem::PopClip);
        }
    }

    /// Paints the text and child elements of a box.
    fn content(
        &mut self,
        node: NodeId,
        frame: Frame,
        surface: &mut Surface,
        layers: &mut Vec<Layer>,
    ) {
        // A positioned or transformed box contains the absolutely
        // positioned boxes inside it.
        let mut frame = frame;
        if self
            .styles
            .get(node)
            .is_some_and(|style| style.position != PositionKind::Static || style.has_transform())
        {
            frame.positioned = (frame.hit_clip, frame.clip);
        }
        self.generated_box(node, &PseudoElement::Before, frame, surface, layers);
        // Inline boxes are painted under the text of their lines.
        for part in self.inline_boxes.remove(&node).unwrap_or_default() {
            self.inline_box(part, frame, surface);
        }
        for (index, run) in self.runs.remove(&node).unwrap_or_default() {
            self.text(index, run, node, frame, surface);
        }
        if let Some((_, rect, color)) = self.caret.filter(|(container, ..)| *container == node) {
            surface.items.push(DisplayItem::Caret {
                rect: frame.local(rect),
                color,
            });
        }
        for child in self.dom.flat_child_elements(node) {
            self.element(child, frame, surface, layers);
        }
        self.generated_box(node, &PseudoElement::After, frame, surface, layers);
    }

    /// Paints one line's part of an inline element's box: its background,
    /// shadows and borders, with its left and right edges only where the
    /// element starts and ends.
    fn inline_box(&mut self, part: &InlineBoxRect, frame: Frame, surface: &mut Surface) {
        let Some(style) = self.styles.get(part.node) else {
            return;
        };
        if style.visibility != Visibility::Visible {
            return;
        }
        let side = |width: Px, kind: BorderStyle| {
            if matches!(kind, BorderStyle::None | BorderStyle::Hidden) {
                0.0
            } else {
                width.0
            }
        };
        let (widths, kinds) = (&style.border_width, &style.border_style);
        let mut border = [
            side(widths.top, kinds.top),
            side(widths.right, kinds.right),
            side(widths.bottom, kinds.bottom),
            side(widths.left, kinds.left),
        ];
        let padding = &style.padding;
        let mut padding = [
            crate::tree::fixed(&padding.top),
            crate::tree::fixed(&padding.right),
            crate::tree::fixed(&padding.bottom),
            crate::tree::fixed(&padding.left),
        ];
        let rect = frame.local(part.rect);
        let mut radii = radii(style, rect);
        if !part.start {
            (border[3], padding[3]) = (0.0, 0.0);
            (radii[0], radii[3]) = (Corner::SQUARE, Corner::SQUARE);
        }
        if !part.end {
            (border[1], padding[1]) = (0.0, 0.0);
            (radii[1], radii[2]) = (Corner::SQUARE, Corner::SQUARE);
        }
        let layer = Layer {
            generated: false,
            pseudo: None,
            node: part.node,
            z: 0,
            auto: true,
            style: style.clone(),
            geometry: BoxGeometry {
                rect: part.rect,
                padding,
                border,
                margin: [0.0; 4],
                content_width: 0.0,
                content_height: 0.0,
                scrollbars: [0.0; 2],
            },
            frame,
        };
        self.decorations(&layer, rect, radii, &mut surface.items);
    }

    fn text(
        &mut self,
        index: usize,
        run: &TextRun,
        container: NodeId,
        frame: Frame,
        surface: &mut Surface,
    ) {
        let rect = frame.local(run.rect);
        let (top, height) = run.content;
        let content = frame
            .region(Rect::new(rect.x, rect.y + top, rect.width, height))
            .bounds();
        let boxless = core::iter::successors(Some(run.element), |element| {
            self.dom.parent_element(*element)
        })
        .take_while(|element| *element != container && !self.layout.boxes.contains_key(element));
        for element in boxless {
            let rect = self
                .scene
                .rects
                .get(&element)
                .map_or(content, |rect| union(*rect, content));
            self.scene.rects.insert(element, rect);
        }
        // An empty element's place shows nothing.
        if run.text.is_empty() {
            return;
        }
        let (visible, pointer) = self.styles.get(run.element).map_or((true, true), |style| {
            (
                style.visibility == Visibility::Visible,
                style.pointer_events == PointerEvents::Auto,
            )
        });
        let placed = Placed {
            run: index,
            region: frame.region(rect),
            clip: frame.hit_clip,
        };
        // Hidden text and text at `font-size: 0` have a place for script
        // geometry but show nothing and take no pointer.
        if !visible || run.font.size <= 0.0 {
            self.scene.unshown.push(placed);
            return;
        }
        self.scene.texts.push(placed);
        self.run_items(index, run, (rect, frame.grayscale || frame.no_lcd), surface);
        if pointer {
            self.scene.hits.push(frame.hit(rect, run.element));
        }
    }

    /// Paints a text run with its selection and spelling marks; `grayscale`
    /// text is smoothed in grayscale.
    fn run_items(
        &self,
        index: usize,
        run: &TextRun,
        (rect, grayscale): (Rect, bool),
        surface: &mut Surface,
    ) {
        self.run_text(index, run, (rect, grayscale), surface);
        self.spelling_marks(index, run, rect, surface);
    }

    /// Wavy red lines under the misspelled words of a run, below their
    /// baseline as Chromium draws them.
    fn spelling_marks(&self, index: usize, run: &TextRun, rect: Rect, surface: &mut Surface) {
        let Some(highlight) = self.highlight else {
            return;
        };
        let Some(ranges) = highlight.misspellings.get(&run.node) else {
            return;
        };
        let Some((_, stops)) = self
            .bounds
            .get(&run.node)
            .and_then(|runs| runs.iter().find(|(run_index, _)| *run_index == index))
        else {
            return;
        };
        let last = stops.len() - 1;
        let column = |offset: u32| {
            stops
                .iter()
                .position(|stop| *stop >= offset)
                .unwrap_or(last)
        };
        let metrics = highlight.measurer.metrics(&run.font);
        let baseline =
            rect.y + (rect.height - metrics.ascent - metrics.descent) / 2.0 + metrics.ascent;
        for (start, end) in ranges {
            let (from, to) = (column(*start), column(*end));
            if from >= to {
                continue;
            }
            let (left, right) = caret::extent(run, highlight.measurer, from, to);
            let (left, right) = (rect.x + left, rect.x + right);
            // A wave two pixels high, rising and falling every two pixels.
            let mut segments = vec![PathSegment::Move(left, baseline + 3.0)];
            let mut x = left;
            let mut up = true;
            while x < right {
                x = (x + 2.0).min(right);
                segments.push(PathSegment::Line(x, baseline + if up { 1.0 } else { 3.0 }));
                up = !up;
            }
            surface.items.push(DisplayItem::Path {
                segments,
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: None,
                fill_rule: FillRule::NonZero,
                stroke: Some(Stroke {
                    color: Color::rgb(255, 0, 0),
                    width: 1.0,
                    cap: LineCap::Butt,
                    join: LineJoin::Round,
                    dashes: Vec::new(),
                    dash_offset: 0.0,
                }),
            });
        }
    }

    /// Paints a run's text, with its selected part on the selection's
    /// background in the selection's color.
    fn run_text(
        &self,
        index: usize,
        run: &TextRun,
        (rect, grayscale): (Rect, bool),
        surface: &mut Surface,
    ) {
        // A tab shows no glyph: its run only takes its place on the line.
        if !run.text.is_empty() && run.text.chars().all(|character| character == '\t') {
            return;
        }
        if let Some((symbol, shape)) = run.symbol {
            let shape = Rect::new(
                rect.x + shape.x,
                rect.y + shape.y,
                shape.width,
                shape.height,
            );
            let round = match symbol {
                Symbol::Disc | Symbol::Circle => [Corner::round(shape.width / 2.0); 4],
                _ => [Corner::SQUARE; 4],
            };
            let (left, top) = (shape.x, shape.y);
            let (right, bottom) = (left + shape.width, top + shape.height);
            let triangle = |points: [(f32, f32); 3]| DisplayItem::Path {
                segments: vec![
                    PathSegment::Move(points[0].0, points[0].1),
                    PathSegment::Line(points[1].0, points[1].1),
                    PathSegment::Line(points[2].0, points[2].1),
                    PathSegment::Close,
                ],
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                fill: Some(run.color),
                fill_rule: FillRule::NonZero,
                stroke: None,
            };
            surface.items.push(match symbol {
                Symbol::DisclosureClosed => {
                    triangle([(left, top), (right, (top + bottom) / 2.0), (left, bottom)])
                }
                Symbol::DisclosureOpen => {
                    triangle([(left, top), (right, top), ((left + right) / 2.0, bottom)])
                }
                Symbol::Circle => DisplayItem::Border {
                    rect: shape,
                    widths: [1.0; 4],
                    colors: [run.color; 4],
                    styles: [BorderStyle::Solid; 4],
                    radii: round,
                },
                Symbol::Disc | Symbol::Square => DisplayItem::Fill {
                    rect: shape,
                    color: run.color,
                    radii: round,
                },
            });
            return;
        }
        let text = |color: Color| DisplayItem::Text {
            rect,
            text: run.text.clone(),
            font: run.font.clone(),
            color,
            underline: run.underline,
            line_through: run.line_through,
            letter_spacing: run.letter_spacing,
            rtl: run.rtl,
            grayscale,
            underline_offset: run.underline_offset,
            decorations: run.decorations.to_vec(),
        };
        let Some((span, left, right, before, after)) = self.selected(index, run) else {
            surface.items.push(text(run.color));
            return;
        };
        let selection = Rect::new(rect.x + left, rect.y, right - left, rect.height);
        if !span.background.is_transparent() {
            surface.items.push(DisplayItem::Fill {
                rect: selection,
                color: span.background,
                radii: [Corner::SQUARE; 4],
            });
        }
        let Some(color) = span.color.filter(|color| *color != run.color) else {
            surface.items.push(text(run.color));
            return;
        };
        // Glyphs may overhang their advance, so each clip reaches past the
        // run's line box.
        let overhang = rect.height;
        let mut part = |from: f32, to: f32, color: Color| {
            surface.items.push(DisplayItem::PushClip {
                rect: Rect::new(
                    from,
                    rect.y - overhang,
                    to - from,
                    rect.height + 2.0 * overhang,
                ),
                radii: [Corner::SQUARE; 4],
            });
            surface.items.push(text(color));
            surface.items.push(DisplayItem::PopClip);
        };
        let end = selection.x + selection.width;
        if before {
            part(rect.x - overhang, selection.x, run.color);
        }
        if !color.is_transparent() {
            part(selection.x, end, color);
        }
        if after {
            part(end, rect.x + rect.width + overhang, run.color);
        }
    }

    /// The selected part of a run: the selection's span, where the part
    /// starts and ends in the run, and whether text comes before and after
    /// it.
    fn selected(&self, index: usize, run: &TextRun) -> Option<(Span, f32, f32, bool, bool)> {
        let highlight = self.highlight?;
        let span = *highlight.spans.get(&run.node)?;
        let (_, stops) = self
            .bounds
            .get(&run.node)?
            .iter()
            .find(|(run_index, _)| *run_index == index)?;
        let last = stops.len() - 1;
        let column = |offset: u32| {
            stops
                .iter()
                .position(|stop| *stop >= offset)
                .unwrap_or(last)
        };
        let (from, to) = (column(span.start), column(span.end));
        (from < to).then(|| {
            let (left, right) = caret::extent(run, highlight.measurer, from, to);
            // Right to left, the text before the selection is on its right.
            let (before, after) = if run.rtl {
                (to < last, from > 0)
            } else {
                (from > 0, to < last)
            };
            (span, left, right, before, after)
        })
    }

    /// Paints a scroll container's content on a child surface.
    /// Whether Chromium composites a scroll container on its own: one that
    /// makes no stacking context of its own but holds positioned boxes with
    /// a negative `z-index` in the one around it, which paint beneath its
    /// content.
    /// Whether Chromium composites an element into a layer of its own,
    /// which decides on ClearType for itself: one with a backdrop filter,
    /// and a scroll container with something to scroll that keeps
    /// ClearType there, opaque, or that negative layers composite.
    fn alone(&self, node: NodeId, style: &ComputedStyle) -> bool {
        if !style.backdrop_filter.0.is_empty() {
            return true;
        }
        let Some(geometry) = self.layout.boxes.get(&node) else {
            return false;
        };
        let client = geometry.client_box();
        overflow_of(style) == Overflowing::Scroll
            && self.viewport_overflow != Some(node)
            && (geometry.content_width > client.width + 0.5
                || geometry.content_height > client.height + 0.5)
            && (style.background_color.a >= 1.0 || self.composited(node, style))
    }

    /// Whether all the text of a composited layer lies on what the layer
    /// paints opaque, as Chromium judges whether the layer keeps ClearType
    /// (`text_known_to_be_on_opaque_background`): in paint order, each
    /// opaque background widens the largest rectangle the layer is known to
    /// cover, and each text, a text field's included, must lie inside it.
    /// Layers composited inside judge their own text.
    fn text_on_opaque(&self, root: NodeId) -> bool {
        // Chromium judges in whole device pixels: an opaque area shrinks to
        // them, a text's area grows to them.
        let pixel = self.layout.scrollbars.1;
        let snap = |rect: Rect, grow: bool| {
            if pixel <= 0.0 {
                return rect;
            }
            // An edge moves to the pixel boundary before or after it.
            let edge = |value: f32, after: bool| {
                let pixels = value / pixel;
                pixel
                    * if after {
                        crate::math::ceil(pixels)
                    } else {
                        crate::math::floor(pixels)
                    }
            };
            let x = edge(rect.x, !grow);
            let y = edge(rect.y, !grow);
            let right = edge(rect.x + rect.width, grow);
            let bottom = edge(rect.y + rect.height, grow);
            Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
        };
        let inside = |outer: Rect, inner: Rect| {
            inner.x >= outer.x
                && inner.y >= outer.y
                && inner.x + inner.width <= outer.x + outer.width
                && inner.y + inner.height <= outer.y + outer.height
        };
        let mut opaque = Rect::new(0.0, 0.0, 0.0, 0.0);
        // Each box comes with the clip of the boxes around it in the layer,
        // and how far their scrolling moves it.
        let mut pending = vec![(root, None::<Rect>, (0.0_f32, 0.0_f32))];
        while let Some((node, clip, (dx, dy))) = pending.pop() {
            // Hidden boxes paint nothing; layers inside judge themselves.
            let Some(style) = self.styles.get(node).filter(|style| {
                style.display != Display::None && (node == root || !self.alone(node, style))
            }) else {
                continue;
            };
            // Text clipped out of sight counts nowhere.
            let on_opaque = |opaque: Rect, area: Rect| {
                area.width <= 0.0 || area.height <= 0.0 || inside(opaque, snap(area, true))
            };
            let (mut inner, mut shift) = (clip, (dx, dy));
            if let Some(geometry) = self.layout.boxes.get(&node) {
                let rect = geometry.rect.offset(-dx, -dy);
                if style.visibility == Visibility::Visible && style.background_color.a >= 1.0 {
                    // A rounded box is known to be opaque from three tenths
                    // of its radii in.
                    let [top_left, top_right, bottom_right, bottom_left] = radii(style, rect);
                    let left = top_left.x.max(bottom_left.x) * 0.3;
                    let top = top_left.y.max(top_right.y) * 0.3;
                    let right = top_right.x.max(bottom_right.x) * 0.3;
                    let bottom = bottom_left.y.max(bottom_right.y) * 0.3;
                    let covered = Rect::new(
                        rect.x + left,
                        rect.y + top,
                        (rect.width - left - right).max(0.0),
                        (rect.height - top - bottom).max(0.0),
                    );
                    opaque = maximum_covered(opaque, snap(intersect(clip, covered), false));
                }
                if crate::native_control(self.dom, node, Some(style)).is_some_and(|kind| {
                    !matches!(
                        kind,
                        ControlKind::CheckBox
                            | ControlKind::Radio
                            | ControlKind::Slider
                            | ControlKind::ProgressBar
                    )
                }) && !on_opaque(
                    opaque,
                    intersect(clip, geometry.content_box().offset(-dx, -dy)),
                ) {
                    return false;
                }
                // A box that clips its overflow clips its content, and one
                // that scrolls moves it too; the layer's own scrolling moves
                // all its content alike.
                let overflow = overflow_of(style);
                if overflow != Overflowing::Visible
                    && self.viewport_overflow != Some(node)
                    && !(node == root && overflow == Overflowing::Scroll)
                {
                    inner = Some(intersect(clip, geometry.padding_box().offset(-dx, -dy)));
                    if overflow == Overflowing::Scroll {
                        let (left, top) = self.dom.scroll(node);
                        let client = geometry.client_box();
                        shift = (
                            dx + clamp(left, geometry.content_width - client.width),
                            dy + clamp(top, geometry.content_height - client.height),
                        );
                    }
                }
            }
            let runs = self.runs.get(&node).into_iter().flatten();
            let generated = self.pseudo_runs.get(&node).into_iter().flatten();
            for (_, run) in runs.chain(generated) {
                let shown = run.symbol.is_none()
                    && run.font.size > 0.0
                    && !run.text.chars().all(|character| character == '\t')
                    && self
                        .styles
                        .get(run.element)
                        .is_none_or(|style| style.visibility == Visibility::Visible);
                let (top, height) = run.content;
                let area = Rect::new(run.rect.x, run.rect.y + top, run.rect.width, height);
                if shown && !on_opaque(opaque, intersect(inner, area.offset(-shift.0, -shift.1))) {
                    return false;
                }
            }
            pending.extend(
                self.dom
                    .flat_child_elements(node)
                    .into_iter()
                    .rev()
                    .map(|child| (child, inner, shift)),
            );
        }
        true
    }

    fn composited(&self, node: NodeId, style: &ComputedStyle) -> bool {
        !stacks(style)
            && self
                .dom
                .flat_child_elements(node)
                .into_iter()
                .any(|child| self.beneath(child))
    }

    /// Whether an element, or a descendant in the same stacking context,
    /// is positioned with a negative `z-index`.
    fn beneath(&self, node: NodeId) -> bool {
        let Some(style) = self.styles.get(node) else {
            return false;
        };
        if style.display == Display::None {
            return false;
        }
        if style.position != PositionKind::Static && style.z_index.0.is_some_and(|z| z < 0) {
            return true;
        }
        !stacks(style)
            && self
                .dom
                .flat_child_elements(node)
                .into_iter()
                .any(|child| self.beneath(child))
    }

    fn scroller(&mut self, layer: &Layer, surface: &mut Surface) {
        let Layer {
            node,
            style,
            geometry,
            frame,
            ..
        } = layer;
        let padding = geometry.padding_box();
        let rect = frame.local(padding);
        let (content_width, content_height) = (geometry.content_width, geometry.content_height);
        let (left, top) = self.dom.scroll(*node);
        // Content scrolls within the room its scrollbars leave.
        let client = geometry.client_box();
        let scroll_x = clamp(left, content_width - client.width);
        let scroll_y = clamp(top, content_height - client.height);
        let window = frame.window(rect);
        // Boxes around that clip, as `overflow: hidden` ones do, clip the
        // surface too.
        let clip = frame.clip.and_then(|clip| {
            let area = intersect(Some(clip), window);
            (area != window).then_some(area.offset(-window.x, -window.y))
        });
        let mut inner = Frame {
            origin: (padding.x, padding.y),
            window: (window.x - scroll_x, window.y - scroll_y),
            scrollport: Rect::new(
                padding.x + scroll_x,
                padding.y + scroll_y,
                rect.width,
                rect.height,
            ),
            // The surface's own opacity covers everything on it, nested
            // surfaces included.
            opacity: 1.0,
            // A scroll container that Chromium composites is a layer of its
            // own: transparent, its text is smoothed in grayscale unless all
            // of it lies on what it paints opaque. Other scroll containers
            // paint into the layer around them.
            grayscale: if self.alone(*node, style) {
                style.background_color.a < 1.0 && !self.text_on_opaque(*node)
            } else {
                frame.grayscale
            },
            ..*frame
        };
        self.clip_frame(&mut inner, window, None);
        // A surface is opaque: it repaints whatever shows through the box,
        // the backgrounds around it. Over other content, as a dialog's
        // translucent overlay is, it shows what lies beneath instead.
        // The surface's window rounds to device pixels and can reach half a
        // pixel past its box: the fill covers that edge too, rather than
        // leaving it half painted.
        let content = Rect::new(
            0.0,
            0.0,
            content_width.max(rect.width) + 1.0,
            content_height.max(rect.height) + 1.0,
        );
        let overlay = style.background_color.a < 1.0
            && matches!(style.position, PositionKind::Absolute | PositionKind::Fixed);
        // A shadow painted before the surface that falls on it, as a tab
        // strip's on the editor below it, shows through a transparent one.
        let shadowed = style.background_color.a < 1.0
            && surface.items.iter().any(|item| shadow_falls_on(item, rect));
        let beneath = overlay || shadowed;
        let mut items = vec![if beneath {
            DisplayItem::Beneath
        } else {
            DisplayItem::Fill {
                rect: content,
                color: self.backdrop(*node),
                radii: [Corner::SQUARE; 4],
            }
        }];
        // What lies beneath holds the box's own background, which the box
        // painted around the surface; over a backdrop it shows again.
        let own = style.background_color;
        if !beneath && !own.is_transparent() && own.a < 1.0 {
            items.push(DisplayItem::Fill {
                rect: content,
                color: own,
                radii: [Corner::SQUARE; 4],
            });
        }
        let mut child = Surface {
            id: node.index(),
            rect,
            content_width,
            content_height,
            scroll_x,
            scroll_y,
            // Scrollbars no author CSS styles are drawn as the browser's.
            scrollbars: Some(Box::new(
                self.styles.scrollbars(*node).cloned().unwrap_or_else(|| {
                    let axis = |overflow: Overflow| {
                        (
                            matches!(overflow, Overflow::Auto | Overflow::Scroll),
                            overflow == Overflow::Scroll,
                        )
                    };
                    nativeboat_ir::Scrollbars::platform(
                        self.layout.scrollbars.0,
                        [axis(style.overflow_x), axis(style.overflow_y)],
                    )
                }),
            )),
            clip,
            opacity: (frame.opacity < 1.0).then_some(frame.opacity),
            items,
            ..Surface::default()
        };
        let mut layers = Vec::new();
        let inner_clip = inner.hit_clip;
        // A scroller's negative layers scroll with its content and paint
        // beneath it, as in Chromium's scrolling contents.
        let mark = (child.items.len(), self.scene.hits.len());
        self.content(*node, inner, &mut child, &mut layers);
        layers.sort_by_key(|layer| layer.z);
        let positive = layers.split_off(layers.partition_point(|layer| layer.z < 0));
        let mut negative = layers;
        negative.extend(self.layers(positive, (None, inner_clip), false, &mut child));
        self.paint_beneath(negative, mark, (None, inner_clip), false, &mut child);
        shadows_over_children(&mut child);
        surface.children.push(child);
        surface
            .items
            .push(DisplayItem::Child(NativeChild::Surface(node.index())));
    }

    /// The color behind an element's own background: the first opaque
    /// background around it, under the translucent ones in between, as a
    /// scroller in a tinted block shows the tint.
    fn backdrop(&self, node: NodeId) -> Color {
        let mut tints = Vec::new();
        let mut base = self.canvas_color;
        let ancestors = core::iter::successors(Some(node), |node| self.dom.parent_element(*node));
        for (depth, element) in ancestors.enumerate() {
            let Some(style) = self.styles.get(element) else {
                continue;
            };
            let color = style.background_color;
            if color.a >= 1.0 {
                base = color;
                break;
            }
            // The element's own translucent background is painted on top.
            if depth > 0 && !color.is_transparent() {
                tints.push(color);
            }
        }
        tints.iter().rev().fold(base, |under, tint| {
            let mix = |over: u8, under: u8| {
                (f32::from(over) * tint.a + f32::from(under) * (1.0 - tint.a) + 0.5) as u8
            };
            Color::rgb(
                mix(tint.r, under.r),
                mix(tint.g, under.g),
                mix(tint.b, under.b),
            )
        })
    }

    fn control_state(
        &self,
        node: NodeId,
        kind: ControlKind,
        style: &ComputedStyle,
    ) -> ControlState {
        let dom = self.dom;
        let attribute = |name: &str| dom.get_attribute(node, name);
        let number = |name: &str, default: f64| {
            attribute(name)
                .and_then(|value| value.trim().parse::<f64>().ok())
                .unwrap_or(default)
        };
        let (value, minimum, maximum) = match kind {
            ControlKind::Slider => {
                let (minimum, maximum) = (number("min", 0.0), number("max", 100.0));
                let maximum = maximum.max(minimum);
                let value = dom
                    .value(node)
                    .trim()
                    .parse::<f64>()
                    .unwrap_or((minimum + maximum) / 2.0);
                (value.clamp(minimum, maximum), minimum, maximum)
            }
            ControlKind::ProgressBar => {
                let maximum = number("max", 1.0).max(0.0);
                (number("value", 0.0).clamp(0.0, maximum), 0.0, maximum)
            }
            _ => (0.0, 0.0, 0.0),
        };
        let text = match kind {
            ControlKind::Button => button_caption(dom, node),
            ControlKind::TextBox
            | ControlKind::Password
            | ControlKind::TextArea
            | ControlKind::Date
            | ControlKind::Time
            | ControlKind::DateTime
            | ControlKind::Month => dom.value(node),
            _ => String::new(),
        };
        let options = dom.options(node);
        let selected = dom.selected_options(node);
        let disabled = attribute("disabled").is_some()
            || core::iter::successors(dom.parent_element(node), |node| dom.parent_element(*node))
                .any(|ancestor| {
                    dom.local_name(ancestor) == Some("fieldset")
                        && dom.has_attribute(ancestor, "disabled")
                });
        ControlState {
            text,
            placeholder: attribute("placeholder").unwrap_or_default().to_owned(),
            checked: dom.checked(node),
            indeterminate: dom.indeterminate(node),
            disabled,
            read_only: attribute("readonly").is_some(),
            options: options
                .iter()
                .map(|option| collapse(&dom.text_content(*option).unwrap_or_default()))
                .collect(),
            selected: options
                .iter()
                .enumerate()
                .filter(|(_, option)| selected.contains(option))
                .map(|(index, _)| index)
                .collect(),
            multiple: attribute("multiple").is_some(),
            value,
            minimum,
            maximum,
            max_length: attribute("maxlength").and_then(|value| value.trim().parse().ok()),
            font: font_of(style),
            color: style.text_fill_color.0.unwrap_or(style.color),
            placeholder_color: self
                .styles
                .pseudo(node, &PseudoElement::Placeholder)
                .map_or(Color::rgb(117, 117, 117), |placeholder| {
                    placeholder.text_fill_color.0.unwrap_or(placeholder.color)
                }),
            text_align: style.text_align,
            background: style.background_color,
            visible: style.visibility == Visibility::Visible,
            bare: false,
            clip: None,
            transform: None,
            grayscale: false,
        }
    }
}

/// Where a background image's tiles go: sized by `background-size` and
/// placed by `background-position` in the padding box `area`, repeated as
/// `background-repeat` says across `clip`, the border box.
pub fn background_tiles(
    style: &ComputedStyle,
    area: Rect,
    clip: Rect,
    natural: (f32, f32),
) -> Vec<Rect> {
    layer_tiles(&ImageLayer::background(style), area, clip, natural)
}

/// How a background or mask image is sized, placed and repeated.
struct ImageLayer<'a> {
    size: &'a nativeboat_css::types::BackgroundSize,
    position: &'a nativeboat_css::types::BackgroundPosition,
    repeat: nativeboat_css::types::BackgroundRepeat,
}

impl<'a> ImageLayer<'a> {
    fn background(style: &'a ComputedStyle) -> Self {
        ImageLayer {
            size: &style.background_size,
            position: &style.background_position,
            repeat: style.background_repeat,
        }
    }

    fn mask(style: &'a ComputedStyle) -> Self {
        ImageLayer {
            size: &style.mask_size,
            position: &style.mask_position,
            repeat: style.mask_repeat,
        }
    }
}

/// The tiles of an image layer of natural size `natural`, placed in `area`
/// and covering `clip`.
fn layer_tiles(
    layer: &ImageLayer<'_>,
    area: Rect,
    clip: Rect,
    (natural_width, natural_height): (f32, f32),
) -> Vec<Rect> {
    use nativeboat_css::types::{BackgroundSize, LengthPercentageAuto, Repeat};
    if natural_width <= 0.0 || natural_height <= 0.0 || area.width <= 0.0 || area.height <= 0.0 {
        return Vec::new();
    }
    let ratio = natural_width / natural_height;
    let (mut width, mut height) = match layer.size {
        BackgroundSize::Auto => (natural_width, natural_height),
        BackgroundSize::Cover | BackgroundSize::Contain => {
            let across = area.width / natural_width;
            let down = area.height / natural_height;
            let scale = if *layer.size == BackgroundSize::Cover {
                across.max(down)
            } else {
                across.min(down)
            };
            (natural_width * scale, natural_height * scale)
        }
        BackgroundSize::Explicit(width, height) => {
            let resolve = |value: &LengthPercentageAuto, basis: f32| match value {
                LengthPercentageAuto::Auto => None,
                LengthPercentageAuto::Length(length) => Some(length.resolve(basis)),
            };
            match (resolve(width, area.width), resolve(height, area.height)) {
                (Some(width), Some(height)) => (width, height),
                (Some(width), None) => (width, width / ratio),
                (None, Some(height)) => (height * ratio, height),
                (None, None) => (natural_width, natural_height),
            }
        }
    };
    let repeat = layer.repeat;
    // `round` scales the tiles so whole ones fill the area.
    let rounded = |extent: f32, tile: f32| extent / crate::math::round(extent / tile).max(1.0);
    match (repeat.x == Repeat::Round, repeat.y == Repeat::Round) {
        (true, true) => {
            width = rounded(area.width, width);
            height = rounded(area.height, height);
        }
        (true, false) => {
            let scaled = rounded(area.width, width);
            if matches!(
                layer.size,
                BackgroundSize::Explicit(_, LengthPercentageAuto::Auto) | BackgroundSize::Auto
            ) {
                height *= scaled / width;
            }
            width = scaled;
        }
        (false, true) => {
            let scaled = rounded(area.height, height);
            if matches!(
                layer.size,
                BackgroundSize::Explicit(LengthPercentageAuto::Auto, _) | BackgroundSize::Auto
            ) {
                width *= scaled / height;
            }
            height = scaled;
        }
        (false, false) => {}
    }
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    // The tile positions along one axis: the first placed by the position,
    // the others a tile (and any spacing) apart across the clip.
    let axis =
        |repeat: Repeat, start: f32, extent: f32, tile: f32, offset: f32, low: f32, high: f32| {
            let place = start + offset;
            match repeat {
                Repeat::NoRepeat => vec![place],
                Repeat::Space if crate::math::floor(extent / tile) >= 2.0 => {
                    let count = crate::math::floor(extent / tile);
                    let gap = (extent - count * tile) / (count - 1.0);
                    let step = tile + gap;
                    let first = start + crate::math::floor((low - start) / step) * step;
                    let mut at = first;
                    let mut placed = Vec::new();
                    while at < high && placed.len() < 4096 {
                        placed.push(at);
                        at += step;
                    }
                    placed
                }
                Repeat::Space => vec![place],
                Repeat::Repeat | Repeat::Round => {
                    let first = place - crate::math::ceil((place - low) / tile) * tile;
                    let mut at = first;
                    let mut placed = Vec::new();
                    while at < high && placed.len() < 4096 {
                        placed.push(at);
                        at += tile;
                    }
                    placed
                }
            }
        };
    let position = layer.position;
    let across = axis(
        repeat.x,
        area.x,
        area.width,
        width,
        position.x.resolve(area.width - width),
        clip.x,
        clip.x + clip.width,
    );
    let down = axis(
        repeat.y,
        area.y,
        area.height,
        height,
        position.y.resolve(area.height - height),
        clip.y,
        clip.y + clip.height,
    );
    let mut tiles = Vec::new();
    for &y in &down {
        for &x in &across {
            if tiles.len() >= 4096 {
                return tiles;
            }
            tiles.push(Rect::new(x, y, width, height));
        }
    }
    tiles
}

/// Builds the native UI of a laid-out document for a viewport.
pub fn scene(dom: &Dom, styles: &Styles, layout: &LayoutResult, viewport: (f32, f32)) -> Scene {
    scene_with(dom, styles, layout, viewport, None)
}

/// Builds the native UI of a laid-out document for a viewport, with its
/// selection and caret.
pub fn scene_with(
    dom: &Dom,
    styles: &Styles,
    layout: &LayoutResult,
    viewport: (f32, f32),
    highlight: Option<&Highlight<'_>>,
) -> Scene {
    scene_over(dom, styles, layout, viewport, highlight, Color::WHITE)
}

/// Builds the native UI of a laid-out document for a viewport, with its
/// selection and caret, over `base`: what the window shows where the root
/// paints no background, as Chromium's base background color, white in
/// opaque windows and transparent in transparent ones.
pub fn scene_over(
    dom: &Dom,
    styles: &Styles,
    layout: &LayoutResult,
    viewport: (f32, f32),
    highlight: Option<&Highlight<'_>>,
    base: Color,
) -> Scene {
    let html = dom.document_element();
    let body = dom.html_child("body");
    let style_of = |node: Option<NodeId>| node.and_then(|node| styles.get(node));
    let html_style = style_of(html);
    let (canvas, canvas_color) = [(html, html_style), (body, style_of(body))]
        .into_iter()
        .find_map(|(node, style)| {
            style
                .filter(|style| !style.background_color.is_transparent())
                .map(|style| (node, style.background_color))
        })
        .unwrap_or((None, base));
    // The root's `overflow` applies to the window, or the body's when the
    // root's is `visible`.
    let viewport_overflow =
        if html_style.is_some_and(|style| overflow_of(style) != Overflowing::Visible) {
            html
        } else {
            body
        };
    let (overflow_x, overflow_y) = style_of(viewport_overflow)
        .map_or((Overflow::Visible, Overflow::Visible), |style| {
            (style.overflow_x, style.overflow_y)
        });
    let fixed = |overflow: Overflow| matches!(overflow, Overflow::Hidden | Overflow::Clip);
    let (width, height) = viewport;
    let content_width = if fixed(overflow_x) {
        width
    } else {
        layout.width.max(width)
    };
    let content_height = if fixed(overflow_y) {
        height
    } else {
        layout.height.max(height)
    };
    let (left, top) = dom.scroll(html.unwrap_or(Dom::DOCUMENT));
    let scroll = (
        clamp(left, content_width - width),
        clamp(top, content_height - height),
    );
    let mut root = Surface {
        id: 0,
        rect: Rect::new(0.0, 0.0, width, height),
        content_width,
        content_height,
        scroll_x: scroll.0,
        scroll_y: scroll.1,
        // The window's scrollbars, as the browser draws them unless author
        // CSS styles them.
        scrollbars: Some(Box::new(
            html.and_then(|node| styles.scrollbars(node))
                .cloned()
                .unwrap_or_else(|| {
                    let axis =
                        |overflow: Overflow| (!fixed(overflow), overflow == Overflow::Scroll);
                    nativeboat_ir::Scrollbars::platform(
                        layout.scrollbars.0,
                        [axis(overflow_x), axis(overflow_y)],
                    )
                }),
        )),
        items: vec![DisplayItem::Fill {
            rect: Rect::new(0.0, 0.0, content_width, content_height),
            color: canvas_color,
            radii: [Corner::SQUARE; 4],
        }],
        ..Surface::default()
    };
    let mut runs: HashMap<NodeId, Vec<(usize, &TextRun)>> = HashMap::new();
    let mut pseudo_runs: HashMap<NodeId, Vec<(usize, &TextRun)>> = HashMap::new();
    for (index, run) in layout.runs.iter().enumerate() {
        let by_container = if run.pseudo.is_some() {
            &mut pseudo_runs
        } else {
            &mut runs
        };
        by_container
            .entry(run.container)
            .or_default()
            .push((index, run));
    }
    let mut inline_boxes: HashMap<NodeId, Vec<&InlineBoxRect>> = HashMap::new();
    for part in &layout.inline_boxes {
        inline_boxes.entry(part.container).or_default().push(part);
    }
    let (bounds, caret) = highlight.map_or_else(Default::default, |highlight| {
        highlight.place(dom, styles, layout)
    });
    let chromium = highlight.map_or(u32::MAX, |highlight| highlight.measurer.chromium());
    let mut painter = Painter {
        dom,
        styles,
        layout,
        runs,
        pseudo_runs,
        inline_boxes,
        canvas,
        canvas_color,
        viewport_overflow,
        root_scroll: scroll,
        highlight,
        bounds,
        caret,
        lcd_backdrop: lcd_backdrop(dom, styles, layout, chromium),
        chromium,
        scene: Scene::default(),
    };
    let frame = Frame {
        origin: (0.0, 0.0),
        window: (-scroll.0, -scroll.1),
        clip: None,
        transform: IDENTITY,
        hit_clip: Some(0),
        scrollport: Rect::new(scroll.0, scroll.1, width, height),
        opacity: 1.0,
        fixed: Fixed {
            mark: 0,
            clip: None,
            hit_clip: Some(0),
        },
        positioned: (Some(0), None),
        // Over a transparent canvas, the window's layer keeps ClearType only
        // for text that all lies on what the page paints opaque.
        grayscale: canvas_color.a < 1.0 && html.is_some_and(|html| !painter.text_on_opaque(html)),
        // A backdrop filter that could fringe ClearType keeps it from all
        // that paints in the window's layer before it, as Chromium reads
        // that back to filter it, and the layer's own text with it.
        no_lcd: painter.lcd_backdrop.is_some(),
    };
    painter.scene.clips.push(Clip {
        region: Geometry {
            rect: Rect::new(0.0, 0.0, width, height),
            transform: IDENTITY,
        },
        parent: None,
        window: frame.window,
        path: None,
    });
    if let Some(mut layer) = html.and_then(|html| painter.layer(html, frame)) {
        // The root element is a stacking context whatever its `z-index`.
        layer.auto = false;
        painter.context(layer, &mut root);
    }
    shadows_over_children(&mut root);
    painter.scene.tree.root = root;
    painter.scene
}

#[cfg(test)]
mod tests;
