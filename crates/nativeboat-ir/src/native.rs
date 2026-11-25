//! The platform-independent native UI tree.
//!
//! The runtime turns the DOM, computed styles and layout into a
//! [`NativeTree`] of surfaces (the window and every scroll container), each
//! with a display list for painted content and the native controls placed on
//! it. [`diff`] computes the changes a platform backend applies to its
//! windows and controls.

use crate::prelude::*;
use nativeboat_css::Color;
use nativeboat_css::types::{
    BorderStyle, FillRule, Filter, LengthPercentageAuto, LineCap, LineJoin, Radius, Size,
};
use serde::{Deserialize, Serialize};

/// Native control types used for HTML form elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlKind {
    /// `<button>` and button-like inputs.
    Button,
    /// Single-line text input.
    TextBox,
    /// `<input type="password">`.
    Password,
    /// `<textarea>`.
    TextArea,
    /// `<input type="checkbox">`.
    CheckBox,
    /// `<input type="radio">`.
    Radio,
    /// `<select>` showing one option.
    ComboBox,
    /// `<select multiple>` or `<select size>`.
    ListBox,
    /// `<input type="range">`.
    Slider,
    /// `<progress>`.
    ProgressBar,
    /// `<input type="date">`.
    Date,
    /// `<input type="time">`.
    Time,
    /// `<input type="datetime-local">`.
    DateTime,
    /// `<input type="month">`.
    Month,
}

/// Maps an element to the native control that renders it, if any.
pub fn control_for<'a>(
    tag: &str,
    attribute: impl Fn(&str) -> Option<&'a str>,
) -> Option<ControlKind> {
    match tag {
        "button" => Some(ControlKind::Button),
        "textarea" => Some(ControlKind::TextArea),
        "progress" => Some(ControlKind::ProgressBar),
        "select" => {
            let size = attribute("size")
                .and_then(|size| size.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if attribute("multiple").is_some() || size > 1 {
                Some(ControlKind::ListBox)
            } else {
                Some(ControlKind::ComboBox)
            }
        }
        "input" => {
            let kind = attribute("type")
                .unwrap_or("text")
                .trim()
                .to_ascii_lowercase();
            match kind.as_str() {
                "hidden" => None,
                "password" => Some(ControlKind::Password),
                "checkbox" => Some(ControlKind::CheckBox),
                "radio" => Some(ControlKind::Radio),
                "button" | "submit" | "reset" | "file" | "image" => Some(ControlKind::Button),
                "range" => Some(ControlKind::Slider),
                "date" => Some(ControlKind::Date),
                "time" => Some(ControlKind::Time),
                "datetime-local" => Some(ControlKind::DateTime),
                "month" => Some(ControlKind::Month),
                _ => Some(ControlKind::TextBox),
            }
        }
        _ => None,
    }
}

/// A rectangle in device-independent pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// Creates a rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether the point lies inside the rectangle.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }

    /// Moves the rectangle.
    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// A font request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    /// Family names in preference order.
    pub families: Vec<String>,
    /// Size in pixels.
    pub size: f32,
    /// Weight from 1 to 1000.
    pub weight: u16,
    /// Whether the font is italic.
    pub italic: bool,
    /// OpenType features asked for, as tags packed little-endian with
    /// their values, such as `tnum` for tabular figures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<(u32, u32)>,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            families: vec!["Segoe UI".to_owned()],
            size: 13.333,
            weight: 400,
            italic: false,
            features: Vec::new(),
        }
    }
}

