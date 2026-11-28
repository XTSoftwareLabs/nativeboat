//! Computed value types and their serialization.

use crate::prelude::*;

use crate::color::Color;
use crate::values::{LengthPercentage, format_float};
use serde::{Deserialize, Serialize};

/// Serializes a computed value like `getComputedStyle`.
pub trait ToCss {
    /// The CSS text of the value.
    fn to_css(&self) -> String;
}

impl ToCss for LengthPercentage {
    fn to_css(&self) -> String {
        LengthPercentage::to_css(self)
    }
}

impl ToCss for Color {
    fn to_css(&self) -> String {
        Color::to_css(self)
    }
}

impl ToCss for f32 {
    fn to_css(&self) -> String {
        format_float(*self)
    }
}

impl ToCss for i32 {
    fn to_css(&self) -> String {
        self.to_string()
    }
}

/// Defines a keyword enum with parsing, serialization and a default.
macro_rules! keywords {
    ($(#[$meta:meta])* $name:ident default $default:ident { $($variant:ident => $first:literal $(| $alias:literal)*,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(
                #[doc = concat!("`", $first, "`")]
                $variant,
            )+
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// Parses a lowercase keyword.
            pub fn from_keyword(keyword: &str) -> Option<Self> {
                match keyword {
                    $($first $(| $alias)* => Some($name::$variant),)+
                    _ => None,
                }
            }

            /// The canonical keyword.
            pub fn keyword(self) -> &'static str {
                match self {
                    $($name::$variant => $first,)+
                }
            }
        }

        impl ToCss for $name {
            fn to_css(&self) -> String {
                self.keyword().to_owned()
            }
        }

        impl crate::parsers::Keyword for $name {
            fn parse_keyword(keyword: &str) -> Option<Self> {
                $name::from_keyword(keyword)
            }
        }
    };
}

keywords! {
    /// The `display` property.
    Display default Inline {
        None => "none",
        Block => "block",
        Inline => "inline",
        InlineBlock => "inline-block",
        Flex => "flex" | "-webkit-flex",
        WebkitBox => "-webkit-box",
        WebkitInlineBox => "-webkit-inline-box",
        InlineFlex => "inline-flex" | "-webkit-inline-flex",
        Grid => "grid",
        InlineGrid => "inline-grid",
        Contents => "contents",
        ListItem => "list-item",
        FlowRoot => "flow-root",
        Table => "table",
        InlineTable => "inline-table",
        TableRowGroup => "table-row-group",
        TableHeaderGroup => "table-header-group",
        TableFooterGroup => "table-footer-group",
        TableRow => "table-row",
        TableCell => "table-cell",
        TableCaption => "table-caption",
        TableColumn => "table-column",
        TableColumnGroup => "table-column-group",
    }
}

impl Display {
    /// Whether the element generates an inline-level box.
    pub fn is_inline_level(self) -> bool {
        matches!(
            self,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::WebkitInlineBox
                | Display::InlineGrid
                | Display::InlineTable
        )
    }

    /// The block-level equivalent, used for floats, absolute boxes and flex items.
    pub fn blockified(self) -> Display {
        match self {
            Display::Inline | Display::InlineBlock => Display::Block,
            Display::InlineFlex => Display::Flex,
            Display::WebkitInlineBox => Display::WebkitBox,
            Display::InlineGrid => Display::Grid,
            Display::InlineTable => Display::Table,
            other => other,
        }
    }
}

keywords! {
    /// The `position` property.
    PositionKind default Static {
        Static => "static",
        Relative => "relative",
        Absolute => "absolute",
        Fixed => "fixed",
        Sticky => "sticky" | "-webkit-sticky",
    }
}

keywords! {
    /// The `box-sizing` property.
    BoxSizing default ContentBox {
        ContentBox => "content-box",
        BorderBox => "border-box",
    }
}

keywords! {
    /// The `float` property.
    Float default None {
        None => "none",
        Left => "left" | "inline-start",
        Right => "right" | "inline-end",
    }
}

keywords! {
    /// The `flex-direction` property.
    FlexDirection default Row {
        Row => "row",
        RowReverse => "row-reverse",
        Column => "column",
        ColumnReverse => "column-reverse",
    }
}

keywords! {
    /// The `flex-wrap` property.
    FlexWrap default NoWrap {
        NoWrap => "nowrap",
        Wrap => "wrap",
        WrapReverse => "wrap-reverse",
    }
}

keywords! {
    /// Box alignment values for `justify-*`, `align-*` and `place-*`.
    Alignment default Normal {
        Normal => "normal",
        Auto => "auto",
        Start => "start" | "left" | "self-start",
        End => "end" | "right" | "self-end",
        FlexStart => "flex-start",
        FlexEnd => "flex-end",
        Center => "center",
        Baseline => "baseline",
        Stretch => "stretch",
        SpaceBetween => "space-between",
        SpaceAround => "space-around",
        SpaceEvenly => "space-evenly",
    }
}

keywords! {
    /// One value of `animation-direction`.
    AnimationDirection default Normal {
        Normal => "normal",
        Reverse => "reverse",
        Alternate => "alternate",
        AlternateReverse => "alternate-reverse",
    }
}

keywords! {
    /// One value of `animation-fill-mode`.
    FillMode default None {
        None => "none",
        Forwards => "forwards",
        Backwards => "backwards",
        Both => "both",
    }
}

keywords! {
    /// One value of `animation-play-state`.
    PlayState default Running {
        Running => "running",
        Paused => "paused",
    }
}

/// `font-variant-numeric`: the numeric forms asked for, by keyword, in
/// canonical order; empty for `normal`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FontVariantNumeric(pub Vec<String>);

impl FontVariantNumeric {
    /// The keywords in canonical order, with the OpenType feature each
    /// turns on and the group only one keyword of may appear from.
    pub const KEYWORDS: &'static [(&'static str, &'static [u8; 4], u8)] = &[
        ("lining-nums", b"lnum", 0),
        ("oldstyle-nums", b"onum", 0),
        ("proportional-nums", b"pnum", 1),
        ("tabular-nums", b"tnum", 1),
        ("diagonal-fractions", b"frac", 2),
        ("stacked-fractions", b"afrc", 2),
        ("ordinal", b"ordn", 3),
        ("slashed-zero", b"zero", 4),
    ];

    /// The OpenType features the keywords turn on.
    pub fn features(&self) -> impl Iterator<Item = [u8; 4]> + '_ {
        Self::KEYWORDS
            .iter()
            .filter(|(keyword, _, _)| self.0.iter().any(|chosen| chosen == keyword))
            .map(|(_, tag, _)| **tag)
    }
}

impl ToCss for FontVariantNumeric {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            "normal".to_owned()
        } else {
            self.0.join(" ")
        }
    }
}

/// `font-feature-settings`: OpenType feature tags with their values, in
/// declaration order; empty for `normal`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FontFeatureSettings(pub Vec<([u8; 4], u32)>);

