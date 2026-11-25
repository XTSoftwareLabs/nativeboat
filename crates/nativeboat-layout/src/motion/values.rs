//! Interpolation of the computed values the renderer can animate.

use crate::prelude::*;
use nativeboat_css::types::*;
use nativeboat_css::values::{Calc, LengthPercentage as Length};
use nativeboat_css::{Color, ComputedStyle, PropertyId};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Value {
    Color(Color),
    Number(f32),
    Pixels(Px),
    Length(Length),
    Auto(LengthPercentageAuto),
    Size(Size),
    MaxSize(MaxSize),
    Radius(Radius),
    Shadows(Shadows),
    Filters(Filters),
    Transform(Transform),
    OptionalColor(OptionalColor),
    LineHeight(LineHeight),
    Weight(FontWeight),
    Visibility(Visibility),
    Paint(Paint),
    ZIndex(ZIndex),
    Integer(i32),
    BackgroundSize(BackgroundSize),
    Position(BackgroundPosition),
    VerticalAlign(VerticalAlign),
    Spacing(Spacing),
    TabSize(TabSize),
    ScrollbarColor(ScrollbarColor),
    AspectRatio(AspectRatio),
    Origin(TransformOrigin),
    Clip(Option<ClipRect>),
    ClipPath(ClipPath),
    Tracks(TrackList),
    Image(BackgroundImage),
}

pub(super) struct Property {
    pub id: PropertyId,
    pub read: fn(&ComputedStyle) -> Value,
    pub write: fn(&mut ComputedStyle, Value),
}

macro_rules! properties {
    ($($id:ident => $kind:ident, $($field:tt).+;)+) => {
        pub(super) const PROPERTIES: &[Property] = &[$(Property {
            id: PropertyId::$id,
            read: |style| Value::$kind(style.$($field).+.to_owned()),
            write: |style, value| {
                if let Value::$kind(value) = value {
                    style.$($field).+ = value;
                }
            },
        }),+];
    };
}

properties! {
    Color => Color, color;
    BackgroundColor => Color, background_color;
    BorderTopColor => Color, border_color.top;
    BorderRightColor => Color, border_color.right;
    BorderBottomColor => Color, border_color.bottom;
    BorderLeftColor => Color, border_color.left;
    Opacity => Number, opacity;
    Width => Size, width;
    Height => Size, height;
    MinWidth => Size, min_width;
    MinHeight => Size, min_height;
    MaxWidth => MaxSize, max_width;
    MaxHeight => MaxSize, max_height;
    MarginTop => Auto, margin.top;
    MarginRight => Auto, margin.right;
    MarginBottom => Auto, margin.bottom;
    MarginLeft => Auto, margin.left;
    PaddingTop => Length, padding.top;
    PaddingRight => Length, padding.right;
    PaddingBottom => Length, padding.bottom;
    PaddingLeft => Length, padding.left;
    BorderTopWidth => Pixels, border_width.top;
    BorderRightWidth => Pixels, border_width.right;
    BorderBottomWidth => Pixels, border_width.bottom;
    BorderLeftWidth => Pixels, border_width.left;
    BorderTopLeftRadius => Radius, border_radius.top_left;
    BorderTopRightRadius => Radius, border_radius.top_right;
    BorderBottomRightRadius => Radius, border_radius.bottom_right;
    BorderBottomLeftRadius => Radius, border_radius.bottom_left;
    Top => Auto, inset.top;
    Right => Auto, inset.right;
    Bottom => Auto, inset.bottom;
    Left => Auto, inset.left;
    FlexGrow => Number, flex_grow;
    FlexShrink => Number, flex_shrink;
    FlexBasis => Size, flex_basis;
    RowGap => Length, row_gap;
    ColumnGap => Length, column_gap;
    FontSize => Pixels, font_size;
    FontWeight => Weight, font_weight;
    LineHeight => LineHeight, line_height;
    LetterSpacing => Pixels, letter_spacing;
    TextIndent => Length, text_indent;
    TextDecorationColor => OptionalColor, text_decoration_color;
    BoxShadow => Shadows, box_shadow;
    OutlineWidth => Pixels, outline_width;
    OutlineOffset => Pixels, outline_offset;
    OutlineColor => OptionalColor, outline_color;
    Transform => Transform, transform;
    Translate => Transform, translate.0;
    Rotate => Transform, rotate.0;
    Scale => Transform, scale.0;
    Filter => Filters, filter;
    BackdropFilter => Filters, backdrop_filter;
    Visibility => Visibility, visibility;
    Fill => Paint, fill;
    FillOpacity => Number, fill_opacity;
    Stroke => Paint, stroke;
    StrokeWidth => Pixels, stroke_width;
    StrokeOpacity => Number, stroke_opacity;
    ZIndex => ZIndex, z_index;
    Order => Integer, order;
    GridTemplateColumns => Tracks, grid_template_columns;
    GridTemplateRows => Tracks, grid_template_rows;
    BackgroundSize => BackgroundSize, background_size;
    BackgroundPosition => Position, background_position;
    ObjectPosition => Position, object_position;
    VerticalAlign => VerticalAlign, vertical_align;
    BorderSpacing => Spacing, border_spacing;
    TabSize => TabSize, tab_size;
    ScrollbarColor => ScrollbarColor, scrollbar_color;
    CaretColor => OptionalColor, caret_color;
    WebkitTextFillColor => OptionalColor, text_fill_color;
    AspectRatio => AspectRatio, aspect_ratio;
    ScrollMarginTop => Pixels, scroll_margin.top;
    ScrollMarginRight => Pixels, scroll_margin.right;
    ScrollMarginBottom => Pixels, scroll_margin.bottom;
    ScrollMarginLeft => Pixels, scroll_margin.left;
    ScrollPaddingTop => Auto, scroll_padding.top;
    ScrollPaddingRight => Auto, scroll_padding.right;
    ScrollPaddingBottom => Auto, scroll_padding.bottom;
    ScrollPaddingLeft => Auto, scroll_padding.left;
    Clip => Clip, clip;
    TransformOrigin => Origin, transform_origin;
    ClipPath => ClipPath, clip_path;
    BackgroundImage => Image, background_image;
}