/// The state of a native control.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ControlState {
    /// Caption of buttons or the value of text controls.
    pub text: String,
    /// Placeholder of text controls.
    pub placeholder: String,
    /// Whether a checkbox or radio button is checked.
    pub checked: bool,
    /// Whether a checkbox is indeterminate.
    pub indeterminate: bool,
    /// Whether the control is disabled.
    pub disabled: bool,
    /// Whether a text control is read-only.
    pub read_only: bool,
    /// Options of combo and list boxes.
    pub options: Vec<String>,
    /// Selected option indexes.
    pub selected: Vec<usize>,
    /// Whether a list box allows several selected options.
    pub multiple: bool,
    /// Value of sliders and progress bars.
    pub value: f64,
    /// Minimum of sliders.
    pub minimum: f64,
    /// Maximum of sliders and progress bars.
    pub maximum: f64,
    /// Maximum text length, if limited.
    pub max_length: Option<u32>,
    /// Font.
    pub font: Font,
    /// Text color.
    pub color: Color,
    /// The placeholder's color, from `::placeholder`.
    pub placeholder_color: Color,
    /// How a text field aligns its text.
    #[serde(default)]
    pub text_align: nativeboat_css::types::TextAlign,
    /// Background color; transparent keeps the native look.
    pub background: Color,
    /// Whether the control is visible.
    pub visible: bool,
    /// Whether author CSS draws the control's box or toggle glyph. Text
    /// fields edit in the content box; toggles retain their owner bounds.
    pub bare: bool,
    /// Visible area relative to the control's bounds, when clipped.
    pub clip: Option<Rect>,
    /// For a control in a rotated or skewed box, which no native control
    /// can show: the map `[a, b, c, d, e, f]` from its own box, in CSS
    /// pixels from its top-left corner, to the window's client area. The
    /// native control is kept out of sight; its image paints through the
    /// transform, where its child marker is, and input there reaches it.
    #[serde(default)]
    pub transform: Option<[f32; 6]>,
    /// Whether the control's text is smoothed in grayscale rather than with
    /// ClearType, as the page's text where it lies is.
    #[serde(default)]
    pub grayscale: bool,
}

/// A native control placed on a surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    /// Stable id: the DOM node id.
    pub id: u64,
    /// Control type.
    pub kind: ControlKind,
    /// Bounds relative to the surface content.
    pub rect: Rect,
    /// State.
    pub state: ControlState,
}

/// A corner's horizontal and vertical radii.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Corner {
    /// Horizontal radius.
    pub x: f32,
    /// Vertical radius.
    pub y: f32,
}

impl Corner {
    /// A square corner.
    pub const SQUARE: Self = Self { x: 0.0, y: 0.0 };

    /// A circular corner.
    pub const fn round(radius: f32) -> Self {
        Self {
            x: radius,
            y: radius,
        }
    }

    /// Whether the corner is square: as CSS has it, when either radius is
    /// zero.
    pub fn is_square(self) -> bool {
        self.x <= 0.0 || self.y <= 0.0
    }

    /// The corner with both radii multiplied by `factor`.
    pub fn scaled(self, factor: f32) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }

    /// The corner with `across` taken from its horizontal radius and `down`
    /// from its vertical one, neither below zero: the corner of an edge
    /// inset by those widths, and square once either radius reaches zero.
    pub fn less(self, across: f32, down: f32) -> Self {
        let corner = Self {
            x: (self.x - across).max(0.0),
            y: (self.y - down).max(0.0),
        };
        if corner.is_square() {
            Self::SQUARE
        } else {
            corner
        }
    }
}

/// Per-corner radii: top-left, top-right, bottom-right, bottom-left.
pub type Radii = [Corner; 4];

/// A piece of a vector path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathSegment {
    /// Starts a subpath at a point.
    Move(f32, f32),
    /// A straight line to a point.
    Line(f32, f32),
    /// A cubic Bézier curve through two control points to a point.
    Cubic(f32, f32, f32, f32, f32, f32),
    /// Closes the subpath.
    Close,
}

/// How a vector path is outlined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Color.
    pub color: Color,
    /// Width in path units.
    pub width: f32,
    /// Line ends.
    pub cap: LineCap,
    /// Corners.
    pub join: LineJoin,
    /// Dash and gap lengths in path units, repeated along the outline;
    /// empty for a solid outline.
    #[serde(default)]
    pub dashes: Vec<f32>,
    /// How far into the dash pattern the outline starts.
    #[serde(default)]
    pub dash_offset: f32,
}