impl ToCss for FontFeatureSettings {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "normal".to_owned();
        }
        self.0
            .iter()
            .map(|(tag, value)| {
                let tag = String::from_utf8_lossy(tag);
                if *value == 1 {
                    format!("\"{tag}\"")
                } else {
                    format!("\"{tag}\" {value}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One value of `animation-iteration-count`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Iterations {
    /// A number of iterations, which may be fractional.
    Count(f32),
    /// `infinite`
    Infinite,
}

impl ToCss for Iterations {
    fn to_css(&self) -> String {
        match self {
            Iterations::Count(count) => format_float(*count),
            Iterations::Infinite => "infinite".to_owned(),
        }
    }
}

keywords! {
    /// The `visibility` property.
    Visibility default Visible {
        Visible => "visible",
        Hidden => "hidden",
        Collapse => "collapse",
    }
}

keywords! {
    /// The `overflow-x` and `overflow-y` properties.
    Overflow default Visible {
        Visible => "visible",
        Hidden => "hidden",
        Clip => "clip",
        Scroll => "scroll",
        Auto => "auto" | "overlay",
    }
}

impl Overflow {
    /// Whether the box scrolls or clips its content.
    pub fn is_scroll_container(self) -> bool {
        matches!(self, Overflow::Hidden | Overflow::Scroll | Overflow::Auto)
    }
}

keywords! {
    /// Border and outline styles.
    BorderStyle default None {
        None => "none",
        Hidden => "hidden",
        Solid => "solid",
        Dashed => "dashed",
        Dotted => "dotted",
        Double => "double",
        Groove => "groove",
        Ridge => "ridge",
        Inset => "inset",
        Outset => "outset",
    }
}

impl BorderStyle {
    /// Whether the style draws nothing and forces a zero width.
    pub fn is_none(self) -> bool {
        matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

keywords! {
    /// The `font-style` property.
    FontStyle default Normal {
        Normal => "normal",
        Italic => "italic",
        Oblique => "oblique",
    }
}

keywords! {
    /// The `text-align` property.
    TextAlign default Start {
        Start => "start" | "match-parent",
        End => "end",
        Left => "left" | "-webkit-left",
        Right => "right" | "-webkit-right",
        Center => "center" | "-webkit-center",
        Justify => "justify",
    }
}

keywords! {
    /// The `direction` property: the base direction of text and the
    /// direction inline content and table columns run in.
    Direction default Ltr {
        Ltr => "ltr",
        Rtl => "rtl",
    }
}

keywords! {
    /// The `unicode-bidi` property: whether an element opens a level of
    /// bidirectional embedding, and whether it isolates its content from
    /// the text around it.
    UnicodeBidi default Normal {
        Normal => "normal",
        Embed => "embed",
        Isolate => "isolate" | "-webkit-isolate",
        BidiOverride => "bidi-override",
        IsolateOverride => "isolate-override" | "-webkit-isolate-override",
        Plaintext => "plaintext" | "-webkit-plaintext",
    }
}

keywords! {
    /// The `text-transform` property.
    TextTransform default None {
        None => "none",
        Capitalize => "capitalize",
        Uppercase => "uppercase",
        Lowercase => "lowercase",
    }
}

keywords! {
    /// The `white-space-collapse` property.
    WhiteSpaceCollapse default Collapse {
        Collapse => "collapse",
        Preserve => "preserve",
        PreserveBreaks => "preserve-breaks",
        BreakSpaces => "break-spaces",
    }
}

keywords! {
    /// The `text-wrap-mode` property.
    TextWrapMode default Wrap {
        Wrap => "wrap",
        NoWrap => "nowrap",
    }
}

keywords! {
    /// The `text-wrap-style` property. Lines break where they fill up for
    /// every style; `balance` and `pretty` only choose among those breaks
    /// in browsers.
    TextWrapStyle default Auto {
        Auto => "auto",
        Balance => "balance",
        Pretty => "pretty",
        Stable => "stable",
    }
}

/// How text handles white space and wrapping: the longhands the
/// `white-space` shorthand sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct WhiteSpace {
    /// `white-space-collapse`.
    pub collapse: WhiteSpaceCollapse,
    /// `text-wrap-mode`.
    pub wrap: TextWrapMode,
}

#[allow(non_upper_case_globals)]
impl WhiteSpace {
    /// `normal`.
    pub const Normal: WhiteSpace =
        WhiteSpace::new(WhiteSpaceCollapse::Collapse, TextWrapMode::Wrap);
    /// `nowrap`.
    pub const NoWrap: WhiteSpace =
        WhiteSpace::new(WhiteSpaceCollapse::Collapse, TextWrapMode::NoWrap);
    /// `pre`.
    pub const Pre: WhiteSpace = WhiteSpace::new(WhiteSpaceCollapse::Preserve, TextWrapMode::NoWrap);
    /// `pre-wrap`.
    pub const PreWrap: WhiteSpace =
        WhiteSpace::new(WhiteSpaceCollapse::Preserve, TextWrapMode::Wrap);
    /// `pre-line`.
    pub const PreLine: WhiteSpace =
        WhiteSpace::new(WhiteSpaceCollapse::PreserveBreaks, TextWrapMode::Wrap);
    /// `break-spaces`.
    pub const BreakSpaces: WhiteSpace =
        WhiteSpace::new(WhiteSpaceCollapse::BreakSpaces, TextWrapMode::Wrap);
    /// The keywords of the shorthand, by the longhands they set.
    pub const KEYWORDS: [(&'static str, WhiteSpace); 6] = [
        ("normal", WhiteSpace::Normal),
        ("nowrap", WhiteSpace::NoWrap),
        ("pre", WhiteSpace::Pre),
        ("pre-wrap", WhiteSpace::PreWrap),
        ("pre-line", WhiteSpace::PreLine),
        ("break-spaces", WhiteSpace::BreakSpaces),
    ];

    /// The value of both longhands.
    pub const fn new(collapse: WhiteSpaceCollapse, wrap: TextWrapMode) -> WhiteSpace {
        WhiteSpace { collapse, wrap }
    }

    /// The longhands a `white-space` keyword sets.
    pub fn from_keyword(keyword: &str) -> Option<WhiteSpace> {
        WhiteSpace::KEYWORDS
            .iter()
            .find(|(name, _)| *name == keyword)
            .map(|(_, value)| *value)
    }

    /// Whether runs of spaces are collapsed.
    pub fn collapses_spaces(self) -> bool {
        matches!(
            self.collapse,
            WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::PreserveBreaks
        )
    }

    /// Whether newlines are kept.
    pub fn keeps_newlines(self) -> bool {
        self.collapse != WhiteSpaceCollapse::Collapse
    }

    /// Whether lines may wrap.
    pub fn wraps(self) -> bool {
        self.wrap == TextWrapMode::Wrap
    }
}

impl ToCss for WhiteSpace {
    /// The `white-space` shorthand: its keyword, or both longhands when no
    /// keyword sets them.
    fn to_css(&self) -> String {
        match WhiteSpace::KEYWORDS.iter().find(|(_, value)| value == self) {
            Some((name, _)) => (*name).to_owned(),
            None => format!("{} {}", self.collapse.keyword(), self.wrap.keyword()),
        }
    }
}

keywords! {
    /// The `-webkit-box-orient` property of `display: -webkit-box`.
    BoxOrient default Horizontal {
        Horizontal => "horizontal" | "inline-axis",
        Vertical => "vertical" | "block-axis",
    }
}

/// The `tab-size` property: how far apart tab stops are, in spaces or as
/// a length.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TabSize {
    /// A number of space advances.
    Spaces(f32),
    /// A length in pixels.
    Length(f32),
}

impl Default for TabSize {
    fn default() -> Self {
        TabSize::Spaces(8.0)
    }
}

/// The `-webkit-line-clamp` property: how many lines a vertical
/// `-webkit-box` shows; `None` is `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LineClamp(pub Option<u32>);

impl ToCss for TabSize {
    fn to_css(&self) -> String {
        match self {
            TabSize::Spaces(spaces) => crate::tokenizer::format_number(f64::from(*spaces)),
            TabSize::Length(px) => format!("{}px", crate::tokenizer::format_number(f64::from(*px))),
        }
    }
}

impl ToCss for LineClamp {
    fn to_css(&self) -> String {
        self.0
            .map_or_else(|| "none".to_owned(), |lines| lines.to_string())
    }
}

keywords! {
    /// The `container-type` property: whether an element is a query
    /// container for its descendants' container query units.
    ContainerType default Normal {
        Normal => "normal",
        Size => "size",
        InlineSize => "inline-size",
    }
}

keywords! {
    /// The `text-overflow` property.
    TextOverflow default Clip {
        Clip => "clip",
        Ellipsis => "ellipsis",
    }
}

keywords! {
    /// The `word-break` property.
    WordBreak default Normal {
        Normal => "normal",
        BreakAll => "break-all",
        KeepAll => "keep-all",
        BreakWord => "break-word",
    }
}

keywords! {
    /// The `overflow-wrap` property.
    OverflowWrap default Normal {
        Normal => "normal",
        BreakWord => "break-word",
        Anywhere => "anywhere",
    }
}

/// The `vertical-align` property.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum VerticalAlign {
    /// `baseline`
    #[default]
    Baseline,
    /// `top`
    Top,
    /// `middle`
    Middle,
    /// `bottom`
    Bottom,
    /// `text-top`
    TextTop,
    /// `text-bottom`
    TextBottom,
    /// `sub`
    Sub,
    /// `super`
    Super,
    /// A length, or a percentage of the line height, that raises the box
    /// above the baseline.
    Length(LengthPercentage),
}

impl VerticalAlign {
    /// Parses a lowercase keyword.
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Some(match keyword {
            "baseline" => VerticalAlign::Baseline,
            "top" => VerticalAlign::Top,
            "middle" => VerticalAlign::Middle,
            "bottom" => VerticalAlign::Bottom,
            "text-top" => VerticalAlign::TextTop,
            "text-bottom" => VerticalAlign::TextBottom,
            "sub" => VerticalAlign::Sub,
            "super" => VerticalAlign::Super,
            _ => return None,
        })
    }
}

impl ToCss for VerticalAlign {
    fn to_css(&self) -> String {
        match self {
            VerticalAlign::Baseline => "baseline".to_owned(),
            VerticalAlign::Top => "top".to_owned(),
            VerticalAlign::Middle => "middle".to_owned(),
            VerticalAlign::Bottom => "bottom".to_owned(),
            VerticalAlign::TextTop => "text-top".to_owned(),
            VerticalAlign::TextBottom => "text-bottom".to_owned(),
            VerticalAlign::Sub => "sub".to_owned(),
            VerticalAlign::Super => "super".to_owned(),
            VerticalAlign::Length(length) => length.to_css(),
        }
    }
}

keywords! {
    /// The `list-style-type` property.
    ListStyleType default Disc {
        Disc => "disc",
        Circle => "circle",
        Square => "square",
        DisclosureClosed => "disclosure-closed",
        DisclosureOpen => "disclosure-open",
        Decimal => "decimal",
        DecimalLeadingZero => "decimal-leading-zero",
        LowerAlpha => "lower-alpha" | "lower-latin",
        UpperAlpha => "upper-alpha" | "upper-latin",
        LowerRoman => "lower-roman",
        UpperRoman => "upper-roman",
        None => "none",
    }
}

keywords! {
    /// The `list-style-position` property.
    ListStylePosition default Outside {
        Outside => "outside",
        Inside => "inside",
    }
}

keywords! {
    /// The `border-collapse` property.
    BorderCollapse default Separate {
        Separate => "separate",
        Collapse => "collapse",
    }
}

keywords! {
    /// The SVG `text-anchor` property: which end of a text run its
    /// position marks.
    TextAnchor default Start {
        Start => "start",
        Middle => "middle",
        End => "end",
    }
}

keywords! {
    /// The SVG `dominant-baseline` and `alignment-baseline` properties:
    /// which line of a text run its `y` position marks.
    SvgBaseline default Auto {
        Auto => "auto",
        Alphabetic => "alphabetic" | "baseline",
        Middle => "middle",
        Central => "central" | "center",
        Hanging => "hanging",
        Mathematical => "mathematical",
        TextBeforeEdge => "text-before-edge" | "before-edge" | "text-top" | "top",
        TextAfterEdge => "text-after-edge" | "after-edge" | "text-bottom" | "bottom",
        Ideographic => "ideographic",
    }
}

keywords! {
    /// The SVG `fill-rule` property.
    FillRule default NonZero {
        NonZero => "nonzero",
        EvenOdd => "evenodd",
    }
}

keywords! {
    /// The SVG `stroke-linecap` property.
    LineCap default Butt {
        Butt => "butt",
        Round => "round",
        Square => "square",
    }
}

keywords! {
    /// The SVG `stroke-linejoin` property.
    LineJoin default Miter {
        Miter => "miter" | "miter-clip" | "arcs",
        Round => "round",
        Bevel => "bevel",
    }
}

/// The SVG `fill` and `stroke` properties.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    /// `none`
    None,
    /// A color.
    Color(Color),
    /// `currentcolor`, which stays symbolic so that it inherits as itself.
    CurrentColor,
}

impl Paint {
    /// The color painted for an element whose `color` is `current`.
    pub fn resolve(self, current: Color) -> Option<Color> {
        match self {
            Paint::None => None,
            Paint::Color(color) => Some(color),
            Paint::CurrentColor => Some(current),
        }
    }
}

impl ToCss for Paint {
    fn to_css(&self) -> String {
        match self {
            Paint::None => "none".to_owned(),
            Paint::Color(color) => color.to_css(),
            Paint::CurrentColor => "currentcolor".to_owned(),
        }
    }
}

keywords! {
    /// The `cursor` property.
    Cursor default Auto {
        Auto => "auto",
        Default => "default",
        None => "none",
        Pointer => "pointer",
        Text => "text" | "vertical-text",
        Move => "move" | "all-scroll",
        NotAllowed => "not-allowed" | "no-drop",
        Wait => "wait",
        Progress => "progress",
        Help => "help",
        Crosshair => "crosshair" | "cell",
        Grab => "grab",
        Grabbing => "grabbing",
        ColResize => "col-resize",
        RowResize => "row-resize",
        EwResize => "ew-resize" | "e-resize" | "w-resize",
        NsResize => "ns-resize" | "n-resize" | "s-resize",
        NeswResize => "nesw-resize" | "ne-resize" | "sw-resize",
        NwseResize => "nwse-resize" | "nw-resize" | "se-resize",
        ContextMenu => "context-menu",
        Copy => "copy" | "alias",
        ZoomIn => "zoom-in",
        ZoomOut => "zoom-out",
    }
}

keywords! {
    /// The `backface-visibility` property: whether an element turned away
    /// from the viewer shows.
    BackfaceVisibility default Visible {
        Visible => "visible",
        Hidden => "hidden",
    }
}

keywords! {
    /// The `isolation` property: `isolate` makes a stacking context.
    Isolation default Auto {
        Auto => "auto",
        Isolate => "isolate",
    }
}

keywords! {
    /// The `pointer-events` property.
    PointerEvents default Auto {
        Auto => "auto" | "visiblepainted" | "visiblefill" | "visiblestroke" | "visible"
            | "painted" | "fill" | "stroke" | "all",
        None => "none",
    }
}

keywords! {
    /// The `app-region` property: the parts of a frameless window that drag
    /// it.
    AppRegion default None {
        None => "none",
        Drag => "drag",
        NoDrag => "no-drag",
    }
}

keywords! {
    /// The `resize` property: whether and how the user can resize a scroll
    /// container by its grip.
    Resize default None {
        None => "none",
        Both => "both",
        Horizontal => "horizontal",
        Vertical => "vertical",
        Block => "block",
        Inline => "inline",
    }
}

impl Resize {
    /// Whether the user can change the width and the height, in
    /// horizontal writing.
    pub fn axes(self) -> (bool, bool) {
        match self {
            Resize::None => (false, false),
            Resize::Both => (true, true),
            Resize::Horizontal | Resize::Inline => (true, false),
            Resize::Vertical | Resize::Block => (false, true),
        }
    }
}

keywords! {
    /// The `table-layout` property: whether a table sizes its columns by
    /// their content or by its first row.
    TableLayout default Auto {
        Auto => "auto",
        Fixed => "fixed",
    }
}

keywords! {
    /// The `caption-side` property: whether a table's captions sit above
    /// or below it.
    CaptionSide default Top {
        Top => "top",
        Bottom => "bottom",
    }
}

keywords! {
    /// The `user-select` property.
    UserSelect default Auto {
        Auto => "auto" | "contain",
        None => "none",
        Text => "text",
        All => "all",
    }
}

keywords! {
    /// The `-webkit-user-drag` property: whether dragging the element drags
    /// it, as images and links do on their own.
    UserDrag default Auto {
        Auto => "auto",
        None => "none",
        Element => "element",
    }
}

keywords! {
    /// The `font-kerning` property: whether glyph pairs kern.
    FontKerning default Auto {
        Auto => "auto",
        Normal => "normal",
        None => "none",
    }
}

keywords! {
    /// The `object-fit` property.
    ObjectFit default Fill {
        Fill => "fill",
        Contain => "contain",
        Cover => "cover",
        None => "none",
        ScaleDown => "scale-down",
    }
}

/// The `clip` rectangle of an absolutely positioned box: where its edges
/// lie, measured from the top left corner of the border box; `None` is
/// `auto`, the border box's own edge.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ClipRect {
    /// Top edge.
    pub top: Option<f32>,
    /// Right edge.
    pub right: Option<f32>,
    /// Bottom edge.
    pub bottom: Option<f32>,
    /// Left edge.
    pub left: Option<f32>,
}

