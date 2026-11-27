//! Shorthand expansion and property aliases.

use crate::prelude::*;

use crate::computed::{ComputedStyle, PropertyId};
use crate::tokenizer::Token;
use crate::values::{ComponentValue, parse_component_values, split_commas, without_whitespace};
use PropertyId as P;

const MARGIN: [P; 4] = [P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft];
const PADDING: [P; 4] = [
    P::PaddingTop,
    P::PaddingRight,
    P::PaddingBottom,
    P::PaddingLeft,
];
const INSET: [P; 4] = [P::Top, P::Right, P::Bottom, P::Left];
const SCROLL_MARGIN: [P; 4] = [
    P::ScrollMarginTop,
    P::ScrollMarginRight,
    P::ScrollMarginBottom,
    P::ScrollMarginLeft,
];
const SCROLL_PADDING: [P; 4] = [
    P::ScrollPaddingTop,
    P::ScrollPaddingRight,
    P::ScrollPaddingBottom,
    P::ScrollPaddingLeft,
];
const BORDER_WIDTH: [P; 4] = [
    P::BorderTopWidth,
    P::BorderRightWidth,
    P::BorderBottomWidth,
    P::BorderLeftWidth,
];
const BORDER_STYLE: [P; 4] = [
    P::BorderTopStyle,
    P::BorderRightStyle,
    P::BorderBottomStyle,
    P::BorderLeftStyle,
];
const BORDER_COLOR: [P; 4] = [
    P::BorderTopColor,
    P::BorderRightColor,
    P::BorderBottomColor,
    P::BorderLeftColor,
];
const RADIUS: [P; 4] = [
    P::BorderTopLeftRadius,
    P::BorderTopRightRadius,
    P::BorderBottomRightRadius,
    P::BorderBottomLeftRadius,
];

/// Maps logical and legacy property names to a physical longhand.
pub fn alias(name: &str) -> Option<PropertyId> {
    Some(match name {
        "margin-inline-start" => P::MarginLeft,
        "-moz-tab-size" | "-o-tab-size" => P::TabSize,
        "margin-inline-end" => P::MarginRight,
        "margin-block-start" => P::MarginTop,
        "margin-block-end" => P::MarginBottom,
        "padding-inline-start" => P::PaddingLeft,
        "padding-inline-end" => P::PaddingRight,
        "padding-block-start" => P::PaddingTop,
        "scroll-margin-block-start" => P::ScrollMarginTop,
        "scroll-margin-block-end" => P::ScrollMarginBottom,
        "scroll-margin-inline-start" => P::ScrollMarginLeft,
        "scroll-margin-inline-end" => P::ScrollMarginRight,
        "scroll-padding-block-start" => P::ScrollPaddingTop,
        "scroll-padding-block-end" => P::ScrollPaddingBottom,
        "scroll-padding-inline-start" => P::ScrollPaddingLeft,
        "scroll-padding-inline-end" => P::ScrollPaddingRight,
        "padding-block-end" => P::PaddingBottom,
        "inset-inline-start" => P::Left,
        "inset-inline-end" => P::Right,
        "inset-block-start" => P::Top,
        "inset-block-end" => P::Bottom,
        "border-inline-start-width" => P::BorderLeftWidth,
        "border-inline-end-width" => P::BorderRightWidth,
        "border-block-start-width" => P::BorderTopWidth,
        "border-block-end-width" => P::BorderBottomWidth,
        "border-inline-start-style" => P::BorderLeftStyle,
        "border-inline-end-style" => P::BorderRightStyle,
        "border-block-start-style" => P::BorderTopStyle,
        "border-block-end-style" => P::BorderBottomStyle,
        "border-inline-start-color" => P::BorderLeftColor,
        "border-inline-end-color" => P::BorderRightColor,
        "border-block-start-color" => P::BorderTopColor,
        "border-block-end-color" => P::BorderBottomColor,
        "border-start-start-radius" => P::BorderTopLeftRadius,
        "border-start-end-radius" => P::BorderTopRightRadius,
        "border-end-end-radius" => P::BorderBottomRightRadius,
        "border-end-start-radius" => P::BorderBottomLeftRadius,
        "inline-size" => P::Width,
        "block-size" => P::Height,
        "min-inline-size" => P::MinWidth,
        "min-block-size" => P::MinHeight,
        "max-inline-size" => P::MaxWidth,
        "max-block-size" => P::MaxHeight,
        "word-wrap" => P::OverflowWrap,
        // Chromium's older names of the flow-relative margins and paddings.
        "-webkit-margin-before" => P::MarginTop,
        "-webkit-margin-after" => P::MarginBottom,
        "-webkit-margin-start" => P::MarginLeft,
        "-webkit-margin-end" => P::MarginRight,
        "-webkit-padding-before" => P::PaddingTop,
        "-webkit-padding-after" => P::PaddingBottom,
        "-webkit-padding-start" => P::PaddingLeft,
        "-webkit-padding-end" => P::PaddingRight,
        "grid-column-gap" => P::ColumnGap,
        "grid-row-gap" => P::RowGap,
        _ => return None,
    })
}

