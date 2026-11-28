//! Computed style and the longhand property table.

use crate::prelude::*;

use crate::color::{Color, SystemColors};
use crate::media::MediaEnvironment;
use crate::parsers::{self, Context};
use crate::types::*;
use crate::values::{ComponentValue, LengthContext, LengthPercentage};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;

/// The environment style computation depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleEnvironment {
    /// Viewport and user preferences.
    pub media: MediaEnvironment,
    /// The root element's computed font size, for `rem`.
    pub root_font_size: f32,
    /// Theme colors.
    pub system: SystemColors,
    /// The family `font-family` starts as: the standard font of the user's
    /// locale, as Chromium's font settings give it.
    pub standard_font: &'static str,
}

impl Default for StyleEnvironment {
    fn default() -> Self {
        StyleEnvironment {
            media: MediaEnvironment::default(),
            root_font_size: 16.0,
            system: SystemColors::LIGHT,
            standard_font: "Times New Roman",
        }
    }
}

/// A cascaded value for one longhand.
#[derive(Debug, Clone, PartialEq)]
pub enum Specified {
    /// A declared value, with `var()` already substituted.
    Value(Vec<ComponentValue>),
    /// `inherit`
    Inherit,
    /// `initial`
    Initial,
    /// `unset`, `revert` and invalid values after substitution.
    Unset,
}

/// The cascaded values of an element.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpecifiedValues {
    /// Winning value per longhand.
    pub values: BTreeMap<PropertyId, Specified>,
    /// Custom properties, including inherited ones.
    pub custom: Rc<BTreeMap<String, String>>,
    /// Whether author CSS styles the background or border, or sets
    /// `appearance: none`: form controls then lose their native look.
    pub author_appearance: bool,
    /// Whether author CSS sets `appearance: none`, which also takes away
    /// the room the native look leaves inside form controls.
    pub appearance_none: bool,
}

/// Defines the longhand properties: name, inheritance, parser and field.
macro_rules! longhands {
    ($($variant:ident => $name:literal, $inherited:literal, $parser:expr, $($field:ident).+;)+) => {
        /// A longhand property the native target computes.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum PropertyId {
            $(
                #[doc = concat!("`", $name, "`")]
                $variant,
            )+
        }

        impl PropertyId {
            /// Every longhand.
            pub const ALL: &'static [PropertyId] = &[$(PropertyId::$variant),+];

            /// Looks up a longhand by its lowercase name.
            pub fn from_name(name: &str) -> Option<PropertyId> {
                match name {
                    $($name => Some(PropertyId::$variant),)+
                    _ => None,
                }
            }

            /// The property name.
            pub fn name(self) -> &'static str {
                match self {
                    $(PropertyId::$variant => $name,)+
                }
            }

            /// Whether the property inherits by default.
            pub fn inherited(self) -> bool {
                match self {
                    $(PropertyId::$variant => $inherited,)+
                }
            }
        }

        impl ComputedStyle {
            /// Parses and stores a value; returns false if it is invalid.
            pub fn apply(&mut self, id: PropertyId, value: &[ComponentValue], context: &Context) -> bool {
                match id {
                    $(PropertyId::$variant => match ($parser)(value, context) {
                        Some(parsed) => {
                            self.$($field).+ = parsed;
                            true
                        }
                        None => false,
                    },)+
                }
            }

            /// Copies one property from another style.
            pub fn copy_property(&mut self, id: PropertyId, from: &ComputedStyle) {
                match id {
                    $(PropertyId::$variant => self.$($field).+ = from.$($field).+.clone(),)+
                }
            }

            /// Whether another style has the same value of one property.
            pub fn same_property(&self, id: PropertyId, other: &ComputedStyle) -> bool {
                match id {
                    $(PropertyId::$variant => self.$($field).+ == other.$($field).+,)+
                }
            }

            /// Serializes one longhand.
            pub fn value_of(&self, id: PropertyId) -> String {
                match id {
                    $(PropertyId::$variant => ToCss::to_css(&self.$($field).+),)+
                }
            }
        }
    };
}

/// The box layout gave an element, which `getComputedStyle` resolves
/// sizes, margins and paddings against.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UsedBox {
    /// The border box's width and height.
    pub size: (f32, f32),
    /// Border widths: top, right, bottom, left.
    pub border: [f32; 4],
    /// Padding widths: top, right, bottom, left.
    pub padding: [f32; 4],
    /// Used margins: top, right, bottom, left.
    pub margin: [f32; 4],
    /// The room scrollbars take: the vertical one's width, then the
    /// horizontal one's height.
    pub scrollbars: [f32; 2],
    /// Whether `width` and `height` apply, as they do to all boxes but
    /// non-replaced inline ones.
    pub sized: bool,
}