impl ToCss for Option<ClipRect> {
    fn to_css(&self) -> String {
        let Some(rect) = self else {
            return "auto".to_owned();
        };
        let edge = |value: Option<f32>| value.map_or("auto".to_owned(), |px| Px(px).to_css());
        format!(
            "rect({}, {}, {}, {})",
            edge(rect.top),
            edge(rect.right),
            edge(rect.bottom),
            edge(rect.left)
        )
    }
}

/// A box edge quantity for each side.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Edges<T> {
    /// Top side.
    pub top: T,
    /// Right side.
    pub right: T,
    /// Bottom side.
    pub bottom: T,
    /// Left side.
    pub left: T,
}

impl<T: Clone> Edges<T> {
    /// The same value on every side.
    pub fn all(value: T) -> Self {
        Edges {
            top: value.clone(),
            right: value.clone(),
            bottom: value.clone(),
            left: value,
        }
    }
}

/// A quantity for each corner.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Corners<T> {
    /// Top-left corner.
    pub top_left: T,
    /// Top-right corner.
    pub top_right: T,
    /// Bottom-right corner.
    pub bottom_right: T,
    /// Bottom-left corner.
    pub bottom_left: T,
}

/// A length in pixels.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Px(pub f32);

impl ToCss for Px {
    fn to_css(&self) -> String {
        format!("{}px", format_float(self.0))
    }
}

/// SVG `stroke-dasharray`: dash and gap lengths in user units, empty for
/// `none`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dashes(pub Vec<f32>);

impl ToCss for Dashes {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        let lengths: Vec<String> = self.0.iter().map(|length| Px(*length).to_css()).collect();
        lengths.join(", ")
    }
}

/// SVG `marker-start`, `marker-mid` and `marker-end`: the URL of a
/// `<marker>`, or `None` for `none`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkerUrl(pub Option<String>);

impl ToCss for MarkerUrl {
    fn to_css(&self) -> String {
        match &self.0 {
            Some(url) => format!("url(\"{url}\")"),
            None => "none".to_owned(),
        }
    }
}

keywords! {
    /// The thickness of a scroll container's scrollbars.
    ScrollbarWidth default Auto {
        Auto => "auto",
        Thin => "thin",
        None => "none",
    }
}

/// `scrollbar-color`: thumb and track colors, or the platform defaults.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ScrollbarColor(pub Option<(Color, Color)>);

impl ToCss for ScrollbarColor {
    fn to_css(&self) -> String {
        match self.0 {
            Some((thumb, track)) => format!("{} {}", thumb.to_css(), track.to_css()),
            None => "auto".to_owned(),
        }
    }
}

