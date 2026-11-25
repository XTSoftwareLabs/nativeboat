//! Conservative recognition of the linear gradients painted faithfully.

use nativeboat_css::color::{SpecifiedColor, SystemColors, parse_color};
use nativeboat_css::tokenizer::Token;
use nativeboat_css::values::{
    ComponentValue, parse_component_values, split_commas, without_whitespace,
};

/// One opaque linear layer with an angle or a single side, and strictly
/// increasing percentage stops. More complex gradients remain partial.
pub(super) fn supported(value: &str) -> Option<()> {
    let values = without_whitespace(&parse_component_values(value));
    let [ComponentValue::Function(name, arguments)] = values.as_slice() else {
        return None;
    };
    if name != "linear-gradient" {
        return None;
    }
    let parts = split_commas(arguments);
    let head = parts.first()?;
    let direction = match head.as_slice() {
        [ComponentValue::Token(Token::Dimension { unit, .. })] => {
            matches!(unit.as_str(), "deg" | "grad" | "rad" | "turn")
        }
        [ComponentValue::Token(Token::Number { value, .. })] => *value == 0.0,
        [to, side] => {
            to.is_ident("to")
                && ["top", "right", "bottom", "left"]
                    .iter()
                    .any(|name| side.is_ident(name))
        }
        _ => false,
    };
    let stops = &parts[usize::from(direction)..];
    if stops.len() < 2 {
        return None;
    }
    let mut previous = f64::NEG_INFINITY;
    for (index, stop) in stops.iter().enumerate() {
        let [color, positions @ ..] = stop.as_slice() else {
            return None;
        };
        let SpecifiedColor::Color(color) = parse_color(color, &SystemColors::LIGHT)? else {
            return None;
        };
        if color.a < 1.0 {
            return None;
        }
        let position = match positions {
            [] if index == 0 => Some(0.0),
            [] if index == stops.len() - 1 => Some(1.0),
            [] => None,
            [ComponentValue::Token(Token::Percentage(value))] => Some(*value / 100.0),
            _ => return None,
        };
        if let Some(position) = position {
            if !(position as f32).is_finite() || position <= previous {
                return None;
            }
            previous = position;
        }
    }
    Some(())
}