/// Painted content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayItem {
    /// The paint position of a native child. It draws through its own HWND;
    /// following painted content must also composite above that HWND.
    Child(NativeChild),
    /// A filled, optionally rounded rectangle.
    Fill {
        /// Bounds.
        rect: Rect,
        /// Fill color.
        color: Color,
        /// Corner radii.
        radii: Radii,
    },
    /// A linear gradient; `angle` in degrees, 180 pointing down.
    Gradient {
        /// Bounds.
        rect: Rect,
        /// Direction.
        angle: f32,
        /// Colors and positions between 0 and 1.
        stops: Vec<(Color, f32)>,
        /// Corner radii.
        radii: Radii,
    },
    /// Box borders.
    Border {
        /// Outer bounds.
        rect: Rect,
        /// Widths: top, right, bottom, left.
        widths: [f32; 4],
        /// Colors: top, right, bottom, left.
        colors: [Color; 4],
        /// Styles: top, right, bottom, left.
        styles: [BorderStyle; 4],
        /// Corner radii.
        radii: Radii,
    },
    /// A run of text on one line.
    Text {
        /// Bounds of the run; the baseline is placed by the backend.
        rect: Rect,
        /// The text.
        text: String,
        /// Font.
        font: Font,
        /// Color.
        color: Color,
        /// Underline.
        underline: bool,
        /// Line-through.
        line_through: bool,
        /// Extra advance after each character, in pixels.
        #[serde(default)]
        letter_spacing: f32,
        /// Whether the text runs right to left, which mirrors its brackets.
        #[serde(default)]
        rtl: bool,
        /// Whether glyphs are smoothed in grayscale rather than with
        /// ClearType, as Chromium draws text in composited layers where
        /// nothing opaque lies beneath it.
        #[serde(default)]
        grayscale: bool,
        /// How far below the baseline the underline begins, in pixels,
        /// when `text-underline-offset` sets it; else the font places it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        underline_offset: Option<f32>,
    },
    /// A filled and/or outlined vector path.
    Path {
        /// The segments, in path units.
        segments: Vec<PathSegment>,
        /// The affine map `[a, b, c, d, e, f]` from path units to content
        /// coordinates: `x' = a·x + c·y + e` and `y' = b·x + d·y + f`.
        transform: [f32; 6],
        /// Fill color.
        fill: Option<Color>,
        /// Which points the fill covers.
        fill_rule: FillRule,
        /// Outline.
        stroke: Option<Stroke>,
    },
    /// A box shadow; the platform approximates the blur.
    Shadow {
        /// The shadow's shape: the box moved by the offset and grown, or
        /// for inset shadows shrunk, by the spread.
        rect: Rect,
        /// The shape's corner radii.
        radii: Radii,
        /// Color.
        color: Color,
        /// Blur radius.
        blur: f32,
        /// Whether the shadow falls inside the box rather than around it.
        inset: bool,
        /// The box: outer shadows are drawn outside it, inset ones inside.
        clip: Rect,
        /// The box's corner radii.
        clip_radii: Radii,
    },
    /// Transforms the following items until the matching
    /// [`DisplayItem::PopTransform`]: `[a, b, c, d, e, f]` maps content
    /// coordinates as `x' = a·x + c·y + e` and `y' = b·x + d·y + f`.
    PushTransform([f32; 6]),
    /// Ends a transform.
    PopTransform,
    /// The bitmap of a `<canvas>` element.
    Canvas {
        /// Bounds.
        rect: Rect,
        /// The platform key of the bitmap.
        key: u64,
        /// Changes whenever the bitmap is drawn on.
        version: u64,
    },
    /// An image asset.
    Image {
        /// Bounds.
        rect: Rect,
        /// Asset URL.
        url: String,
    },
    /// Clips following items until the matching [`DisplayItem::PopClip`].
    PushClip {
        /// Clip bounds.
        rect: Rect,
        /// Corner radii.
        radii: Radii,
    },
    /// Clips following items to a path, in content coordinates, until the
    /// matching [`DisplayItem::PopClip`].
    PushClipPath {
        /// The outline.
        segments: Vec<PathSegment>,
        /// Which points it covers.
        fill_rule: FillRule,
    },
    /// Ends a clip.
    PopClip,
    /// Applies opacity to following items until [`DisplayItem::PopOpacity`].
    PushOpacity(f32),
    /// Ends an opacity group.
    PopOpacity,
    /// The first item of a surface without an opaque background: what lies
    /// beneath it in the window shows through, as it would were the surface
    /// not a window of its own.
    Beneath,
    /// Paints the following items as one group until the matching
    /// [`DisplayItem::PopFilter`], then applies the filter functions to it.
    PushFilter(Vec<Filter>),
    /// Ends a filter group.
    PopFilter,
    /// Paints the following items as one group until the matching
    /// [`DisplayItem::PopMask`], then keeps of it only what the alpha of
    /// these items, painted over its area, covers.
    PushMask(Vec<DisplayItem>),
    /// Ends a mask group.
    PopMask,
    /// Applies filter functions to what is already painted inside a box,
    /// before the box itself paints.
    Backdrop {
        /// The box.
        rect: Rect,
        /// Its corner radii.
        radii: Radii,
        /// The functions, in order.
        filters: Vec<Filter>,
    },
    /// The text insertion caret, which the platform blinks and shows only
    /// while the window has the keyboard focus.
    Caret {
        /// Bounds.
        rect: Rect,
        /// Color.
        color: Color,
    },
}