/// Removes a vendor prefix if the unprefixed name is known.
pub fn unprefixed(name: &str) -> &str {
    for prefix in ["-webkit-", "-moz-", "-ms-"] {
        if let Some(rest) = name.strip_prefix(prefix)
            && (PropertyId::from_name(rest).is_some()
                || alias(rest).is_some()
                || longhands_of(rest).is_some())
        {
            return rest;
        }
    }
    name
}

/// The longhands a shorthand sets, or `None` if `name` is not a shorthand.
pub fn longhands_of(name: &str) -> Option<Vec<PropertyId>> {
    let list: &[P] = match name {
        "margin" => &MARGIN,
        "padding" => &PADDING,
        "inset" => &INSET,
        "scroll-margin" => &SCROLL_MARGIN,
        "scroll-padding" => &SCROLL_PADDING,
        "scroll-margin-block" => &[P::ScrollMarginTop, P::ScrollMarginBottom],
        "scroll-margin-inline" => &[P::ScrollMarginLeft, P::ScrollMarginRight],
        "scroll-padding-block" => &[P::ScrollPaddingTop, P::ScrollPaddingBottom],
        "scroll-padding-inline" => &[P::ScrollPaddingLeft, P::ScrollPaddingRight],
        "border-width" => &BORDER_WIDTH,
        "border-style" => &BORDER_STYLE,
        "border-color" => &BORDER_COLOR,
        "border-radius" => &RADIUS,
        "margin-inline" => &[P::MarginLeft, P::MarginRight],
        "margin-block" => &[P::MarginTop, P::MarginBottom],
        "padding-inline" => &[P::PaddingLeft, P::PaddingRight],
        "padding-block" => &[P::PaddingTop, P::PaddingBottom],
        "inset-inline" => &[P::Left, P::Right],
        "inset-block" => &[P::Top, P::Bottom],
        "border-inline-width" => &[P::BorderLeftWidth, P::BorderRightWidth],
        "border-inline-style" => &[P::BorderLeftStyle, P::BorderRightStyle],
        "border-inline-color" => &[P::BorderLeftColor, P::BorderRightColor],
        "border-block-width" => &[P::BorderTopWidth, P::BorderBottomWidth],
        "border-block-style" => &[P::BorderTopStyle, P::BorderBottomStyle],
        "border-block-color" => &[P::BorderTopColor, P::BorderBottomColor],
        "border" => &[
            P::BorderTopWidth,
            P::BorderRightWidth,
            P::BorderBottomWidth,
            P::BorderLeftWidth,
            P::BorderTopStyle,
            P::BorderRightStyle,
            P::BorderBottomStyle,
            P::BorderLeftStyle,
            P::BorderTopColor,
            P::BorderRightColor,
            P::BorderBottomColor,
            P::BorderLeftColor,
        ],
        "border-top" | "border-block-start" => {
            &[P::BorderTopWidth, P::BorderTopStyle, P::BorderTopColor]
        }
        "border-right" | "border-inline-end" => &[
            P::BorderRightWidth,
            P::BorderRightStyle,
            P::BorderRightColor,
        ],
        "border-bottom" | "border-block-end" => &[
            P::BorderBottomWidth,
            P::BorderBottomStyle,
            P::BorderBottomColor,
        ],
        "border-left" | "border-inline-start" => {
            &[P::BorderLeftWidth, P::BorderLeftStyle, P::BorderLeftColor]
        }
        "border-inline" => &[
            P::BorderLeftWidth,
            P::BorderLeftStyle,
            P::BorderLeftColor,
            P::BorderRightWidth,
            P::BorderRightStyle,
            P::BorderRightColor,
        ],
        "border-block" => &[
            P::BorderTopWidth,
            P::BorderTopStyle,
            P::BorderTopColor,
            P::BorderBottomWidth,
            P::BorderBottomStyle,
            P::BorderBottomColor,
        ],
        "outline" => &[P::OutlineWidth, P::OutlineStyle, P::OutlineColor],
        "transition" => &[
            P::TransitionProperty,
            P::TransitionDuration,
            P::TransitionTimingFunction,
            P::TransitionDelay,
        ],
        "animation" => &ANIMATION,
        "background" => &[
            P::BackgroundColor,
            P::BackgroundImage,
            P::BackgroundRepeat,
            P::BackgroundPosition,
            P::BackgroundSize,
        ],
        "mask" => &[P::MaskImage, P::MaskRepeat, P::MaskPosition, P::MaskSize],
        "flex" => &[P::FlexGrow, P::FlexShrink, P::FlexBasis],
        "flex-flow" => &[P::FlexDirection, P::FlexWrap],
        "gap" | "grid-gap" => &[P::RowGap, P::ColumnGap],
        "place-items" => &[P::AlignItems, P::JustifyItems],
        "place-content" => &[P::AlignContent, P::JustifyContent],
        "place-self" => &[P::AlignSelf, P::JustifySelf],
        "overflow" => &[P::OverflowX, P::OverflowY],
        "container" => &[P::ContainerName, P::ContainerType],
        "marker" => &[P::MarkerStart, P::MarkerMid, P::MarkerEnd],
        "font" => &[
            P::FontStyle,
            P::FontWeight,
            P::FontSize,
            P::LineHeight,
            P::FontFamily,
        ],
        "text-decoration" => &[P::TextDecorationLine, P::TextDecorationColor],
        "list-style" => &[P::ListStyleType, P::ListStylePosition],
        "grid-column" => &[P::GridColumnStart, P::GridColumnEnd],
        "grid-row" => &[P::GridRowStart, P::GridRowEnd],
        "grid-area" => &[
            P::GridRowStart,
            P::GridColumnStart,
            P::GridRowEnd,
            P::GridColumnEnd,
        ],
        _ => return None,
    };
    Some(list.to_vec())
}