fn mix(a: f32, b: f32, progress: f32) -> f32 {
    a + (b - a) * progress
}

fn color(a: Color, b: Color, progress: f32) -> Color {
    let alpha = mix(a.a, b.a, progress).clamp(0.0, 1.0);
    if alpha == 0.0 {
        return Color::TRANSPARENT;
    }
    let (a_alpha, b_alpha) = (a.a, b.a);
    let channel = |a: u8, b: u8| {
        crate::math::round(mix(a as f32 * a_alpha, b as f32 * b_alpha, progress) / alpha)
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b),
        alpha,
    )
}

fn length(a: &Length, b: &Length, progress: f32) -> Length {
    match (a, b) {
        (Length::Px(a), Length::Px(b)) => Length::Px(mix(*a, *b, progress)),
        (Length::Percent(a), Length::Percent(b)) => Length::Percent(mix(*a, *b, progress)),
        _ => {
            let expression = |value: &Length| match value {
                Length::Px(value) => Calc::Px(*value),
                Length::Percent(value) => Calc::Percent(*value),
                Length::Calc(value) => *value.clone(),
            };
            Length::Calc(Box::new(Calc::Sum(vec![
                Calc::Product(Box::new(expression(a)), 1.0 - progress),
                Calc::Product(Box::new(expression(b)), progress),
            ])))
        }
    }
}

fn position(a: &BackgroundPosition, b: &BackgroundPosition, progress: f32) -> BackgroundPosition {
    BackgroundPosition {
        x: length(&a.x, &b.x, progress),
        y: length(&a.y, &b.y, progress),
    }
}

fn radius(a: &Radius, b: &Radius, progress: f32) -> Radius {
    Radius {
        horizontal: length(&a.horizontal, &b.horizontal, progress),
        vertical: length(&a.vertical, &b.vertical, progress),
    }
}

/// Lengths or `auto`; `auto` only meets itself.
fn optional_length(
    a: &LengthPercentageAuto,
    b: &LengthPercentageAuto,
    progress: f32,
) -> Option<LengthPercentageAuto> {
    Some(match (a, b) {
        (LengthPercentageAuto::Length(a), LengthPercentageAuto::Length(b)) => {
            LengthPercentageAuto::Length(length(a, b, progress))
        }
        (LengthPercentageAuto::Auto, LengthPercentageAuto::Auto) => LengthPercentageAuto::Auto,
        _ => return None,
    })
}

