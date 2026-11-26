//! Media queries.

use crate::prelude::*;

use crate::tokenizer::Token;
use serde::{Deserialize, Serialize};

/// The environment media queries are evaluated against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnvironment {
    /// Viewport width in CSS pixels.
    pub width: f64,
    /// Viewport height in CSS pixels.
    pub height: f64,
    /// Device pixels per CSS pixel.
    pub resolution: f64,
    /// Whether the system uses a dark color scheme.
    pub dark: bool,
    /// Whether the user asked for reduced motion.
    pub reduced_motion: bool,
    /// Whether a high contrast theme is active.
    pub high_contrast: bool,
    /// Whether the document is laid out for printing: the `print` media
    /// type instead of `screen`.
    pub print: bool,
}

impl Default for MediaEnvironment {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            resolution: 1.0,
            dark: false,
            reduced_motion: false,
            high_contrast: false,
            print: false,
        }
    }
}

/// A comparison in a media feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparison {
    /// `=` or `feature: value`
    Equal,
    /// `<`
    Less,
    /// `<=` or `max-`
    LessOrEqual,
    /// `>`
    Greater,
    /// `>=` or `min-`
    GreaterOrEqual,
}

impl Comparison {
    fn flip(self) -> Comparison {
        match self {
            Comparison::Less => Comparison::Greater,
            Comparison::LessOrEqual => Comparison::GreaterOrEqual,
            Comparison::Greater => Comparison::Less,
            Comparison::GreaterOrEqual => Comparison::LessOrEqual,
            Comparison::Equal => Comparison::Equal,
        }
    }

    pub(crate) fn test(self, actual: f64, expected: f64) -> bool {
        match self {
            Comparison::Equal => (actual - expected).abs() < 1e-9,
            Comparison::Less => actual < expected,
            Comparison::LessOrEqual => actual <= expected,
            Comparison::Greater => actual > expected,
            Comparison::GreaterOrEqual => actual >= expected,
        }
    }
}

/// A media feature value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaValue {
    /// A number, length in CSS pixels, ratio or resolution in dppx.
    Number(f64),
    /// A keyword.
    Ident(String),
}

/// A media feature test such as `(max-width: 600px)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFeature {
    /// Feature name without `min-`/`max-` prefix.
    pub name: String,
    /// Comparison and value, or `None` for boolean tests like `(hover)`.
    pub test: Option<(Comparison, MediaValue)>,
}

/// One media query of a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaQuery {
    /// Whether the query starts with `not`.
    pub negated: bool,
    /// The media type, lowercased; `None` means `all`.
    pub media_type: Option<String>,
    /// Feature tests that must all hold.
    pub features: Vec<MediaFeature>,
}

/// A comma-separated media query list. An empty list matches everything.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaQueryList(pub Vec<MediaQuery>);

impl MediaQueryList {
    /// Whether any query matches the environment.
    pub fn matches(&self, environment: &MediaEnvironment) -> bool {
        self.0.is_empty() || self.0.iter().any(|query| query.matches(environment))
    }
}

impl MediaQuery {
    /// Whether the query matches the environment.
    pub fn matches(&self, environment: &MediaEnvironment) -> bool {
        let type_matches = match self.media_type.as_deref() {
            None | Some("all") => true,
            Some("screen") => !environment.print,
            Some("print") => environment.print,
            Some(_) => false,
        };
        let result = type_matches
            && self
                .features
                .iter()
                .all(|feature| feature.matches(environment));
        result != self.negated
    }
}

impl MediaFeature {
    /// Whether the feature holds in the environment.
    pub fn matches(&self, environment: &MediaEnvironment) -> bool {
        let numeric = match self.name.as_str() {
            "width" => Some(environment.width),
            "height" => Some(environment.height),
            "aspect-ratio" => Some(environment.width / environment.height),
            "resolution" => Some(environment.resolution),
            "color" => Some(8.0),
            "monochrome" => Some(0.0),
            _ => None,
        };
        if let Some(actual) = numeric {
            return match &self.test {
                None => actual != 0.0,
                Some((comparison, MediaValue::Number(expected))) => {
                    comparison.test(actual, *expected)
                }
                Some(_) => false,
            };
        }
        let keyword = match self.name.as_str() {
            "orientation" if environment.height >= environment.width => "portrait",
            "orientation" => "landscape",
            "prefers-color-scheme" if environment.dark => "dark",
            "prefers-color-scheme" => "light",
            "prefers-reduced-motion" if environment.reduced_motion => "reduce",
            "prefers-reduced-motion" => "no-preference",
            "prefers-contrast" if environment.high_contrast => "more",
            "prefers-contrast" => "no-preference",
            "forced-colors" if environment.high_contrast => "active",
            "forced-colors" => "none",
            "hover" | "any-hover" => "hover",
            "pointer" | "any-pointer" => "fine",
            "display-mode" => "standalone",
            _ => return false,
        };
        match &self.test {
            None => !matches!(keyword, "none" | "no-preference"),
            Some((Comparison::Equal, MediaValue::Ident(expected))) => expected == keyword,
            Some(_) => false,
        }
    }
}