type Longhands = Vec<(PropertyId, Vec<ComponentValue>)>;

fn text(source: &str) -> Vec<ComponentValue> {
    parse_component_values(source)
}

fn one(value: &ComponentValue) -> Vec<ComponentValue> {
    vec![value.clone()]
}

fn valid(id: PropertyId, value: &ComponentValue) -> bool {
    ComputedStyle::is_valid(id, core::slice::from_ref(value))
}

/// Assigns 1–4 values to sides in top, right, bottom, left order.
fn four(ids: [PropertyId; 4], values: &[ComponentValue]) -> Option<Longhands> {
    let [top, right, bottom, left] = match values {
        [all] => [all, all, all, all],
        [vertical, horizontal] => [vertical, horizontal, vertical, horizontal],
        [top, horizontal, bottom] => [top, horizontal, bottom, horizontal],
        [top, right, bottom, left] => [top, right, bottom, left],
        _ => return None,
    };
    Some(
        ids.into_iter()
            .zip([top, right, bottom, left])
            .map(|(id, value)| (id, one(value)))
            .collect(),
    )
}

/// Assigns one or two values to a start and an end longhand.
fn two(ids: [PropertyId; 2], values: &[ComponentValue]) -> Option<Longhands> {
    let [first, second] = match values {
        [both] => [both, both],
        [first, second] => [first, second],
        _ => return None,
    };
    Some(vec![(ids[0], one(first)), (ids[1], one(second))])
}