/// A native child in a surface's paint order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeChild {
    /// A nested surface.
    Surface(u64),
    /// A native control.
    Control(u64),
}

/// A scrollbar part's box paint, independent of its changing position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollbarPaint {
    /// Whether the part is visible.
    pub visible: bool,
    /// Background color, with the part's opacity applied.
    pub background: Color,
    /// Top, right, bottom and left border widths.
    pub widths: [f32; 4],
    /// Border colors in the same order.
    pub colors: [Color; 4],
    /// Border styles in the same order.
    pub borders: [BorderStyle; 4],
    /// Corner radii clockwise from top left.
    pub radii: [Radius; 4],
    /// Margins in top, right, bottom, left order.
    pub margins: [LengthPercentageAuto; 4],
}

/// Normal, hover and active paint for a scrollbar part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollbarPart(pub [ScrollbarPaint; 3]);

/// A horizontal or vertical scrollbar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollbarAxis {
    /// Whether overflow allows a scrollbar on this axis.
    pub enabled: bool,
    /// Shows a track even when the content fits (`overflow: scroll`).
    pub force: bool,
    /// Width for the vertical axis, height for the horizontal axis.
    pub thickness: Size,
    /// The whole bar's paint.
    pub bar: ScrollbarPart,
    /// The track's paint, above the bar's background.
    pub track: ScrollbarPart,
    /// The draggable thumb's paint.
    pub thumb: ScrollbarPart,
}

/// Author-styled scrollbars, painted in viewport coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scrollbars {
    /// Horizontal, then vertical.
    pub axes: [ScrollbarAxis; 2],
    /// The area between two visible scrollbars.
    pub corner: ScrollbarPart,
    /// For scrollbars no author CSS styles, the look the browser draws
    /// them in, with arrow buttons; their parts' paint is then unused.
    #[serde(default)]
    pub platform: Option<crate::scrollbar::Look>,
}

impl ScrollbarPaint {
    /// The paint of a part that does not show.
    pub fn hidden() -> Self {
        Self {
            visible: false,
            background: Color::TRANSPARENT,
            widths: [0.0; 4],
            colors: [Color::TRANSPARENT; 4],
            borders: [BorderStyle::None; 4],
            radii: core::array::from_fn(|_| Radius::default()),
            margins: core::array::from_fn(|_| LengthPercentageAuto::zero()),
        }
    }
}

impl Scrollbars {
    /// Scrollbars no author CSS styles, drawn as the browser draws them in
    /// a look, on the axes overflow gives them, horizontal first: whether
    /// it lets one show, and whether one shows when the content fits.
    pub fn platform(look: crate::scrollbar::Look, axes: [(bool, bool); 2]) -> Self {
        let hidden = ScrollbarPart(core::array::from_fn(|_| ScrollbarPaint::hidden()));
        Self {
            axes: axes.map(|(enabled, force)| ScrollbarAxis {
                enabled,
                force,
                thickness: Size::Auto,
                bar: hidden.clone(),
                track: hidden.clone(),
                thumb: hidden.clone(),
            }),
            corner: hidden,
            platform: Some(look),
        }
    }
}

/// The window or a scroll container.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Surface {
    /// Stable id: 0 for the window, otherwise the DOM node id.
    pub id: u64,
    /// Bounds relative to the parent surface content.
    pub rect: Rect,
    /// Size of the scrollable content.
    pub content_width: f32,
    /// Height of the scrollable content.
    pub content_height: f32,
    /// Horizontal scroll offset.
    pub scroll_x: f32,
    /// Vertical scroll offset.
    pub scroll_y: f32,
    /// `None` keeps the native platform scrollbar appearance.
    #[serde(default)]
    pub scrollbars: Option<Box<Scrollbars>>,
    /// The part of the surface that shows, relative to its bounds, when
    /// boxes around it clip it; `None` shows it all.
    #[serde(default)]
    pub clip: Option<Rect>,
    /// The opacity of translucent boxes around the surface, which it shows
    /// through; `None` when it is opaque.
    #[serde(default)]
    pub opacity: Option<f32>,
    /// Painted content in content coordinates.
    pub items: Vec<DisplayItem>,
    /// Native controls in content coordinates, bottom to top.
    pub controls: Vec<Control>,
    /// Nested scroll containers, bottom to top.
    pub children: Vec<Surface>,
}

