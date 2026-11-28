//! Parsers turning declared values into computed values.

use crate::prelude::*;

use crate::color::{Color, SpecifiedColor, SystemColors, parse_color};
use crate::tokenizer::Token;
use crate::types::*;
use crate::values::{
    ComponentValue, LengthContext, LengthPercentage, length_percentage, non_negative, number,
    split_commas, without_whitespace,
};

/// What computed values depend on.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// Lengths resolve against the element's font size (the parent's for `font-size`).
    pub length: LengthContext,
    /// The parent's font size in pixels.
    pub parent_font_size: f32,
    /// The parent's font weight.
    pub parent_font_weight: u16,
    /// The element's `color`, used for `currentcolor`.
    pub color: Color,
    /// Theme colors for system color keywords.
    pub system: &'a SystemColors,
}

/// The single non-whitespace component value.
pub fn single(values: &[ComponentValue]) -> Option<&ComponentValue> {
    let mut found = None;
    for value in values {
        if matches!(value, ComponentValue::Token(Token::Whitespace)) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(value);
    }
    found
}

fn single_ident(values: &[ComponentValue]) -> Option<String> {
    single(values)?.ident()
}

/// Parses a keyword enum value.
pub fn keyword<K: Keyword>(values: &[ComponentValue], _: &Context) -> Option<K> {
    K::parse_keyword(&single_ident(values)?)
}

/// `display`, including the two-keyword syntax.
pub fn display(values: &[ComponentValue], _: &Context) -> Option<Display> {
    let words: Option<Vec<String>> = without_whitespace(values)
        .iter()
        .map(ComponentValue::ident)
        .collect();
    let words = words?;
    match words.as_slice() {
        [word] => Display::from_keyword(word),
        [outer, inner] => {
            let inline = match outer.as_str() {
                "inline" => true,
                "block" => false,
                _ => return None,
            };
            Some(match (inline, inner.as_str()) {
                (true, "flow") => Display::Inline,
                (false, "flow") => Display::Block,
                (true, "flow-root") => Display::InlineBlock,
                (false, "flow-root") => Display::FlowRoot,
                (true, "flex") => Display::InlineFlex,
                (false, "flex") => Display::Flex,
                (true, "grid") => Display::InlineGrid,
                (false, "grid") => Display::Grid,
                (true, "table") => Display::InlineTable,
                (false, "table") => Display::Table,
                _ => return None,
            })
        }
        _ => None,
    }
}

/// Box alignment, accepting `safe`, `unsafe`, `first`, `last` and `legacy` prefixes.
pub fn alignment(values: &[ComponentValue], _: &Context) -> Option<Alignment> {
    let words: Option<Vec<String>> = without_whitespace(values)
        .iter()
        .map(ComponentValue::ident)
        .collect();
    let words = words?;
    let words: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|word| !matches!(*word, "safe" | "unsafe" | "first" | "last" | "legacy"))
        .collect();
    match words.as_slice() {
        [word] => Alignment::from_keyword(word),
        _ => None,
    }
}

/// A `<length-percentage>`.
pub fn length(values: &[ComponentValue], context: &Context) -> Option<LengthPercentage> {
    length_percentage(single(values)?, &context.length)
}

/// A non-negative `<length-percentage>`.
pub fn padding(values: &[ComponentValue], context: &Context) -> Option<LengthPercentage> {
    non_negative(single(values)?, &context.length)
}

/// A `<length-percentage>` or `auto`.
pub fn length_auto(values: &[ComponentValue], context: &Context) -> Option<LengthPercentageAuto> {
    let value = single(values)?;
    if value.is_ident("auto") {
        return Some(LengthPercentageAuto::Auto);
    }
    length_percentage(value, &context.length).map(LengthPercentageAuto::Length)
}

/// `width`, `height`, `min-width`, `min-height` and `flex-basis`.
pub fn size(values: &[ComponentValue], context: &Context) -> Option<Size> {
    let value = single(values)?;
    Some(match value.ident().as_deref() {
        Some("auto" | "content" | "stretch" | "-webkit-fill-available") => Size::Auto,
        Some("min-content" | "-webkit-min-content") => Size::MinContent,
        Some("max-content" | "-webkit-max-content") => Size::MaxContent,
        Some("fit-content" | "-webkit-fit-content") => Size::FitContent,
        Some(_) => return None,
        None => Size::Length(non_negative(value, &context.length)?),
    })
}

/// `max-width` and `max-height`.
pub fn max_size(values: &[ComponentValue], context: &Context) -> Option<MaxSize> {
    if single(values)?.is_ident("none") {
        return Some(MaxSize::None);
    }
    Some(match size(values, context)? {
        Size::Auto => return None,
        Size::Length(length) => MaxSize::Length(length),
        Size::MinContent => MaxSize::MinContent,
        Size::MaxContent => MaxSize::MaxContent,
        Size::FitContent => MaxSize::FitContent,
    })
}

/// A color, resolving `currentcolor`.
pub fn color(values: &[ComponentValue], context: &Context) -> Option<Color> {
    Some(match parse_color(single(values)?, context.system)? {
        SpecifiedColor::Color(color) => color,
        SpecifiedColor::CurrentColor => context.color,
    })
}

/// A color where `currentcolor` and `auto` stay symbolic.
pub fn optional_color(values: &[ComponentValue], context: &Context) -> Option<OptionalColor> {
    let value = single(values)?;
    if value.is_ident("auto") {
        return Some(OptionalColor(None));
    }
    Some(OptionalColor(match parse_color(value, context.system)? {
        SpecifiedColor::Color(color) => Some(color),
        SpecifiedColor::CurrentColor => None,
    }))
}

/// `accent-color`: `auto` or a color.
pub fn accent_color(values: &[ComponentValue], context: &Context) -> Option<AccentColor> {
    if single(values)?.is_ident("auto") {
        return Some(AccentColor::Auto);
    }
    optional_color(values, context).map(AccentColor::Color)
}

fn line_width(value: &ComponentValue, context: &Context) -> Option<f32> {
    Some(match value.ident().as_deref() {
        Some("thin") => 1.0,
        Some("medium") => 3.0,
        Some("thick") => 5.0,
        Some(_) => return None,
        None => match non_negative(value, &context.length)? {
            LengthPercentage::Px(px) => px,
            _ => return None,
        },
    })
}

/// `border-*-width` and `outline-width`.
pub fn border_width(values: &[ComponentValue], context: &Context) -> Option<Px> {
    line_width(single(values)?, context).map(Px)
}

/// A pixel length such as `outline-offset`.
pub fn pixels(values: &[ComponentValue], context: &Context) -> Option<Px> {
    match length(values, context)? {
        LengthPercentage::Px(px) => Some(Px(px)),
        _ => None,
    }
}

/// `clip`: `auto` or `rect(top, right, bottom, left)`, each a length or
/// `auto`, with or without commas.
pub fn clip(values: &[ComponentValue], context: &Context) -> Option<Option<ClipRect>> {
    let value = single(values)?;
    if value.is_ident("auto") {
        return Some(None);
    }
    let ComponentValue::Function(name, arguments) = value else {
        return None;
    };
    if !name.eq_ignore_ascii_case("rect") {
        return None;
    }
    let parts: Vec<ComponentValue> = without_whitespace(arguments)
        .into_iter()
        .filter(|part| !matches!(part, ComponentValue::Token(Token::Comma)))
        .collect();
    let [top, right, bottom, left] = parts.as_slice() else {
        return None;
    };
    let edge = |part: &ComponentValue| -> Option<Option<f32>> {
        if part.is_ident("auto") {
            return Some(None);
        }
        pixels(core::slice::from_ref(part), context).map(|px| Some(px.0))
    };
    Some(Some(ClipRect {
        top: edge(top)?,
        right: edge(right)?,
        bottom: edge(bottom)?,
        left: edge(left)?,
    }))
}

/// `scrollbar-color`: exactly two colors, in thumb/track order.
pub fn scrollbar_color(values: &[ComponentValue], context: &Context) -> Option<ScrollbarColor> {
    if single(values).is_some_and(|value| value.is_ident("auto")) {
        return Some(ScrollbarColor(None));
    }
    let parts = without_whitespace(values);
    let [thumb, track] = parts.as_slice() else {
        return None;
    };
    Some(ScrollbarColor(Some((
        color(core::slice::from_ref(thumb), context)?,
        color(core::slice::from_ref(track), context)?,
    ))))
}

/// `letter-spacing`.
pub fn spacing(values: &[ComponentValue], context: &Context) -> Option<Px> {
    if single(values)?.is_ident("normal") {
        return Some(Px(0.0));
    }
    pixels(values, context)
}

/// `text-underline-offset`: `auto` or a length.
pub fn underline_offset(values: &[ComponentValue], context: &Context) -> Option<UnderlineOffset> {
    if single(values)?.is_ident("auto") {
        return Some(UnderlineOffset(None));
    }
    pixels(values, context).map(|offset| UnderlineOffset(Some(offset.0)))
}

/// `row-gap` and `column-gap`.
pub fn gap(values: &[ComponentValue], context: &Context) -> Option<LengthPercentage> {
    if single(values)?.is_ident("normal") {
        return Some(LengthPercentage::Px(0.0));
    }
    padding(values, context)
}