/// Classifies `width || style || color` components of a border-like shorthand.
fn border_parts(values: &[ComponentValue]) -> Option<[Vec<ComponentValue>; 3]> {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    for value in values {
        if style.is_none() && valid(P::BorderTopStyle, value) {
            style = Some(one(value));
        } else if width.is_none() && valid(P::BorderTopWidth, value) {
            width = Some(one(value));
        } else if color.is_none() && valid(P::BorderTopColor, value) {
            color = Some(one(value));
        } else {
            return None;
        }
    }
    if values.is_empty() {
        return None;
    }
    Some([
        width.unwrap_or_else(|| text("medium")),
        style.unwrap_or_else(|| text("none")),
        color.unwrap_or_else(|| text("currentcolor")),
    ])
}

/// Expands a shorthand into longhand values; `None` means the value is invalid.
pub fn expand(name: &str, values: &[ComponentValue]) -> Option<Longhands> {
    let parts = without_whitespace(values);
    let ids = longhands_of(name)?;
    match name {
        "margin" | "padding" | "inset" | "border-width" | "border-style" | "border-color"
        | "scroll-margin" | "scroll-padding" => four([ids[0], ids[1], ids[2], ids[3]], &parts),
        "margin-inline"
        | "margin-block"
        | "padding-inline"
        | "padding-block"
        | "inset-inline"
        | "inset-block"
        | "border-inline-width"
        | "border-inline-style"
        | "border-inline-color"
        | "border-block-width"
        | "border-block-style"
        | "border-block-color"
        | "gap"
        | "grid-gap"
        | "place-items"
        | "place-content"
        | "place-self"
        | "scroll-margin-block"
        | "scroll-margin-inline"
        | "scroll-padding-block"
        | "scroll-padding-inline" => two([ids[0], ids[1]], &parts),
        "overflow" => two([P::OverflowX, P::OverflowY], &parts),
        // `container: <name> [/ <type>]`; the type defaults to `normal`.
        "container" => {
            let mut halves = parts.split(|part| *part == ComponentValue::Token(Token::Delim('/')));
            let name = halves.next().unwrap_or_default();
            let kind = halves.next();
            if name.is_empty() || halves.next().is_some() || kind.is_some_and(<[_]>::is_empty) {
                return None;
            }
            Some(vec![
                (P::ContainerName, name.to_vec()),
                (
                    P::ContainerType,
                    kind.map_or_else(|| text("normal"), <[_]>::to_vec),
                ),
            ])
        }
        "marker" => Some(ids.iter().map(|id| (*id, parts.clone())).collect()),
        "border-radius" => border_radius(&parts),
        "border" | "border-inline" | "border-block" => {
            let [width, style, color] = border_parts(&parts)?;
            let sides = ids.len() / 3;
            let mut longhands = Vec::new();
            for side in 0..sides {
                let group = if name == "border" {
                    [ids[side], ids[side + 4], ids[side + 8]]
                } else {
                    [ids[side * 3], ids[side * 3 + 1], ids[side * 3 + 2]]
                };
                longhands.push((group[0], width.clone()));
                longhands.push((group[1], style.clone()));
                longhands.push((group[2], color.clone()));
            }
            Some(longhands)
        }
        "border-top"
        | "border-right"
        | "border-bottom"
        | "border-left"
        | "border-block-start"
        | "border-block-end"
        | "border-inline-start"
        | "border-inline-end" => {
            let [width, style, color] = border_parts(&parts)?;
            Some(vec![(ids[0], width), (ids[1], style), (ids[2], color)])
        }
        "outline" => {
            let [width, style, color] = border_parts(&parts)?;
            let color = if color == text("currentcolor") {
                text("auto")
            } else {
                color
            };
            Some(vec![(ids[0], width), (ids[1], style), (ids[2], color)])
        }
        "background" => background(values),
        "mask" => mask(values),
        "transition" => transition(values),
        "animation" => animation(values),
        "flex" => flex(&parts),
        "flex-flow" => {
            let mut direction = text("row");
            let mut wrap = text("nowrap");
            for value in &parts {
                if valid(P::FlexDirection, value) {
                    direction = one(value);
                } else if valid(P::FlexWrap, value) {
                    wrap = one(value);
                } else {
                    return None;
                }
            }
            Some(vec![(P::FlexDirection, direction), (P::FlexWrap, wrap)])
        }
        "font" => font(&parts),
        "text-decoration" => {
            let mut line = Vec::new();
            let mut color = text("currentcolor");
            for value in &parts {
                let is_style = value.ident().is_some_and(|word| {
                    matches!(
                        word.as_str(),
                        "solid" | "double" | "dotted" | "dashed" | "wavy"
                    )
                });
                if is_style {
                    continue;
                }
                if valid(P::TextDecorationLine, value) {
                    line.push(value.clone());
                    line.push(ComponentValue::Token(Token::Whitespace));
                } else if valid(P::TextDecorationColor, value) {
                    color = one(value);
                } else {
                    return None;
                }
            }
            if line.is_empty() {
                line = text("none");
            }
            Some(vec![
                (P::TextDecorationLine, line),
                (P::TextDecorationColor, color),
            ])
        }
        "list-style" => {
            let mut kind = None;
            let mut position = None;
            let mut image = false;
            let mut none = 0;
            for value in &parts {
                let word = value.ident();
                match word.as_deref() {
                    Some("none") => none += 1,
                    Some("inside" | "outside") => {
                        if position.is_some() {
                            return None;
                        }
                        position = Some(one(value));
                    }
                    _ if valid(P::ListStyleType, value) => {
                        if kind.is_some() {
                            return None;
                        }
                        kind = Some(one(value));
                    }
                    _ if matches!(value, ComponentValue::Function(name, _) if name == "url")
                        || matches!(value, ComponentValue::Token(Token::Url(_))) =>
                    {
                        if image {
                            return None;
                        }
                        image = true;
                    }
                    _ => return None,
                }
            }
            // `none` can fill the type, the image, or both when neither is
            // explicit. In `disc outside none`, it fills only the image.
            if none > usize::from(kind.is_none()) + usize::from(!image) {
                return None;
            }
            Some(vec![
                (
                    P::ListStyleType,
                    kind.unwrap_or_else(|| text(if none > 0 { "none" } else { "disc" })),
                ),
                (
                    P::ListStylePosition,
                    position.unwrap_or_else(|| text("outside")),
                ),
            ])
        }
        "grid-column" | "grid-row" => {
            let lines = split_slashes(&parts);
            let (start, end) = match lines.as_slice() {
                [start] => (start.clone(), text("auto")),
                [start, end] => (start.clone(), end.clone()),
                _ => return None,
            };
            Some(vec![(ids[0], start), (ids[1], end)])
        }
        _ => {
            let lines = split_slashes(&parts);
            if lines.len() > 4 {
                return None;
            }
            Some(
                ids.into_iter()
                    .enumerate()
                    .map(|(index, id)| {
                        (
                            id,
                            lines.get(index).cloned().unwrap_or_else(|| text("auto")),
                        )
                    })
                    .collect(),
            )
        }
    }
}

