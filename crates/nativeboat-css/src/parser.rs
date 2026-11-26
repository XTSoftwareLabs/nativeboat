//! Stylesheet and declaration list parsing.

use crate::prelude::*;

use crate::container::{ContainerQuery, parse_container_query};
use crate::media::{MediaQueryList, parse_media_query_list};
use crate::selector::{SelectorList, parse_selector_list};
use crate::tokenizer::{Spanned, Token, serialize_tokens, tokenize, tokens};
use serde::{Deserialize, Serialize};

/// A line and column in the source, both starting at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Position {
    /// Line number, starting at 1.
    pub line: u32,
    /// Column number in characters, starting at 1.
    pub column: u32,
}

/// A property declaration such as `color: red !important`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Declaration {
    /// Property name; lowercased unless it is a custom property.
    pub name: String,
    /// Normalized value text without `!important`.
    pub value: String,
    /// Whether the declaration is `!important`.
    pub important: bool,
    /// Location of the property name.
    pub position: Position,
}

/// A style rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleRule {
    /// The selectors.
    pub selectors: SelectorList,
    /// Declarations in source order.
    pub declarations: Vec<Declaration>,
    /// Location of the first selector.
    pub position: Position,
}

/// A keyframe of an `@keyframes` rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Offsets between 0 and 1.
    pub offsets: Vec<f64>,
    /// Declarations of the keyframe.
    pub declarations: Vec<Declaration>,
}

/// A rule of a stylesheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Rule {
    /// A style rule.
    Style(StyleRule),
    /// `@media`
    Media {
        /// The media query list.
        query: MediaQueryList,
        /// Rules applied when the query matches.
        rules: Vec<Rule>,
    },
    /// `@container`
    Container {
        /// The query.
        query: ContainerQuery,
        /// Rules applied to elements whose query container satisfies it.
        rules: Vec<Rule>,
    },
    /// `@supports`
    Supports {
        /// The condition text.
        condition: String,
        /// Rules applied when the condition holds.
        rules: Vec<Rule>,
    },
    /// `@layer name { ... }`
    Layer {
        /// The layer name, empty for an anonymous layer.
        name: String,
        /// Rules of the layer.
        rules: Vec<Rule>,
    },
    /// `@layer a, b;`: declares layers, in order, without rules.
    LayerOrder(Vec<String>),
    /// `@import`
    Import {
        /// The imported URL.
        url: String,
        /// The media query list.
        media: MediaQueryList,
        /// Location of the rule.
        position: Position,
    },
    /// `@font-face`
    FontFace(Vec<Declaration>),
    /// `@property`: a registered custom property.
    Property {
        /// The custom property's name, with its `--`.
        name: String,
        /// The `syntax` descriptor, such as `<length>` or `*`.
        syntax: String,
        /// Whether elements inherit the property from their parent.
        inherits: bool,
        /// The value elements have when nothing sets it.
        initial: Option<String>,
    },
    /// `@page`, with its page selector and declarations.
    Page {
        /// The page selector, such as `:first`, or empty.
        selector: String,
        /// The page's declarations, such as `margin` and `size`.
        declarations: Vec<Declaration>,
    },
    /// `@keyframes`
    Keyframes {
        /// Animation name.
        name: String,
        /// Keyframes in source order.
        keyframes: Vec<Keyframe>,
    },
    /// An at-rule the compiler does not interpret.
    Unknown {
        /// At-rule name without `@`.
        name: String,
        /// Prelude text.
        prelude: String,
        /// Location of the rule.
        position: Position,
    },
}

/// A parsed stylesheet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Stylesheet {
    /// Top-level rules in source order.
    pub rules: Vec<Rule>,
    /// The tree its rules apply in: 0 for the document, else the node id of
    /// the shadow root whose `<style>` it is.
    #[serde(default)]
    pub scope: u32,
}

/// A problem found while parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Description of the problem.
    pub message: String,
    /// Where it was found.
    pub position: Position,
}

/// Parses a stylesheet, collecting diagnostics for rules that were dropped.
pub fn parse_stylesheet(source: &str) -> (Stylesheet, Vec<Diagnostic>) {
    let mut parser = Parser::new(source);
    let rules = parser.rules(0, parser.tokens.len());
    (Stylesheet { rules, scope: 0 }, parser.diagnostics)
}

/// Parses a declaration list such as a `style` attribute.
pub fn parse_declaration_list(source: &str) -> Vec<Declaration> {
    let mut parser = Parser::new(source);
    let end = parser.tokens.len();
    parser.style_block(0, end, None).0
}