/// Computed values of all supported properties.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    /// `display`
    pub display: Display,
    /// `position`
    pub position: PositionKind,
    /// `box-sizing`
    pub box_sizing: BoxSizing,
    /// `float`
    pub float: Float,
    /// `width`
    pub width: Size,
    /// `height`
    pub height: Size,
    /// `min-width`
    pub min_width: Size,
    /// `min-height`
    pub min_height: Size,
    /// `max-width`
    pub max_width: MaxSize,
    /// `max-height`
    pub max_height: MaxSize,
    /// `margin-*`
    pub margin: Edges<LengthPercentageAuto>,
    /// `padding-*`
    pub padding: Edges<LengthPercentage>,
    /// `scroll-margin`: room kept around the box when it is scrolled into
    /// view.
    pub scroll_margin: Edges<Px>,
    /// `scroll-padding`: room a scroll container keeps around what it
    /// scrolls into view.
    pub scroll_padding: Edges<LengthPercentageAuto>,
    /// `clip` of absolutely positioned boxes.
    pub clip: Option<ClipRect>,
    /// `border-*-width`, zero when the style is `none`
    pub border_width: Edges<Px>,
    /// `border-*-style`
    pub border_style: Edges<BorderStyle>,
    /// `border-*-color`
    pub border_color: Edges<Color>,
    /// `border-*-radius`
    pub border_radius: Corners<Radius>,
    /// `top`, `right`, `bottom`, `left`
    pub inset: Edges<LengthPercentageAuto>,
    /// `z-index`
    pub z_index: ZIndex,
    /// `isolation`
    pub isolation: Isolation,
    /// `backface-visibility`
    pub backface_visibility: BackfaceVisibility,
    /// `flex-direction`
    pub flex_direction: FlexDirection,
    /// `flex-wrap`
    pub flex_wrap: FlexWrap,
    /// `flex-grow`
    pub flex_grow: f32,
    /// `flex-shrink`
    pub flex_shrink: f32,
    /// `flex-basis`
    pub flex_basis: Size,
    /// `order`
    pub order: i32,
    /// `justify-content`
    pub justify_content: Alignment,
    /// `align-content`
    pub align_content: Alignment,
    /// `align-items`
    pub align_items: Alignment,
    /// `align-self`
    pub align_self: Alignment,
    /// `justify-items`
    pub justify_items: Alignment,
    /// `justify-self`
    pub justify_self: Alignment,
    /// `row-gap`
    pub row_gap: LengthPercentage,
    /// `column-gap`
    pub column_gap: LengthPercentage,
    /// `grid-template-columns`
    pub grid_template_columns: TrackList,
    /// `grid-template-rows`
    pub grid_template_rows: TrackList,
    /// `grid-auto-columns`
    pub grid_auto_columns: TrackSizes,
    /// `grid-auto-rows`
    pub grid_auto_rows: TrackSizes,
    /// `grid-auto-flow`
    pub grid_auto_flow: GridAutoFlow,
    /// `grid-template-areas`
    pub grid_template_areas: GridTemplateAreas,
    /// `grid-column-start`
    pub grid_column_start: GridLine,
    /// `grid-column-end`
    pub grid_column_end: GridLine,
    /// `grid-row-start`
    pub grid_row_start: GridLine,
    /// `grid-row-end`
    pub grid_row_end: GridLine,
    /// `color`
    pub color: Color,
    /// `background-color`
    pub background_color: Color,
    /// `background-image`
    pub background_image: BackgroundImage,
    /// How the background image repeats.
    pub background_repeat: BackgroundRepeat,
    /// The background image's size.
    pub background_size: BackgroundSize,
    /// Where the background image sits.
    pub background_position: BackgroundPosition,
    /// `mask-image`: the image whose alpha masks the element.
    pub mask_image: BackgroundImage,
    /// How the mask image repeats.
    pub mask_repeat: BackgroundRepeat,
    /// The mask image's size.
    pub mask_size: BackgroundSize,
    /// Where the mask image sits.
    pub mask_position: BackgroundPosition,
    /// `opacity`
    pub opacity: f32,
    /// `visibility`
    pub visibility: Visibility,
    /// `overflow-x`
    pub overflow_x: Overflow,
    /// `overflow-y`
    pub overflow_y: Overflow,
    /// `scrollbar-width`
    pub scrollbar_width: ScrollbarWidth,
    /// `scrollbar-color`
    pub scrollbar_color: ScrollbarColor,
    /// `cursor`
    pub cursor: Cursor,
    /// `pointer-events`
    pub pointer_events: PointerEvents,
    /// `user-select`
    pub user_select: UserSelect,
    /// `-webkit-user-drag`
    pub user_drag: UserDrag,
    /// `resize`
    pub resize: Resize,
    /// `table-layout`
    pub table_layout: TableLayout,
    /// `caption-side`
    pub caption_side: CaptionSide,
    /// `app-region`
    pub app_region: AppRegion,
    /// `font-family`
    pub font_family: FontFamily,
    /// `font-size`
    pub font_size: Px,
    /// `font-weight`
    pub font_weight: FontWeight,
    /// `font-style`
    pub font_style: FontStyle,
    /// `line-height`
    pub line_height: LineHeight,
    /// `text-align`
    pub text_align: TextAlign,
    /// `text-decoration-line`
    pub text_decoration_line: TextDecorationLine,
    /// `text-decoration-color`
    pub text_decoration_color: OptionalColor,
    /// `text-transform`
    pub text_transform: TextTransform,
    /// `white-space-collapse` and `text-wrap-mode`
    pub white_space: WhiteSpace,
    /// `text-wrap-style`
    pub text_wrap_style: TextWrapStyle,
    /// `text-overflow`
    pub text_overflow: TextOverflow,
    /// How a `-webkit-box` lays out its children.
    pub box_orient: BoxOrient,
    /// How many lines a vertical `-webkit-box` shows.
    pub line_clamp: LineClamp,
    /// `word-break`
    pub word_break: WordBreak,
    /// `overflow-wrap`
    pub overflow_wrap: OverflowWrap,
    /// `letter-spacing`
    pub letter_spacing: Px,
    /// `tab-size`
    pub tab_size: TabSize,
    /// `text-indent`
    pub text_indent: LengthPercentage,
    /// `vertical-align`
    pub vertical_align: VerticalAlign,
    /// `list-style-type`
    pub list_style_type: ListStyleType,
    /// `list-style-position`
    pub list_style_position: ListStylePosition,
    /// `border-collapse`
    pub border_collapse: BorderCollapse,
    /// `border-spacing`
    pub border_spacing: Spacing,
    /// `content`
    pub content: Content,
    /// `counter-reset`
    pub counter_reset: CounterChanges,
    /// `counter-increment`
    pub counter_increment: CounterChanges,
    /// `counter-set`
    pub counter_set: CounterChanges,
    /// `aspect-ratio`
    pub aspect_ratio: AspectRatio,
    /// `object-fit`
    pub object_fit: ObjectFit,
    /// `object-position`
    pub object_position: BackgroundPosition,
    /// `box-shadow`
    pub box_shadow: Shadows,
    /// `outline-width`
    pub outline_width: Px,
    /// `outline-style`
    pub outline_style: BorderStyle,
    /// `outline-color`
    pub outline_color: OptionalColor,
    /// `outline-offset`
    pub outline_offset: Px,
    /// `caret-color`; `None` paints the caret in the text color.
    pub caret_color: OptionalColor,
    /// `accent-color`, the color checked checkboxes and radio buttons fill
    /// with.
    pub accent_color: AccentColor,
    /// `-webkit-text-fill-color`; `None` fills text in its `color`.
    pub text_fill_color: OptionalColor,
    /// `text-underline-offset`
    pub text_underline_offset: UnderlineOffset,
    /// `transform`
    pub transform: Transform,
    /// `translate`
    pub translate: IndividualTransform,
    /// `rotate`
    pub rotate: IndividualTransform,
    /// `scale`
    pub scale: IndividualTransform,
    /// `transform-origin`
    pub transform_origin: TransformOrigin,
    /// `filter`
    pub filter: Filters,
    /// `backdrop-filter`
    pub backdrop_filter: Filters,
    /// `clip-path`
    pub clip_path: ClipPath,
    /// `transition-property`
    pub transition_property: List<Ident>,
    /// `transition-duration`
    pub transition_duration: List<Seconds>,
    /// `transition-delay`
    pub transition_delay: List<Seconds>,
    /// `transition-timing-function`
    pub transition_timing_function: List<Easing>,
    /// `direction`
    pub direction: Direction,
    /// `unicode-bidi`
    pub unicode_bidi: UnicodeBidi,
    /// `font-variant-numeric`
    pub font_variant_numeric: FontVariantNumeric,
    /// `font-feature-settings`
    pub font_feature_settings: FontFeatureSettings,
    /// `font-kerning`
    pub font_kerning: FontKerning,
    /// `animation-name`
    pub animation_name: List<Ident>,
    /// `animation-duration`
    pub animation_duration: List<Seconds>,
    /// `animation-timing-function`
    pub animation_timing_function: List<Easing>,
    /// `animation-delay`
    pub animation_delay: List<Seconds>,
    /// `animation-iteration-count`
    pub animation_iteration_count: List<Iterations>,
    /// `animation-direction`
    pub animation_direction: List<AnimationDirection>,
    /// `animation-fill-mode`
    pub animation_fill_mode: List<FillMode>,
    /// `animation-play-state`
    pub animation_play_state: List<PlayState>,
    /// SVG `fill`
    pub fill: Paint,
    /// SVG `fill-opacity`
    pub fill_opacity: f32,
    /// SVG `fill-rule`
    pub fill_rule: FillRule,
    /// SVG `stroke`
    pub stroke: Paint,
    /// SVG `stroke-width` in user units
    pub stroke_width: Px,
    /// SVG `stroke-opacity`
    pub stroke_opacity: f32,
    /// SVG `stroke-linecap`
    pub stroke_linecap: LineCap,
    /// SVG `stroke-linejoin`
    pub stroke_linejoin: LineJoin,
    /// SVG `stroke-dasharray`
    pub stroke_dasharray: Dashes,
    /// SVG `stroke-dashoffset` in user units
    pub stroke_dashoffset: Px,
    /// SVG `marker-start`
    pub marker_start: MarkerUrl,
    /// SVG `marker-mid`
    pub marker_mid: MarkerUrl,
    /// SVG `marker-end`
    pub marker_end: MarkerUrl,
    /// SVG `text-anchor`
    pub text_anchor: TextAnchor,
    /// SVG `dominant-baseline`
    pub dominant_baseline: SvgBaseline,
    /// SVG `alignment-baseline`
    pub alignment_baseline: SvgBaseline,
    /// `container-type`
    pub container_type: ContainerType,
    /// `container-name`
    pub container_name: ContainerName,
    /// Custom properties.
    pub custom: Rc<BTreeMap<String, String>>,
    /// Whether author CSS styles the background or border, or sets
    /// `appearance: none`: form controls then lose their native look.
    pub author_appearance: bool,
    /// Whether author CSS sets `appearance: none`, which also takes away
    /// the room the native look leaves inside form controls.
    pub appearance_none: bool,
    /// `display` before absolute positioning, floating or a flex or grid
    /// container made the box block-level: where an absolutely positioned
    /// box that was inline sits when its insets are automatic.
    pub original_display: Display,
    /// The content box size the container query units of this element's
    /// descendants resolve against: its own when it is a query container,
    /// as last laid out, else its parent's.
    pub query_size: Option<(f32, f32)>,
    /// Whether a declaration took a property that is not inherited from the
    /// parent with `inherit`.
    pub inherits_explicitly: bool,
}