fn split_slashes(values: &[ComponentValue]) -> Vec<Vec<ComponentValue>> {
    values
        .split(|value| matches!(value, ComponentValue::Token(Token::Delim('/'))))
        .map(|part| {
            let mut spaced = Vec::new();
            for value in part {
                if !spaced.is_empty() {
                    spaced.push(ComponentValue::Token(Token::Whitespace));
                }
                spaced.push(value.clone());
            }
            spaced
        })
        .collect()
}

fn border_radius(values: &[ComponentValue]) -> Option<Longhands> {
    let groups = values
        .split(|value| matches!(value, ComponentValue::Token(Token::Delim('/'))))
        .collect::<Vec<_>>();
    let (horizontal, vertical) = match groups.as_slice() {
        [horizontal] => (*horizontal, *horizontal),
        [horizontal, vertical] => (*horizontal, *vertical),
        _ => return None,
    };
    let horizontal = four(RADIUS, horizontal)?;
    let vertical = four(RADIUS, vertical)?;
    Some(
        horizontal
            .into_iter()
            .zip(vertical)
            .map(|((id, mut x), (_, y))| {
                x.push(ComponentValue::Token(Token::Whitespace));
                x.extend(y);
                (id, x)
            })
            .collect(),
    )
}

fn background(values: &[ComponentValue]) -> Option<Longhands> {
    let layers = split_commas(values);
    let mut color = text("transparent");
    let mut image = text("none");
    let (mut repeat, mut position, mut size) = (Vec::new(), Vec::new(), Vec::new());
    let is_repeat = |value: &ComponentValue| {
        [
            "repeat",
            "no-repeat",
            "repeat-x",
            "repeat-y",
            "space",
            "round",
        ]
        .iter()
        .any(|name| value.is_ident(name))
    };
    let ignored = |value: &ComponentValue| {
        [
            "scroll",
            "fixed",
            "local",
            "border-box",
            "padding-box",
            "content-box",
            "text",
        ]
        .iter()
        .any(|name| value.is_ident(name))
    };
    for (index, layer) in layers.iter().enumerate() {
        let parts = without_whitespace(layer);
        let first = index == 0;
        let mut sizing = false;
        for value in &parts {
            if matches!(value, ComponentValue::Token(Token::Delim('/'))) {
                if position.is_empty() || sizing {
                    return None;
                }
                sizing = true;
            } else if sizing && (size.len() < 2) && valid(P::BackgroundSize, value) {
                if first {
                    size.push(value.clone());
                }
            } else if index == layers.len() - 1 && valid(P::BackgroundColor, value) {
                color = one(value);
                sizing = false;
            } else if valid(P::BackgroundImage, value) {
                if first {
                    image = one(value);
                }
                sizing = false;
            } else if is_repeat(value) {
                if first {
                    repeat.push(value.clone());
                }
                sizing = false;
            } else if ignored(value) {
                sizing = false;
            } else if !sizing {
                if first {
                    position.push(value.clone());
                }
            } else {
                return None;
            }
        }
    }
    let spaced = |values: Vec<ComponentValue>| {
        let mut spaced = Vec::new();
        for value in values {
            if !spaced.is_empty() {
                spaced.push(ComponentValue::Token(Token::Whitespace));
            }
            spaced.push(value);
        }
        spaced
    };
    let (repeat, position, size) = (spaced(repeat), spaced(position), spaced(size));
    let checked = |id: PropertyId, values: Vec<ComponentValue>, default: &str| {
        if values.is_empty() {
            Some(text(default))
        } else {
            ComputedStyle::is_valid(id, &values).then_some(values)
        }
    };
    Some(vec![
        (P::BackgroundColor, color),
        (P::BackgroundImage, image),
        (
            P::BackgroundRepeat,
            checked(P::BackgroundRepeat, repeat, "repeat")?,
        ),
        (
            P::BackgroundPosition,
            checked(P::BackgroundPosition, position, "0% 0%")?,
        ),
        (P::BackgroundSize, checked(P::BackgroundSize, size, "auto")?),
    ])
}