fn shape_radius(a: &ShapeRadius, b: &ShapeRadius, progress: f32) -> Option<ShapeRadius> {
    match (a, b) {
        (ShapeRadius::Length(a), ShapeRadius::Length(b)) => {
            Some(ShapeRadius::Length(length(a, b, progress)))
        }
        _ => (a == b).then(|| a.clone()),
    }
}

/// Shapes of the same kind interpolate their lengths; polygons need as
/// many points and the same fill rule.
fn shape(a: &BasicShape, b: &BasicShape, progress: f32) -> Option<BasicShape> {
    Some(match (a, b) {
        (BasicShape::Inset(a_offsets, a_radii), BasicShape::Inset(b_offsets, b_radii)) => {
            BasicShape::Inset(
                core::array::from_fn(|side| length(&a_offsets[side], &b_offsets[side], progress)),
                core::array::from_fn(|corner| radius(&a_radii[corner], &b_radii[corner], progress)),
            )
        }
        (BasicShape::Circle(a_radius, a_at), BasicShape::Circle(b_radius, b_at)) => {
            BasicShape::Circle(
                shape_radius(a_radius, b_radius, progress)?,
                position(a_at, b_at, progress),
            )
        }
        (BasicShape::Ellipse(a_x, a_y, a_at), BasicShape::Ellipse(b_x, b_y, b_at)) => {
            BasicShape::Ellipse(
                shape_radius(a_x, b_x, progress)?,
                shape_radius(a_y, b_y, progress)?,
                position(a_at, b_at, progress),
            )
        }
        (BasicShape::Polygon(a_rule, a_points), BasicShape::Polygon(b_rule, b_points))
            if a_rule == b_rule && a_points.len() == b_points.len() =>
        {
            BasicShape::Polygon(
                *a_rule,
                a_points
                    .iter()
                    .zip(b_points)
                    .map(|((ax, ay), (bx, by))| {
                        (length(ax, bx, progress), length(ay, by, progress))
                    })
                    .collect(),
            )
        }
        _ => return None,
    })
}

/// Track sizes of the same kind interpolate their lengths and fractions.
fn track(a: &TrackSize, b: &TrackSize, progress: f32) -> Option<TrackSize> {
    Some(match (a, b) {
        (TrackSize::Length(a), TrackSize::Length(b)) => TrackSize::Length(length(a, b, progress)),
        (TrackSize::Fr(a), TrackSize::Fr(b)) => TrackSize::Fr(mix(*a, *b, progress).max(0.0)),
        (TrackSize::MinMax(a_min, a_max), TrackSize::MinMax(b_min, b_max)) => TrackSize::MinMax(
            Box::new(track(a_min, b_min, progress)?),
            Box::new(track(a_max, b_max, progress)?),
        ),
        (TrackSize::FitContent(a), TrackSize::FitContent(b)) => {
            TrackSize::FitContent(length(a, b, progress))
        }
        _ => return (a == b).then(|| a.clone()),
    })
}

fn tracks(a: &[TrackSize], b: &[TrackSize], progress: f32) -> Option<Vec<TrackSize>> {
    if a.len() != b.len() {
        return None;
    }
    a.iter()
        .zip(b)
        .map(|(a, b)| track(a, b, progress))
        .collect()
}