/// `width`, `height`, `min-*` and `flex-basis` values.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Size {
    /// `auto`
    #[default]
    Auto,
    /// A length or percentage.
    Length(LengthPercentage),
    /// `min-content`
    MinContent,
    /// `max-content`
    MaxContent,
    /// `fit-content`
    FitContent,
}

impl ToCss for Size {
    fn to_css(&self) -> String {
        match self {
            Size::Auto => "auto".to_owned(),
            Size::Length(length) => length.to_css(),
            Size::MinContent => "min-content".to_owned(),
            Size::MaxContent => "max-content".to_owned(),
            Size::FitContent => "fit-content".to_owned(),
        }
    }
}

/// `max-width` and `max-height` values.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum MaxSize {
    /// `none`
    #[default]
    None,
    /// A length or percentage.
    Length(LengthPercentage),
    /// `min-content`
    MinContent,
    /// `max-content`
    MaxContent,
    /// `fit-content`
    FitContent,
}

impl ToCss for MaxSize {
    fn to_css(&self) -> String {
        match self {
            MaxSize::None => "none".to_owned(),
            MaxSize::Length(length) => length.to_css(),
            MaxSize::MinContent => "min-content".to_owned(),
            MaxSize::MaxContent => "max-content".to_owned(),
            MaxSize::FitContent => "fit-content".to_owned(),
        }
    }
}

/// A length, percentage or `auto` (margins and insets).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum LengthPercentageAuto {
    /// `auto`
    #[default]
    Auto,
    /// A length or percentage.
    Length(LengthPercentage),
}

impl LengthPercentageAuto {
    /// Zero pixels.
    pub fn zero() -> Self {
        LengthPercentageAuto::Length(LengthPercentage::Px(0.0))
    }
}

impl ToCss for LengthPercentageAuto {
    fn to_css(&self) -> String {
        match self {
            LengthPercentageAuto::Auto => "auto".to_owned(),
            LengthPercentageAuto::Length(length) => length.to_css(),
        }
    }
}

/// A corner radius with horizontal and vertical components.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Radius {
    /// Horizontal radius.
    pub horizontal: LengthPercentage,
    /// Vertical radius.
    pub vertical: LengthPercentage,
}

impl ToCss for Radius {
    fn to_css(&self) -> String {
        if self.horizontal == self.vertical {
            self.horizontal.to_css()
        } else {
            format!("{} {}", self.horizontal.to_css(), self.vertical.to_css())
        }
    }
}

/// The `border-spacing` property: the gaps between table cells.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Spacing {
    /// Between columns.
    pub horizontal: Px,
    /// Between rows.
    pub vertical: Px,
}

impl ToCss for Spacing {
    fn to_css(&self) -> String {
        if self.horizontal == self.vertical {
            self.horizontal.to_css()
        } else {
            format!("{} {}", self.horizontal.to_css(), self.vertical.to_css())
        }
    }
}

/// The `z-index` property; `None` is `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ZIndex(pub Option<i32>);

impl ToCss for ZIndex {
    fn to_css(&self) -> String {
        self.0
            .map_or_else(|| "auto".to_owned(), |value| value.to_string())
    }
}

/// The `font-weight` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontWeight(pub u16);

impl Default for FontWeight {
    fn default() -> Self {
        FontWeight(400)
    }
}

impl ToCss for FontWeight {
    fn to_css(&self) -> String {
        self.0.to_string()
    }
}

/// The `font-family` property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontFamily(pub Vec<String>);

impl Default for FontFamily {
    fn default() -> Self {
        FontFamily(vec!["Times New Roman".to_owned()])
    }
}

impl ToCss for FontFamily {
    fn to_css(&self) -> String {
        self.0
            .iter()
            .map(|family| {
                if family.contains(' ') {
                    format!("\"{family}\"")
                } else {
                    family.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The `line-height` property.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum LineHeight {
    /// `normal`
    #[default]
    Normal,
    /// A multiple of the font size.
    Number(f32),
    /// Pixels.
    Px(f32),
}

impl LineHeight {
    /// The used line height for a font size.
    pub fn resolve(self, font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.2,
            LineHeight::Number(number) => font_size * number,
            LineHeight::Px(px) => px,
        }
    }
}

impl ToCss for LineHeight {
    fn to_css(&self) -> String {
        match self {
            LineHeight::Normal => "normal".to_owned(),
            LineHeight::Number(number) => format_float(*number),
            LineHeight::Px(px) => format!("{}px", format_float(*px)),
        }
    }
}

/// The `text-decoration-line` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextDecorationLine {
    /// `underline`
    pub underline: bool,
    /// `overline`
    pub overline: bool,
    /// `line-through`
    pub line_through: bool,
}

impl TextDecorationLine {
    /// Whether no line is drawn.
    pub fn is_none(self) -> bool {
        !(self.underline || self.overline || self.line_through)
    }
}

impl ToCss for TextDecorationLine {
    fn to_css(&self) -> String {
        let mut words = Vec::new();
        if self.underline {
            words.push("underline");
        }
        if self.overline {
            words.push("overline");
        }
        if self.line_through {
            words.push("line-through");
        }
        if words.is_empty() {
            words.push("none");
        }
        words.join(" ")
    }
}

/// `text-underline-offset`: how far below the alphabetic baseline
/// underlines begin, in pixels; `None` for `auto`, the font's position.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct UnderlineOffset(pub Option<f32>);

impl ToCss for UnderlineOffset {
    fn to_css(&self) -> String {
        self.0
            .map_or_else(|| "auto".to_owned(), |offset| Px(offset).to_css())
    }
}

/// A color that may be `currentcolor`, stored as `None`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct OptionalColor(pub Option<Color>);

impl ToCss for OptionalColor {
    fn to_css(&self) -> String {
        self.0
            .map_or_else(|| "currentcolor".to_owned(), |color| color.to_css())
    }
}

/// `accent-color`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum AccentColor {
    /// The platform's accent.
    #[default]
    Auto,
    /// A color, which may be `currentcolor`.
    Color(OptionalColor),
}

impl AccentColor {
    /// The color, `currentcolor` being `current`; `None` for `auto`.
    pub fn resolve(self, current: Color) -> Option<Color> {
        match self {
            AccentColor::Auto => None,
            AccentColor::Color(color) => Some(color.0.unwrap_or(current)),
        }
    }
}

impl ToCss for AccentColor {
    fn to_css(&self) -> String {
        match self {
            AccentColor::Auto => "auto".to_owned(),
            AccentColor::Color(color) => color.to_css(),
        }
    }
}

/// A gradient color stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Stop color.
    pub color: Color,
    /// Position as a fraction of the gradient line, if given.
    pub position: Option<f32>,
}

/// The `background-image` property.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum BackgroundImage {
    /// `none`
    #[default]
    None,
    /// `url(...)`
    Url(String),
    /// `linear-gradient(...)`; the angle is in degrees, 180 pointing down.
    LinearGradient {
        /// Direction in degrees.
        angle: f32,
        /// Color stops.
        stops: Vec<ColorStop>,
    },
    /// `radial-gradient(...)`, painted from the center outwards.
    RadialGradient {
        /// Color stops.
        stops: Vec<ColorStop>,
    },
    /// Two images blended part of the way from one to the other, as a
    /// transition between them presents them: `cross-fade()`.
    CrossFade {
        /// The image faded out.
        from: Box<BackgroundImage>,
        /// The image faded in.
        to: Box<BackgroundImage>,
        /// How far the fade is, from 0 to 1.
        progress: f32,
    },
}

impl BackgroundImage {
    /// The URLs of the images it shows.
    pub fn urls(&self) -> Vec<&str> {
        match self {
            BackgroundImage::Url(url) => vec![url.as_str()],
            BackgroundImage::CrossFade { from, to, .. } => {
                let mut urls = from.urls();
                urls.extend(to.urls());
                urls
            }
            _ => Vec::new(),
        }
    }
}

impl ToCss for BackgroundImage {
    fn to_css(&self) -> String {
        let stops = |stops: &[ColorStop]| {
            stops
                .iter()
                .map(|stop| match stop.position {
                    Some(position) => {
                        format!(
                            "{} {}%",
                            stop.color.to_css(),
                            format_float(position * 100.0)
                        )
                    }
                    None => stop.color.to_css(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            BackgroundImage::None => "none".to_owned(),
            BackgroundImage::Url(url) => format!("url(\"{url}\")"),
            BackgroundImage::LinearGradient { angle, stops: list } => {
                format!(
                    "linear-gradient({}deg, {})",
                    format_float(*angle),
                    stops(list)
                )
            }
            BackgroundImage::RadialGradient { stops: list } => {
                format!("radial-gradient({})", stops(list))
            }
            BackgroundImage::CrossFade { from, to, progress } => format!(
                "cross-fade({} {}%, {})",
                from.to_css(),
                format_float((1.0 - progress) * 100.0),
                to.to_css()
            ),
        }
    }
}

/// How a background image repeats along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    /// `repeat`
    #[default]
    Repeat,
    /// `no-repeat`
    NoRepeat,
    /// `space`: whole tiles, spread out.
    Space,
    /// `round`: whole tiles, scaled to fit.
    Round,
}

impl Repeat {
    fn name(self) -> &'static str {
        match self {
            Repeat::Repeat => "repeat",
            Repeat::NoRepeat => "no-repeat",
            Repeat::Space => "space",
            Repeat::Round => "round",
        }
    }
}

/// The `background-repeat` property, across and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BackgroundRepeat {
    /// Across.
    pub x: Repeat,
    /// Down.
    pub y: Repeat,
}

