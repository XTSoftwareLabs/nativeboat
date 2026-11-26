//! Container queries: the preludes of `@container` rules, and whether a
//! query container's size satisfies them.

use crate::prelude::*;

use crate::media::{Comparison, MediaFeature, MediaValue, parse_feature};
use crate::tokenizer::Token;
use serde::{Deserialize, Serialize};

/// The prelude of an `@container` rule: the name of the containers it
/// queries, if it names one, and its condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerQuery {
    /// The `container-name` the query container must have.
    pub name: Option<String>,
    /// What its size must satisfy.
    pub condition: ContainerCondition,
}

/// A container condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContainerCondition {
    /// A size feature, such as `(max-width: 240px)`; a range such as
    /// `(200px < width <= 400px)` gives two tests that must both hold.
    Feature(Vec<MediaFeature>),
    /// `not ...`
    Not(Box<ContainerCondition>),
    /// `... and ...`
    And(Vec<ContainerCondition>),
    /// `... or ...`
    Or(Vec<ContainerCondition>),
    /// A test this engine does not evaluate, such as `style(...)`, which
    /// is unknown and so never holds.
    Unknown,
}

/// The size of a query container, on the axes its `container-type` lets
/// queries test: `inline-size` containers have no height to query.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QueryContainer {
    /// Content box width in CSS pixels.
    pub width: Option<f32>,
    /// Content box height in CSS pixels.
    pub height: Option<f32>,
}

impl ContainerCondition {
    /// Whether the container satisfies the condition: `Some(false)` when it
    /// does not, `None` when it is unknown, as a feature the container
    /// cannot be queried for is.
    pub fn evaluate(&self, container: &QueryContainer) -> Option<bool> {
        match self {
            ContainerCondition::Feature(tests) => {
                let mut result = Some(true);
                for test in tests {
                    result = and(result, feature(test, container));
                }
                result
            }
            ContainerCondition::Not(inner) => inner.evaluate(container).map(|result| !result),
            ContainerCondition::And(parts) => parts.iter().fold(Some(true), |result, part| {
                and(result, part.evaluate(container))
            }),
            ContainerCondition::Or(parts) => {
                parts.iter().fold(Some(false), |result, part| {
                    match (result, part.evaluate(container)) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    }
                })
            }
            ContainerCondition::Unknown => None,
        }
    }
}

impl ContainerQuery {
    /// Whether the query container satisfies the query; unknown is false.
    pub fn matches(&self, container: &QueryContainer) -> bool {
        self.condition.evaluate(container) == Some(true)
    }
}

/// Three-valued `and`.
fn and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// Whether a container satisfies one size feature test.
fn feature(test: &MediaFeature, container: &QueryContainer) -> Option<bool> {
    let width = container.width.map(f64::from);
    let height = container.height.map(f64::from);
    let actual = match test.name.as_str() {
        "width" | "inline-size" => width?,
        "height" | "block-size" => height?,
        "aspect-ratio" => width? / height?,
        "orientation" => {
            let orientation = if height? >= width? {
                "portrait"
            } else {
                "landscape"
            };
            return Some(match &test.test {
                None => true,
                Some((Comparison::Equal, MediaValue::Ident(expected))) => expected == orientation,
                Some(_) => false,
            });
        }
        _ => return None,
    };
    Some(match &test.test {
        None => actual != 0.0,
        Some((comparison, MediaValue::Number(expected))) => comparison.test(actual, *expected),
        Some(_) => false,
    })
}

/// Parses an `@container` prelude, such as `sidebar (max-width: 240px)`.
/// `None` for a prelude that is not a container query, which drops the
/// rule.
pub fn parse_container_query(tokens: &[Token]) -> Option<ContainerQuery> {
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|token| **token != Token::Whitespace)
        .collect();
    let (name, rest) = match tokens.first() {
        Some(Token::Ident(word))
            if !["not", "and", "or", "none"]
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
        {
            (Some(word.clone()), &tokens[1..])
        }
        _ => (None, &tokens[..]),
    };
    let (condition, used) = condition(rest)?;
    (used == rest.len()).then_some(ContainerQuery { name, condition })
}

/// `not <in-parens>`, or `<in-parens>` joined by one kind of `and`/`or`.
fn condition(tokens: &[&Token]) -> Option<(ContainerCondition, usize)> {
    if let Some(Token::Ident(word)) = tokens.first()
        && word.eq_ignore_ascii_case("not")
    {
        let (inner, used) = in_parens(&tokens[1..])?;
        return Some((ContainerCondition::Not(Box::new(inner)), used + 1));
    }
    let (first, mut index) = in_parens(tokens)?;
    let mut parts = vec![first];
    let mut joiner: Option<String> = None;
    while let Some(Token::Ident(word)) = tokens.get(index) {
        let word = word.to_ascii_lowercase();
        if !matches!(word.as_str(), "and" | "or") || joiner.as_ref().is_some_and(|j| *j != word) {
            return None;
        }
        joiner = Some(word);
        let (part, used) = in_parens(&tokens[index + 1..])?;
        parts.push(part);
        index += used + 1;
    }
    let condition = match joiner.as_deref() {
        None => parts.pop()?,
        Some("and") => ContainerCondition::And(parts),
        _ => ContainerCondition::Or(parts),
    };
    Some((condition, index))
}

/// `( <condition> )`, `( <size-feature> )`, or a function such as
/// `style(...)`, which is unknown.
fn in_parens(tokens: &[&Token]) -> Option<(ContainerCondition, usize)> {
    let close = |from: usize| {
        let mut depth = 0usize;
        for (index, token) in tokens.iter().enumerate().skip(from) {
            match token {
                Token::LeftParen | Token::Function(_) => depth += 1,
                Token::RightParen if depth == 0 => return Some(index),
                Token::RightParen => depth -= 1,
                _ => {}
            }
        }
        None
    };
    match tokens.first()? {
        Token::LeftParen => {
            let end = close(1)?;
            let inner = &tokens[1..end];
            let nested = matches!(inner.first(), Some(Token::LeftParen))
                || matches!(inner.first(), Some(Token::Ident(word)) if word.eq_ignore_ascii_case("not"));
            let parsed = if nested {
                condition(inner)
                    .and_then(|(condition, used)| (used == inner.len()).then_some(condition))
            } else {
                parse_feature(inner).map(ContainerCondition::Feature)
            };
            Some((parsed.unwrap_or(ContainerCondition::Unknown), end + 1))
        }
        Token::Function(_) => Some((ContainerCondition::Unknown, close(1)? + 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