/// Track lists of the same structure, as Chromium interpolates grid
/// templates: as many entries, repeats with the same count.
fn track_list(a: &TrackList, b: &TrackList, progress: f32) -> Option<TrackList> {
    if a.0.len() != b.0.len() || a.0.is_empty() {
        return None;
    }
    a.0.iter()
        .zip(&b.0)
        .map(|(a, b)| match (a, b) {
            (TrackEntry::Single(a), TrackEntry::Single(b)) => {
                Some(TrackEntry::Single(track(a, b, progress)?))
            }
            (TrackEntry::Repeat(a_count, a), TrackEntry::Repeat(b_count, b))
                if a_count == b_count =>
            {
                Some(TrackEntry::Repeat(*a_count, tracks(a, b, progress)?))
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(TrackList)
}

fn shadows(a: &Shadows, b: &Shadows, progress: f32) -> Option<Shadows> {
    let mut result = Vec::new();
    for index in 0..a.0.len().max(b.0.len()) {
        let transparent = |inset| Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 0.0,
            color: Color::TRANSPARENT,
            inset,
        };
        let first =
            a.0.get(index)
                .cloned()
                .unwrap_or_else(|| transparent(b.0[index].inset));
        let second =
            b.0.get(index)
                .cloned()
                .unwrap_or_else(|| transparent(first.inset));
        if first.inset != second.inset {
            return None;
        }
        result.push(Shadow {
            x: mix(first.x, second.x, progress),
            y: mix(first.y, second.y, progress),
            blur: mix(first.blur, second.blur, progress).max(0.0),
            spread: mix(first.spread, second.spread, progress),
            color: color(first.color, second.color, progress),
            inset: first.inset,
        });
    }
    Some(Shadows(result))
}

/// Two functions of the same kind part of the way between them.
fn filter(a: &Filter, b: &Filter, progress: f32) -> Option<Filter> {
    let amount = |a: f32, b: f32| mix(a, b, progress).max(0.0);
    let fraction = |a: f32, b: f32| mix(a, b, progress).clamp(0.0, 1.0);
    Some(match (a.clone(), b.clone()) {
        (Filter::Blur(a), Filter::Blur(b)) => Filter::Blur(amount(a, b)),
        (Filter::Brightness(a), Filter::Brightness(b)) => Filter::Brightness(amount(a, b)),
        (Filter::Contrast(a), Filter::Contrast(b)) => Filter::Contrast(amount(a, b)),
        (Filter::Saturate(a), Filter::Saturate(b)) => Filter::Saturate(amount(a, b)),
        (Filter::Grayscale(a), Filter::Grayscale(b)) => Filter::Grayscale(fraction(a, b)),
        (Filter::Invert(a), Filter::Invert(b)) => Filter::Invert(fraction(a, b)),
        (Filter::Opacity(a), Filter::Opacity(b)) => Filter::Opacity(fraction(a, b)),
        (Filter::Sepia(a), Filter::Sepia(b)) => Filter::Sepia(fraction(a, b)),
        (Filter::HueRotate(a), Filter::HueRotate(b)) => Filter::HueRotate(mix(a, b, progress)),
        (
            Filter::DropShadow {
                x: ax,
                y: ay,
                blur: ab,
                color: ac,
            },
            Filter::DropShadow {
                x: bx,
                y: by,
                blur: bb,
                color: bc,
            },
        ) => Filter::DropShadow {
            x: mix(ax, bx, progress),
            y: mix(ay, by, progress),
            blur: amount(ab, bb),
            color: color(ac, bc, progress),
        },
        _ => return None,
    })
}

/// Lists interpolate function by function when their kinds match in
/// order; the shorter list continues with functions that change nothing.
fn filters(a: &Filters, b: &Filters, progress: f32) -> Option<Filters> {
    (0..a.0.len().max(b.0.len()))
        .map(|index| {
            let first =
                a.0.get(index)
                    .cloned()
                    .unwrap_or_else(|| b.0[index].identity());
            let second = b.0.get(index).cloned().unwrap_or_else(|| first.identity());
            filter(&first, &second, progress)
        })
        .collect::<Option<Vec<_>>>()
        .map(Filters)
}

/// Interpolates transforms function by function when their functions pair
/// up, `none` and the missing end of a shorter list counting as identity
/// functions; otherwise their matrices interpolate once the box's size is
/// known.
pub(super) fn transform(a: &Transform, b: &Transform, progress: f32) -> Transform {
    use TransformFunction as F;
    let pad = |list: &Transform, other: &Transform| -> Vec<F> {
        list.0
            .iter()
            .cloned()
            .chain(
                other.0[list.0.len().min(other.0.len())..]
                    .iter()
                    .map(F::identity),
            )
            .collect()
    };
    let (from, to) = (pad(a, b), pad(b, a));
    let paired: Option<Vec<F>> = from
        .iter()
        .zip(&to)
        .map(|pair| {
            Some(match pair {
                (F::Translate(ax, ay), F::Translate(bx, by)) => {
                    F::Translate(length(ax, bx, progress), length(ay, by, progress))
                }
                (F::Scale(ax, ay), F::Scale(bx, by)) => {
                    F::Scale(mix(*ax, *bx, progress), mix(*ay, *by, progress))
                }
                (F::Rotate(a), F::Rotate(b)) => F::Rotate(mix(*a, *b, progress)),
                (F::Skew(ax, ay), F::Skew(bx, by)) => {
                    F::Skew(mix(*ax, *bx, progress), mix(*ay, *by, progress))
                }
                (F::Matrix(a), F::Matrix(b)) => F::Matrix(interpolate_matrices(*a, *b, progress)),
                _ => return None,
            })
        })
        .collect();
    match paired {
        Some(functions) => Transform(functions),
        None => Transform(vec![F::Mix(
            Box::new(a.clone()),
            Box::new(b.clone()),
            progress,
        )]),
    }
}

impl Value {
    pub fn interpolate(&self, to: &Self, progress: f32) -> Option<Self> {
        Some(match (self, to) {
            (Self::Color(a), Self::Color(b)) => Self::Color(color(*a, *b, progress)),
            (Self::Number(a), Self::Number(b)) => Self::Number(mix(*a, *b, progress)),
            (Self::Pixels(a), Self::Pixels(b)) => Self::Pixels(Px(mix(a.0, b.0, progress))),
            (Self::Length(a), Self::Length(b)) => Self::Length(length(a, b, progress)),
            (
                Self::Auto(LengthPercentageAuto::Length(a)),
                Self::Auto(LengthPercentageAuto::Length(b)),
            ) => Self::Auto(LengthPercentageAuto::Length(length(a, b, progress))),
            (Self::Size(Size::Length(a)), Self::Size(Size::Length(b))) => {
                Self::Size(Size::Length(length(a, b, progress)))
            }
            (Self::MaxSize(MaxSize::Length(a)), Self::MaxSize(MaxSize::Length(b))) => {
                Self::MaxSize(MaxSize::Length(length(a, b, progress)))
            }
            (Self::Radius(a), Self::Radius(b)) => Self::Radius(radius(a, b, progress)),
            (Self::Shadows(a), Self::Shadows(b)) => Self::Shadows(shadows(a, b, progress)?),
            (Self::Filters(a), Self::Filters(b)) => Self::Filters(filters(a, b, progress)?),
            (Self::Transform(a), Self::Transform(b)) => Self::Transform(transform(a, b, progress)),
            (
                Self::OptionalColor(OptionalColor(Some(a))),
                Self::OptionalColor(OptionalColor(Some(b))),
            ) => Self::OptionalColor(OptionalColor(Some(color(*a, *b, progress)))),
            (Self::LineHeight(LineHeight::Number(a)), Self::LineHeight(LineHeight::Number(b))) => {
                Self::LineHeight(LineHeight::Number(mix(*a, *b, progress)))
            }
            (Self::LineHeight(LineHeight::Px(a)), Self::LineHeight(LineHeight::Px(b))) => {
                Self::LineHeight(LineHeight::Px(mix(*a, *b, progress)))
            }
            (Self::Weight(a), Self::Weight(b)) => Self::Weight(FontWeight(
                crate::math::round(mix(a.0 as f32, b.0 as f32, progress)).clamp(1.0, 1000.0) as u16,
            )),
            (Self::Visibility(a), Self::Visibility(b))
                if *a == Visibility::Visible || *b == Visibility::Visible =>
            {
                Self::Visibility(if progress <= 0.0 {
                    *a
                } else if progress >= 1.0 {
                    *b
                } else {
                    Visibility::Visible
                })
            }
            (Self::Paint(Paint::Color(a)), Self::Paint(Paint::Color(b))) => {
                Self::Paint(Paint::Color(color(*a, *b, progress)))
            }
            (Self::Integer(a), Self::Integer(b)) => {
                Self::Integer(crate::math::round(mix(*a as f32, *b as f32, progress)) as i32)
            }
            (
                Self::BackgroundSize(BackgroundSize::Explicit(a_width, a_height)),
                Self::BackgroundSize(BackgroundSize::Explicit(b_width, b_height)),
            ) => Self::BackgroundSize(BackgroundSize::Explicit(
                optional_length(a_width, b_width, progress)?,
                optional_length(a_height, b_height, progress)?,
            )),
            (Self::Position(a), Self::Position(b)) => Self::Position(position(a, b, progress)),
            (
                Self::VerticalAlign(VerticalAlign::Length(a)),
                Self::VerticalAlign(VerticalAlign::Length(b)),
            ) => Self::VerticalAlign(VerticalAlign::Length(length(a, b, progress))),
            (Self::Spacing(a), Self::Spacing(b)) => Self::Spacing(Spacing {
                horizontal: Px(mix(a.horizontal.0, b.horizontal.0, progress).max(0.0)),
                vertical: Px(mix(a.vertical.0, b.vertical.0, progress).max(0.0)),
            }),
            (Self::TabSize(TabSize::Spaces(a)), Self::TabSize(TabSize::Spaces(b))) => {
                Self::TabSize(TabSize::Spaces(mix(*a, *b, progress).max(0.0)))
            }
            (Self::TabSize(TabSize::Length(a)), Self::TabSize(TabSize::Length(b))) => {
                Self::TabSize(TabSize::Length(mix(*a, *b, progress).max(0.0)))
            }
            (
                Self::ScrollbarColor(ScrollbarColor(Some((a_thumb, a_track)))),
                Self::ScrollbarColor(ScrollbarColor(Some((b_thumb, b_track)))),
            ) => Self::ScrollbarColor(ScrollbarColor(Some((
                color(*a_thumb, *b_thumb, progress),
                color(*a_track, *b_track, progress),
            )))),
            // Ratios interpolate by their logarithms, as CSS specifies.
            (Self::AspectRatio(AspectRatio(Some(a))), Self::AspectRatio(AspectRatio(Some(b))))
                if *a > 0.0 && *b > 0.0 =>
            {
                Self::AspectRatio(AspectRatio(Some(libm::expf(mix(
                    libm::logf(*a),
                    libm::logf(*b),
                    progress,
                )))))
            }
            (Self::Origin(a), Self::Origin(b)) => Self::Origin(TransformOrigin {
                x: length(&a.x, &b.x, progress),
                y: length(&a.y, &b.y, progress),
                z: Px(mix(a.z.0, b.z.0, progress)),
            }),
            // Edges that are `auto` on one side only change discretely.
            (Self::Clip(Some(a)), Self::Clip(Some(b))) => {
                let edge = |a: Option<f32>, b: Option<f32>| match (a, b) {
                    (Some(a), Some(b)) => Some(Some(mix(a, b, progress))),
                    (None, None) => Some(None),
                    _ => None,
                };
                Self::Clip(Some(ClipRect {
                    top: edge(a.top, b.top)?,
                    right: edge(a.right, b.right)?,
                    bottom: edge(a.bottom, b.bottom)?,
                    left: edge(a.left, b.left)?,
                }))
            }
            (
                Self::ClipPath(ClipPath::Shape(a, a_box)),
                Self::ClipPath(ClipPath::Shape(b, b_box)),
            ) if a_box == b_box => Self::ClipPath(ClipPath::Shape(shape(a, b, progress)?, *a_box)),
            (Self::Tracks(a), Self::Tracks(b)) => Self::Tracks(track_list(a, b, progress)?),
            // Images cross-fade, as Chromium blends them; `none` changes
            // discretely.
            (Self::Image(a), Self::Image(b))
                if *a != BackgroundImage::None && *b != BackgroundImage::None =>
            {
                Self::Image(if progress <= 0.0 {
                    a.clone()
                } else if progress >= 1.0 {
                    b.clone()
                } else {
                    BackgroundImage::CrossFade {
                        from: Box::new(a.clone()),
                        to: Box::new(b.clone()),
                        progress,
                    }
                })
            }
            // Integers round to the nearest; `auto` changes discretely.
            (Self::ZIndex(ZIndex(Some(a))), Self::ZIndex(ZIndex(Some(b)))) => Self::ZIndex(ZIndex(
                Some(crate::math::round(mix(*a as f32, *b as f32, progress)) as i32),
            )),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests;