/// Border radius: one or two non-negative lengths.
pub fn radius(values: &[ComponentValue], context: &Context) -> Option<Radius> {
    let parts = without_whitespace(values);
    let parsed: Option<Vec<LengthPercentage>> = parts
        .iter()
        .map(|part| non_negative(part, &context.length))
        .collect();
    match parsed?.as_slice() {
        [both] => Some(Radius {
            horizontal: both.clone(),
            vertical: both.clone(),
        }),
        [horizontal, vertical] => Some(Radius {
            horizontal: horizontal.clone(),
            vertical: vertical.clone(),
        }),
        _ => None,
    }
}

/// `border-spacing`: one or two non-negative lengths.
pub fn border_spacing(values: &[ComponentValue], context: &Context) -> Option<Spacing> {
    let parts = without_whitespace(values);
    let lengths: Option<Vec<f32>> = parts
        .iter()
        .map(|part| match non_negative(part, &context.length)? {
            LengthPercentage::Px(px) => Some(px),
            _ => None,
        })
        .collect();
    match lengths?.as_slice() {
        [both] => Some(Spacing {
            horizontal: Px(*both),
            vertical: Px(*both),
        }),
        [horizontal, vertical] => Some(Spacing {
            horizontal: Px(*horizontal),
            vertical: Px(*vertical),
        }),
        _ => None,
    }
}

/// A plain number.
pub fn plain_number(values: &[ComponentValue], context: &Context) -> Option<f32> {
    number(single(values)?, &context.length)
}

/// SVG `fill` and `stroke`: `none`, a color, or a `url()` paint server,
/// which is not supported and paints its fallback instead.
pub fn paint(values: &[ComponentValue], context: &Context) -> Option<Paint> {
    let values = without_whitespace(values);
    let (value, rest) = match values.as_slice() {
        [ComponentValue::Token(Token::Url(_)), rest @ ..] => (rest.first(), rest.get(1..)),
        [ComponentValue::Function(name, _), rest @ ..] if name == "url" => {
            (rest.first(), rest.get(1..))
        }
        [value, rest @ ..] => (Some(value), Some(rest)),
        [] => return None,
    };
    if rest.is_some_and(|rest| !rest.is_empty()) {
        return None;
    }
    let Some(value) = value else {
        return Some(Paint::None);
    };
    if value.is_ident("none") {
        return Some(Paint::None);
    }
    Some(match parse_color(value, context.system)? {
        SpecifiedColor::Color(color) => Paint::Color(color),
        SpecifiedColor::CurrentColor => Paint::CurrentColor,
    })
}

/// SVG `stroke-width`: a non-negative length, or a number of user units.
pub fn stroke_width(values: &[ComponentValue], context: &Context) -> Option<Px> {
    let value = single(values)?;
    let width = match value {
        ComponentValue::Token(Token::Number { value, .. }) => *value as f32,
        other => match length_percentage(other, &context.length)? {
            LengthPercentage::Px(px) => px,
            _ => return None,
        },
    };
    (width >= 0.0).then_some(Px(width))
}

/// SVG `stroke-dasharray`: `none`, or non-negative lengths and numbers
/// separated by commas or spaces.
pub fn stroke_dasharray(values: &[ComponentValue], context: &Context) -> Option<Dashes> {
    let parts: Vec<ComponentValue> = without_whitespace(values)
        .into_iter()
        .filter(|part| !matches!(part, ComponentValue::Token(Token::Comma)))
        .collect();
    match parts.as_slice() {
        [] => None,
        [value] if value.is_ident("none") => Some(Dashes::default()),
        parts => parts
            .iter()
            .map(|part| stroke_width(core::slice::from_ref(part), context).map(|px| px.0))
            .collect::<Option<Vec<f32>>>()
            .map(Dashes),
    }
}

/// SVG `stroke-dashoffset`: a length or number, which may be negative.
pub fn stroke_dashoffset(values: &[ComponentValue], context: &Context) -> Option<Px> {
    match single(values)? {
        ComponentValue::Token(Token::Number { value, .. }) => Some(Px(*value as f32)),
        _ => pixels(values, context),
    }
}