struct Parser {
    tokens: Vec<Spanned>,
    lines: Vec<usize>,
    source: String,
    diagnostics: Vec<Diagnostic>,
    /// The last position found, as a byte offset and its column: rules are
    /// parsed in source order, so the next column is counted on from it.
    /// Minified style sheets are one long line.
    cursor: core::cell::Cell<(usize, usize)>,
}

fn is_opener(token: &Token) -> bool {
    matches!(
        token,
        Token::LeftBrace | Token::LeftParen | Token::LeftBracket | Token::Function(_)
    )
}

fn is_closer(token: &Token) -> bool {
    matches!(
        token,
        Token::RightBrace | Token::RightParen | Token::RightBracket
    )
}

fn trim_whitespace(tokens: &[Token]) -> &[Token] {
    let start = tokens
        .iter()
        .position(|token| *token != Token::Whitespace)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|token| *token != Token::Whitespace)
        .map_or(start, |index| index + 1);
    &tokens[start..end]
}

impl Parser {
    fn new(source: &str) -> Self {
        let mut lines = vec![0];
        lines.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self {
            tokens: tokenize(source),
            lines,
            source: source.to_owned(),
            diagnostics: Vec::new(),
            cursor: core::cell::Cell::new((0, 0)),
        }
    }

    fn position(&self, index: usize) -> Position {
        let offset = self
            .tokens
            .get(index)
            .map_or(self.source.len(), |token| token.offset);
        let line = self.lines.partition_point(|start| *start <= offset) - 1;
        let start = self.lines[line];
        let (known, counted) = self.cursor.get();
        let column = if (start..=offset).contains(&known) {
            counted + self.source[known..offset].chars().count()
        } else {
            self.source[start..offset].chars().count()
        };
        self.cursor.set((offset, column));
        Position {
            line: line as u32 + 1,
            column: column as u32 + 1,
        }
    }

    fn token(&self, index: usize) -> &Token {
        &self.tokens[index].token
    }

    fn slice(&self, start: usize, end: usize) -> Vec<Token> {
        self.tokens[start..end]
            .iter()
            .map(|spanned| spanned.token.clone())
            .collect()
    }

    fn diagnose(&mut self, index: usize, message: String) {
        let position = self.position(index);
        self.diagnostics.push(Diagnostic { message, position });
    }

