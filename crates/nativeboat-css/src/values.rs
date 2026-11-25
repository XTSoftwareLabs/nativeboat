//! Component values, lengths and `calc()`.

use crate::prelude::*;

use crate::tokenizer::{Token, tokens};
use serde::{Deserialize, Serialize};

/// A component value: a token, a function or a simple block.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentValue {
    /// A preserved token.
    Token(Token),
    /// A function with its arguments.
    Function(String, Vec<ComponentValue>),
    /// A `( ... )` block.
    Parens(Vec<ComponentValue>),
    /// A `[ ... ]` block.
    Brackets(Vec<ComponentValue>),
}

impl ComponentValue {
    /// Returns the identifier if this is one, lowercased.
    pub fn ident(&self) -> Option<String> {
        match self {
            ComponentValue::Token(Token::Ident(name)) => Some(name.to_ascii_lowercase()),
            _ => None,
        }
    }

    /// Whether this is the given identifier, ignoring ASCII case.
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(self, ComponentValue::Token(Token::Ident(ident)) if ident.eq_ignore_ascii_case(name))
    }

    /// Returns the plain number if this is one.
    pub fn number(&self) -> Option<f64> {
        match self {
            ComponentValue::Token(Token::Number { value, .. }) => Some(*value),
            _ => None,
        }
    }

    fn is_whitespace(&self) -> bool {
        matches!(self, ComponentValue::Token(Token::Whitespace))
    }

    fn is_comma(&self) -> bool {
        matches!(self, ComponentValue::Token(Token::Comma))
    }
}

/// Parses text into component values, keeping whitespace tokens.
pub fn parse_component_values(text: &str) -> Vec<ComponentValue> {
    let tokens = tokens(text);
    let mut index = 0;
    consume_list(&tokens, &mut index, None)
}

fn consume_list(tokens: &[Token], index: &mut usize, close: Option<&Token>) -> Vec<ComponentValue> {
    let mut values = Vec::new();
    while let Some(token) = tokens.get(*index) {
        *index += 1;
        if Some(token) == close {
            break;
        }
        values.push(match token {
            Token::Function(name) => ComponentValue::Function(
                name.to_ascii_lowercase(),
                consume_list(tokens, index, Some(&Token::RightParen)),
            ),
            Token::LeftParen => {
                ComponentValue::Parens(consume_list(tokens, index, Some(&Token::RightParen)))
            }
            Token::LeftBracket => {
                ComponentValue::Brackets(consume_list(tokens, index, Some(&Token::RightBracket)))
            }
            other => ComponentValue::Token(other.clone()),
        });
    }
    values
}

/// Removes whitespace component values.
pub fn without_whitespace(values: &[ComponentValue]) -> Vec<ComponentValue> {
    values
        .iter()
        .filter(|value| !value.is_whitespace())
        .cloned()
        .collect()
}

/// Splits values at top-level commas, dropping whitespace.
pub fn split_commas(values: &[ComponentValue]) -> Vec<Vec<ComponentValue>> {
    values
        .split(ComponentValue::is_comma)
        .map(without_whitespace)
        .collect()
}

/// What relative lengths resolve against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LengthContext {
    /// The element's font size in pixels (the parent's for `font-size`).
    pub font_size: f32,
    /// The root element's font size in pixels.
    pub root_font_size: f32,
    /// Viewport width in pixels.
    pub viewport_width: f32,
    /// Viewport height in pixels.
    pub viewport_height: f32,
    /// The content box width and height of the nearest query container,
    /// for container query units; the viewport's without one.
    pub container: Option<(f32, f32)>,
}

impl Default for LengthContext {
    fn default() -> Self {
        Self {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
            container: None,
        }
    }
}