/// SVG `marker-start`, `marker-mid` and `marker-end`: `none` or a URL.
pub fn marker(values: &[ComponentValue], _context: &Context) -> Option<MarkerUrl> {
    match without_whitespace(values).as_slice() {
        [value] if value.is_ident("none") => Some(MarkerUrl(None)),
        [ComponentValue::Token(Token::Url(url))] => Some(MarkerUrl(Some(url.clone()))),
        [ComponentValue::Function(name, arguments)] if name.eq_ignore_ascii_case("url") => {
            match without_whitespace(arguments).as_slice() {
                [ComponentValue::Token(Token::String(url))] => Some(MarkerUrl(Some(url.clone()))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `tab-size`: a non-negative number of spaces or length.
pub fn tab_size(values: &[ComponentValue], context: &Context) -> Option<crate::types::TabSize> {
    if let Some(spaces) = non_negative_number(values, context) {
        return Some(crate::types::TabSize::Spaces(spaces));
    }
    pixels(values, context)
        .filter(|length| length.0 >= 0.0)
        .map(|length| crate::types::TabSize::Length(length.0))
}

/// A non-negative number such as `flex-grow`.
pub fn non_negative_number(values: &[ComponentValue], context: &Context) -> Option<f32> {
    plain_number(values, context).filter(|value| *value >= 0.0)
}

/// An integer such as `order`.
pub fn integer(values: &[ComponentValue], _: &Context) -> Option<i32> {
    match single(values)? {
        ComponentValue::Token(Token::Number {
            value,
            integer: true,
        }) => Some(*value as i32),
        _ => None,
    }
}

/// `-webkit-line-clamp`: `none` or a positive number of lines.
pub fn line_clamp(values: &[ComponentValue], context: &Context) -> Option<LineClamp> {
    if single(values)?.is_ident("none") {
        return Some(LineClamp(None));
    }
    let lines = integer(values, context)?;
    (lines > 0).then_some(LineClamp(Some(lines as u32)))
}

/// `z-index`.
pub fn z_index(values: &[ComponentValue], context: &Context) -> Option<ZIndex> {
    if single(values)?.is_ident("auto") {
        return Some(ZIndex(None));
    }
    integer(values, context).map(|value| ZIndex(Some(value)))
}

/// `opacity`: a number or percentage clamped to [0, 1].
pub fn opacity(values: &[ComponentValue], context: &Context) -> Option<f32> {
    let value = match single(values)? {
        ComponentValue::Token(Token::Percentage(percent)) => *percent as f32 / 100.0,
        other => number(other, &context.length)?,
    };
    Some(value.clamp(0.0, 1.0))
}

/// `font-size`; relative units resolve against the parent's font size.
pub fn font_size(values: &[ComponentValue], context: &Context) -> Option<Px> {
    let value = single(values)?;
    let parent = context.parent_font_size;
    let size = match value.ident().as_deref() {
        Some("xx-small") => 9.0,
        Some("x-small") => 10.0,
        Some("small") => 13.0,
        Some("medium") => 16.0,
        Some("large") => 18.0,
        Some("x-large") => 24.0,
        Some("xx-large") => 32.0,
        Some("xxx-large") => 48.0,
        Some("smaller") => parent / 1.2,
        Some("larger") => parent * 1.2,
        Some(_) => return None,
        None => non_negative(value, &context.length)?.resolve(parent),
    };
    Some(Px(size))
}

/// `font-weight`.
pub fn font_weight(values: &[ComponentValue], context: &Context) -> Option<FontWeight> {
    let value = single(values)?;
    let parent = context.parent_font_weight;
    let weight = match value.ident().as_deref() {
        Some("normal") => 400,
        Some("bold") => 700,
        Some("bolder") => match parent {
            0..350 => 400,
            350..550 => 700,
            _ => 900,
        },
        Some("lighter") => match parent {
            0..550 => 100,
            550..750 => 400,
            _ => 700,
        },
        Some(_) => return None,
        None => {
            let weight = number(value, &context.length)?;
            if !(1.0..=1000.0).contains(&weight) {
                return None;
            }
            crate::math::round(weight) as u16
        }
    };
    Some(FontWeight(weight))
}

/// `font-family`.
pub fn font_family(values: &[ComponentValue], _: &Context) -> Option<FontFamily> {
    let mut families = Vec::new();
    for part in split_commas(values) {
        let family = match part.as_slice() {
            [ComponentValue::Token(Token::String(name))] => name.clone(),
            [] => return None,
            words => {
                let names: Option<Vec<String>> = words
                    .iter()
                    .map(|word| match word {
                        ComponentValue::Token(Token::Ident(name)) => Some(name.clone()),
                        _ => None,
                    })
                    .collect();
                names?.join(" ")
            }
        };
        families.push(family);
    }
    Some(FontFamily(families))
}

/// `line-height`; percentages resolve against the element's font size.
pub fn line_height(values: &[ComponentValue], context: &Context) -> Option<LineHeight> {
    let value = single(values)?;
    if value.is_ident("normal") {
        return Some(LineHeight::Normal);
    }
    if let Some(number) = number(value, &context.length) {
        return (number >= 0.0).then_some(LineHeight::Number(number));
    }
    let length = non_negative(value, &context.length)?;
    Some(LineHeight::Px(length.resolve(context.length.font_size)))
}

/// `text-decoration-line`.
pub fn decoration_line(values: &[ComponentValue], _: &Context) -> Option<TextDecorationLine> {
    let mut line = TextDecorationLine::default();
    for word in without_whitespace(values) {
        match word.ident()?.as_str() {
            "none" => {}
            "underline" => line.underline = true,
            "overline" => line.overline = true,
            "line-through" => line.line_through = true,
            "blink" => {}
            _ => return None,
        }
    }
    Some(line)
}

/// `text-decoration-thickness`: `auto`, `from-font`, a length or a
/// percentage of the font size.
pub fn decoration_thickness(
    values: &[ComponentValue],
    context: &Context,
) -> Option<DecorationThickness> {
    let value = single(values)?;
    if value.is_ident("auto") {
        return Some(DecorationThickness::Auto);
    }
    if value.is_ident("from-font") {
        return Some(DecorationThickness::FromFont);
    }
    if let ComponentValue::Token(Token::Percentage(percent)) = value {
        return Some(DecorationThickness::Px(
            *percent as f32 / 100.0 * context.length.font_size,
        ));
    }
    pixels(values, context).map(|px| DecorationThickness::Px(px.0))
}

/// `text-underline-position`: `auto`, or `from-font` or `under` with or
/// without `left` or `right`.
pub fn underline_position(values: &[ComponentValue], _: &Context) -> Option<UnderlinePosition> {
    let words = without_whitespace(values)
        .iter()
        .map(ComponentValue::ident)
        .collect::<Option<Vec<_>>>()?;
    let mut position = UnderlinePosition::default();
    match words.as_slice() {
        [only] if only == "auto" => return Some(position),
        [] => return None,
        words if words.len() > 2 => return None,
        _ => {}
    }
    for word in &words {
        let base = position.from_font || position.under;
        match word.as_str() {
            "from-font" if !base => position.from_font = true,
            "under" if !base => position.under = true,
            "left" | "right" if position.right.is_none() => {
                position.right = Some(word == "right");
            }
            _ => return None,
        }
    }
    Some(position)
}

/// `vertical-align`: a keyword, or a length or percentage.
pub fn vertical_align(values: &[ComponentValue], context: &Context) -> Option<VerticalAlign> {
    match single_ident(values) {
        Some(word) => VerticalAlign::from_keyword(&word),
        None => length(values, context).map(VerticalAlign::Length),
    }
}

/// `cursor`: image fallbacks are skipped and the keyword is used.
pub fn cursor(values: &[ComponentValue], context: &Context) -> Option<Cursor> {
    let parts = split_commas(values);
    let last = parts.last()?;
    keyword(last, context)
}

/// A counter's name: a custom identifier, which keeps its case.
fn counter_name(values: &[ComponentValue]) -> Option<String> {
    match single(values)? {
        ComponentValue::Token(Token::Ident(name))
            if !["none", "initial", "inherit", "unset", "default"]
                .contains(&name.to_ascii_lowercase().as_str()) =>
        {
            Some(name.clone())
        }
        _ => None,
    }
}

/// A counter style; the list style types are the counter styles known.
fn counter_style(values: &[ComponentValue]) -> Option<ListStyleType> {
    ListStyleType::from_keyword(&single_ident(values)?)
}

/// `counter-reset`, `counter-increment` and `counter-set`: `none`, or names
/// each followed by an optional integer, else the default value.
pub fn counter_changes(values: &[ComponentValue], default: i32) -> Option<CounterChanges> {
    let parts = without_whitespace(values);
    if single(&parts).is_some_and(|value| value.is_ident("none")) {
        return Some(CounterChanges::default());
    }
    let mut changes: Vec<(String, i32)> = Vec::new();
    for part in parts {
        match part {
            ComponentValue::Token(Token::Number {
                value,
                integer: true,
            }) => changes.last_mut()?.1 = value as i32,
            _ => changes.push((counter_name(core::slice::from_ref(&part))?, default)),
        }
    }
    (!changes.is_empty()).then_some(CounterChanges(changes))
}

/// `container-name`: `none`, or names other than the words container
/// queries reserve.
pub fn container_name(values: &[ComponentValue], _: &Context) -> Option<ContainerName> {
    let parts = without_whitespace(values);
    if single(&parts).is_some_and(|value| value.is_ident("none")) {
        return Some(ContainerName::default());
    }
    let mut names = Vec::new();
    for part in parts {
        let ComponentValue::Token(Token::Ident(name)) = part else {
            return None;
        };
        let reserved = [
            "none", "and", "or", "not", "default", "initial", "inherit", "unset", "revert",
        ];
        if reserved.iter().any(|word| name.eq_ignore_ascii_case(word)) {
            return None;
        }
        names.push(name);
    }
    (!names.is_empty()).then_some(ContainerName(names))
}

/// `counter-reset`.
pub fn counter_reset(values: &[ComponentValue], _: &Context) -> Option<CounterChanges> {
    counter_changes(values, 0)
}

/// `counter-increment`.
pub fn counter_increment(values: &[ComponentValue], _: &Context) -> Option<CounterChanges> {
    counter_changes(values, 1)
}

/// `content`.
pub fn content(values: &[ComponentValue], _: &Context) -> Option<Content> {
    let parts = without_whitespace(values);
    match single(&parts).and_then(ComponentValue::ident).as_deref() {
        Some("normal") => return Some(Content::Normal),
        Some("none") => return Some(Content::None),
        _ => {}
    }
    let mut items = Vec::new();
    for part in parts {
        items.push(match part {
            ComponentValue::Token(Token::String(text)) => ContentItem::Text(text),
            ComponentValue::Function(name, arguments) if name == "attr" => {
                ContentItem::Attr(single(&arguments)?.ident()?)
            }
            ComponentValue::Function(name, arguments) if name == "counter" => {
                match split_commas(&arguments).as_slice() {
                    [name] => ContentItem::Counter(counter_name(name)?, ListStyleType::Decimal),
                    [name, kind] => ContentItem::Counter(counter_name(name)?, counter_style(kind)?),
                    _ => return None,
                }
            }
            ComponentValue::Function(name, arguments) if name == "counters" => {
                let parts = split_commas(&arguments);
                let (name, separator, kind) = match parts.as_slice() {
                    [name, separator] => (name, separator, ListStyleType::Decimal),
                    [name, separator, kind] => (name, separator, counter_style(kind)?),
                    _ => return None,
                };
                let ComponentValue::Token(Token::String(separator)) = single(separator)? else {
                    return None;
                };
                ContentItem::Counters(counter_name(name)?, separator.clone(), kind)
            }
            ComponentValue::Token(Token::Ident(word)) => match word.to_ascii_lowercase().as_str() {
                "open-quote" => ContentItem::Text("\u{201c}".to_owned()),
                "close-quote" => ContentItem::Text("\u{201d}".to_owned()),
                "no-open-quote" | "no-close-quote" => ContentItem::Text(String::new()),
                _ => return None,
            },
            _ => return None,
        });
    }
    Some(Content::Items(items))
}

fn angle(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Dimension { value, unit }) => {
            let value = *value as f32;
            Some(match unit.to_ascii_lowercase().as_str() {
                "deg" => value,
                "rad" => value.to_degrees(),
                "grad" => value * 0.9,
                "turn" => value * 360.0,
                _ => return None,
            })
        }
        ComponentValue::Token(Token::Number { value, .. }) if *value == 0.0 => Some(0.0),
        _ => None,
    }
}

fn color_stop(part: &[ComponentValue], context: &Context) -> Option<Vec<ColorStop>> {
    let (color_value, positions) = part.split_first()?;
    let color = color(core::slice::from_ref(color_value), context)?;
    let mut stops = Vec::new();
    for position in positions {
        let position = match position {
            ComponentValue::Token(Token::Percentage(percent)) => Some(*percent as f32 / 100.0),
            other => {
                length_percentage(other, &context.length)?;
                None
            }
        };
        stops.push(ColorStop { color, position });
    }
    if stops.is_empty() {
        stops.push(ColorStop {
            color,
            position: None,
        });
    }
    Some(stops)
}

fn gradient_stops(parts: &[Vec<ComponentValue>], context: &Context) -> Option<Vec<ColorStop>> {
    let mut stops = Vec::new();
    for part in parts {
        stops.extend(color_stop(part, context)?);
    }
    (stops.len() >= 2).then_some(stops)
}

/// A gradient's first argument without its `in <space> [<hue-method> hue]`
/// clause, which may come before or after the rest, and the interpolation
/// it names.
fn gradient_interpolation(
    head: &[ComponentValue],
) -> Option<(Option<crate::color::Interpolation>, Vec<ComponentValue>)> {
    let Some(start) = head.iter().position(|value| value.is_ident("in")) else {
        return Some((None, head.to_vec()));
    };
    let length = if head
        .get(start + 3)
        .is_some_and(|value| value.is_ident("hue"))
    {
        4
    } else {
        2
    };
    let clause = head.get(start..start + length)?;
    let interpolation = crate::color::Interpolation::parse(clause)?;
    let mut rest = head[..start].to_vec();
    rest.extend_from_slice(&head[start + length..]);
    Some((Some(interpolation), rest))
}

/// How many parts each pair of stops is split into when colors interpolate
/// in another space than painting's sRGB.
const INTERPOLATED_STEPS: usize = 8;

/// Stops interpolated in another space than sRGB, as stops in between that
/// painting's sRGB interpolation joins: missing positions are filled in as
/// CSS Images says.
fn interpolated(
    mut stops: Vec<ColorStop>,
    interpolation: Option<crate::color::Interpolation>,
) -> Vec<ColorStop> {
    let Some(interpolation) = interpolation.filter(|method| !method.is_srgb()) else {
        return stops;
    };
    let last = stops.len() - 1;
    stops[0].position.get_or_insert(0.0);
    stops[last].position.get_or_insert(1.0);
    // Positions never go back.
    let mut furthest = 0.0f32;
    for stop in stops.iter_mut() {
        if let Some(position) = stop.position.as_mut() {
            furthest = furthest.max(*position);
            *position = furthest;
        }
    }
    // Stops without positions spread evenly between those with.
    let mut index = 0;
    while index < last {
        let next = (index + 1..=last)
            .find(|candidate| stops[*candidate].position.is_some())
            .unwrap_or(last);
        let (from, to) = (
            stops[index].position.unwrap_or(0.0),
            stops[next].position.unwrap_or(1.0),
        );
        for (step, between) in (index + 1..next).enumerate() {
            let fraction = (step + 1) as f32 / (next - index) as f32;
            stops[between].position = Some(from + (to - from) * fraction);
        }
        index = next;
    }
    let mut result = Vec::with_capacity(stops.len() * INTERPOLATED_STEPS);
    for pair in stops.windows(2) {
        let [from, to] = pair else { continue };
        let (start, end) = (from.position.unwrap_or(0.0), to.position.unwrap_or(1.0));
        for step in 0..INTERPOLATED_STEPS {
            let progress = step as f32 / INTERPOLATED_STEPS as f32;
            result.push(ColorStop {
                color: interpolation.between(from.color, to.color, progress),
                position: Some(start + (end - start) * progress),
            });
        }
    }
    result.push(stops[last].clone());
    result
}

fn side_angle(words: &[ComponentValue]) -> Option<f32> {
    let names: Option<Vec<String>> = words.iter().map(ComponentValue::ident).collect();
    let names = names?;
    let mut horizontal = 0.0;
    let mut vertical = 0.0;
    for name in &names {
        match name.as_str() {
            "top" => vertical = -1.0,
            "bottom" => vertical = 1.0,
            "left" => horizontal = -1.0,
            "right" => horizontal = 1.0,
            _ => return None,
        }
    }
    let angle: f32 = crate::math::atan2(horizontal, -vertical).to_degrees();
    Some(crate::math::rem_euclid(angle, 360.0))
}

/// `background-repeat`: the first layer.
pub fn background_repeat(values: &[ComponentValue], _: &Context) -> Option<BackgroundRepeat> {
    let layers = split_commas(values);
    let words: Vec<String> = layers
        .first()?
        .iter()
        .map(|value| value.ident().map(|name| name.to_ascii_lowercase()))
        .collect::<Option<_>>()?;
    let repeat = |word: &str| match word {
        "repeat" => Some(Repeat::Repeat),
        "no-repeat" => Some(Repeat::NoRepeat),
        "space" => Some(Repeat::Space),
        "round" => Some(Repeat::Round),
        _ => None,
    };
    match words.as_slice() {
        [one] if one == "repeat-x" => Some(BackgroundRepeat {
            x: Repeat::Repeat,
            y: Repeat::NoRepeat,
        }),
        [one] if one == "repeat-y" => Some(BackgroundRepeat {
            x: Repeat::NoRepeat,
            y: Repeat::Repeat,
        }),
        [one] => repeat(one).map(|both| BackgroundRepeat { x: both, y: both }),
        [x, y] => Some(BackgroundRepeat {
            x: repeat(x)?,
            y: repeat(y)?,
        }),
        _ => None,
    }
}

/// `background-size`: the first layer.
pub fn background_size(values: &[ComponentValue], context: &Context) -> Option<BackgroundSize> {
    let layers = split_commas(values);
    let side = |value: &ComponentValue| {
        if value.is_ident("auto") {
            Some(LengthPercentageAuto::Auto)
        } else {
            non_negative(value, &context.length).map(LengthPercentageAuto::Length)
        }
    };
    match layers.first()?.as_slice() {
        [one] if one.is_ident("cover") => Some(BackgroundSize::Cover),
        [one] if one.is_ident("contain") => Some(BackgroundSize::Contain),
        [one] if one.is_ident("auto") => Some(BackgroundSize::Auto),
        [width] => Some(BackgroundSize::Explicit(
            side(width)?,
            LengthPercentageAuto::Auto,
        )),
        [width, height] => Some(BackgroundSize::Explicit(side(width)?, side(height)?)),
        _ => None,
    }
}

/// `background-position`: the first layer.
pub fn background_position(
    values: &[ComponentValue],
    context: &Context,
) -> Option<BackgroundPosition> {
    let layers = split_commas(values);
    position(layers.first()?, context)
}

/// `object-position`: one position.
pub fn object_position(values: &[ComponentValue], context: &Context) -> Option<BackgroundPosition> {
    position(&without_whitespace(values), context)
}

/// `clip-path`: `none`, a basic shape and its box, a box, or a URL.
pub fn clip_path(values: &[ComponentValue], context: &Context) -> Option<ClipPath> {
    let parts = without_whitespace(values);
    let geometry = |value: &ComponentValue| {
        value
            .ident()
            .and_then(|word| GeometryBox::from_keyword(&word.to_ascii_lowercase()))
    };
    match parts.as_slice() {
        [value] if value.is_ident("none") => Some(ClipPath::None),
        [ComponentValue::Token(Token::Url(url))] => Some(ClipPath::Url(url.clone())),
        [ComponentValue::Function(name, arguments)] if name.eq_ignore_ascii_case("url") => {
            match without_whitespace(arguments).as_slice() {
                [ComponentValue::Token(Token::String(url))] => Some(ClipPath::Url(url.clone())),
                _ => None,
            }
        }
        [value] if geometry(value).is_some() => geometry(value).map(ClipPath::Box),
        [shape] => Some(ClipPath::Shape(
            basic_shape(shape, context)?,
            GeometryBox::BorderBox,
        )),
        [first, second] => {
            let (shape, geometry) = match (geometry(first), geometry(second)) {
                (Some(geometry), None) => (second, geometry),
                (None, Some(geometry)) => (first, geometry),
                _ => return None,
            };
            Some(ClipPath::Shape(basic_shape(shape, context)?, geometry))
        }
        _ => None,
    }
}

/// A `<basic-shape>` function: `inset()`, `circle()`, `ellipse()` or
/// `polygon()`.
fn basic_shape(value: &ComponentValue, context: &Context) -> Option<BasicShape> {
    let ComponentValue::Function(name, arguments) = value else {
        return None;
    };
    let length = |value: &ComponentValue| length_percentage(value, &context.length);
    // A shape's radii, then its center after `at`, which is the middle of
    // the box when left out.
    let radii_and_center = |values: &[ComponentValue]| {
        let at = values.iter().position(|value| value.is_ident("at"));
        let (radii, center) = match at {
            Some(at) => (&values[..at], Some(position(&values[at + 1..], context)?)),
            None => (values, None),
        };
        let radii = radii
            .iter()
            .map(|value| {
                match value
                    .ident()
                    .map(|word| word.to_ascii_lowercase())
                    .as_deref()
                {
                    Some("closest-side") => Some(ShapeRadius::ClosestSide),
                    Some("farthest-side") => Some(ShapeRadius::FarthestSide),
                    _ => match length(value)? {
                        LengthPercentage::Px(px) if px < 0.0 => None,
                        LengthPercentage::Percent(percent) if percent < 0.0 => None,
                        other => Some(ShapeRadius::Length(other)),
                    },
                }
            })
            .collect::<Option<Vec<_>>>()?;
        let middle = || BackgroundPosition {
            x: LengthPercentage::Percent(0.5),
            y: LengthPercentage::Percent(0.5),
        };
        Some((radii, center.unwrap_or_else(middle)))
    };
    let values = without_whitespace(arguments);
    match name.to_ascii_lowercase().as_str() {
        "inset" => {
            let round = values.iter().position(|value| value.is_ident("round"));
            let (offsets, radii) = match round {
                Some(round) => (&values[..round], Some(&values[round + 1..])),
                None => (values.as_slice(), None),
            };
            let offsets = offsets.iter().map(length).collect::<Option<Vec<_>>>()?;
            let offsets = expand_sides(&offsets)?;
            let radii = match radii {
                Some(radii) => border_radii(radii, context)?,
                None => Default::default(),
            };
            Some(BasicShape::Inset(offsets, radii))
        }
        "circle" => {
            let (radii, center) = radii_and_center(&values)?;
            match radii.as_slice() {
                [] => Some(BasicShape::Circle(ShapeRadius::ClosestSide, center)),
                [radius] => Some(BasicShape::Circle(radius.clone(), center)),
                _ => None,
            }
        }
        "ellipse" => {
            let (radii, center) = radii_and_center(&values)?;
            match radii.as_slice() {
                [] => Some(BasicShape::Ellipse(
                    ShapeRadius::ClosestSide,
                    ShapeRadius::ClosestSide,
                    center,
                )),
                [x, y] => Some(BasicShape::Ellipse(x.clone(), y.clone(), center)),
                _ => None,
            }
        }
        "polygon" => {
            let mut groups = split_commas(arguments);
            let even_odd = match groups.first().map(Vec::as_slice) {
                Some([rule]) if rule.is_ident("evenodd") => true,
                Some([rule]) if rule.is_ident("nonzero") => false,
                _ => {
                    groups.insert(0, Vec::new());
                    false
                }
            };
            let points = groups[1..]
                .iter()
                .map(|point| match point.as_slice() {
                    [x, y] => Some((length(x)?, length(y)?)),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            (!points.is_empty()).then_some(BasicShape::Polygon(even_odd, points))
        }
        "path" => {
            let groups = split_commas(arguments);
            let (rule, data) = match groups.as_slice() {
                [data] => (None, data),
                [rule, data] => (Some(rule), data),
                _ => return None,
            };
            let even_odd = match rule.map(Vec::as_slice) {
                None => false,
                Some([rule]) if rule.is_ident("evenodd") => true,
                Some([rule]) if rule.is_ident("nonzero") => false,
                _ => return None,
            };
            match data.as_slice() {
                [ComponentValue::Token(Token::String(data))] => {
                    Some(BasicShape::Path(even_odd, data.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// One to four values for the top, right, bottom and left, as margins and
/// paddings expand them.
fn expand_sides<T: Clone>(values: &[T]) -> Option<[T; 4]> {
    Some(match values {
        [all] => [all.clone(), all.clone(), all.clone(), all.clone()],
        [vertical, horizontal] => [
            vertical.clone(),
            horizontal.clone(),
            vertical.clone(),
            horizontal.clone(),
        ],
        [top, horizontal, bottom] => [
            top.clone(),
            horizontal.clone(),
            bottom.clone(),
            horizontal.clone(),
        ],
        [top, right, bottom, left] => [top.clone(), right.clone(), bottom.clone(), left.clone()],
        _ => return None,
    })
}

/// The corner radii of `border-radius` syntax: one to four horizontal
/// radii, and after a slash one to four vertical ones, top left first.
fn border_radii(values: &[ComponentValue], context: &Context) -> Option<[Radius; 4]> {
    let slash = values
        .iter()
        .position(|value| matches!(value, ComponentValue::Token(Token::Delim('/'))));
    let (horizontal, vertical) = match slash {
        Some(slash) => (&values[..slash], &values[slash + 1..]),
        None => (values, values),
    };
    let corners = |values: &[ComponentValue]| -> Option<[LengthPercentage; 4]> {
        let lengths = values
            .iter()
            .map(|value| non_negative(value, &context.length))
            .collect::<Option<Vec<_>>>()?;
        expand_sides(&lengths)
    };
    let (horizontal, vertical) = (corners(horizontal)?, corners(vertical)?);
    Some(core::array::from_fn(|corner| Radius {
        horizontal: horizontal[corner].clone(),
        vertical: vertical[corner].clone(),
    }))
}

/// Which axis a position keyword names.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
    Either,
}

/// A `<position>`: one or two keywords or lengths, or keywords naming the
/// edges that offsets after them are measured from.
fn position(values: &[ComponentValue], context: &Context) -> Option<BackgroundPosition> {
    let keyword = |value: &ComponentValue| -> Option<(Axis, f32)> {
        Some(match value.ident()?.to_ascii_lowercase().as_str() {
            "left" => (Axis::X, 0.0),
            "right" => (Axis::X, 1.0),
            "top" => (Axis::Y, 0.0),
            "bottom" => (Axis::Y, 1.0),
            "center" => (Axis::Either, 0.5),
            _ => return None,
        })
    };
    let length = |value: &ComponentValue| length_percentage(value, &context.length);
    let percent = LengthPercentage::Percent;
    match values {
        [one] => Some(match keyword(one) {
            Some((Axis::Y, at)) => BackgroundPosition {
                x: percent(0.5),
                y: percent(at),
            },
            Some((_, at)) => BackgroundPosition {
                x: percent(at),
                y: percent(0.5),
            },
            None => BackgroundPosition {
                x: length(one)?,
                y: percent(0.5),
            },
        }),
        [first, second] => {
            // `top left` names the vertical position first.
            let swap = matches!(keyword(first), Some((Axis::Y, _)))
                || matches!(keyword(second), Some((Axis::X, _)));
            let (across, down) = if swap {
                (second, first)
            } else {
                (first, second)
            };
            let (across_keyword, down_keyword) = (keyword(across), keyword(down));
            if matches!(across_keyword, Some((Axis::Y, _)))
                || matches!(down_keyword, Some((Axis::X, _)))
                || (swap && (across_keyword.is_none() || down_keyword.is_none()))
            {
                return None;
            }
            Some(BackgroundPosition {
                x: match across_keyword {
                    Some((_, at)) => percent(at),
                    None => length(across)?,
                },
                y: match down_keyword {
                    Some((_, at)) => percent(at),
                    None => length(down)?,
                },
            })
        }
        [_, _, _] | [_, _, _, _] => {
            // Edge keywords with offsets, as in `right 10px bottom 20%`.
            let (mut x, mut y) = (None, None);
            let mut index = 0;
            while index < values.len() {
                let (axis, at) = keyword(&values[index])?;
                let offset = values
                    .get(index + 1)
                    .filter(|value| keyword(value).is_none());
                let value = match offset {
                    Some(offset) => {
                        let offset = length(offset)?;
                        if at == 1.0 {
                            from_far_edge(offset)
                        } else if at == 0.0 {
                            offset
                        } else {
                            return None;
                        }
                    }
                    None => percent(at),
                };
                let slot = match axis {
                    Axis::X => &mut x,
                    Axis::Y => &mut y,
                    Axis::Either if x.is_none() && y.is_some() => &mut x,
                    Axis::Either => &mut y,
                };
                if slot.is_some() {
                    return None;
                }
                *slot = Some(value);
                index += if offset.is_some() { 2 } else { 1 };
            }
            Some(BackgroundPosition {
                x: x.unwrap_or(percent(0.5)),
                y: y.unwrap_or(percent(0.5)),
            })
        }
        _ => None,
    }
}

/// An offset from the right or bottom edge as one from the left or top.
fn from_far_edge(offset: LengthPercentage) -> LengthPercentage {
    use crate::values::Calc;
    let offset = match offset {
        LengthPercentage::Px(value) => Calc::Px(value),
        LengthPercentage::Percent(fraction) => return LengthPercentage::Percent(1.0 - fraction),
        LengthPercentage::Calc(calc) => *calc,
    };
    LengthPercentage::Calc(Box::new(Calc::Sum(vec![
        Calc::Percent(1.0),
        Calc::Product(Box::new(offset), -1.0),
    ])))
}

/// `background-image`: the first layer.
pub fn background_image(values: &[ComponentValue], context: &Context) -> Option<BackgroundImage> {
    let layers = split_commas(values);
    let first = layers.first()?;
    let [layer] = first.as_slice() else {
        return None;
    };
    match layer {
        ComponentValue::Token(Token::Ident(name)) if name.eq_ignore_ascii_case("none") => {
            Some(BackgroundImage::None)
        }
        ComponentValue::Token(Token::Url(url)) => Some(BackgroundImage::Url(url.clone())),
        ComponentValue::Function(name, arguments) => {
            let parts = split_commas(arguments);
            match name.as_str() {
                "url" => match single(arguments)? {
                    ComponentValue::Token(Token::String(url)) => {
                        Some(BackgroundImage::Url(url.clone()))
                    }
                    _ => None,
                },
                "linear-gradient" | "repeating-linear-gradient" | "-webkit-linear-gradient" => {
                    let (interpolation, head) = gradient_interpolation(parts.first()?)?;
                    let (angle, stops) = match head.as_slice() {
                        [] => (180.0, &parts[1..]),
                        [value] if angle(value).is_some() => (angle(value)?, &parts[1..]),
                        [to, sides @ ..] if to.is_ident("to") => (side_angle(sides)?, &parts[1..]),
                        _ if interpolation.is_some() => return None,
                        _ => (180.0, &parts[..]),
                    };
                    Some(BackgroundImage::LinearGradient {
                        angle,
                        stops: interpolated(gradient_stops(stops, context)?, interpolation),
                    })
                }
                "radial-gradient" | "repeating-radial-gradient" => {
                    let (interpolation, head) = gradient_interpolation(parts.first()?)?;
                    let stops = match color_stop(&head, context) {
                        Some(_) if interpolation.is_none() => &parts[..],
                        _ => &parts[1..],
                    };
                    Some(BackgroundImage::RadialGradient {
                        stops: interpolated(gradient_stops(stops, context)?, interpolation),
                    })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// `box-shadow`.
pub fn box_shadow(values: &[ComponentValue], context: &Context) -> Option<Shadows> {
    if single(values).is_some_and(|value| value.is_ident("none")) {
        return Some(Shadows::default());
    }
    let mut shadows = Vec::new();
    for part in split_commas(values) {
        let mut lengths = Vec::new();
        let mut shadow_color = None;
        let mut inset = false;
        for value in &part {
            if value.is_ident("inset") {
                inset = true;
            } else if let Some(LengthPercentage::Px(px)) = length_percentage(value, &context.length)
            {
                lengths.push(px);
            } else {
                shadow_color = Some(color(core::slice::from_ref(value), context)?);
            }
        }
        let (x, y, blur, spread) = match lengths.as_slice() {
            [x, y] => (*x, *y, 0.0, 0.0),
            [x, y, blur] => (*x, *y, *blur, 0.0),
            [x, y, blur, spread] => (*x, *y, *blur, *spread),
            _ => return None,
        };
        shadows.push(Shadow {
            x,
            y,
            blur,
            spread,
            color: shadow_color.unwrap_or(context.color),
            inset,
        });
    }
    Some(Shadows(shadows))
}

/// `filter` and `backdrop-filter`: a list of filter functions. SVG filter
/// references are not supported and make the declaration invalid.
pub fn filter(values: &[ComponentValue], context: &Context) -> Option<Filters> {
    if single(values).is_some_and(|value| value.is_ident("none")) {
        return Some(Filters::default());
    }
    let mut filters = Vec::new();
    let functions = without_whitespace(values);
    if functions.is_empty() {
        return None;
    }
    for value in functions {
        let ComponentValue::Function(name, arguments) = value else {
            return None;
        };
        let arguments = without_whitespace(&arguments);
        let amount = |clamp: bool| -> Option<f32> {
            let amount = match arguments.as_slice() {
                [] => 1.0,
                [ComponentValue::Token(Token::Percentage(percent))] => *percent as f32 / 100.0,
                [other] => number(other, &context.length)?,
                _ => return None,
            };
            (amount >= 0.0).then_some(if clamp { amount.min(1.0) } else { amount })
        };
        filters.push(match name.as_str() {
            "blur" => Filter::Blur(match arguments.as_slice() {
                [] => 0.0,
                [value] => match length_percentage(value, &context.length)? {
                    LengthPercentage::Px(px) if px >= 0.0 => px,
                    _ => return None,
                },
                _ => return None,
            }),
            "brightness" => Filter::Brightness(amount(false)?),
            "contrast" => Filter::Contrast(amount(false)?),
            "grayscale" => Filter::Grayscale(amount(true)?),
            "invert" => Filter::Invert(amount(true)?),
            "opacity" => Filter::Opacity(amount(true)?),
            "saturate" => Filter::Saturate(amount(false)?),
            "sepia" => Filter::Sepia(amount(true)?),
            "hue-rotate" => Filter::HueRotate(match arguments.as_slice() {
                [] => 0.0,
                [value] => angle(value)?,
                _ => return None,
            }),
            "drop-shadow" => {
                let mut lengths = Vec::new();
                let mut shadow_color = None;
                let mut color_after = false;
                for argument in &arguments {
                    match length_percentage(argument, &context.length) {
                        Some(LengthPercentage::Px(px)) if !color_after => lengths.push(px),
                        Some(_) => return None,
                        None if shadow_color.is_none() => {
                            shadow_color = Some(color(core::slice::from_ref(argument), context)?);
                            color_after = !lengths.is_empty();
                        }
                        None => return None,
                    }
                }
                let (x, y, blur) = match lengths.as_slice() {
                    [x, y] => (*x, *y, 0.0),
                    [x, y, blur] if *blur >= 0.0 => (*x, *y, *blur),
                    _ => return None,
                };
                Filter::DropShadow {
                    x,
                    y,
                    blur,
                    color: shadow_color.unwrap_or(context.color),
                }
            }
            _ => return None,
        });
    }
    Some(Filters(filters))
}

fn origin_axis(
    value: &ComponentValue,
    horizontal: bool,
    context: &Context,
) -> Option<LengthPercentage> {
    match value.ident().as_deref() {
        Some("center") => Some(LengthPercentage::Percent(0.5)),
        Some("left") if horizontal => Some(LengthPercentage::Percent(0.0)),
        Some("right") if horizontal => Some(LengthPercentage::Percent(1.0)),
        Some("top") if !horizontal => Some(LengthPercentage::Percent(0.0)),
        Some("bottom") if !horizontal => Some(LengthPercentage::Percent(1.0)),
        _ => length_percentage(value, &context.length),
    }
}

/// `transform-origin`: ordered offsets or either-order keyword pairs.
pub fn transform_origin(values: &[ComponentValue], context: &Context) -> Option<TransformOrigin> {
    let parts = without_whitespace(values);
    if !(1..=3).contains(&parts.len()) {
        return None;
    }
    if parts.len() == 1 {
        let vertical = parts[0].is_ident("top") || parts[0].is_ident("bottom");
        let offset = origin_axis(&parts[0], !vertical, context)?;
        let center = LengthPercentage::Percent(0.5);
        return Some(TransformOrigin {
            x: if vertical {
                center.clone()
            } else {
                offset.clone()
            },
            y: if vertical { offset } else { center },
            z: Px(0.0),
        });
    }
    let (x, y) = origin_axis(&parts[0], true, context)
        .zip(origin_axis(&parts[1], false, context))
        .or_else(|| {
            // Reordering is valid only for keyword pairs, never lengths.
            if parts[0].ident().is_some() && parts[1].ident().is_some() {
                origin_axis(&parts[1], true, context).zip(origin_axis(&parts[0], false, context))
            } else {
                None
            }
        })?;
    let z = match parts.get(2) {
        Some(value) => match length_percentage(value, &context.length)? {
            LengthPercentage::Px(value) => Px(value),
            _ => return None,
        },
        None => Px(0.0),
    };
    Some(TransformOrigin { x, y, z })
}

/// `transform`: translation, scale and rotation are kept; other functions are ignored.
/// `translate`: `none` or one to three lengths; the third, a depth, shows
/// as nothing without perspective.
pub fn translate(values: &[ComponentValue], context: &Context) -> Option<IndividualTransform> {
    let values = without_whitespace(values);
    let function = match values.as_slice() {
        [value] if value.is_ident("none") => return Some(IndividualTransform::default()),
        [x] => TransformFunction::Translate(
            length_percentage(x, &context.length)?,
            LengthPercentage::Px(0.0),
        ),
        [x, y] => TransformFunction::Translate(
            length_percentage(x, &context.length)?,
            length_percentage(y, &context.length)?,
        ),
        [x, y, z] => {
            let LengthPercentage::Px(_) = length_percentage(z, &context.length)? else {
                return None;
            };
            TransformFunction::Translate(
                length_percentage(x, &context.length)?,
                length_percentage(y, &context.length)?,
            )
        }
        _ => return None,
    };
    Some(IndividualTransform(Transform(vec![function])))
}

/// `rotate`: `none`, an angle, or an axis and an angle.
pub fn rotate(values: &[ComponentValue], context: &Context) -> Option<IndividualTransform> {
    let values = without_whitespace(values);
    let axis = |name: &str| match name {
        "x" => Some([1.0, 0.0, 0.0]),
        "y" => Some([0.0, 1.0, 0.0]),
        "z" => Some([0.0, 0.0, 1.0]),
        _ => None,
    };
    let function = match values.as_slice() {
        [value] if value.is_ident("none") => return Some(IndividualTransform::default()),
        [turn] => TransformFunction::Rotate(angle(turn)?),
        [name, turn] | [turn, name] if name.ident().is_some() => {
            match axis(name.ident()?.to_ascii_lowercase().as_str())? {
                [0.0, 0.0, 1.0] => TransformFunction::Rotate(angle(turn)?),
                axis => TransformFunction::Rotate3d(axis, angle(turn)?),
            }
        }
        [x, y, z, turn] => TransformFunction::Rotate3d(
            [
                number(x, &context.length)?,
                number(y, &context.length)?,
                number(z, &context.length)?,
            ],
            angle(turn)?,
        ),
        _ => return None,
    };
    Some(IndividualTransform(Transform(vec![function])))
}

/// `scale`: `none` or one to three numbers or percentages; one scales both
/// axes, and the third, a depth, shows as nothing without perspective.
pub fn scale(values: &[ComponentValue], context: &Context) -> Option<IndividualTransform> {
    let values = without_whitespace(values);
    if let [value] = values.as_slice()
        && value.is_ident("none")
    {
        return Some(IndividualTransform::default());
    }
    let factors: Option<Vec<f32>> = values
        .iter()
        .map(|value| match value {
            ComponentValue::Token(Token::Percentage(percent)) => Some(*percent as f32 / 100.0),
            other => number(other, &context.length),
        })
        .collect();
    let (x, y) = match factors?.as_slice() {
        [both] => (*both, *both),
        [x, y] | [x, y, _] => (*x, *y),
        _ => return None,
    };
    Some(IndividualTransform(Transform(vec![
        TransformFunction::Scale(x, y),
    ])))
}

pub fn transform(values: &[ComponentValue], context: &Context) -> Option<Transform> {
    let mut result = Transform::default();
    if single(values).is_some_and(|value| value.is_ident("none")) {
        return Some(result);
    }
    let functions = &mut result.0;
    for value in without_whitespace(values) {
        let ComponentValue::Function(name, arguments) = value else {
            return None;
        };
        let arguments: Vec<ComponentValue> =
            split_commas(&arguments).into_iter().flatten().collect();
        let lengths = || -> Option<Vec<LengthPercentage>> {
            arguments
                .iter()
                .map(|argument| length_percentage(argument, &context.length))
                .collect()
        };
        let numbers = || -> Option<Vec<f32>> {
            arguments
                .iter()
                .map(|argument| number(argument, &context.length))
                .collect()
        };
        let angles = || -> Option<Vec<f32>> { arguments.iter().map(angle).collect() };
        let zero = LengthPercentage::Px(0.0);
        let function = match name.to_ascii_lowercase().as_str() {
            "translate" | "translate3d" => {
                let lengths = lengths()?;
                let count = if name.eq_ignore_ascii_case("translate3d") {
                    3..=3
                } else {
                    1..=2
                };
                if !count.contains(&lengths.len()) {
                    return None;
                }
                TransformFunction::Translate(
                    lengths[0].clone(),
                    lengths.get(1).cloned().unwrap_or(zero),
                )
            }
            "translatex" => TransformFunction::Translate(one(lengths()?)?, zero),
            "translatey" => TransformFunction::Translate(zero, one(lengths()?)?),
            "scale" => match numbers()?.as_slice() {
                [both] => TransformFunction::Scale(*both, *both),
                [x, y] => TransformFunction::Scale(*x, *y),
                _ => return None,
            },
            "scale3d" => match numbers()?.as_slice() {
                [x, y, _] => TransformFunction::Scale(*x, *y),
                _ => return None,
            },
            "scalex" => TransformFunction::Scale(one(numbers()?)?, 1.0),
            "scaley" => TransformFunction::Scale(1.0, one(numbers()?)?),
            "rotate" | "rotatez" => TransformFunction::Rotate(one(angles()?)?),
            "skew" => match angles()?.as_slice() {
                [x] => TransformFunction::Skew(*x, 0.0),
                [x, y] => TransformFunction::Skew(*x, *y),
                _ => return None,
            },
            "skewx" => TransformFunction::Skew(one(angles()?)?, 0.0),
            "skewy" => TransformFunction::Skew(0.0, one(angles()?)?),
            "matrix" => TransformFunction::Matrix(numbers()?.try_into().ok()?),
            "rotatex" => TransformFunction::Rotate3d([1.0, 0.0, 0.0], one(angles()?)?),
            "rotatey" => TransformFunction::Rotate3d([0.0, 1.0, 0.0], one(angles()?)?),
            "rotate3d" => {
                let [x, y, z, rotation] = arguments.as_slice() else {
                    return None;
                };
                let axis = [x, y, z].map(|value| number(value, &context.length));
                let [Some(x), Some(y), Some(z)] = axis else {
                    return None;
                };
                TransformFunction::Rotate3d([x, y, z], angle(rotation)?)
            }
            "matrix3d" => TransformFunction::Matrix3d(numbers()?.try_into().ok()?),
            "translatez" => {
                one(lengths()?)?;
                continue;
            }
            "scalez" => {
                one(numbers()?)?;
                continue;
            }
            "perspective" => {
                let [depth] = arguments.as_slice() else {
                    return None;
                };
                if !depth.is_ident("none") {
                    length_percentage(depth, &context.length)?;
                }
                continue;
            }
            _ => return None,
        };
        functions.push(function);
    }
    Some(result)
}

/// The only value of a function's arguments.
fn one<T>(values: Vec<T>) -> Option<T> {
    let mut values = values.into_iter();
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

/// `aspect-ratio`.
pub fn aspect_ratio(values: &[ComponentValue], context: &Context) -> Option<AspectRatio> {
    let parts: Vec<ComponentValue> = without_whitespace(values)
        .into_iter()
        .filter(|value| !value.is_ident("auto"))
        .collect();
    let ratio = match parts.as_slice() {
        [] => return Some(AspectRatio(None)),
        [value] => number(value, &context.length)?,
        [left, ComponentValue::Token(Token::Delim('/')), right] => {
            number(left, &context.length)? / number(right, &context.length)?
        }
        _ => return None,
    };
    (ratio.is_finite() && ratio > 0.0).then_some(AspectRatio(Some(ratio)))
}

fn track_size(value: &ComponentValue, context: &Context) -> Option<TrackSize> {
    match value {
        ComponentValue::Token(Token::Ident(name)) => match name.to_ascii_lowercase().as_str() {
            "auto" => Some(TrackSize::Auto),
            "min-content" => Some(TrackSize::MinContent),
            "max-content" => Some(TrackSize::MaxContent),
            _ => None,
        },
        ComponentValue::Token(Token::Dimension { value, unit })
            if unit.eq_ignore_ascii_case("fr") =>
        {
            (*value >= 0.0).then_some(TrackSize::Fr(*value as f32))
        }
        ComponentValue::Function(name, arguments) if name == "minmax" => {
            let parts = split_commas(arguments);
            let [minimum, maximum] = parts.as_slice() else {
                return None;
            };
            Some(TrackSize::MinMax(
                Box::new(track_size(single(minimum)?, context)?),
                Box::new(track_size(single(maximum)?, context)?),
            ))
        }
        ComponentValue::Function(name, arguments) if name == "fit-content" => Some(
            TrackSize::FitContent(non_negative(single(arguments)?, &context.length)?),
        ),
        other => non_negative(other, &context.length).map(TrackSize::Length),
    }
}

fn track_sizes_of(values: &[ComponentValue], context: &Context) -> Option<Vec<TrackSize>> {
    values
        .iter()
        .filter(|value| !matches!(value, ComponentValue::Brackets(_)))
        .map(|value| track_size(value, context))
        .collect()
}

/// `grid-template-columns` and `grid-template-rows`.
pub fn track_list(values: &[ComponentValue], context: &Context) -> Option<TrackList> {
    let values = without_whitespace(values);
    if single(&values).is_some_and(|value| value.is_ident("none")) {
        return Some(TrackList::default());
    }
    let mut entries = Vec::new();
    for value in values {
        match value {
            ComponentValue::Brackets(_) => {}
            ComponentValue::Function(name, arguments) if name == "repeat" => {
                let comma = arguments
                    .iter()
                    .position(|argument| matches!(argument, ComponentValue::Token(Token::Comma)))?;
                let count = match single(&arguments[..comma])? {
                    ComponentValue::Token(Token::Number {
                        value,
                        integer: true,
                    }) if *value >= 1.0 => RepeatCount::Count(*value as u16),
                    other if other.is_ident("auto-fill") => RepeatCount::AutoFill,
                    other if other.is_ident("auto-fit") => RepeatCount::AutoFit,
                    _ => return None,
                };
                let tracks = track_sizes_of(&without_whitespace(&arguments[comma + 1..]), context)?;
                if tracks.is_empty() {
                    return None;
                }
                entries.push(TrackEntry::Repeat(count, tracks));
            }
            other => entries.push(TrackEntry::Single(track_size(&other, context)?)),
        }
    }
    (!entries.is_empty()).then_some(TrackList(entries))
}

/// `grid-auto-columns` and `grid-auto-rows`.
pub fn track_sizes(values: &[ComponentValue], context: &Context) -> Option<TrackSizes> {
    let sizes = track_sizes_of(&without_whitespace(values), context)?;
    (!sizes.is_empty()).then_some(TrackSizes(sizes))
}

/// `grid-row-start` and the other placement longhands.
pub fn grid_line(values: &[ComponentValue], _: &Context) -> Option<GridLine> {
    let values = without_whitespace(values);
    let mut span = false;
    let mut line = None;
    for value in &values {
        match value {
            ComponentValue::Token(Token::Ident(word)) if word.eq_ignore_ascii_case("span") => {
                span = true;
            }
            ComponentValue::Token(Token::Ident(word))
                if word.eq_ignore_ascii_case("auto") && values.len() == 1 =>
            {
                return Some(GridLine::Auto);
            }
            ComponentValue::Token(Token::Ident(word)) if values.len() == 1 => {
                return Some(GridLine::Named(word.clone()));
            }
            ComponentValue::Token(Token::Number {
                value,
                integer: true,
            }) if *value != 0.0 => line = Some(*value as i32),
            _ => return None,
        }
    }
    match (span, line) {
        (true, Some(count)) if count > 0 => Some(GridLine::Span(count as u16)),
        (true, None) => Some(GridLine::Span(1)),
        (false, Some(line)) => Some(GridLine::Line(line as i16)),
        _ => None,
    }
}

/// `grid-template-areas`: `none`, or strings of cells, each a name or a
/// run of `.` for none, every string as long as the first; each name must
/// fill a rectangle.
pub fn grid_template_areas(values: &[ComponentValue], _: &Context) -> Option<GridTemplateAreas> {
    let values = without_whitespace(values);
    if let [ComponentValue::Token(Token::Ident(word))] = values.as_slice()
        && word.eq_ignore_ascii_case("none")
    {
        return Some(GridTemplateAreas::default());
    }
    let mut rows: Vec<Vec<Option<String>>> = Vec::new();
    for value in &values {
        let ComponentValue::Token(Token::String(text)) = value else {
            return None;
        };
        let row = area_cells(text)?;
        if row.is_empty() || rows.first().is_some_and(|first| first.len() != row.len()) {
            return None;
        }
        rows.push(row);
    }
    let columns = rows.first()?.len();
    let mut areas: Vec<GridArea> = Vec::new();
    for (row, cells) in rows.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            let Some(name) = cell else { continue };
            if areas.iter().any(|area| &area.name == name) {
                continue;
            }
            // The area's rectangle runs right and down from its first cell.
            let width = cells[column..]
                .iter()
                .take_while(|other| other.as_ref() == Some(name))
                .count();
            let height = rows[row..]
                .iter()
                .take_while(|other| other[column].as_ref() == Some(name))
                .count();
            let filled = rows.iter().enumerate().all(|(y, cells)| {
                cells.iter().enumerate().all(|(x, other)| {
                    let inside =
                        (row..row + height).contains(&y) && (column..column + width).contains(&x);
                    inside == (other.as_ref() == Some(name))
                })
            });
            if !filled {
                return None;
            }
            areas.push(GridArea {
                name: name.clone(),
                row_start: row as u16 + 1,
                row_end: (row + height) as u16 + 1,
                column_start: column as u16 + 1,
                column_end: (column + width) as u16 + 1,
            });
        }
    }
    Some(GridTemplateAreas {
        areas,
        rows: rows.len() as u16,
        columns: columns as u16,
    })
}

/// The cells of a `grid-template-areas` string: names, and `None` for each
/// run of `.`; `None` for a string with other characters.
fn area_cells(text: &str) -> Option<Vec<Option<String>>> {
    let mut cells = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&next) = chars.peek() {
        if next.is_whitespace() {
            chars.next();
        } else if next == '.' {
            while chars.peek() == Some(&'.') {
                chars.next();
            }
            cells.push(None);
        } else if next.is_alphanumeric() || next == '-' || next == '_' || !next.is_ascii() {
            let mut name = String::new();
            while let Some(&part) = chars.peek() {
                if part.is_alphanumeric() || part == '-' || part == '_' || !part.is_ascii() {
                    name.push(part);
                    chars.next();
                } else {
                    break;
                }
            }
            cells.push(Some(name));
        } else {
            return None;
        }
    }
    Some(cells)
}

/// `grid-auto-flow`.
pub fn grid_auto_flow(values: &[ComponentValue], _: &Context) -> Option<GridAutoFlow> {
    let mut flow = GridAutoFlow::default();
    let mut seen = false;
    for value in without_whitespace(values) {
        match value.ident()?.as_str() {
            "row" => seen = true,
            "column" => {
                flow.column = true;
                seen = true;
            }
            "dense" => flow.dense = true,
            _ => return None,
        }
    }
    (seen || flow.dense).then_some(flow)
}

/// Implemented by keyword enums.
pub trait Keyword: Sized {
    /// Parses a lowercase keyword.
    fn parse_keyword(keyword: &str) -> Option<Self>;
}

fn time(value: &ComponentValue) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Dimension { value, unit }) => {
            match unit.to_ascii_lowercase().as_str() {
                "s" => Some(*value as f32),
                "ms" => Some(*value as f32 / 1000.0),
                _ => None,
            }
        }
        _ => None,
    }
}

fn easing(value: &ComponentValue, context: &Context) -> Option<Easing> {
    if let Some(name) = value.ident() {
        return Some(match name.as_str() {
            "linear" => Easing::LINEAR,
            "ease" => Easing::EASE,
            "ease-in" => Easing::EASE_IN,
            "ease-out" => Easing::EASE_OUT,
            "ease-in-out" => Easing::EASE_IN_OUT,
            "step-start" => Easing::Steps(1, StepPosition::Start),
            "step-end" => Easing::Steps(1, StepPosition::End),
            _ => return None,
        });
    }
    let ComponentValue::Function(name, arguments) = value else {
        return None;
    };
    let parts = split_commas(arguments);
    match (name.as_str(), parts.as_slice()) {
        ("cubic-bezier", [x1, y1, x2, y2]) => {
            let (x1, y1, x2, y2) = (
                plain_number(x1, context)?,
                plain_number(y1, context)?,
                plain_number(x2, context)?,
                plain_number(y2, context)?,
            );
            ((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2))
                .then_some(Easing::CubicBezier(x1, y1, x2, y2))
        }
        ("steps", [count, positions @ ..]) => {
            let count = integer(count, context).filter(|count| *count > 0)? as u32;
            let position = match positions {
                [] => StepPosition::End,
                [position] => match single_ident(position)?.as_str() {
                    "start" | "jump-start" => StepPosition::Start,
                    "end" | "jump-end" => StepPosition::End,
                    "jump-none" if count > 1 => StepPosition::None,
                    "jump-both" => StepPosition::Both,
                    _ => return None,
                },
                _ => return None,
            };
            Some(Easing::Steps(count, position))
        }
        _ => None,
    }
}

fn list<T>(
    values: &[ComponentValue],
    context: &Context,
    item: fn(&ComponentValue, &Context) -> Option<T>,
) -> Option<List<T>> {
    let items: Option<Vec<T>> = split_commas(values)
        .iter()
        .map(|part| match part.as_slice() {
            [value] => item(value, context),
            _ => None,
        })
        .collect();
    Some(List(items?))
}

/// Non-negative transition durations.
pub fn durations(values: &[ComponentValue], context: &Context) -> Option<List<Seconds>> {
    list(values, context, |value, _| {
        time(value).filter(|time| *time >= 0.0).map(Seconds)
    })
}

/// Transition delays, including negative delays.
pub fn delays(values: &[ComponentValue], context: &Context) -> Option<List<Seconds>> {
    list(values, context, |value, _| time(value).map(Seconds))
}

/// Transition timing functions.
pub fn easings(values: &[ComponentValue], context: &Context) -> Option<List<Easing>> {
    list(values, context, easing)
}

/// `font-variant-numeric`: `normal`, or at most one keyword of each group.
pub fn font_variant_numeric(values: &[ComponentValue], _: &Context) -> Option<FontVariantNumeric> {
    let words: Vec<String> = without_whitespace(values)
        .iter()
        .map(|value| value.ident().map(|word| word.to_ascii_lowercase()))
        .collect::<Option<_>>()?;
    if words.len() == 1 && words[0] == "normal" {
        return Some(FontVariantNumeric::default());
    }
    let mut groups = [false; 5];
    for word in &words {
        let (_, _, group) = FontVariantNumeric::KEYWORDS
            .iter()
            .find(|(keyword, _, _)| keyword == word)?;
        if core::mem::replace(&mut groups[usize::from(*group)], true) {
            return None;
        }
    }
    (!words.is_empty()).then(|| {
        FontVariantNumeric(
            FontVariantNumeric::KEYWORDS
                .iter()
                .filter(|(keyword, _, _)| words.iter().any(|word| word == keyword))
                .map(|(keyword, _, _)| (*keyword).to_owned())
                .collect(),
        )
    })
}

/// `font-feature-settings`: `normal`, or four-letter tags each with an
/// optional value, `on` (1) or `off` (0).
pub fn font_feature_settings(
    values: &[ComponentValue],
    _: &Context,
) -> Option<FontFeatureSettings> {
    if single_ident(values).is_some_and(|word| word.eq_ignore_ascii_case("normal")) {
        return Some(FontFeatureSettings::default());
    }
    split_commas(values)
        .iter()
        .map(|part| {
            let part = without_whitespace(part);
            let (tag, rest) = part.split_first()?;
            let ComponentValue::Token(Token::String(tag)) = tag else {
                return None;
            };
            let tag: [u8; 4] = tag
                .as_bytes()
                .try_into()
                .ok()
                .filter(|tag: &[u8; 4]| tag.iter().all(|byte| (0x20..=0x7E).contains(byte)))?;
            let value = match rest {
                [] => 1,
                [
                    ComponentValue::Token(Token::Number {
                        value,
                        integer: true,
                    }),
                ] if *value >= 0.0 => *value as u32,
                [ComponentValue::Token(Token::Ident(word))] => {
                    match word.to_ascii_lowercase().as_str() {
                        "on" => 1,
                        "off" => 0,
                        _ => return None,
                    }
                }
                _ => return None,
            };
            Some((tag, value))
        })
        .collect::<Option<Vec<_>>>()
        .map(FontFeatureSettings)
}

/// Animation names: identifiers or strings, and `none` for no animation.
pub fn animation_names(values: &[ComponentValue], context: &Context) -> Option<List<Ident>> {
    list(values, context, |value, _| match value {
        ComponentValue::Token(Token::Ident(name))
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "default"
            ) =>
        {
            Some(Ident(if name.eq_ignore_ascii_case("none") {
                "none".to_owned()
            } else {
                name.clone()
            }))
        }
        ComponentValue::Token(Token::String(name)) => Some(Ident(name.clone())),
        _ => None,
    })
}

/// Animation iteration counts: non-negative numbers or `infinite`.
pub fn iteration_counts(values: &[ComponentValue], context: &Context) -> Option<List<Iterations>> {
    list(values, context, |value, _| match value {
        ComponentValue::Token(Token::Number { value, .. }) if *value >= 0.0 => {
            Some(Iterations::Count(*value as f32))
        }
        ComponentValue::Token(Token::Ident(word)) if word.eq_ignore_ascii_case("infinite") => {
            Some(Iterations::Infinite)
        }
        _ => None,
    })
}

/// A comma-separated list of keywords.
pub fn keyword_list<K: Keyword>(values: &[ComponentValue], context: &Context) -> Option<List<K>> {
    list(values, context, |value, _| match value {
        ComponentValue::Token(Token::Ident(word)) => K::parse_keyword(&word.to_ascii_lowercase()),
        _ => None,
    })
}

/// Transition property names; `none` must appear on its own.
pub fn transition_properties(values: &[ComponentValue], context: &Context) -> Option<List<Ident>> {
    let names = list(values, context, |value, _| match value {
        ComponentValue::Token(Token::Ident(name))
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "default"
            ) =>
        {
            Some(Ident(
                if name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("none") {
                    name.to_ascii_lowercase()
                } else {
                    name.clone()
                },
            ))
        }
        _ => None,
    })?;
    (!names.0.iter().any(|name| name.0 == "none") || names.0.len() == 1).then_some(names)
}

#[cfg(test)]
mod tests;