/// Parses a media query list from prelude tokens.
pub fn parse_media_query_list(tokens: &[Token]) -> MediaQueryList {
    let mut queries = Vec::new();
    for part in tokens.split(|token| *token == Token::Comma) {
        let part: Vec<&Token> = part
            .iter()
            .filter(|token| **token != Token::Whitespace)
            .collect();
        if part.is_empty() {
            continue;
        }
        queries.push(parse_query(&part).unwrap_or(MediaQuery {
            negated: true,
            media_type: None,
            features: Vec::new(),
        }));
    }
    MediaQueryList(queries)
}

fn parse_query(tokens: &[&Token]) -> Option<MediaQuery> {
    let mut query = MediaQuery {
        negated: false,
        media_type: None,
        features: Vec::new(),
    };
    let mut index = 0;
    let mut expect_and = false;
    while index < tokens.len() {
        match tokens[index] {
            Token::Ident(word) if !expect_and => {
                match word.to_ascii_lowercase().as_str() {
                    "not" if index == 0 => query.negated = true,
                    "only" if index == 0 => {}
                    media_type if query.media_type.is_none() => {
                        query.media_type = Some(media_type.to_owned());
                        expect_and = true;
                    }
                    _ => return None,
                }
                index += 1;
            }
            Token::Ident(word) if word.eq_ignore_ascii_case("and") => {
                expect_and = false;
                index += 1;
            }
            Token::LeftParen if !expect_and => {
                let end = tokens[index..]
                    .iter()
                    .position(|token| **token == Token::RightParen)?
                    + index;
                query
                    .features
                    .extend(parse_feature(&tokens[index + 1..end])?);
                index = end + 1;
                expect_and = true;
            }
            _ => return None,
        }
    }
    Some(query)
}

fn value_of(token: &Token) -> Option<MediaValue> {
    Some(match token {
        Token::Number { value, .. } => MediaValue::Number(*value),
        Token::Dimension { value, unit } => {
            MediaValue::Number(match unit.to_ascii_lowercase().as_str() {
                "px" | "dppx" | "x" => *value,
                "em" | "rem" => value * 16.0,
                "pt" => value * 96.0 / 72.0,
                "pc" => value * 16.0,
                "in" => value * 96.0,
                "cm" => value * 96.0 / 2.54,
                "mm" => value * 96.0 / 25.4,
                "dpi" => value / 96.0,
                "dpcm" => value * 2.54 / 96.0,
                _ => return None,
            })
        }
        Token::Ident(word) => MediaValue::Ident(word.to_ascii_lowercase()),
        _ => return None,
    })
}

/// Parses a comparison operator at the start of `tokens`.
fn comparison(tokens: &[&Token]) -> Option<(Comparison, usize)> {
    let equals = tokens.get(1) == Some(&&Token::Delim('='));
    Some(match tokens.first()? {
        Token::Delim('<') if equals => (Comparison::LessOrEqual, 2),
        Token::Delim('<') => (Comparison::Less, 1),
        Token::Delim('>') if equals => (Comparison::GreaterOrEqual, 2),
        Token::Delim('>') => (Comparison::Greater, 1),
        Token::Delim('=') => (Comparison::Equal, 1),
        _ => return None,
    })
}

/// Parses the inside of a media feature's parentheses.
pub(crate) fn parse_feature(tokens: &[&Token]) -> Option<Vec<MediaFeature>> {
    let feature = |name: &str, test| MediaFeature {
        name: name.to_ascii_lowercase(),
        test,
    };
    // Ratios such as 16/9 are folded into a single number.
    let tokens = fold_ratios(tokens);
    let tokens: Vec<&Token> = tokens.iter().collect();
    match tokens.as_slice() {
        [Token::Ident(name)] => Some(vec![feature(name, None)]),
        [Token::Ident(name), Token::Colon, value] => {
            let value = value_of(value)?;
            let lower = name.to_ascii_lowercase();
            Some(vec![
                match (lower.strip_prefix("min-"), lower.strip_prefix("max-")) {
                    (Some(base), _) => feature(base, Some((Comparison::GreaterOrEqual, value))),
                    (_, Some(base)) => feature(base, Some((Comparison::LessOrEqual, value))),
                    _ => feature(&lower, Some((Comparison::Equal, value))),
                },
            ])
        }
        [Token::Ident(name), rest @ ..] => {
            let (operator, used) = comparison(rest)?;
            let [value] = &rest[used..] else {
                return None;
            };
            Some(vec![feature(name, Some((operator, value_of(value)?)))])
        }
        [value, rest @ ..] => {
            let (operator, used) = comparison(rest)?;
            let Some(Token::Ident(name)) = rest.get(used) else {
                return None;
            };
            let mut features = vec![feature(name, Some((operator.flip(), value_of(value)?)))];
            let tail = &rest[used + 1..];
            if !tail.is_empty() {
                let (operator, used) = comparison(tail)?;
                let [value] = &tail[used..] else {
                    return None;
                };
                features.push(feature(name, Some((operator, value_of(value)?))));
            }
            Some(features)
        }
        [] => None,
    }
}

fn fold_ratios(tokens: &[&Token]) -> Vec<Token> {
    let mut folded: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if let (
            Token::Number { value: left, .. },
            Some(Token::Delim('/')),
            Some(Token::Number { value: right, .. }),
        ) = (tokens[index], tokens.get(index + 1), tokens.get(index + 2))
        {
            folded.push(Token::Number {
                value: left / right,
                integer: false,
            });
            index += 3;
            continue;
        }
        folded.push(tokens[index].clone());
        index += 1;
    }
    folded
}

#[cfg(test)]
mod tests;