impl LengthContext {
    /// Converts a dimension to pixels.
    pub fn to_px(&self, value: f64, unit: &str) -> Option<f32> {
        let value = value as f32;
        Some(match unit.to_ascii_lowercase().as_str() {
            "px" => value,
            "em" => value * self.font_size,
            "rem" => value * self.root_font_size,
            "ex" | "ch" => value * self.font_size * 0.5,
            "lh" => value * self.font_size * 1.2,
            "vw" | "svw" | "lvw" | "dvw" => value * self.viewport_width / 100.0,
            "vh" | "svh" | "lvh" | "dvh" => value * self.viewport_height / 100.0,
            "vmin" => value * self.viewport_width.min(self.viewport_height) / 100.0,
            "vmax" => value * self.viewport_width.max(self.viewport_height) / 100.0,
            "cqw" | "cqi" | "cqh" | "cqb" | "cqmin" | "cqmax" => {
                let (width, height) = self
                    .container
                    .unwrap_or((self.viewport_width, self.viewport_height));
                value
                    * match unit.to_ascii_lowercase().as_str() {
                        "cqw" | "cqi" => width,
                        "cqh" | "cqb" => height,
                        "cqmin" => width.min(height),
                        _ => width.max(height),
                    }
                    / 100.0
            }
            "pt" => value * 96.0 / 72.0,
            "pc" => value * 16.0,
            "in" => value * 96.0,
            "cm" => value * 96.0 / 2.54,
            "mm" => value * 96.0 / 25.4,
            "q" => value * 96.0 / 101.6,
            _ => return None,
        })
    }
}

/// A computed `calc()` expression over pixels and percentages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Calc {
    /// Pixels.
    Px(f32),
    /// A fraction of the percentage basis (0.5 is 50%).
    Percent(f32),
    /// The sum of the terms.
    Sum(Vec<Calc>),
    /// A term scaled by a number.
    Product(Box<Calc>, f32),
    /// The smallest term.
    Min(Vec<Calc>),
    /// The largest term.
    Max(Vec<Calc>),
}

impl Calc {
    fn to_css(&self) -> String {
        match self {
            Self::Px(value) => format!("{}px", format_float(*value)),
            Self::Percent(value) => format!("{}%", format_float(*value * 100.0)),
            Self::Sum(terms) => format!(
                "({})",
                terms
                    .iter()
                    .map(Self::to_css)
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
            Self::Product(term, factor) => {
                format!("({} * {})", term.to_css(), format_float(*factor))
            }
            Self::Min(terms) | Self::Max(terms) => format!(
                "{}({})",
                if matches!(self, Self::Min(_)) {
                    "min"
                } else {
                    "max"
                },
                terms
                    .iter()
                    .map(Self::to_css)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    /// Evaluates the expression against a percentage basis.
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            Calc::Px(value) => *value,
            Calc::Percent(fraction) => fraction * basis,
            Calc::Sum(terms) => terms.iter().map(|term| term.resolve(basis)).sum(),
            Calc::Product(term, factor) => term.resolve(basis) * factor,
            Calc::Min(terms) => terms
                .iter()
                .map(|term| term.resolve(basis))
                .fold(f32::INFINITY, f32::min),
            Calc::Max(terms) => terms
                .iter()
                .map(|term| term.resolve(basis))
                .fold(f32::NEG_INFINITY, f32::max),
        }
    }

    fn has_percent(&self) -> bool {
        match self {
            Calc::Px(_) => false,
            Calc::Percent(_) => true,
            Calc::Product(term, _) => term.has_percent(),
            Calc::Sum(terms) | Calc::Min(terms) | Calc::Max(terms) => {
                terms.iter().any(Calc::has_percent)
            }
        }
    }
}

/// A computed length or percentage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LengthPercentage {
    /// Pixels.
    Px(f32),
    /// A fraction of the percentage basis (0.5 is 50%).
    Percent(f32),
    /// An expression mixing pixels and percentages.
    Calc(Box<Calc>),
}

impl Default for LengthPercentage {
    fn default() -> Self {
        LengthPercentage::Px(0.0)
    }
}

impl LengthPercentage {
    /// Resolves against a percentage basis.
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            LengthPercentage::Px(value) => *value,
            LengthPercentage::Percent(fraction) => fraction * basis,
            LengthPercentage::Calc(calc) => calc.resolve(basis),
        }
    }

    /// Serializes the computed value.
    pub fn to_css(&self) -> String {
        match self {
            LengthPercentage::Px(value) => format!("{}px", format_float(*value)),
            LengthPercentage::Percent(fraction) => format!("{}%", format_float(fraction * 100.0)),
            LengthPercentage::Calc(calc) => format!("calc({})", calc.to_css()),
        }
    }
}