/// The `mask` shorthand: a background's image, position, size and repeat;
/// its mode, origin, clip and compositing keywords take their defaults.
fn mask(values: &[ComponentValue]) -> Option<Longhands> {
    const DEFAULTS: [&str; 13] = [
        "alpha",
        "luminance",
        "match-source",
        "add",
        "subtract",
        "intersect",
        "exclude",
        "no-clip",
        "border-box",
        "padding-box",
        "content-box",
        "fill-box",
        "stroke-box",
    ];
    let values: Vec<ComponentValue> = values
        .iter()
        .filter(|value| {
            !DEFAULTS.iter().any(|name| value.is_ident(name)) && !value.is_ident("view-box")
        })
        .cloned()
        .collect();
    let longhands = background(&values)?;
    Some(
        longhands
            .into_iter()
            .filter_map(|(id, value)| {
                let id = match id {
                    P::BackgroundImage => P::MaskImage,
                    P::BackgroundRepeat => P::MaskRepeat,
                    P::BackgroundPosition => P::MaskPosition,
                    P::BackgroundSize => P::MaskSize,
                    _ => return None,
                };
                Some((id, value))
            })
            .collect(),
    )
}

fn flex(values: &[ComponentValue]) -> Option<Longhands> {
    let numbers: Vec<&ComponentValue> = values
        .iter()
        .filter(|value| value.number().is_some())
        .collect();
    let others: Vec<&ComponentValue> = values
        .iter()
        .filter(|value| value.number().is_none())
        .collect();
    let result =
        |grow: Vec<ComponentValue>, shrink: Vec<ComponentValue>, basis: Vec<ComponentValue>| {
            Some(vec![
                (P::FlexGrow, grow),
                (P::FlexShrink, shrink),
                (P::FlexBasis, basis),
            ])
        };
    match (numbers.as_slice(), others.as_slice()) {
        ([], [keyword]) if keyword.is_ident("none") => result(text("0"), text("0"), text("auto")),
        ([], [keyword]) if keyword.is_ident("auto") => result(text("1"), text("1"), text("auto")),
        ([grow], []) => result(one(grow), text("1"), text("0%")),
        ([grow, shrink], []) => result(one(grow), one(shrink), text("0%")),
        ([grow, shrink, basis], []) if valid(P::FlexBasis, basis) => {
            result(one(grow), one(shrink), one(basis))
        }
        ([], [basis]) if valid(P::FlexBasis, basis) => result(text("1"), text("1"), one(basis)),
        ([grow], [basis]) if valid(P::FlexBasis, basis) => result(one(grow), text("1"), one(basis)),
        ([grow, shrink], [basis]) if valid(P::FlexBasis, basis) => {
            result(one(grow), one(shrink), one(basis))
        }
        _ => None,
    }
}