    /// Returns the index of the token closing the opener at `open`, or `end`.
    fn matching(&self, open: usize, end: usize) -> usize {
        let mut depth = 0usize;
        for index in open..end {
            let token = self.token(index);
            if is_opener(token) {
                depth += 1;
            } else if is_closer(token) {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
        }
        end
    }

    /// Finds the first top-level token from `start` satisfying `stop`.
    fn scan(&self, start: usize, end: usize, stop: fn(&Token) -> bool) -> usize {
        let mut index = start;
        while index < end {
            let token = self.token(index);
            if stop(token) {
                return index;
            }
            index = if is_opener(token) {
                self.matching(index, end) + 1
            } else {
                index + 1
            };
        }
        end
    }

    fn rules(&mut self, start: usize, end: usize) -> Vec<Rule> {
        let mut rules = Vec::new();
        let mut index = start;
        while index < end {
            match self.token(index) {
                Token::Whitespace | Token::Cdo | Token::Cdc | Token::Semicolon => index += 1,
                Token::AtKeyword(name) => {
                    let name = name.to_ascii_lowercase();
                    index = self.at_rule(index, name, end, None, &mut rules);
                }
                _ => index = self.qualified_rule(index, end, None, &mut rules),
            }
        }
        rules
    }

    /// Parses an at-rule starting at `start`; returns the index after it.
    fn at_rule(
        &mut self,
        start: usize,
        name: String,
        end: usize,
        parent: Option<&str>,
        rules: &mut Vec<Rule>,
    ) -> usize {
        let stop = self.scan(start + 1, end, |token| {
            matches!(token, Token::Semicolon | Token::LeftBrace)
        });
        let prelude = self.slice(start + 1, stop);
        let prelude = trim_whitespace(&prelude).to_vec();
        let position = self.position(start);
        let block = (stop < end && *self.token(stop) == Token::LeftBrace).then(|| {
            let close = self.matching(stop, end);
            (stop + 1, close)
        });
        let next = block.map_or(stop + 1, |(_, close)| close + 1);
        let nested = |parser: &mut Parser, (from, to): (usize, usize)| match parent {
            Some(selector) => {
                let (declarations, mut nested) = parser.style_block(from, to, Some(selector));
                if !declarations.is_empty() {
                    let selectors = SelectorList::parse(selector).unwrap_or_default();
                    nested.insert(
                        0,
                        Rule::Style(StyleRule {
                            selectors,
                            declarations,
                            position,
                        }),
                    );
                }
                nested
            }
            None => parser.rules(from, to),
        };
        match (name.as_str(), block) {
            ("media", Some(block)) => rules.push(Rule::Media {
                query: parse_media_query_list(&prelude),
                rules: nested(self, block),
            }),
            // A prelude that is not a container query drops the rule.
            ("container", Some(block)) => {
                if let Some(query) = parse_container_query(&prelude) {
                    rules.push(Rule::Container {
                        query,
                        rules: nested(self, block),
                    });
                }
            }
            ("supports", Some(block)) => rules.push(Rule::Supports {
                condition: serialize_tokens(&prelude),
                rules: nested(self, block),
            }),
            ("layer", Some(block)) => rules.push(Rule::Layer {
                name: serialize_tokens(&prelude),
                rules: nested(self, block),
            }),
            ("layer", None) => {
                let names = serialize_tokens(&prelude)
                    .split(',')
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .collect();
                rules.push(Rule::LayerOrder(names));
            }
            ("charset" | "namespace", None) => {}
            ("import", None) => match import_url(&prelude) {
                Some((url, rest)) => rules.push(Rule::Import {
                    url,
                    media: parse_media_query_list(rest),
                    position,
                }),
                None => self.diagnose(start, "invalid @import".to_owned()),
            },
            ("font-face", Some((from, to))) => {
                rules.push(Rule::FontFace(self.style_block(from, to, None).0));
            }
            ("property", Some((from, to))) => {
                let name = match prelude.as_slice() {
                    [Token::Ident(name)] if name.starts_with("--") => name.clone(),
                    _ => {
                        self.diagnose(start, "invalid @property name".to_owned());
                        return next;
                    }
                };
                let declarations = self.style_block(from, to, None).0;
                let descriptor = |wanted: &str| {
                    declarations
                        .iter()
                        .rev()
                        .find(|declaration| declaration.name.eq_ignore_ascii_case(wanted))
                        .map(|declaration| declaration.value.trim().to_owned())
                };
                let syntax = descriptor("syntax")
                    .map(|syntax| syntax.trim_matches(|c| c == '"' || c == '\'').to_owned());
                let inherits = descriptor("inherits").map(|value| value.to_ascii_lowercase());
                let initial = descriptor("initial-value");
                // The syntax and inheritance are required, and so is an
                // initial value unless any value is allowed.
                match (syntax, inherits.as_deref()) {
                    (Some(syntax), Some(inherits @ ("true" | "false")))
                        if syntax == "*" || initial.is_some() =>
                    {
                        rules.push(Rule::Property {
                            name,
                            syntax,
                            inherits: inherits == "true",
                            initial,
                        });
                    }
                    _ => self.diagnose(start, format!("invalid @property {name}")),
                }
            }
            ("page", Some((from, to))) => rules.push(Rule::Page {
                selector: serialize_tokens(&prelude),
                declarations: self.style_block(from, to, None).0,
            }),
            ("keyframes" | "-webkit-keyframes", Some((from, to))) => {
                let name = match prelude.first() {
                    Some(Token::Ident(name) | Token::String(name)) => name.clone(),
                    _ => String::new(),
                };
                let keyframes = self.keyframes(from, to);
                rules.push(Rule::Keyframes { name, keyframes });
            }
            _ => rules.push(Rule::Unknown {
                name,
                prelude: serialize_tokens(&prelude),
                position,
            }),
        }
        next
    }

    fn keyframes(&mut self, start: usize, end: usize) -> Vec<Keyframe> {
        let mut keyframes = Vec::new();
        let mut index = start;
        while index < end {
            if *self.token(index) == Token::Whitespace {
                index += 1;
                continue;
            }
            let open = self.scan(index, end, |token| *token == Token::LeftBrace);
            if open == end {
                break;
            }
            let close = self.matching(open, end);
            let mut offsets = Vec::new();
            for token in self.slice(index, open) {
                match token {
                    Token::Percentage(value) => offsets.push(value / 100.0),
                    Token::Ident(word) if word.eq_ignore_ascii_case("from") => offsets.push(0.0),
                    Token::Ident(word) if word.eq_ignore_ascii_case("to") => offsets.push(1.0),
                    _ => {}
                }
            }
            let declarations = self.style_block(open + 1, close, None).0;
            keyframes.push(Keyframe {
                offsets,
                declarations,
            });
            index = close + 1;
        }
        keyframes
    }

    /// Parses a qualified rule starting at `start`; returns the index after it.
    fn qualified_rule(
        &mut self,
        start: usize,
        end: usize,
        parent: Option<&str>,
        rules: &mut Vec<Rule>,
    ) -> usize {
        let open = self.scan(start, end, |token| *token == Token::LeftBrace);
        if open == end {
            self.diagnose(
                start,
                "unexpected end of stylesheet in rule prelude".to_owned(),
            );
            return end;
        }
        let close = self.matching(open, end);
        let prelude = self.slice(start, open);
        let selector_text = match parent {
            Some(parent) => resolve_nesting(parent, &prelude),
            None => serialize_tokens(trim_whitespace(&prelude)),
        };
        match parse_selector_list(&tokens(&selector_text)) {
            Ok(selectors) => {
                let position = self.position(start);
                let (declarations, nested) =
                    self.style_block(open + 1, close, Some(&selector_text));
                rules.push(Rule::Style(StyleRule {
                    selectors,
                    declarations,
                    position,
                }));
                rules.extend(nested);
            }
            Err(message) => {
                self.diagnose(
                    start,
                    format!("invalid selector `{selector_text}`: {message}"),
                );
            }
        }
        close + 1
    }

    /// Parses declarations and nested rules of a block.
    fn style_block(
        &mut self,
        start: usize,
        end: usize,
        selector: Option<&str>,
    ) -> (Vec<Declaration>, Vec<Rule>) {
        let mut declarations = Vec::new();
        let mut nested = Vec::new();
        let mut index = start;
        while index < end {
            match self.token(index) {
                Token::Whitespace | Token::Semicolon => {
                    index += 1;
                    continue;
                }
                Token::AtKeyword(name) => {
                    let name = name.to_ascii_lowercase();
                    index = self.at_rule(index, name, end, selector, &mut nested);
                    continue;
                }
                _ => {}
            }
            let stop = self.scan(index, end, |token| {
                matches!(token, Token::Semicolon | Token::LeftBrace)
            });
            if stop < end && *self.token(stop) == Token::LeftBrace {
                match selector {
                    Some(parent) => {
                        index = self.qualified_rule(index, end, Some(parent), &mut nested);
                    }
                    None => {
                        self.diagnose(index, "nested rules are not allowed here".to_owned());
                        index = self.matching(stop, end) + 1;
                    }
                }
                continue;
            }
            match self.declaration(index, stop) {
                Some(declaration) => declarations.push(declaration),
                None => self.diagnose(index, "invalid declaration".to_owned()),
            }
            index = stop + 1;
        }
        (declarations, nested)
    }

    fn declaration(&self, start: usize, end: usize) -> Option<Declaration> {
        let tokens = self.slice(start, end);
        let tokens = trim_whitespace(&tokens);
        let Some(Token::Ident(name)) = tokens.first() else {
            return None;
        };
        let rest = trim_whitespace(&tokens[1..]);
        let Some(Token::Colon) = rest.first() else {
            return None;
        };
        let mut value = trim_whitespace(&rest[1..]);
        let mut important = false;
        if let [head @ .., Token::Delim('!'), Token::Ident(word)] = value
            && word.eq_ignore_ascii_case("important")
        {
            important = true;
            value = trim_whitespace(head);
        }
        let name = if name.starts_with("--") {
            name.clone()
        } else {
            name.to_ascii_lowercase()
        };
        // `unicode-range` is read from its source text, as CSS Syntax says:
        // tokens lose `U+0000-00FF`'s digits.
        if name == "unicode-range" && !important {
            let colon = (start..end).find(|index| self.tokens[*index].token == Token::Colon)?;
            let from = self
                .tokens
                .get(colon + 1)
                .map_or(self.source.len(), |token| token.offset);
            let to = self
                .tokens
                .get(end)
                .map_or(self.source.len(), |token| token.offset);
            return Some(Declaration {
                name,
                value: self.source.get(from..to.max(from))?.trim().to_owned(),
                important,
                position: self.position(start),
            });
        }
        Some(Declaration {
            name,
            value: serialize_tokens(value),
            important,
            position: self.position(start),
        })
    }
}

/// Builds the selector text of a nested rule from its parent selector.
fn resolve_nesting(parent: &str, prelude: &[Token]) -> String {
    let parent = format!(":is({parent})");
    prelude
        .split(|token| *token == Token::Comma)
        .map(|part| {
            let text = serialize_tokens(trim_whitespace(part));
            if text.contains('&') {
                text.replace('&', &parent)
            } else {
                format!("{parent} {text}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Extracts the URL of an `@import` prelude and the remaining tokens.
fn import_url(prelude: &[Token]) -> Option<(String, &[Token])> {
    match prelude {
        [Token::String(url) | Token::Url(url), rest @ ..] => Some((url.clone(), rest)),
        [Token::Function(name), rest @ ..] if name.eq_ignore_ascii_case("url") => {
            let rest = trim_whitespace(rest);
            let [Token::String(url), rest @ ..] = rest else {
                return None;
            };
            let rest = trim_whitespace(rest);
            Some((url.clone(), rest.get(1..).unwrap_or_default()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