/// Formats a float without a trailing `.0` and with at most 3 decimals.
pub fn format_float(value: f32) -> String {
    let rounded = crate::math::round(value * 1000.0) / 1000.0;
    if f64::from(rounded) - libm::trunc(f64::from(rounded)) == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// Parses a `<length-percentage>`, including `calc()`, `min()`, `max()` and `clamp()`.
pub fn length_percentage(
    value: &ComponentValue,
    context: &LengthContext,
) -> Option<LengthPercentage> {
    match value {
        ComponentValue::Token(Token::Dimension { value, unit }) => {
            context.to_px(*value, unit).map(LengthPercentage::Px)
        }
        ComponentValue::Token(Token::Percentage(value)) => {
            Some(LengthPercentage::Percent(*value as f32 / 100.0))
        }
        ComponentValue::Token(Token::Number { value, .. }) if *value == 0.0 => {
            Some(LengthPercentage::Px(0.0))
        }
        ComponentValue::Function(name, arguments) => {
            let calc = math_function(name, arguments, context)?;
            Some(match calc {
                Value::Length(calc) if calc.has_percent() => LengthPercentage::Calc(Box::new(calc)),
                Value::Length(calc) => LengthPercentage::Px(calc.resolve(0.0)),
                Value::Number(0.0) => LengthPercentage::Px(0.0),
                Value::Number(_) => return None,
            })
        }
        _ => None,
    }
}

/// Parses a non-negative `<length-percentage>`.
pub fn non_negative(value: &ComponentValue, context: &LengthContext) -> Option<LengthPercentage> {
    let parsed = length_percentage(value, context)?;
    match parsed {
        LengthPercentage::Px(px) if px < 0.0 => None,
        LengthPercentage::Percent(fraction) if fraction < 0.0 => None,
        other => Some(other),
    }
}

/// Parses a `<number>`, including math functions that produce numbers.
pub fn number(value: &ComponentValue, context: &LengthContext) -> Option<f32> {
    match value {
        ComponentValue::Token(Token::Number { value, .. }) => Some(*value as f32),
        ComponentValue::Function(name, arguments) => match math_function(name, arguments, context)?
        {
            Value::Number(number) => Some(number),
            Value::Length(_) => None,
        },
        _ => None,
    }
}

/// The value of a math expression: a number or a length.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f32),
    Length(Calc),
}

fn math_function(
    name: &str,
    arguments: &[ComponentValue],
    context: &LengthContext,
) -> Option<Value> {
    let terms = || -> Option<Vec<Value>> {
        split_commas(arguments)
            .iter()
            .map(|argument| sum(argument, context))
            .collect()
    };
    match name {
        "calc" | "-webkit-calc" => sum(&without_whitespace(arguments), context),
        "min" | "max" => {
            let terms = terms()?;
            let first = terms.first()?;
            if let Value::Number(_) = first {
                let numbers: Option<Vec<f32>> = terms
                    .iter()
                    .map(|term| match term {
                        Value::Number(number) => Some(*number),
                        Value::Length(_) => None,
                    })
                    .collect();
                let numbers = numbers?;
                let fold = if name == "min" { f32::min } else { f32::max };
                let start = if name == "min" {
                    f32::INFINITY
                } else {
                    f32::NEG_INFINITY
                };
                return Some(Value::Number(numbers.into_iter().fold(start, fold)));
            }
            let lengths = lengths(terms)?;
            Some(Value::Length(if name == "min" {
                Calc::Min(lengths)
            } else {
                Calc::Max(lengths)
            }))
        }
        "clamp" => {
            let terms = terms()?;
            let [minimum, preferred, maximum] = <[Value; 3]>::try_from(terms).ok()?;
            let lengths = lengths(vec![minimum, preferred, maximum])?;
            let [minimum, preferred, maximum] = <[Calc; 3]>::try_from(lengths).ok()?;
            Some(Value::Length(Calc::Max(vec![
                minimum,
                Calc::Min(vec![preferred, maximum]),
            ])))
        }
        _ => None,
    }
}

fn lengths(terms: Vec<Value>) -> Option<Vec<Calc>> {
    terms
        .into_iter()
        .map(|term| match term {
            Value::Length(calc) => Some(calc),
            Value::Number(0.0) => Some(Calc::Px(0.0)),
            Value::Number(_) => None,
        })
        .collect()
}