impl ComputedStyle {
    /// Whether children computed under another parent style would compute
    /// the same under this one, unless they `inherit` a property that is
    /// not inherited: the inherited properties, custom properties, query
    /// container sizes and what blockifies them are the same.
    pub fn inherits_same(&self, other: &ComputedStyle) -> bool {
        self.display == other.display
            && self.box_orient == other.box_orient
            && self.query_size == other.query_size
            && (Rc::ptr_eq(&self.custom, &other.custom) || self.custom == other.custom)
            && PropertyId::ALL
                .iter()
                .filter(|id| id.inherited())
                .all(|id| self.same_property(*id, other))
    }
}

impl Default for ComputedStyle {
    fn default() -> Self {
        let medium = Px(3.0);
        ComputedStyle {
            display: Display::Inline,
            position: PositionKind::Static,
            box_sizing: BoxSizing::ContentBox,
            float: Float::None,
            width: Size::Auto,
            height: Size::Auto,
            min_width: Size::Auto,
            min_height: Size::Auto,
            max_width: MaxSize::None,
            max_height: MaxSize::None,
            margin: Edges::all(LengthPercentageAuto::zero()),
            padding: Edges::all(LengthPercentage::Px(0.0)),
            scroll_margin: Edges::all(Px(0.0)),
            scroll_padding: Edges::all(LengthPercentageAuto::Auto),
            clip: None,
            border_width: Edges::all(medium),
            border_style: Edges::all(BorderStyle::None),
            border_color: Edges::all(Color::BLACK),
            border_radius: Corners::default(),
            inset: Edges::all(LengthPercentageAuto::Auto),
            z_index: ZIndex(None),
            isolation: Isolation::Auto,
            backface_visibility: BackfaceVisibility::Visible,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Size::Auto,
            order: 0,
            justify_content: Alignment::Normal,
            align_content: Alignment::Normal,
            align_items: Alignment::Normal,
            align_self: Alignment::Auto,
            justify_items: Alignment::Normal,
            justify_self: Alignment::Auto,
            row_gap: LengthPercentage::Px(0.0),
            column_gap: LengthPercentage::Px(0.0),
            grid_template_columns: TrackList::default(),
            grid_template_rows: TrackList::default(),
            grid_auto_columns: TrackSizes::default(),
            grid_auto_rows: TrackSizes::default(),
            grid_auto_flow: GridAutoFlow::default(),
            grid_template_areas: GridTemplateAreas::default(),
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            background_image: BackgroundImage::None,
            background_repeat: BackgroundRepeat {
                x: Repeat::Repeat,
                y: Repeat::Repeat,
            },
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition {
                x: LengthPercentage::Percent(0.0),
                y: LengthPercentage::Percent(0.0),
            },
            mask_image: BackgroundImage::None,
            mask_repeat: BackgroundRepeat {
                x: Repeat::Repeat,
                y: Repeat::Repeat,
            },
            mask_size: BackgroundSize::Auto,
            mask_position: BackgroundPosition {
                x: LengthPercentage::Percent(0.0),
                y: LengthPercentage::Percent(0.0),
            },
            opacity: 1.0,
            visibility: Visibility::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            scrollbar_width: ScrollbarWidth::Auto,
            scrollbar_color: ScrollbarColor(None),
            cursor: Cursor::Auto,
            pointer_events: PointerEvents::Auto,
            user_select: UserSelect::Auto,
            user_drag: UserDrag::Auto,
            resize: Resize::None,
            table_layout: TableLayout::Auto,
            caption_side: CaptionSide::Top,
            app_region: AppRegion::None,
            font_family: FontFamily::default(),
            font_size: Px(16.0),
            font_weight: FontWeight(400),
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Start,
            text_decoration_line: TextDecorationLine::default(),
            text_decoration_color: OptionalColor(None),
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            text_wrap_style: TextWrapStyle::Auto,
            text_overflow: TextOverflow::Clip,
            box_orient: BoxOrient::Horizontal,
            line_clamp: LineClamp(None),
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            letter_spacing: Px(0.0),
            tab_size: TabSize::Spaces(8.0),
            text_indent: LengthPercentage::Px(0.0),
            vertical_align: VerticalAlign::Baseline,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            border_collapse: BorderCollapse::Separate,
            border_spacing: Spacing::default(),
            content: Content::Normal,
            counter_reset: CounterChanges::default(),
            counter_increment: CounterChanges::default(),
            counter_set: CounterChanges::default(),
            aspect_ratio: AspectRatio(None),
            object_fit: ObjectFit::Fill,
            object_position: BackgroundPosition {
                x: LengthPercentage::Percent(0.5),
                y: LengthPercentage::Percent(0.5),
            },
            box_shadow: Shadows::default(),
            outline_width: medium,
            outline_style: BorderStyle::None,
            outline_color: OptionalColor(None),
            outline_offset: Px(0.0),
            caret_color: OptionalColor(None),
            accent_color: AccentColor::Auto,
            text_fill_color: OptionalColor(None),
            text_underline_offset: UnderlineOffset(None),
            transform: Transform::default(),
            translate: IndividualTransform::default(),
            rotate: IndividualTransform::default(),
            scale: IndividualTransform::default(),
            transform_origin: TransformOrigin::default(),
            filter: Filters::default(),
            backdrop_filter: Filters::default(),
            clip_path: ClipPath::None,
            transition_property: List(vec![Ident("all".into())]),
            transition_duration: List(vec![Seconds(0.0)]),
            transition_delay: List(vec![Seconds(0.0)]),
            transition_timing_function: List(vec![Easing::EASE]),
            direction: Direction::Ltr,
            unicode_bidi: UnicodeBidi::Normal,
            font_variant_numeric: FontVariantNumeric::default(),
            font_feature_settings: FontFeatureSettings::default(),
            font_kerning: FontKerning::Auto,
            animation_name: List(vec![Ident("none".into())]),
            animation_duration: List(vec![Seconds(0.0)]),
            animation_timing_function: List(vec![Easing::EASE]),
            animation_delay: List(vec![Seconds(0.0)]),
            animation_iteration_count: List(vec![Iterations::Count(1.0)]),
            animation_direction: List(vec![AnimationDirection::Normal]),
            animation_fill_mode: List(vec![FillMode::None]),
            animation_play_state: List(vec![PlayState::Running]),
            fill: Paint::Color(Color::BLACK),
            fill_opacity: 1.0,
            fill_rule: FillRule::NonZero,
            stroke: Paint::None,
            stroke_width: Px(1.0),
            stroke_opacity: 1.0,
            stroke_linecap: LineCap::Butt,
            stroke_linejoin: LineJoin::Miter,
            stroke_dasharray: Dashes::default(),
            stroke_dashoffset: Px(0.0),
            marker_start: MarkerUrl::default(),
            marker_mid: MarkerUrl::default(),
            marker_end: MarkerUrl::default(),
            text_anchor: TextAnchor::Start,
            dominant_baseline: SvgBaseline::Auto,
            alignment_baseline: SvgBaseline::Auto,
            container_type: ContainerType::Normal,
            container_name: ContainerName::default(),
            custom: Rc::default(),
            author_appearance: false,
            appearance_none: false,
            original_display: Display::Inline,
            query_size: None,
            inherits_explicitly: false,
        }
    }
}