/// The native UI of one window.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NativeTree {
    /// The window surface.
    pub root: Surface,
}

/// A change to apply to the platform UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Change {
    /// Changes the part of a surface that shows and how opaque it is.
    SurfaceView {
        /// Surface id.
        surface: u64,
        /// The visible part relative to the surface's bounds, or all of it.
        clip: Option<Rect>,
        /// The surface's opacity, or opaque.
        opacity: Option<f32>,
    },
    /// Changes a surface's scrollbar appearance.
    Scrollbars {
        /// Surface id.
        surface: u64,
        /// The new appearance, or the platform defaults.
        style: Option<Box<Scrollbars>>,
    },
    /// Creates a scroll container.
    AddSurface {
        /// New surface id.
        id: u64,
        /// Parent surface id.
        parent: u64,
    },
    /// Moves, resizes or scrolls a surface.
    UpdateSurface {
        /// Surface id.
        id: u64,
        /// Bounds.
        rect: Rect,
        /// Content width.
        content_width: f32,
        /// Content height.
        content_height: f32,
        /// Horizontal scroll offset.
        scroll_x: f32,
        /// Vertical scroll offset.
        scroll_y: f32,
    },
    /// Destroys a surface and everything on it.
    RemoveSurface {
        /// Surface id.
        id: u64,
    },
    /// Replaces the display list of a surface.
    Paint {
        /// Surface id.
        surface: u64,
        /// The new display list.
        items: Vec<DisplayItem>,
    },
    /// Creates a control on a surface.
    AddControl {
        /// Surface id.
        surface: u64,
        /// The control.
        control: Control,
    },
    /// Updates the bounds or state of a control.
    UpdateControl {
        /// The control with its new bounds and state.
        control: Control,
    },
    /// Destroys a control.
    RemoveControl {
        /// Control id.
        id: u64,
    },
    /// Restacks the controls of a surface bottom to top.
    Order {
        /// Surface id.
        surface: u64,
        /// Control ids bottom to top.
        controls: Vec<u64>,
    },
    /// Restacks native surfaces and controls together, bottom to top.
    Stack {
        /// Parent surface.
        surface: u64,
        /// Native children in paint order.
        children: Vec<NativeChild>,
    },
    /// Selects text in a text control.
    Select {
        /// Control id.
        id: u64,
        /// Start offset in UTF-16 units, counting a line break once.
        start: u32,
        /// End offset.
        end: u32,
    },
}

struct Flat<'t> {
    surfaces: Vec<(u64, &'t Surface)>,
    controls: HashMap<u64, (u64, &'t Control)>,
}

fn flatten(tree: &NativeTree) -> Flat<'_> {
    let mut flat = Flat {
        surfaces: Vec::new(),
        controls: HashMap::new(),
    };
    let mut pending = vec![(0, &tree.root)];
    while let Some((parent, surface)) = pending.pop() {
        flat.surfaces.push((parent, surface));
        for control in &surface.controls {
            flat.controls.insert(control.id, (surface.id, control));
        }
        for child in surface.children.iter().rev() {
            pending.push((surface.id, child));
        }
    }
    flat
}

fn geometry_changed(old: &Surface, new: &Surface) -> bool {
    old.rect != new.rect
        || old.content_width != new.content_width
        || old.content_height != new.content_height
        || old.scroll_x != new.scroll_x
        || old.scroll_y != new.scroll_y
}