impl ToCss for BackgroundRepeat {
    fn to_css(&self) -> String {
        match (self.x, self.y) {
            (Repeat::Repeat, Repeat::NoRepeat) => "repeat-x".to_owned(),
            (Repeat::NoRepeat, Repeat::Repeat) => "repeat-y".to_owned(),
            (x, y) if x == y => x.name().to_owned(),
            (x, y) => format!("{} {}", x.name(), y.name()),
        }
    }
}

/// The `background-size` property.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum BackgroundSize {
    /// `auto`: the image's own size.
    #[default]
    Auto,
    /// `cover`
    Cover,
    /// `contain`
    Contain,
    /// A width and a height, either of them `auto`.
    Explicit(LengthPercentageAuto, LengthPercentageAuto),
}

impl ToCss for BackgroundSize {
    fn to_css(&self) -> String {
        let side = |value: &LengthPercentageAuto| match value {
            LengthPercentageAuto::Auto => "auto".to_owned(),
            LengthPercentageAuto::Length(length) => length.to_css(),
        };
        match self {
            BackgroundSize::Auto => "auto".to_owned(),
            BackgroundSize::Cover => "cover".to_owned(),
            BackgroundSize::Contain => "contain".to_owned(),
            BackgroundSize::Explicit(width, LengthPercentageAuto::Auto) => side(width),
            BackgroundSize::Explicit(width, height) => format!("{} {}", side(width), side(height)),
        }
    }
}

keywords! {
    /// The box a `clip-path` shape is placed in.
    GeometryBox default BorderBox {
        BorderBox => "border-box",
        PaddingBox => "padding-box",
        ContentBox => "content-box",
        MarginBox => "margin-box",
        FillBox => "fill-box",
        StrokeBox => "stroke-box",
        ViewBox => "view-box",
    }
}

/// The radius of a `circle()` or `ellipse()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShapeRadius {
    /// A length, percentages of the box's width, height or, for circles,
    /// its normalized diagonal.
    Length(LengthPercentage),
    /// To the box's side nearest the center.
    ClosestSide,
    /// To the box's side furthest from the center.
    FarthestSide,
}

impl ToCss for ShapeRadius {
    fn to_css(&self) -> String {
        match self {
            ShapeRadius::Length(length) => length.to_css(),
            ShapeRadius::ClosestSide => "closest-side".to_owned(),
            ShapeRadius::FarthestSide => "farthest-side".to_owned(),
        }
    }
}

/// A basic shape of `clip-path`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BasicShape {
    /// `inset()`: offsets from the top, right, bottom and left edges, and
    /// the corner radii, top left first.
    Inset([LengthPercentage; 4], [Radius; 4]),
    /// `circle()`: its radius and center.
    Circle(ShapeRadius, BackgroundPosition),
    /// `ellipse()`: its radii and center.
    Ellipse(ShapeRadius, ShapeRadius, BackgroundPosition),
    /// `polygon()`: whether it fills by the even-odd rule, and its points.
    Polygon(bool, Vec<(LengthPercentage, LengthPercentage)>),
    /// `path()`: whether it fills by the even-odd rule, and its SVG path
    /// data, in pixels from the reference box's corner.
    Path(bool, String),
}

impl ToCss for BasicShape {
    fn to_css(&self) -> String {
        match self {
            BasicShape::Inset(offsets, radii) => {
                let offsets = offsets.iter().map(ToCss::to_css).collect::<Vec<_>>();
                let rounded = radii.iter().any(|radius| *radius != Radius::default());
                let radii = radii.iter().map(ToCss::to_css).collect::<Vec<_>>();
                if rounded {
                    format!("inset({} round {})", offsets.join(" "), radii.join(" "))
                } else {
                    format!("inset({})", offsets.join(" "))
                }
            }
            BasicShape::Circle(radius, at) => {
                format!("circle({} at {})", radius.to_css(), at.to_css())
            }
            BasicShape::Ellipse(x, y, at) => {
                format!("ellipse({} {} at {})", x.to_css(), y.to_css(), at.to_css())
            }
            BasicShape::Polygon(even_odd, points) => {
                let points = points
                    .iter()
                    .map(|(x, y)| format!("{} {}", x.to_css(), y.to_css()))
                    .collect::<Vec<_>>()
                    .join(", ");
                if *even_odd {
                    format!("polygon(evenodd, {points})")
                } else {
                    format!("polygon({points})")
                }
            }
            BasicShape::Path(even_odd, data) => {
                let data = data.replace('\\', "\\\\").replace('"', "\\\"");
                if *even_odd {
                    format!("path(evenodd, \"{data}\")")
                } else {
                    format!("path(\"{data}\")")
                }
            }
        }
    }
}

/// The `clip-path` property.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum ClipPath {
    /// No clipping.
    #[default]
    None,
    /// A basic shape in a box.
    Shape(BasicShape, GeometryBox),
    /// The shape of a box, with the element's rounded corners.
    Box(GeometryBox),
    /// A reference to an SVG `<clipPath>`.
    Url(String),
}

impl ToCss for ClipPath {
    fn to_css(&self) -> String {
        match self {
            ClipPath::None => "none".to_owned(),
            ClipPath::Shape(shape, GeometryBox::BorderBox) => shape.to_css(),
            ClipPath::Shape(shape, geometry) => format!("{} {}", shape.to_css(), geometry.to_css()),
            ClipPath::Box(geometry) => geometry.to_css(),
            ClipPath::Url(url) => format!("url(\"{url}\")"),
        }
    }
}

/// The `background-position` property: where a percentage puts that point
/// of the image at that point of the padding box, lengths from its corner.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BackgroundPosition {
    /// Across.
    pub x: LengthPercentage,
    /// Down.
    pub y: LengthPercentage,
}

impl ToCss for BackgroundPosition {
    fn to_css(&self) -> String {
        format!("{} {}", self.x.to_css(), self.y.to_css())
    }
}

/// A box shadow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    /// Horizontal offset in pixels.
    pub x: f32,
    /// Vertical offset in pixels.
    pub y: f32,
    /// Blur radius in pixels.
    pub blur: f32,
    /// Spread distance in pixels.
    pub spread: f32,
    /// Shadow color.
    pub color: Color,
    /// Whether the shadow is drawn inside the border.
    pub inset: bool,
}

/// The `box-shadow` property.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Shadows(pub Vec<Shadow>);