longhands! {
    Display => "display", false, parsers::display, display;
    Position => "position", false, parsers::keyword::<PositionKind>, position;
    BoxSizing => "box-sizing", false, parsers::keyword::<BoxSizing>, box_sizing;
    Float => "float", false, parsers::keyword::<Float>, float;
    Width => "width", false, parsers::size, width;
    Height => "height", false, parsers::size, height;
    MinWidth => "min-width", false, parsers::size, min_width;
    MinHeight => "min-height", false, parsers::size, min_height;
    MaxWidth => "max-width", false, parsers::max_size, max_width;
    MaxHeight => "max-height", false, parsers::max_size, max_height;
    MarginTop => "margin-top", false, parsers::length_auto, margin.top;
    MarginRight => "margin-right", false, parsers::length_auto, margin.right;
    MarginBottom => "margin-bottom", false, parsers::length_auto, margin.bottom;
    MarginLeft => "margin-left", false, parsers::length_auto, margin.left;
    PaddingTop => "padding-top", false, parsers::padding, padding.top;
    PaddingRight => "padding-right", false, parsers::padding, padding.right;
    PaddingBottom => "padding-bottom", false, parsers::padding, padding.bottom;
    PaddingLeft => "padding-left", false, parsers::padding, padding.left;
    BorderTopWidth => "border-top-width", false, parsers::border_width, border_width.top;
    BorderRightWidth => "border-right-width", false, parsers::border_width, border_width.right;
    BorderBottomWidth => "border-bottom-width", false, parsers::border_width, border_width.bottom;
    BorderLeftWidth => "border-left-width", false, parsers::border_width, border_width.left;
    BorderTopStyle => "border-top-style", false, parsers::keyword::<BorderStyle>, border_style.top;
    BorderRightStyle => "border-right-style", false, parsers::keyword::<BorderStyle>, border_style.right;
    BorderBottomStyle => "border-bottom-style", false, parsers::keyword::<BorderStyle>, border_style.bottom;
    BorderLeftStyle => "border-left-style", false, parsers::keyword::<BorderStyle>, border_style.left;
    BorderTopColor => "border-top-color", false, parsers::color, border_color.top;
    BorderRightColor => "border-right-color", false, parsers::color, border_color.right;
    BorderBottomColor => "border-bottom-color", false, parsers::color, border_color.bottom;
    BorderLeftColor => "border-left-color", false, parsers::color, border_color.left;
    BorderTopLeftRadius => "border-top-left-radius", false, parsers::radius, border_radius.top_left;
    BorderTopRightRadius => "border-top-right-radius", false, parsers::radius, border_radius.top_right;
    BorderBottomRightRadius => "border-bottom-right-radius", false, parsers::radius, border_radius.bottom_right;
    BorderBottomLeftRadius => "border-bottom-left-radius", false, parsers::radius, border_radius.bottom_left;
    Top => "top", false, parsers::length_auto, inset.top;
    Right => "right", false, parsers::length_auto, inset.right;
    Bottom => "bottom", false, parsers::length_auto, inset.bottom;
    Left => "left", false, parsers::length_auto, inset.left;
    ZIndex => "z-index", false, parsers::z_index, z_index;
    Isolation => "isolation", false, parsers::keyword::<Isolation>, isolation;
    BackfaceVisibility => "backface-visibility", false, parsers::keyword::<BackfaceVisibility>, backface_visibility;
    FlexDirection => "flex-direction", false, parsers::keyword::<FlexDirection>, flex_direction;
    FlexWrap => "flex-wrap", false, parsers::keyword::<FlexWrap>, flex_wrap;
    FlexGrow => "flex-grow", false, parsers::non_negative_number, flex_grow;
    FlexShrink => "flex-shrink", false, parsers::non_negative_number, flex_shrink;
    FlexBasis => "flex-basis", false, parsers::size, flex_basis;
    Order => "order", false, parsers::integer, order;
    JustifyContent => "justify-content", false, parsers::alignment, justify_content;
    AlignContent => "align-content", false, parsers::alignment, align_content;
    AlignItems => "align-items", false, parsers::alignment, align_items;
    AlignSelf => "align-self", false, parsers::alignment, align_self;
    JustifyItems => "justify-items", false, parsers::alignment, justify_items;
    JustifySelf => "justify-self", false, parsers::alignment, justify_self;
    RowGap => "row-gap", false, parsers::gap, row_gap;
    ColumnGap => "column-gap", false, parsers::gap, column_gap;
    GridTemplateColumns => "grid-template-columns", false, parsers::track_list, grid_template_columns;
    GridTemplateRows => "grid-template-rows", false, parsers::track_list, grid_template_rows;
    GridAutoColumns => "grid-auto-columns", false, parsers::track_sizes, grid_auto_columns;
    GridAutoRows => "grid-auto-rows", false, parsers::track_sizes, grid_auto_rows;
    GridAutoFlow => "grid-auto-flow", false, parsers::grid_auto_flow, grid_auto_flow;
    GridTemplateAreas => "grid-template-areas", false, parsers::grid_template_areas, grid_template_areas;
    GridColumnStart => "grid-column-start", false, parsers::grid_line, grid_column_start;
    GridColumnEnd => "grid-column-end", false, parsers::grid_line, grid_column_end;
    GridRowStart => "grid-row-start", false, parsers::grid_line, grid_row_start;
    GridRowEnd => "grid-row-end", false, parsers::grid_line, grid_row_end;
    Color => "color", true, parsers::color, color;
    BackgroundColor => "background-color", false, parsers::color, background_color;
    BackgroundImage => "background-image", false, parsers::background_image, background_image;
    BackgroundRepeat => "background-repeat", false, parsers::background_repeat, background_repeat;
    BackgroundSize => "background-size", false, parsers::background_size, background_size;
    BackgroundPosition => "background-position", false, parsers::background_position, background_position;
    MaskImage => "mask-image", false, parsers::background_image, mask_image;
    MaskRepeat => "mask-repeat", false, parsers::background_repeat, mask_repeat;
    MaskSize => "mask-size", false, parsers::background_size, mask_size;
    MaskPosition => "mask-position", false, parsers::background_position, mask_position;
    Opacity => "opacity", false, parsers::opacity, opacity;
    Visibility => "visibility", true, parsers::keyword::<Visibility>, visibility;
    OverflowX => "overflow-x", false, parsers::keyword::<Overflow>, overflow_x;
    OverflowY => "overflow-y", false, parsers::keyword::<Overflow>, overflow_y;
    ScrollbarWidth => "scrollbar-width", false, parsers::keyword::<ScrollbarWidth>, scrollbar_width;
    ScrollbarColor => "scrollbar-color", true, parsers::scrollbar_color, scrollbar_color;
    Cursor => "cursor", true, parsers::cursor, cursor;
    PointerEvents => "pointer-events", true, parsers::keyword::<PointerEvents>, pointer_events;
    UserSelect => "user-select", false, parsers::keyword::<UserSelect>, user_select;
    WebkitUserDrag => "-webkit-user-drag", false, parsers::keyword::<UserDrag>, user_drag;
    Resize => "resize", false, parsers::keyword::<Resize>, resize;
    TableLayout => "table-layout", false, parsers::keyword::<TableLayout>, table_layout;
    CaptionSide => "caption-side", true, parsers::keyword::<CaptionSide>, caption_side;
    AppRegion => "app-region", false, parsers::keyword::<AppRegion>, app_region;
    FontFamily => "font-family", true, parsers::font_family, font_family;
    FontSize => "font-size", true, parsers::font_size, font_size;
    FontWeight => "font-weight", true, parsers::font_weight, font_weight;
    FontStyle => "font-style", true, parsers::keyword::<FontStyle>, font_style;
    LineHeight => "line-height", true, parsers::line_height, line_height;
    TextAlign => "text-align", true, parsers::keyword::<TextAlign>, text_align;
    TextDecorationLine => "text-decoration-line", false, parsers::decoration_line, text_decoration_line;
    TextDecorationColor => "text-decoration-color", false, parsers::optional_color, text_decoration_color;
    TextUnderlineOffset => "text-underline-offset", true, parsers::underline_offset, text_underline_offset;
    TextTransform => "text-transform", true, parsers::keyword::<TextTransform>, text_transform;
    WhiteSpaceCollapse => "white-space-collapse", true, parsers::keyword::<WhiteSpaceCollapse>, white_space.collapse;
    TextWrapMode => "text-wrap-mode", true, parsers::keyword::<TextWrapMode>, white_space.wrap;
    TextWrapStyle => "text-wrap-style", true, parsers::keyword::<TextWrapStyle>, text_wrap_style;
    TextOverflow => "text-overflow", false, parsers::keyword::<TextOverflow>, text_overflow;
    WebkitBoxOrient => "-webkit-box-orient", false, parsers::keyword::<BoxOrient>, box_orient;
    WebkitLineClamp => "-webkit-line-clamp", false, parsers::line_clamp, line_clamp;
    WordBreak => "word-break", true, parsers::keyword::<WordBreak>, word_break;
    OverflowWrap => "overflow-wrap", true, parsers::keyword::<OverflowWrap>, overflow_wrap;
    LetterSpacing => "letter-spacing", true, parsers::spacing, letter_spacing;
    ContainerType => "container-type", false, parsers::keyword::<ContainerType>, container_type;
    ContainerName => "container-name", false, parsers::container_name, container_name;
    TabSize => "tab-size", true, parsers::tab_size, tab_size;
    TextIndent => "text-indent", true, parsers::length, text_indent;
    VerticalAlign => "vertical-align", false, parsers::vertical_align, vertical_align;
    ListStyleType => "list-style-type", true, parsers::keyword::<ListStyleType>, list_style_type;
    ListStylePosition => "list-style-position", true, parsers::keyword::<ListStylePosition>, list_style_position;
    BorderCollapse => "border-collapse", true, parsers::keyword::<BorderCollapse>, border_collapse;
    BorderSpacing => "border-spacing", true, parsers::border_spacing, border_spacing;
    Content => "content", false, parsers::content, content;
    CounterReset => "counter-reset", false, parsers::counter_reset, counter_reset;
    CounterIncrement => "counter-increment", false, parsers::counter_increment, counter_increment;
    CounterSet => "counter-set", false, parsers::counter_reset, counter_set;
    AspectRatio => "aspect-ratio", false, parsers::aspect_ratio, aspect_ratio;
    ObjectFit => "object-fit", false, parsers::keyword::<ObjectFit>, object_fit;
    ObjectPosition => "object-position", false, parsers::object_position, object_position;
    BoxShadow => "box-shadow", false, parsers::box_shadow, box_shadow;
    OutlineWidth => "outline-width", false, parsers::border_width, outline_width;
    OutlineStyle => "outline-style", false, parsers::keyword::<BorderStyle>, outline_style;
    OutlineColor => "outline-color", false, parsers::optional_color, outline_color;
    OutlineOffset => "outline-offset", false, parsers::pixels, outline_offset;
    CaretColor => "caret-color", true, parsers::optional_color, caret_color;
    AccentColor => "accent-color", true, parsers::accent_color, accent_color;
    WebkitTextFillColor => "-webkit-text-fill-color", true, parsers::optional_color, text_fill_color;
    ScrollMarginTop => "scroll-margin-top", false, parsers::pixels, scroll_margin.top;
    ScrollMarginRight => "scroll-margin-right", false, parsers::pixels, scroll_margin.right;
    ScrollMarginBottom => "scroll-margin-bottom", false, parsers::pixels, scroll_margin.bottom;
    ScrollMarginLeft => "scroll-margin-left", false, parsers::pixels, scroll_margin.left;
    ScrollPaddingTop => "scroll-padding-top", false, parsers::length_auto, scroll_padding.top;
    ScrollPaddingRight => "scroll-padding-right", false, parsers::length_auto, scroll_padding.right;
    ScrollPaddingBottom => "scroll-padding-bottom", false, parsers::length_auto, scroll_padding.bottom;
    ScrollPaddingLeft => "scroll-padding-left", false, parsers::length_auto, scroll_padding.left;
    Clip => "clip", false, parsers::clip, clip;
    Transform => "transform", false, parsers::transform, transform;
    Translate => "translate", false, parsers::translate, translate;
    Rotate => "rotate", false, parsers::rotate, rotate;
    Scale => "scale", false, parsers::scale, scale;
    TransformOrigin => "transform-origin", false, parsers::transform_origin, transform_origin;
    Filter => "filter", false, parsers::filter, filter;
    BackdropFilter => "backdrop-filter", false, parsers::filter, backdrop_filter;
    ClipPath => "clip-path", false, parsers::clip_path, clip_path;
    TransitionProperty => "transition-property", false, parsers::transition_properties, transition_property;
    TransitionDuration => "transition-duration", false, parsers::durations, transition_duration;
    TransitionDelay => "transition-delay", false, parsers::delays, transition_delay;
    TransitionTimingFunction => "transition-timing-function", false, parsers::easings, transition_timing_function;
    Direction => "direction", true, parsers::keyword::<Direction>, direction;
    UnicodeBidi => "unicode-bidi", false, parsers::keyword::<UnicodeBidi>, unicode_bidi;
    FontVariantNumeric => "font-variant-numeric", true, parsers::font_variant_numeric, font_variant_numeric;
    FontFeatureSettings => "font-feature-settings", true, parsers::font_feature_settings, font_feature_settings;
    FontKerning => "font-kerning", true, parsers::keyword::<FontKerning>, font_kerning;
    AnimationName => "animation-name", false, parsers::animation_names, animation_name;
    AnimationDuration => "animation-duration", false, parsers::durations, animation_duration;
    AnimationTimingFunction => "animation-timing-function", false, parsers::easings, animation_timing_function;
    AnimationDelay => "animation-delay", false, parsers::delays, animation_delay;
    AnimationIterationCount => "animation-iteration-count", false, parsers::iteration_counts, animation_iteration_count;
    AnimationDirection => "animation-direction", false, parsers::keyword_list::<AnimationDirection>, animation_direction;
    AnimationFillMode => "animation-fill-mode", false, parsers::keyword_list::<FillMode>, animation_fill_mode;
    AnimationPlayState => "animation-play-state", false, parsers::keyword_list::<PlayState>, animation_play_state;
    Fill => "fill", true, parsers::paint, fill;
    FillOpacity => "fill-opacity", true, parsers::opacity, fill_opacity;
    FillRule => "fill-rule", true, parsers::keyword::<FillRule>, fill_rule;
    Stroke => "stroke", true, parsers::paint, stroke;
    StrokeWidth => "stroke-width", true, parsers::stroke_width, stroke_width;
    StrokeOpacity => "stroke-opacity", true, parsers::opacity, stroke_opacity;
    StrokeLinecap => "stroke-linecap", true, parsers::keyword::<LineCap>, stroke_linecap;
    StrokeLinejoin => "stroke-linejoin", true, parsers::keyword::<LineJoin>, stroke_linejoin;
    StrokeDasharray => "stroke-dasharray", true, parsers::stroke_dasharray, stroke_dasharray;
    StrokeDashoffset => "stroke-dashoffset", true, parsers::stroke_dashoffset, stroke_dashoffset;
    MarkerStart => "marker-start", true, parsers::marker, marker_start;
    MarkerMid => "marker-mid", true, parsers::marker, marker_mid;
    MarkerEnd => "marker-end", true, parsers::marker, marker_end;
    TextAnchor => "text-anchor", true, parsers::keyword::<TextAnchor>, text_anchor;
    DominantBaseline => "dominant-baseline", true, parsers::keyword::<SvgBaseline>, dominant_baseline;
    AlignmentBaseline => "alignment-baseline", false, parsers::keyword::<SvgBaseline>, alignment_baseline;
}

