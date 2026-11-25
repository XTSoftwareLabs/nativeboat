//! `@page` rules: the margins of printed pages.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::media::MediaEnvironment;
use crate::parser::{Rule, Stylesheet};
use crate::tokenizer::Token;
use crate::values::{ComponentValue, LengthContext, parse_component_values};

/// The margins the `@page` rules of `sheets` give every page, in CSS pixels
/// as top, right, bottom and left; `None` where no rule sets a side. Rules
/// for named or `:first`/`:left`/`:right` pages are not applied to all
/// pages, and rules in `@media` blocks apply when the environment matches.
pub fn page_margins(sheets: &[&Stylesheet], environment: &MediaEnvironment) -> [Option<f32>; 4] {
    fn visit(rules: &[Rule], environment: &MediaEnvironment, margins: &mut [Option<f32>; 4]) {
        for rule in rules {
            match rule {
                Rule::Media { query, rules } if query.matches(environment) => {
                    visit(rules, environment, margins);
                }
                Rule::Supports { rules, .. } | Rule::Layer { rules, .. } => {
                    visit(rules, environment, margins);
                }
                Rule::Page {
                    selector,
                    declarations,
                } if selector.trim().is_empty() => {
                    for declaration in declarations {
                        apply(&declaration.name, &declaration.value, margins);
                    }
                }
                _ => {}
            }
        }
    }
    let mut margins = [None; 4];
    for sheet in sheets {
        visit(&sheet.rules, environment, &mut margins);
    }
    margins
}

/// Applies a `margin` or `margin-*` declaration's absolute lengths.
fn apply(name: &str, value: &str, margins: &mut [Option<f32>; 4]) {
    let context = LengthContext::default();
    let lengths: Vec<Option<f32>> = parse_component_values(value)
        .iter()
        .filter_map(|component| match component {
            ComponentValue::Token(Token::Dimension { value, unit }) => {
                Some(context.to_px(*value, unit))
            }
            ComponentValue::Token(Token::Number { value, .. }) if *value == 0.0 => Some(Some(0.0)),
            ComponentValue::Token(Token::Whitespace) => None,
            _ => Some(None),
        })
        .collect();
    let side = |index: usize| match name {
        "margin-top" => index == 0,
        "margin-right" => index == 1,
        "margin-bottom" => index == 2,
        "margin-left" => index == 3,
        _ => false,
    };
    if name == "margin" {
        // One to four values, as the shorthand spreads them.
        let order: &[usize] = match lengths.len() {
            1 => &[0, 0, 0, 0],
            2 => &[0, 1, 0, 1],
            3 => &[0, 1, 2, 1],
            4 => &[0, 1, 2, 3],
            _ => return,
        };
        if lengths.iter().any(Option::is_none) {
            return;
        }
        for (side, value) in order.iter().enumerate() {
            margins[side] = lengths[*value];
        }
    } else if let [Some(length)] = lengths[..] {
        for (index, margin) in margins.iter_mut().enumerate() {
            if side(index) {
                *margin = Some(length);
            }
        }
    }
}

#[cfg(test)]
mod tests;