impl ToCss for Shadows {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        self.0
            .iter()
            .map(|shadow| {
                format!(
                    "{} {}px {}px {}px {}px{}",
                    shadow.color.to_css(),
                    format_float(shadow.x),
                    format_float(shadow.y),
                    format_float(shadow.blur),
                    format_float(shadow.spread),
                    if shadow.inset { " inset" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A CSS filter function; amounts are numbers where `1` means 100%.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Filter {
    /// `blur()`: the Gaussian standard deviation in pixels.
    Blur(f32),
    /// `brightness()`
    Brightness(f32),
    /// `contrast()`
    Contrast(f32),
    /// `grayscale()`, at most 1.
    Grayscale(f32),
    /// `hue-rotate()` in degrees.
    HueRotate(f32),
    /// `invert()`, at most 1.
    Invert(f32),
    /// `opacity()`, at most 1.
    Opacity(f32),
    /// `saturate()`
    Saturate(f32),
    /// `sepia()`, at most 1.
    Sepia(f32),
    /// `drop-shadow()`: a blurred, colored copy of the alpha behind the input.
    DropShadow {
        /// Horizontal offset in pixels.
        x: f32,
        /// Vertical offset in pixels.
        y: f32,
        /// Blur radius in pixels; the standard deviation is half of it.
        blur: f32,
        /// Shadow color.
        color: Color,
    },
}

impl Filter {
    /// The function with the same name that changes nothing, used to
    /// interpolate from a shorter list.
    pub fn identity(&self) -> Filter {
        match self {
            Filter::Blur(_) => Filter::Blur(0.0),
            Filter::Brightness(_) => Filter::Brightness(1.0),
            Filter::Contrast(_) => Filter::Contrast(1.0),
            Filter::Grayscale(_) => Filter::Grayscale(0.0),
            Filter::HueRotate(_) => Filter::HueRotate(0.0),
            Filter::Invert(_) => Filter::Invert(0.0),
            Filter::Opacity(_) => Filter::Opacity(1.0),
            Filter::Saturate(_) => Filter::Saturate(1.0),
            Filter::Sepia(_) => Filter::Sepia(0.0),
            Filter::DropShadow { .. } => Filter::DropShadow {
                x: 0.0,
                y: 0.0,
                blur: 0.0,
                color: Color::TRANSPARENT,
            },
        }
    }
}

impl ToCss for Filter {
    fn to_css(&self) -> String {
        let (name, amount) = match self {
            Filter::Blur(radius) => return format!("blur({}px)", format_float(*radius)),
            Filter::HueRotate(angle) => return format!("hue-rotate({}deg)", format_float(*angle)),
            Filter::DropShadow { x, y, blur, color } => {
                return format!(
                    "drop-shadow({} {}px {}px {}px)",
                    color.to_css(),
                    format_float(*x),
                    format_float(*y),
                    format_float(*blur)
                );
            }
            Filter::Brightness(amount) => ("brightness", amount),
            Filter::Contrast(amount) => ("contrast", amount),
            Filter::Grayscale(amount) => ("grayscale", amount),
            Filter::Invert(amount) => ("invert", amount),
            Filter::Opacity(amount) => ("opacity", amount),
            Filter::Saturate(amount) => ("saturate", amount),
            Filter::Sepia(amount) => ("sepia", amount),
        };
        format!("{name}({})", format_float(*amount))
    }
}

/// The `filter` and `backdrop-filter` properties: functions applied in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Filters(pub Vec<Filter>);

impl ToCss for Filters {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        self.0
            .iter()
            .map(ToCss::to_css)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The `transform-origin` offsets within a CSS reference box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformOrigin {
    /// Horizontal offset; percentages resolve against the reference-box width.
    pub x: LengthPercentage,
    /// Vertical offset; percentages resolve against the reference-box height.
    pub y: LengthPercentage,
    /// Depth offset. It cancels for the supported two-dimensional transforms.
    pub z: Px,
}

impl Default for TransformOrigin {
    fn default() -> Self {
        Self {
            x: LengthPercentage::Percent(0.5),
            y: LengthPercentage::Percent(0.5),
            z: Px(0.0),
        }
    }
}

impl ToCss for TransformOrigin {
    fn to_css(&self) -> String {
        let xy = format!("{} {}", self.x.to_css(), self.y.to_css());
        if self.z.0 == 0.0 {
            xy
        } else {
            format!("{xy} {}", self.z.to_css())
        }
    }
}

/// The `transform` property: its functions in order, empty for `none`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Transform(pub Vec<TransformFunction>);

/// The `translate`, `rotate` and `scale` properties: their one function,
/// empty for `none`. They apply before `transform`, in that order.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IndividualTransform(pub Transform);

impl ToCss for IndividualTransform {
    fn to_css(&self) -> String {
        let Some(function) = self.0.0.first() else {
            return "none".to_owned();
        };
        let zero = |length: &LengthPercentage| *length == LengthPercentage::Px(0.0);
        match function {
            TransformFunction::Translate(x, y) if zero(y) => x.to_css(),
            TransformFunction::Translate(x, y) => format!("{} {}", x.to_css(), y.to_css()),
            TransformFunction::Rotate(angle) => format!("{}deg", format_float(*angle)),
            TransformFunction::Rotate3d([x, y, z], angle) => {
                let axis = match [*x, *y, *z] {
                    [1.0, 0.0, 0.0] => "x".to_owned(),
                    [0.0, 1.0, 0.0] => "y".to_owned(),
                    [0.0, 0.0, 1.0] => "z".to_owned(),
                    _ => format!(
                        "{} {} {}",
                        format_float(*x),
                        format_float(*y),
                        format_float(*z)
                    ),
                };
                format!("{axis} {}deg", format_float(*angle))
            }
            TransformFunction::Scale(x, y) if x == y => format_float(*x),
            TransformFunction::Scale(x, y) => format!("{} {}", format_float(*x), format_float(*y)),
            other => other.to_css(),
        }
    }
}

/// A 2D affine matrix `[a, b, c, d, e, f]`, as CSS `matrix()` takes it: a
/// point `(x, y)` goes to `(a x + c y + e, b x + d y + f)`.
pub type Matrix = [f32; 6];

/// The matrix that changes nothing.
pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The product of two matrices: `inner` applies first.
pub fn multiply(outer: Matrix, inner: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = outer;
    let [g, h, i, j, k, l] = inner;
    [
        a * g + c * h,
        b * g + d * h,
        a * i + c * j,
        b * i + d * j,
        a * k + c * l + e,
        b * k + d * l + f,
    ]
}

/// One function of a `transform`, in two dimensions: three-dimensional
/// functions keep what they show without perspective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransformFunction {
    /// `translate()` and its one-axis and 3D forms.
    Translate(LengthPercentage, LengthPercentage),
    /// `scale()` and its one-axis and 3D forms.
    Scale(f32, f32),
    /// `rotate()`, in degrees.
    Rotate(f32),
    /// `skew()`, `skewX()` and `skewY()`, in degrees.
    Skew(f32, f32),
    /// `matrix()`.
    Matrix(Matrix),
    /// `rotateX()`, `rotateY()` and `rotate3d()`: an axis and an angle in
    /// degrees. Without perspective, the rotation shows as its projection.
    Rotate3d([f32; 3], f32),
    /// `matrix3d()`, in column-major order; it shows as its projection.
    Matrix3d([f32; 16]),
    /// Part of the way from one transform to another whose functions do
    /// not pair up: their matrices, once the box's size resolves them, are
    /// decomposed and interpolated.
    Mix(Box<Transform>, Box<Transform>, f32),
}

impl TransformFunction {
    /// The identity function of the same kind, which a transition from or
    /// to `none` interpolates with.
    pub fn identity(&self) -> TransformFunction {
        match self {
            TransformFunction::Translate(..) => {
                TransformFunction::Translate(LengthPercentage::Px(0.0), LengthPercentage::Px(0.0))
            }
            TransformFunction::Scale(..) => TransformFunction::Scale(1.0, 1.0),
            TransformFunction::Rotate(_) => TransformFunction::Rotate(0.0),
            TransformFunction::Skew(..) => TransformFunction::Skew(0.0, 0.0),
            TransformFunction::Rotate3d(axis, _) => TransformFunction::Rotate3d(*axis, 0.0),
            TransformFunction::Matrix(_)
            | TransformFunction::Matrix3d(_)
            | TransformFunction::Mix(..) => TransformFunction::Matrix(IDENTITY),
        }
    }

    /// The function's linear part in three dimensions, rows first, and its
    /// translation, for a border box of a size.
    fn affine_3d(&self, width: f32, height: f32) -> ([[f32; 3]; 3], [f32; 3]) {
        match self {
            TransformFunction::Rotate3d(axis, degrees) => (rotation_3d(*axis, *degrees), [0.0; 3]),
            TransformFunction::Matrix3d(m) => (
                [[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]],
                [m[12], m[13], m[14]],
            ),
            other => {
                let [a, b, c, d, e, f] = other.matrix(width, height);
                ([[a, c, 0.0], [b, d, 0.0], [0.0, 0.0, 1.0]], [e, f, 0.0])
            }
        }
    }

    /// The function's matrix for a border box of a size, against which
    /// percentages resolve.
    pub fn matrix(&self, width: f32, height: f32) -> Matrix {
        match self {
            TransformFunction::Translate(x, y) => {
                [1.0, 0.0, 0.0, 1.0, x.resolve(width), y.resolve(height)]
            }
            TransformFunction::Scale(x, y) => [*x, 0.0, 0.0, *y, 0.0, 0.0],
            TransformFunction::Rotate(degrees) => {
                let radians = degrees.to_radians();
                let (sin, cos) = (libm::sinf(radians), libm::cosf(radians));
                [cos, sin, -sin, cos, 0.0, 0.0]
            }
            TransformFunction::Skew(x, y) => [
                1.0,
                libm::tanf(y.to_radians()),
                libm::tanf(x.to_radians()),
                1.0,
                0.0,
                0.0,
            ],
            TransformFunction::Matrix(matrix) => *matrix,
            TransformFunction::Rotate3d(..) | TransformFunction::Matrix3d(_) => {
                let ([[a, c, _], [b, d, _], _], [e, f, _]) = self.affine_3d(width, height);
                [a, b, c, d, e, f]
            }
            TransformFunction::Mix(from, to, progress) => interpolate_matrices(
                from.matrix(width, height),
                to.matrix(width, height),
                *progress,
            ),
        }
    }
}

impl Transform {
    /// Whether the transform is `none`.
    pub fn is_identity(&self) -> bool {
        self.0.is_empty()
    }

    /// The transform's matrix for a border box of a size, against which
    /// percentages resolve; `transform-origin` is left out.
    pub fn matrix(&self, width: f32, height: f32) -> Matrix {
        self.0.iter().fold(IDENTITY, |matrix, function| {
            multiply(matrix, function.matrix(width, height))
        })
    }

    /// Whether the transform turns the element's back to the viewer, as a
    /// half turn about a vertical or horizontal axis does: `backface-visibility:
    /// hidden` then hides it. Flips in the plane, such as `scaleX(-1)`, do not.
    pub fn faces_back(&self, width: f32, height: f32) -> bool {
        let mut linear = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for function in &self.0 {
            let (inner, _) = function.affine_3d(width, height);
            linear = core::array::from_fn(|row| {
                core::array::from_fn(|column| {
                    (0..3).map(|k| linear[row][k] * inner[k][column]).sum()
                })
            });
        }
        let [[a, b, c], [d, e, f], [g, h, i]] = linear;
        let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        // The inverse's depth scale, which is negative facing away.
        determinant != 0.0 && (a * e - b * d) / determinant < 0.0
    }

    /// Whether the transform's matrix is the same for every box size.
    fn is_absolute(&self) -> bool {
        self.0.iter().all(|function| match function {
            TransformFunction::Translate(x, y) => {
                matches!(x, LengthPercentage::Px(_)) && matches!(y, LengthPercentage::Px(_))
            }
            TransformFunction::Mix(from, to, _) => from.is_absolute() && to.is_absolute(),
            _ => true,
        })
    }
}

/// The rotation matrix about an axis, rows first, as CSS `rotate3d()`
/// defines it.
fn rotation_3d(axis: [f32; 3], degrees: f32) -> [[f32; 3]; 3] {
    let length = libm::sqrtf(axis.iter().map(|value| value * value).sum());
    if length == 0.0 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let [x, y, z] = axis.map(|value| value / length);
    let half = degrees.to_radians() / 2.0;
    let (sin, cos) = (libm::sinf(half), libm::cosf(half));
    let (squared, both) = (sin * sin, sin * cos);
    [
        [
            1.0 - 2.0 * (y * y + z * z) * squared,
            2.0 * (x * y * squared - z * both),
            2.0 * (x * z * squared + y * both),
        ],
        [
            2.0 * (x * y * squared + z * both),
            1.0 - 2.0 * (x * x + z * z) * squared,
            2.0 * (y * z * squared - x * both),
        ],
        [
            2.0 * (x * z * squared - y * both),
            2.0 * (y * z * squared + x * both),
            1.0 - 2.0 * (x * x + y * y) * squared,
        ],
    ]
}

/// A matrix decomposed as CSS interpolates it: a translation, a rotation in
/// degrees, the rest of the linear part and a scale.
struct Decomposed {
    translate: [f32; 2],
    scale: [f32; 2],
    angle: f32,
    rest: [f32; 4],
}

fn decompose(matrix: Matrix) -> Decomposed {
    let [a, b, c, d, e, f] = matrix;
    let (mut row0, mut row1) = ([a, b], [c, d]);
    let mut scale = [libm::sqrtf(a * a + b * b), libm::sqrtf(c * c + d * d)];
    // A negative determinant flips one axis.
    if a * d - b * c < 0.0 {
        if a < d {
            scale[0] = -scale[0];
        } else {
            scale[1] = -scale[1];
        }
    }
    if scale[0] != 0.0 {
        row0 = [row0[0] / scale[0], row0[1] / scale[0]];
    }
    if scale[1] != 0.0 {
        row1 = [row1[0] / scale[1], row1[1] / scale[1]];
    }
    let angle = libm::atan2f(row0[1], row0[0]);
    if angle != 0.0 {
        let (sin, cos) = (-row0[1], row0[0]);
        let [m11, m12] = row0;
        let [m21, m22] = row1;
        row0 = [cos * m11 + sin * m21, cos * m12 + sin * m22];
        row1 = [-sin * m11 + cos * m21, -sin * m12 + cos * m22];
    }
    Decomposed {
        translate: [e, f],
        scale,
        angle: angle.to_degrees(),
        rest: [row0[0], row0[1], row1[0], row1[1]],
    }
}

/// Interpolates two matrices by their decompositions, as CSS transitions
/// between transforms whose functions do not pair up.
pub fn interpolate_matrices(from: Matrix, to: Matrix, progress: f32) -> Matrix {
    let (mut from, mut to) = (decompose(from), decompose(to));
    // One axis flipped in each is a rotation instead.
    if (from.scale[0] < 0.0 && to.scale[1] < 0.0) || (from.scale[1] < 0.0 && to.scale[0] < 0.0) {
        from.scale = [-from.scale[0], -from.scale[1]];
        from.angle += if from.angle < 0.0 { 180.0 } else { -180.0 };
    }
    // The short way around.
    if from.angle == 0.0 {
        from.angle = 360.0;
    }
    if to.angle == 0.0 {
        to.angle = 360.0;
    }
    if libm::fabsf(from.angle - to.angle) > 180.0 {
        if from.angle > to.angle {
            from.angle -= 360.0;
        } else {
            to.angle -= 360.0;
        }
    }
    let mix = |a: f32, b: f32| a + (b - a) * progress;
    let rest = [
        mix(from.rest[0], to.rest[0]),
        mix(from.rest[1], to.rest[1]),
        mix(from.rest[2], to.rest[2]),
        mix(from.rest[3], to.rest[3]),
    ];
    let radians = mix(from.angle, to.angle).to_radians();
    let (sin, cos) = (libm::sinf(radians), libm::cosf(radians));
    let linear = multiply(
        multiply(
            [cos, sin, -sin, cos, 0.0, 0.0],
            [rest[0], rest[1], rest[2], rest[3], 0.0, 0.0],
        ),
        [
            mix(from.scale[0], to.scale[0]),
            0.0,
            0.0,
            mix(from.scale[1], to.scale[1]),
            0.0,
            0.0,
        ],
    );
    [
        linear[0],
        linear[1],
        linear[2],
        linear[3],
        mix(from.translate[0], to.translate[0]),
        mix(from.translate[1], to.translate[1]),
    ]
}

/// A number as browsers serialize matrices: at most six significant
/// digits.
fn significant(value: f32) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_owned();
    }
    let digits = libm::floorf(libm::log10f(libm::fabsf(value))) as i32 + 1;
    let decimals = (6 - digits).clamp(0, 6) as usize;
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text.as_str()
    };
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

impl ToCss for TransformFunction {
    fn to_css(&self) -> String {
        match self {
            TransformFunction::Translate(x, y) => {
                format!("translate({}, {})", x.to_css(), y.to_css())
            }
            TransformFunction::Scale(x, y) => {
                format!("scale({}, {})", format_float(*x), format_float(*y))
            }
            TransformFunction::Rotate(degrees) => format!("rotate({}deg)", format_float(*degrees)),
            TransformFunction::Skew(x, y) => {
                format!("skew({}deg, {}deg)", format_float(*x), format_float(*y))
            }
            TransformFunction::Matrix(matrix) => {
                format!("matrix({})", matrix.map(significant).join(", "))
            }
            TransformFunction::Rotate3d([x, y, z], degrees) => format!(
                "rotate3d({}, {}, {}, {}deg)",
                format_float(*x),
                format_float(*y),
                format_float(*z),
                format_float(*degrees)
            ),
            TransformFunction::Matrix3d(matrix) => {
                format!("matrix3d({})", matrix.map(significant).join(", "))
            }
            TransformFunction::Mix(from, to, progress) => {
                format!(
                    "interpolate({}, {}, {})",
                    from.to_css(),
                    to.to_css(),
                    format_float(*progress)
                )
            }
        }
    }
}

impl ToCss for Transform {
    /// `none`, or the matrix browsers report; transforms with percentages
    /// list their functions, as their matrix needs the box's size.
    fn to_css(&self) -> String {
        if self.is_identity() {
            return "none".to_owned();
        }
        if self.is_absolute() {
            return TransformFunction::Matrix(self.matrix(0.0, 0.0)).to_css();
        }
        self.0
            .iter()
            .map(ToCss::to_css)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The `aspect-ratio` property; `None` is `auto`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AspectRatio(pub Option<f32>);

impl ToCss for AspectRatio {
    fn to_css(&self) -> String {
        self.0.map_or_else(
            || "auto".to_owned(),
            |ratio| format!("{} / 1", format_float(ratio)),
        )
    }
}

/// A piece of generated content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentItem {
    /// Literal text.
    Text(String),
    /// The value of an attribute.
    Attr(String),
    /// `counter(name, style)`: the innermost value of a counter.
    Counter(String, ListStyleType),
    /// `counters(name, separator, style)`: every value of a counter, from
    /// the outermost, joined by the separator.
    Counters(String, String, ListStyleType),
}

/// The `content` property.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Content {
    /// `normal`
    #[default]
    Normal,
    /// `none`
    None,
    /// Generated content.
    Items(Vec<ContentItem>),
}

impl ToCss for Content {
    fn to_css(&self) -> String {
        match self {
            Content::Normal => "normal".to_owned(),
            Content::None => "none".to_owned(),
            Content::Items(items) => items
                .iter()
                .map(|item| match item {
                    ContentItem::Text(text) => format!("\"{text}\""),
                    ContentItem::Attr(name) => format!("attr({name})"),
                    ContentItem::Counter(name, ListStyleType::Decimal) => {
                        format!("counter({name})")
                    }
                    ContentItem::Counter(name, kind) => {
                        format!("counter({name}, {})", kind.keyword())
                    }
                    ContentItem::Counters(name, separator, ListStyleType::Decimal) => {
                        format!("counters({name}, \"{separator}\")")
                    }
                    ContentItem::Counters(name, separator, kind) => {
                        format!("counters({name}, \"{separator}\", {})", kind.keyword())
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// The `container-name` property: the names container queries may give
/// to choose this element as their query container.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContainerName(pub Vec<String>);

impl ToCss for ContainerName {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        self.0.join(" ")
    }
}

/// `counter-reset`, `counter-increment` or `counter-set`: counter names, each
/// with its value or step.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CounterChanges(pub Vec<(String, i32)>);

impl ToCss for CounterChanges {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        self.0
            .iter()
            .map(|(name, value)| format!("{name} {value}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A grid track size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrackSize {
    /// A length or percentage.
    Length(LengthPercentage),
    /// A flexible fraction.
    Fr(f32),
    /// `auto`
    Auto,
    /// `min-content`
    MinContent,
    /// `max-content`
    MaxContent,
    /// `minmax(min, max)`
    MinMax(Box<TrackSize>, Box<TrackSize>),
    /// `fit-content(limit)`
    FitContent(LengthPercentage),
}

impl ToCss for TrackSize {
    fn to_css(&self) -> String {
        match self {
            TrackSize::Length(length) => length.to_css(),
            TrackSize::Fr(fraction) => format!("{}fr", format_float(*fraction)),
            TrackSize::Auto => "auto".to_owned(),
            TrackSize::MinContent => "min-content".to_owned(),
            TrackSize::MaxContent => "max-content".to_owned(),
            TrackSize::MinMax(minimum, maximum) => {
                format!("minmax({}, {})", minimum.to_css(), maximum.to_css())
            }
            TrackSize::FitContent(limit) => format!("fit-content({})", limit.to_css()),
        }
    }
}

/// How often a `repeat()` repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatCount {
    /// A fixed count.
    Count(u16),
    /// `auto-fill`
    AutoFill,
    /// `auto-fit`
    AutoFit,
}

/// An entry of a track list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrackEntry {
    /// A single track.
    Single(TrackSize),
    /// `repeat(count, tracks)`
    Repeat(RepeatCount, Vec<TrackSize>),
}

/// `grid-template-columns` and `grid-template-rows`; empty is `none`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackList(pub Vec<TrackEntry>);

impl ToCss for TrackList {
    fn to_css(&self) -> String {
        if self.0.is_empty() {
            return "none".to_owned();
        }
        let list = |tracks: &[TrackSize]| {
            tracks
                .iter()
                .map(ToCss::to_css)
                .collect::<Vec<_>>()
                .join(" ")
        };
        self.0
            .iter()
            .map(|entry| match entry {
                TrackEntry::Single(track) => track.to_css(),
                TrackEntry::Repeat(count, tracks) => {
                    let count = match count {
                        RepeatCount::Count(count) => count.to_string(),
                        RepeatCount::AutoFill => "auto-fill".to_owned(),
                        RepeatCount::AutoFit => "auto-fit".to_owned(),
                    };
                    format!("repeat({count}, {})", list(tracks))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `grid-auto-columns` and `grid-auto-rows`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSizes(pub Vec<TrackSize>);

impl Default for TrackSizes {
    fn default() -> Self {
        TrackSizes(vec![TrackSize::Auto])
    }
}

impl ToCss for TrackSizes {
    fn to_css(&self) -> String {
        self.0
            .iter()
            .map(ToCss::to_css)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A grid placement line.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GridLine {
    /// `auto`
    #[default]
    Auto,
    /// A line number; negative numbers count from the end.
    Line(i16),
    /// `span n`
    Span(u16),
    /// A named line, or the start or end line of a named area, as the
    /// side the line is placed on asks.
    Named(String),
}

impl ToCss for GridLine {
    fn to_css(&self) -> String {
        match self {
            GridLine::Auto => "auto".to_owned(),
            GridLine::Line(line) => line.to_string(),
            GridLine::Span(span) => format!("span {span}"),
            GridLine::Named(name) => name.clone(),
        }
    }
}

/// A named area of `grid-template-areas`, between 1-based grid lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridArea {
    /// The area's name.
    pub name: String,
    /// The row line it starts at.
    pub row_start: u16,
    /// The row line it ends at.
    pub row_end: u16,
    /// The column line it starts at.
    pub column_start: u16,
    /// The column line it ends at.
    pub column_end: u16,
}

/// The `grid-template-areas` property: its named areas and the size of its
/// template in tracks; no rows for `none`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridTemplateAreas {
    /// The named areas.
    pub areas: Vec<GridArea>,
    /// How many rows the template has.
    pub rows: u16,
    /// How many columns the template has.
    pub columns: u16,
}

impl ToCss for GridTemplateAreas {
    fn to_css(&self) -> String {
        if self.rows == 0 {
            return "none".to_owned();
        }
        let mut cells = vec![vec!["."; usize::from(self.columns)]; usize::from(self.rows)];
        for area in &self.areas {
            for row in area.row_start..area.row_end {
                for column in area.column_start..area.column_end {
                    cells[usize::from(row - 1)][usize::from(column - 1)] = area.name.as_str();
                }
            }
        }
        cells
            .iter()
            .map(|row| format!("\"{}\"", row.join(" ")))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The `grid-auto-flow` property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridAutoFlow {
    /// Whether items flow by column.
    pub column: bool,
    /// Whether the dense packing algorithm is used.
    pub dense: bool,
}

impl ToCss for GridAutoFlow {
    fn to_css(&self) -> String {
        let direction = if self.column { "column" } else { "row" };
        if self.dense {
            format!("{direction} dense")
        } else {
            direction.to_owned()
        }
    }
}

#[cfg(test)]
mod tests;

/// A transition easing curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    /// Cubic Bézier control points between (0, 0) and (1, 1).
    CubicBezier(f32, f32, f32, f32),
    /// Equal steps with a specified jump position.
    Steps(u32, StepPosition),
}

/// Where a step easing function jumps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StepPosition {
    /// Jump at the beginning of every step.
    Start,
    /// Jump at the end of every step.
    End,
    /// No jumps at either endpoint.
    None,
    /// Jumps at both endpoints.
    Both,
}

impl Easing {
    /// Linear interpolation.
    pub const LINEAR: Self = Self::CubicBezier(0.0, 0.0, 1.0, 1.0);
    /// The CSS default easing.
    pub const EASE: Self = Self::CubicBezier(0.25, 0.1, 0.25, 1.0);
    /// Accelerate.
    pub const EASE_IN: Self = Self::CubicBezier(0.42, 0.0, 1.0, 1.0);
    /// Decelerate.
    pub const EASE_OUT: Self = Self::CubicBezier(0.0, 0.0, 0.58, 1.0);
    /// Accelerate and then decelerate.
    pub const EASE_IN_OUT: Self = Self::CubicBezier(0.42, 0.0, 0.58, 1.0);

    /// Convert elapsed progress to eased progress; curves may overshoot.
    pub fn apply(self, progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        match self {
            Self::CubicBezier(x1, y1, x2, y2) => {
                if progress == 0.0 || progress == 1.0 {
                    return progress;
                }
                let curve = |a: f32, b: f32, t: f32| {
                    3.0 * a * t * crate::math::pow(1.0 - t, 2.0)
                        + 3.0 * b * t * t * (1.0 - t)
                        + t * t * t
                };
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..32 {
                    let middle = (low + high) / 2.0;
                    if curve(x1, x2, middle) < progress {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                curve(y1, y2, (low + high) / 2.0)
            }
            Self::Steps(count, position) => {
                let count = count.max(if position == StepPosition::None { 2 } else { 1 });
                let (offset, jumps) = match position {
                    StepPosition::Start => (1.0, count),
                    StepPosition::End => (0.0, count),
                    StepPosition::None => (0.0, count - 1),
                    StepPosition::Both => (1.0, count + 1),
                };
                (crate::math::floor(progress * count as f32) + offset).min(jumps as f32)
                    / jumps as f32
            }
        }
    }
}

impl ToCss for Easing {
    fn to_css(&self) -> String {
        match *self {
            Self::CubicBezier(x1, y1, x2, y2) => {
                let named = [
                    (Self::LINEAR, "linear"),
                    (Self::EASE, "ease"),
                    (Self::EASE_IN, "ease-in"),
                    (Self::EASE_OUT, "ease-out"),
                    (Self::EASE_IN_OUT, "ease-in-out"),
                ];
                match named.iter().find(|(curve, _)| curve == self) {
                    Some((_, name)) => (*name).into(),
                    None => format!(
                        "cubic-bezier({}, {}, {}, {})",
                        format_float(x1),
                        format_float(y1),
                        format_float(x2),
                        format_float(y2)
                    ),
                }
            }
            Self::Steps(count, position) => {
                let position = match position {
                    StepPosition::Start => "start",
                    StepPosition::End => "end",
                    StepPosition::None => "jump-none",
                    StepPosition::Both => "jump-both",
                };
                format!("steps({count}, {position})")
            }
        }
    }
}

/// A CSS duration or delay in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Seconds(pub f32);

impl ToCss for Seconds {
    fn to_css(&self) -> String {
        format!("{}s", format_float(self.0))
    }
}

/// A property name as written in a transition list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ident(pub String);

impl ToCss for Ident {
    fn to_css(&self) -> String {
        self.0.clone()
    }
}

/// A comma-separated CSS list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct List<T>(pub Vec<T>);

impl<T: ToCss> ToCss for List<T> {
    fn to_css(&self) -> String {
        self.0
            .iter()
            .map(ToCss::to_css)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl<T> List<T> {
    /// Lists repeat when they are shorter than the property list.
    pub fn at(&self, index: usize) -> Option<&T> {
        self.0.get(index % self.0.len().max(1))
    }
}