/// Serializes four sides in their shortest form.
fn sides(values: [String; 4]) -> String {
    let [top, right, bottom, left] = values;
    if left != right {
        format!("{top} {right} {bottom} {left}")
    } else if bottom != top {
        format!("{top} {right} {bottom}")
    } else if right != top {
        format!("{top} {right}")
    } else {
        top
    }
}

impl ComputedStyle {
    /// The transform the box shows: `translate`, `rotate` and `scale`, then
    /// `transform`, as CSS Transforms 2 applies them.
    pub fn full_transform(&self) -> alloc::borrow::Cow<'_, Transform> {
        let individual = [&self.translate, &self.rotate, &self.scale];
        if individual.iter().all(|part| part.0.0.is_empty()) {
            return alloc::borrow::Cow::Borrowed(&self.transform);
        }
        let mut functions: Vec<TransformFunction> = individual
            .iter()
            .flat_map(|part| part.0.0.iter().cloned())
            .collect();
        functions.extend(self.transform.0.iter().cloned());
        alloc::borrow::Cow::Owned(Transform(functions))
    }

    /// Whether the box is transformed, by `transform` or the individual
    /// transform properties.
    pub fn has_transform(&self) -> bool {
        !self.transform.0.is_empty()
            || !self.translate.0.0.is_empty()
            || !self.rotate.0.0.is_empty()
            || !self.scale.0.0.is_empty()
    }

    /// The initial style for an environment.
    pub fn initial(environment: &StyleEnvironment) -> ComputedStyle {
        let color = environment.system.canvas_text;
        ComputedStyle {
            color,
            border_color: Edges::all(color),
            font_family: FontFamily(vec![environment.standard_font.to_owned()]),
            ..ComputedStyle::default()
        }
    }

    /// Computes the style of an element from its cascaded values.
    pub fn compute(
        specified: &SpecifiedValues,
        parent: Option<&ComputedStyle>,
        environment: &StyleEnvironment,
    ) -> ComputedStyle {
        let mut initial = ComputedStyle::initial(environment);
        let parent = parent.unwrap_or(&initial).clone();
        let mut style = initial.clone();
        for id in PropertyId::ALL {
            if id.inherited() {
                style.copy_property(*id, &parent);
            }
        }
        style.custom = specified.custom.clone();
        style.author_appearance = specified.author_appearance;
        style.appearance_none = specified.appearance_none;
        style.query_size = parent.query_size;
        let mut context = Context {
            length: LengthContext {
                font_size: parent.font_size.0,
                root_font_size: environment.root_font_size,
                viewport_width: environment.media.width as f32,
                viewport_height: environment.media.height as f32,
                container: parent.query_size,
            },
            parent_font_size: parent.font_size.0,
            parent_font_weight: parent.font_weight.0,
            color: parent.color,
            system: &environment.system,
        };
        let get = |id: PropertyId| specified.values.get(&id);
        if let Some(value) = get(PropertyId::FontSize) {
            style.set(PropertyId::FontSize, value, &parent, &initial, &context);
        }
        context.length.font_size = style.font_size.0;
        if let Some(value) = get(PropertyId::Color) {
            style.set(PropertyId::Color, value, &parent, &initial, &context);
        }
        context.color = style.color;
        style.border_color = Edges::all(style.color);
        initial.border_color = style.border_color;
        for (id, value) in &specified.values {
            if matches!(value, Specified::Inherit) && !id.inherited() {
                style.inherits_explicitly = true;
            }
            if !matches!(id, PropertyId::FontSize | PropertyId::Color) {
                style.set(*id, value, &parent, &initial, &context);
            }
        }
        let zero_if_none = |width: &mut Px, style: BorderStyle| {
            if style.is_none() {
                *width = Px(0.0);
            }
        };
        zero_if_none(&mut style.border_width.top, style.border_style.top);
        zero_if_none(&mut style.border_width.right, style.border_style.right);
        zero_if_none(&mut style.border_width.bottom, style.border_style.bottom);
        zero_if_none(&mut style.border_width.left, style.border_style.left);
        zero_if_none(&mut style.outline_width, style.outline_style);
        style.snap_to_device_pixels(environment.media.resolution as f32);
        // Flex and grid items, floats and absolute boxes are block-level
        // (CSS Display's blockification).
        // A vertical `-webkit-box` lays its content out in lines, as
        // Chromium does for `-webkit-line-clamp`.
        let item = matches!(
            parent.display,
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
        ) || (matches!(
            parent.display,
            Display::WebkitBox | Display::WebkitInlineBox
        ) && parent.box_orient == BoxOrient::Horizontal);
        style.original_display = style.display;
        if item
            || style.float != Float::None
            || matches!(style.position, PositionKind::Absolute | PositionKind::Fixed)
        {
            style.display = style.display.blockified();
        }
        style
    }

    /// The longhands transitions and animations interpolate; others keep
    /// their computed value while an animation runs.
    pub const ANIMATABLE: &'static [PropertyId] = &[
        PropertyId::Color,
        PropertyId::BackgroundColor,
        PropertyId::BorderTopColor,
        PropertyId::BorderRightColor,
        PropertyId::BorderBottomColor,
        PropertyId::BorderLeftColor,
        PropertyId::Opacity,
        PropertyId::Width,
        PropertyId::Height,
        PropertyId::MinWidth,
        PropertyId::MinHeight,
        PropertyId::MaxWidth,
        PropertyId::MaxHeight,
        PropertyId::MarginTop,
        PropertyId::MarginRight,
        PropertyId::MarginBottom,
        PropertyId::MarginLeft,
        PropertyId::PaddingTop,
        PropertyId::PaddingRight,
        PropertyId::PaddingBottom,
        PropertyId::PaddingLeft,
        PropertyId::BorderTopWidth,
        PropertyId::BorderRightWidth,
        PropertyId::BorderBottomWidth,
        PropertyId::BorderLeftWidth,
        PropertyId::BorderTopLeftRadius,
        PropertyId::BorderTopRightRadius,
        PropertyId::BorderBottomRightRadius,
        PropertyId::BorderBottomLeftRadius,
        PropertyId::Top,
        PropertyId::Right,
        PropertyId::Bottom,
        PropertyId::Left,
        PropertyId::FlexGrow,
        PropertyId::FlexShrink,
        PropertyId::FlexBasis,
        PropertyId::RowGap,
        PropertyId::ColumnGap,
        PropertyId::FontSize,
        PropertyId::FontWeight,
        PropertyId::LineHeight,
        PropertyId::LetterSpacing,
        PropertyId::TextIndent,
        PropertyId::TextDecorationColor,
        PropertyId::BoxShadow,
        PropertyId::OutlineWidth,
        PropertyId::OutlineOffset,
        PropertyId::OutlineColor,
        PropertyId::Transform,
        PropertyId::Translate,
        PropertyId::Rotate,
        PropertyId::Scale,
        PropertyId::Filter,
        PropertyId::BackdropFilter,
        PropertyId::Visibility,
        PropertyId::Fill,
        PropertyId::FillOpacity,
        PropertyId::Stroke,
        PropertyId::StrokeWidth,
        PropertyId::StrokeOpacity,
        PropertyId::ZIndex,
        PropertyId::Order,
        PropertyId::GridTemplateColumns,
        PropertyId::GridTemplateRows,
        PropertyId::BackgroundSize,
        PropertyId::BackgroundPosition,
        PropertyId::ObjectPosition,
        PropertyId::VerticalAlign,
        PropertyId::BorderSpacing,
        PropertyId::TabSize,
        PropertyId::ScrollbarColor,
        PropertyId::CaretColor,
        PropertyId::WebkitTextFillColor,
        PropertyId::AspectRatio,
        PropertyId::ScrollMarginTop,
        PropertyId::ScrollMarginRight,
        PropertyId::ScrollMarginBottom,
        PropertyId::ScrollMarginLeft,
        PropertyId::ScrollPaddingTop,
        PropertyId::ScrollPaddingRight,
        PropertyId::ScrollPaddingBottom,
        PropertyId::ScrollPaddingLeft,
        PropertyId::Clip,
        PropertyId::TransformOrigin,
        PropertyId::ClipPath,
        PropertyId::BackgroundImage,
    ];

    /// The style with cascaded values applied over it: lengths are relative
    /// to its own font size, `currentcolor` is its color and `inherit`
    /// keeps its value.
    pub fn with_specified(
        &self,
        values: &BTreeMap<PropertyId, Specified>,
        environment: &StyleEnvironment,
    ) -> ComputedStyle {
        let initial = ComputedStyle::initial(environment);
        let context = Context {
            length: LengthContext {
                font_size: self.font_size.0,
                root_font_size: environment.root_font_size,
                viewport_width: environment.media.width as f32,
                viewport_height: environment.media.height as f32,
                container: self.query_size,
            },
            parent_font_size: self.font_size.0,
            parent_font_weight: self.font_weight.0,
            color: self.color,
            system: &environment.system,
        };
        let mut style = self.clone();
        for (id, value) in values {
            style.set(*id, value, self, &initial, &context);
        }
        style.snap_to_device_pixels(environment.media.resolution as f32);
        style
    }

    /// Chromium keeps border and outline widths, the outline offset and
    /// border spacing in whole device pixels: widths round down to them but
    /// stay at least one when set, the others round toward zero.
    fn snap_to_device_pixels(&mut self, scale: f32) {
        if scale <= 0.0 {
            return;
        }
        // Lengths a hair under a whole number of pixels count as it.
        let whole = |value: f32| {
            let device =
                crate::math::floor(if value < 0.0 { -value } else { value } * scale + 1e-3);
            if value < 0.0 {
                -device / scale
            } else {
                device / scale
            }
        };
        let width = |value: &mut Px| {
            if value.0 > 0.0 {
                value.0 = whole(value.0).max(1.0 / scale);
            }
        };
        width(&mut self.border_width.top);
        width(&mut self.border_width.right);
        width(&mut self.border_width.bottom);
        width(&mut self.border_width.left);
        width(&mut self.outline_width);
        self.outline_offset.0 = whole(self.outline_offset.0);
        self.border_spacing.horizontal.0 = whole(self.border_spacing.horizontal.0);
        self.border_spacing.vertical.0 = whole(self.border_spacing.vertical.0);
    }

    fn set(
        &mut self,
        id: PropertyId,
        value: &Specified,
        parent: &ComputedStyle,
        initial: &ComputedStyle,
        context: &Context,
    ) {
        let inherit = match value {
            Specified::Value(values) => {
                if self.apply(id, values, context) {
                    return;
                }
                id.inherited()
            }
            Specified::Inherit => true,
            Specified::Initial => false,
            Specified::Unset => id.inherited(),
        };
        self.copy_property(id, if inherit { parent } else { initial });
    }

    /// Whether `value` is a valid value for the longhand.
    pub fn is_valid(id: PropertyId, value: &[ComponentValue]) -> bool {
        let system = SystemColors::LIGHT;
        let context = Context {
            length: LengthContext::default(),
            parent_font_size: 16.0,
            parent_font_weight: 400,
            color: Color::BLACK,
            system: &system,
        };
        ComputedStyle::default().apply(id, value, &context)
    }

    /// The value `getComputedStyle` reports for a property, including common
    /// shorthands, for an element that layout did not place.
    pub fn property_value(&self, name: &str) -> Option<String> {
        self.resolved_value(name, None)
    }

    /// The value `getComputedStyle` reports for a property, including common
    /// shorthands: the resolved value, which for a box layout placed takes
    /// sizes, margins and paddings from `used`, as Chromium does.
    pub fn resolved_value(&self, name: &str, used: Option<&UsedBox>) -> Option<String> {
        if let Some(value) = name.strip_prefix("--") {
            return self.custom.get(&format!("--{value}")).cloned();
        }
        // Logical properties report the physical sides they map to.
        if let Some(id) = PropertyId::from_name(name).or_else(|| crate::shorthand::alias(name)) {
            return Some(self.resolved_value_of(id, used));
        }
        let four = |ids: [PropertyId; 4]| sides(ids.map(|id| self.resolved_value_of(id, used)));
        Some(match name {
            "margin" => four([
                PropertyId::MarginTop,
                PropertyId::MarginRight,
                PropertyId::MarginBottom,
                PropertyId::MarginLeft,
            ]),
            "padding" => four([
                PropertyId::PaddingTop,
                PropertyId::PaddingRight,
                PropertyId::PaddingBottom,
                PropertyId::PaddingLeft,
            ]),
            "inset" => four([
                PropertyId::Top,
                PropertyId::Right,
                PropertyId::Bottom,
                PropertyId::Left,
            ]),
            "border-width" => four([
                PropertyId::BorderTopWidth,
                PropertyId::BorderRightWidth,
                PropertyId::BorderBottomWidth,
                PropertyId::BorderLeftWidth,
            ]),
            "border-style" => four([
                PropertyId::BorderTopStyle,
                PropertyId::BorderRightStyle,
                PropertyId::BorderBottomStyle,
                PropertyId::BorderLeftStyle,
            ]),
            "border-color" => four([
                PropertyId::BorderTopColor,
                PropertyId::BorderRightColor,
                PropertyId::BorderBottomColor,
                PropertyId::BorderLeftColor,
            ]),
            "border-radius" => four([
                PropertyId::BorderTopLeftRadius,
                PropertyId::BorderTopRightRadius,
                PropertyId::BorderBottomRightRadius,
                PropertyId::BorderBottomLeftRadius,
            ]),
            "marker" => {
                let start = self.marker_start.to_css();
                if self.marker_mid.to_css() == start && self.marker_end.to_css() == start {
                    start
                } else {
                    String::new()
                }
            }
            "overflow" => {
                let x = self.overflow_x.to_css();
                let y = self.overflow_y.to_css();
                if x == y { x } else { format!("{x} {y}") }
            }
            "gap" => {
                let row = self.row_gap.to_css();
                let column = self.column_gap.to_css();
                if row == column {
                    row
                } else {
                    format!("{row} {column}")
                }
            }
            "flex" => format!(
                "{} {} {}",
                self.flex_grow.to_css(),
                self.flex_shrink.to_css(),
                self.flex_basis.to_css()
            ),
            "flex-flow" => format!(
                "{} {}",
                self.flex_direction.to_css(),
                self.flex_wrap.to_css()
            ),
            "text-decoration" => self.text_decoration_line.to_css(),
            "white-space" => self.white_space.to_css(),
            // Chromium leaves out what is initial.
            "text-wrap" => match (self.white_space.wrap, self.text_wrap_style) {
                (mode, TextWrapStyle::Auto) => mode.to_css(),
                (TextWrapMode::Wrap, style) => style.to_css(),
                (mode, style) => format!("{} {}", mode.to_css(), style.to_css()),
            },
            "margin-block" | "margin-inline" | "padding-block" | "padding-inline"
            | "inset-block" | "inset-inline" => {
                let start = self.resolved_value(&format!("{name}-start"), used)?;
                let end = self.resolved_value(&format!("{name}-end"), used)?;
                if start == end {
                    start
                } else {
                    format!("{start} {end}")
                }
            }
            _ => return None,
        })
    }

    /// Whether a property's resolved value can depend on layout, which
    /// callers then bring up to date for `resolved_value`.
    pub fn resolves_against_layout(name: &str) -> bool {
        matches!(name, "width" | "height" | "margin" | "padding")
            || name.starts_with("margin-")
            || name.starts_with("padding-")
    }

    /// One longhand's resolved value. Line heights are in pixels; for a box
    /// layout placed (`used`), so are `width` and `height` when they apply,
    /// and margins and paddings that are not fixed lengths.
    pub fn resolved_value_of(&self, id: PropertyId, used: Option<&UsedBox>) -> String {
        if id == PropertyId::LineHeight && self.line_height != LineHeight::Normal {
            return Px(self.used_line_height()).to_css();
        }
        let Some(used) = used else {
            return self.value_of(id);
        };
        let fixed = |value: &str| value.ends_with("px");
        let edge = match id {
            PropertyId::MarginTop | PropertyId::PaddingTop => 0,
            PropertyId::MarginRight | PropertyId::PaddingRight => 1,
            PropertyId::MarginBottom | PropertyId::PaddingBottom => 2,
            PropertyId::MarginLeft | PropertyId::PaddingLeft => 3,
            PropertyId::Width | PropertyId::Height if used.sized => {
                let (size, edges, scrollbar) = if id == PropertyId::Width {
                    (used.size.0, [1, 3], used.scrollbars[0])
                } else {
                    (used.size.1, [0, 2], used.scrollbars[1])
                };
                let size = match self.box_sizing {
                    BoxSizing::BorderBox => size,
                    BoxSizing::ContentBox => edges
                        .iter()
                        .fold(size - scrollbar, |size, &edge| {
                            size - used.border[edge] - used.padding[edge]
                        })
                        .max(0.0),
                };
                return Px(size).to_css();
            }
            _ => return self.value_of(id),
        };
        let value = self.value_of(id);
        if fixed(&value) {
            return value;
        }
        let margin = matches!(
            id,
            PropertyId::MarginTop
                | PropertyId::MarginRight
                | PropertyId::MarginBottom
                | PropertyId::MarginLeft
        );
        Px(if margin {
            used.margin[edge]
        } else {
            used.padding[edge]
        })
        .to_css()
    }

    /// The used line height in pixels.
    pub fn used_line_height(&self) -> f32 {
        self.line_height.resolve(self.font_size.0)
    }
}

#[cfg(test)]
mod tests;