fn font(values: &[ComponentValue]) -> Option<Longhands> {
    if let [keyword] = values
        && keyword.ident().is_some_and(|word| {
            matches!(
                word.as_str(),
                "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar"
            )
        })
    {
        return Some(vec![
            (P::FontStyle, text("normal")),
            (P::FontWeight, text("normal")),
            (P::FontSize, text("12px")),
            (P::LineHeight, text("normal")),
            (P::FontFamily, text("\"Segoe UI\"")),
        ]);
    }
    let mut style = text("normal");
    let mut weight = text("normal");
    let mut index = 0;
    while let Some(value) = values.get(index) {
        let word = value.ident();
        match word.as_deref() {
            Some("normal") => {}
            Some(
                "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed"
                | "semi-condensed" | "semi-expanded" | "expanded" | "extra-expanded"
                | "ultra-expanded",
            ) => {}
            _ if valid(P::FontStyle, value) => style = one(value),
            _ if word.is_some() && valid(P::FontWeight, value) => weight = one(value),
            _ if value.number().is_some() && valid(P::FontWeight, value) => weight = one(value),
            _ => break,
        }
        index += 1;
    }
    let size = values.get(index)?;
    if !valid(P::FontSize, size) {
        return None;
    }
    index += 1;
    let mut line_height = text("normal");
    if matches!(
        values.get(index),
        Some(ComponentValue::Token(Token::Delim('/')))
    ) {
        let value = values.get(index + 1)?;
        if !valid(P::LineHeight, value) {
            return None;
        }
        line_height = one(value);
        index += 2;
    }
    let family: Vec<ComponentValue> = values[index..]
        .iter()
        .flat_map(|value| [value.clone(), ComponentValue::Token(Token::Whitespace)])
        .collect();
    if !ComputedStyle::is_valid(P::FontFamily, &family) {
        return None;
    }
    Some(vec![
        (P::FontStyle, style),
        (P::FontWeight, weight),
        (P::FontSize, one(size)),
        (P::LineHeight, line_height),
        (P::FontFamily, family),
    ])
}