/// Computes the changes that turn `old` into `new`.
///
/// Removals come first (controls, then surfaces innermost first), then new
/// surfaces outermost first, surface updates, control additions and updates,
/// restacking and finally repaints.
pub fn diff(old: &NativeTree, new: &NativeTree) -> Vec<Change> {
    let before = flatten(old);
    let after = flatten(new);
    let mut changes = Vec::new();
    let old_surfaces: HashMap<u64, (u64, &Surface)> = before
        .surfaces
        .iter()
        .map(|(parent, surface)| (surface.id, (*parent, *surface)))
        .collect();
    let new_surfaces: HashMap<u64, (u64, &Surface)> = after
        .surfaces
        .iter()
        .map(|(parent, surface)| (surface.id, (*parent, *surface)))
        .collect();
    let surface_survives = |id: u64| match (old_surfaces.get(&id), new_surfaces.get(&id)) {
        (Some((old_parent, _)), Some((new_parent, _))) => old_parent == new_parent,
        _ => false,
    };

    let mut removed_controls: Vec<u64> = before
        .controls
        .iter()
        .filter(|(id, (surface, old))| match after.controls.get(id) {
            Some((new_surface, new)) => {
                new_surface != surface || new.kind != old.kind || !surface_survives(*surface)
            }
            None => true,
        })
        .map(|(id, _)| *id)
        .collect();
    removed_controls.sort_unstable();
    changes.extend(
        removed_controls
            .iter()
            .map(|id| Change::RemoveControl { id: *id }),
    );

    for (_, surface) in before.surfaces.iter().rev() {
        if surface.id != 0 && !surface_survives(surface.id) {
            changes.push(Change::RemoveSurface { id: surface.id });
        }
    }
    let mut created_surfaces = HashSet::new();
    for (parent, surface) in &after.surfaces {
        if surface.id != 0 && !surface_survives(surface.id) {
            created_surfaces.insert(surface.id);
            changes.push(Change::AddSurface {
                id: surface.id,
                parent: *parent,
            });
        }
    }
    for (_, surface) in &after.surfaces {
        let previous = old_surfaces
            .get(&surface.id)
            .filter(|_| !created_surfaces.contains(&surface.id))
            .and_then(|(_, old)| old.scrollbars.as_ref());
        if previous != surface.scrollbars.as_ref() {
            changes.push(Change::Scrollbars {
                surface: surface.id,
                style: surface.scrollbars.clone(),
            });
        }
        let old_view = old_surfaces
            .get(&surface.id)
            .filter(|_| !created_surfaces.contains(&surface.id))
            .map_or((None, None), |(_, old)| (old.clip, old.opacity));
        if old_view != (surface.clip, surface.opacity) {
            changes.push(Change::SurfaceView {
                surface: surface.id,
                clip: surface.clip,
                opacity: surface.opacity,
            });
        }
        let changed = match old_surfaces.get(&surface.id) {
            Some((_, old)) if !created_surfaces.contains(&surface.id) => {
                geometry_changed(old, surface)
            }
            _ => true,
        };
        if changed {
            changes.push(Change::UpdateSurface {
                id: surface.id,
                rect: surface.rect,
                content_width: surface.content_width,
                content_height: surface.content_height,
                scroll_x: surface.scroll_x,
                scroll_y: surface.scroll_y,
            });
        }
    }
    for (_, surface) in &after.surfaces {
        for control in &surface.controls {
            let existing = before
                .controls
                .get(&control.id)
                .filter(|_| !removed_controls.contains(&control.id));
            match existing {
                Some((_, old)) if *old == control => {}
                Some(_) => changes.push(Change::UpdateControl {
                    control: control.clone(),
                }),
                None => changes.push(Change::AddControl {
                    surface: surface.id,
                    control: control.clone(),
                }),
            }
        }
    }
    for (_, surface) in &after.surfaces {
        let order: Vec<u64> = surface.controls.iter().map(|control| control.id).collect();
        let previous: Option<Vec<u64>> = old_surfaces
            .get(&surface.id)
            .filter(|_| !created_surfaces.contains(&surface.id))
            .map(|(_, old)| {
                old.controls
                    .iter()
                    .map(|control| control.id)
                    .filter(|id| order.contains(id))
                    .collect()
            });
        let added_any = order.iter().any(|id| {
            !previous
                .as_ref()
                .is_some_and(|previous| previous.contains(id))
        });
        if order.len() > 1 && (added_any || previous.as_ref() != Some(&order)) {
            changes.push(Change::Order {
                surface: surface.id,
                controls: order,
            });
        }
    }
    for (_, surface) in &after.surfaces {
        let children = |items: &[DisplayItem]| {
            items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Child(child) => Some(*child),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let order = children(&surface.items);
        let previous = old_surfaces
            .get(&surface.id)
            .filter(|_| !created_surfaces.contains(&surface.id))
            .map(|(_, old)| children(&old.items));
        if !order.is_empty()
            && (previous.as_ref() != Some(&order)
                || order.iter().any(|child| match child {
                    NativeChild::Surface(id) => created_surfaces.contains(id),
                    NativeChild::Control(id) => removed_controls.contains(id),
                }))
        {
            changes.push(Change::Stack {
                surface: surface.id,
                children: order,
            });
        }
        let repaint = match old_surfaces.get(&surface.id) {
            Some((_, old)) if !created_surfaces.contains(&surface.id) => old.items != surface.items,
            _ => true,
        };
        if repaint {
            changes.push(Change::Paint {
                surface: surface.id,
                items: surface.items.clone(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests;