/// Parses `a + b - c` sums of products.
fn sum(values: &[ComponentValue], context: &LengthContext) -> Option<Value> {
    let mut terms: Vec<Value> = Vec::new();
    let mut sign = 1.0;
    let mut start = 0;
    let mut index = 0;
    let mut pending: Option<f32> = None;
    while index <= values.len() {
        let at_operator = match values.get(index) {
            None => true,
            Some(ComponentValue::Token(Token::Delim(operator @ ('+' | '-')))) if index > start => {
                pending = Some(if *operator == '+' { 1.0 } else { -1.0 });
                true
            }
            Some(_) => false,
        };
        if at_operator {
            let term = product(&values[start..index], context)?;
            terms.push(scale(term, sign));
            sign = pending.take().unwrap_or(1.0);
            start = index + 1;
        }
        index += 1;
    }
    let mut numbers = 0.0;
    let mut lengths = Vec::new();
    let mut has_number = false;
    for term in terms {
        match term {
            Value::Number(number) => {
                numbers += number;
                has_number = true;
            }
            Value::Length(calc) => lengths.push(calc),
        }
    }
    match (has_number, lengths.len()) {
        (true, 0) => Some(Value::Number(numbers)),
        (false, 1) => lengths.pop().map(Value::Length),
        (false, _) => Some(Value::Length(Calc::Sum(lengths))),
        (true, _) => None,
    }
}

fn scale(value: Value, factor: f32) -> Value {
    match value {
        Value::Number(number) => Value::Number(number * factor),
        Value::Length(calc) if factor == 1.0 => Value::Length(calc),
        Value::Length(calc) => Value::Length(Calc::Product(Box::new(calc), factor)),
    }
}

/// Parses `a * b / c` products.
fn product(values: &[ComponentValue], context: &LengthContext) -> Option<Value> {
    if values.len().is_multiple_of(2) {
        return None;
    }
    let mut result: Option<Value> = None;
    let mut operator = '*';
    for (index, value) in values.iter().enumerate() {
        if index % 2 == 1 {
            match value {
                ComponentValue::Token(Token::Delim(op @ ('*' | '/'))) => operator = *op,
                _ => return None,
            }
            continue;
        }
        let operand = atom(value, context)?;
        result = Some(match (result, operand) {
            (None, operand) => operand,
            (Some(Value::Number(left)), Value::Number(right)) if operator == '*' => {
                Value::Number(left * right)
            }
            (Some(Value::Number(left)), Value::Number(right)) => Value::Number(left / right),
            (Some(Value::Length(left)), Value::Number(right)) if operator == '*' => {
                Value::Length(Calc::Product(Box::new(left), right))
            }
            (Some(Value::Length(left)), Value::Number(right)) => {
                Value::Length(Calc::Product(Box::new(left), 1.0 / right))
            }
            (Some(Value::Number(left)), Value::Length(right)) if operator == '*' => {
                Value::Length(Calc::Product(Box::new(right), left))
            }
            _ => return None,
        });
    }
    result
}

fn atom(value: &ComponentValue, context: &LengthContext) -> Option<Value> {
    match value {
        ComponentValue::Token(Token::Number { value, .. }) => Some(Value::Number(*value as f32)),
        ComponentValue::Token(Token::Dimension { value, unit }) => context
            .to_px(*value, unit)
            .map(|px| Value::Length(Calc::Px(px))),
        ComponentValue::Token(Token::Percentage(value)) => {
            Some(Value::Length(Calc::Percent(*value as f32 / 100.0)))
        }
        ComponentValue::Parens(inner) => sum(&without_whitespace(inner), context),
        ComponentValue::Function(name, arguments) => math_function(name, arguments, context),
        ComponentValue::Token(Token::Ident(name)) => {
            Some(Value::Number(match name.to_ascii_lowercase().as_str() {
                "pi" => core::f32::consts::PI,
                "e" => core::f32::consts::E,
                "infinity" => CALC_INFINITY,
                "-infinity" => -CALC_INFINITY,
                _ => return None,
            }))
        }
        _ => None,
    }
}

/// `infinity` in math functions, as Chromium clamps it: the largest length
/// layout represents, so `calc(infinity * 1px)` is 33554432px.
const CALC_INFINITY: f32 = 33_554_432.0;

#[cfg(test)]
mod tests;