#[cfg(test)]
mod tests;

/// The longhands of `animation`, in the order its expansion lists them.
const ANIMATION: [P; 8] = [
    P::AnimationName,
    P::AnimationDuration,
    P::AnimationTimingFunction,
    P::AnimationDelay,
    P::AnimationIterationCount,
    P::AnimationDirection,
    P::AnimationFillMode,
    P::AnimationPlayState,
];

/// `animation`: per layer, the first time is the duration and the second
/// the delay; keywords of the other longhands go to them while each is
/// unset, and the remaining identifier names the animation.
fn animation(values: &[ComponentValue]) -> Option<Longhands> {
    const DEFAULTS: [&str; 8] = ["none", "0s", "ease", "0s", "1", "normal", "none", "running"];
    let mut lists: [Vec<ComponentValue>; 8] = Default::default();
    for part in split_commas(values) {
        if part.is_empty() {
            return None;
        }
        let mut fields: [Option<Vec<ComponentValue>>; 8] = Default::default();
        for value in part {
            let time = matches!(&value, ComponentValue::Token(Token::Dimension { unit, .. })
                if unit.eq_ignore_ascii_case("s") || unit.eq_ignore_ascii_case("ms"));
            let keyword = |id: P| {
                fields[ANIMATION.iter().position(|&each| each == id).unwrap()].is_none()
                    && valid(id, &value)
            };
            let index = if time {
                if fields[1].is_none() { 1 } else { 3 }
            } else if keyword(P::AnimationTimingFunction) {
                2
            } else if keyword(P::AnimationIterationCount) {
                4
            } else if keyword(P::AnimationDirection) {
                5
            } else if keyword(P::AnimationFillMode) {
                6
            } else if keyword(P::AnimationPlayState) {
                7
            } else {
                0
            };
            if fields[index].is_some() {
                return None;
            }
            fields[index] = Some(one(&value));
        }
        for ((list, field), default) in lists.iter_mut().zip(fields).zip(DEFAULTS) {
            if !list.is_empty() {
                list.push(ComponentValue::Token(Token::Comma));
            }
            list.extend(field.unwrap_or_else(|| text(default)));
        }
    }
    let expanded: Longhands = ANIMATION.into_iter().zip(lists).collect();
    expanded
        .iter()
        .all(|(id, values)| ComputedStyle::is_valid(*id, values))
        .then_some(expanded)
}

fn transition(values: &[ComponentValue]) -> Option<Longhands> {
    let mut lists: [Vec<ComponentValue>; 4] = Default::default();
    for part in split_commas(values) {
        if part.is_empty() {
            return None;
        }
        let mut fields: [Option<Vec<ComponentValue>>; 4] = Default::default();
        for value in part {
            let index = if matches!(value,ComponentValue::Token(Token::Dimension{ref unit,..}) if unit.eq_ignore_ascii_case("s")||unit.eq_ignore_ascii_case("ms"))
            {
                if fields[1].is_none() { 1 } else { 3 }
            } else if valid(P::TransitionTimingFunction, &value) {
                2
            } else {
                0
            };
            if fields[index].is_some() {
                return None;
            }
            fields[index] = Some(one(&value));
        }
        for ((list, field), default) in lists
            .iter_mut()
            .zip(fields)
            .zip(["all", "0s", "ease", "0s"])
        {
            if !list.is_empty() {
                list.push(ComponentValue::Token(Token::Comma));
            }
            list.extend(field.unwrap_or_else(|| text(default)));
        }
    }
    let ids = [
        P::TransitionProperty,
        P::TransitionDuration,
        P::TransitionTimingFunction,
        P::TransitionDelay,
    ];
    let expanded: Longhands = ids.into_iter().zip(lists).collect();
    expanded
        .iter()
        .all(|(id, values)| ComputedStyle::is_valid(*id, values))
        .then_some(expanded)
}
