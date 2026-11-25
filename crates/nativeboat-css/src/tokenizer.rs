//! CSS tokenizer following CSS Syntax Level 3.

use crate::prelude::*;

use serde::{Deserialize, Serialize};

/// A CSS token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Token {
    /// An identifier such as `display`.
    Ident(String),
    /// A function name followed by `(`, such as `rgb(`.
    Function(String),
    /// An at-keyword such as `@media`.
    AtKeyword(String),
    /// A hash such as `#fff` or `#main`; `id` is true when it is a valid identifier.
    Hash {
        /// The text after `#`.
        value: String,
        /// Whether the value is a valid identifier.
        id: bool,
    },
    /// A quoted string.
    String(String),
    /// A string interrupted by a newline.
    BadString,
    /// An unquoted `url(...)`.
    Url(String),
    /// A malformed unquoted `url(...)`.
    BadUrl,
    /// A single code point that forms no other token.
    Delim(char),
    /// A number.
    Number {
        /// Numeric value.
        value: f64,
        /// Whether the number was written as an integer.
        integer: bool,
    },
    /// A percentage such as `50%`.
    Percentage(f64),
    /// A number with a unit such as `12px`.
    Dimension {
        /// Numeric value.
        value: f64,
        /// Unit, as written.
        unit: String,
    },
    /// One or more whitespace characters.
    Whitespace,
    /// `<!--`
    Cdo,
    /// `-->`
    Cdc,
    /// `:`
    Colon,
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `[`
    LeftBracket,
    /// `]`
    RightBracket,
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `{`
    LeftBrace,
    /// `}`
    RightBrace,
}

impl Token {
    /// Serializes the token back to CSS text.
    pub fn to_css(&self) -> String {
        match self {
            Token::Ident(name) => escape_identifier(name),
            Token::Function(name) => format!("{}(", escape_identifier(name)),
            Token::AtKeyword(name) => format!("@{}", escape_identifier(name)),
            Token::Hash { value, .. } => format!("#{value}"),
            Token::String(value) => {
                format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
            }
            Token::BadString | Token::BadUrl => String::new(),
            Token::Url(value) => format!("url({value})"),
            Token::Delim(character) => character.to_string(),
            Token::Number { value, .. } => format_number(*value),
            Token::Percentage(value) => format!("{}%", format_number(*value)),
            Token::Dimension { value, unit } => format!("{}{unit}", format_number(*value)),
            Token::Whitespace => " ".to_owned(),
            Token::Cdo => "<!--".to_owned(),
            Token::Cdc => "-->".to_owned(),
            Token::Colon => ":".to_owned(),
            Token::Semicolon => ";".to_owned(),
            Token::Comma => ",".to_owned(),
            Token::LeftBracket => "[".to_owned(),
            Token::RightBracket => "]".to_owned(),
            Token::LeftParen => "(".to_owned(),
            Token::RightParen => ")".to_owned(),
            Token::LeftBrace => "{".to_owned(),
            Token::RightBrace => "}".to_owned(),
        }
    }
}

/// Formats a number without a trailing `.0`.
pub fn format_number(value: f64) -> String {
    if crate::math::fract(value) == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn escape_identifier(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for (index, character) in name.chars().enumerate() {
        let leading_digit = index == 0 && character.is_ascii_digit();
        if leading_digit || !(is_name_char(character)) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Serializes a token list, collapsing whitespace.
pub fn serialize_tokens(tokens: &[Token]) -> String {
    let mut text = String::new();
    for token in tokens {
        text.push_str(&token.to_css());
    }
    text
}

/// A token with the byte offset where it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    /// The token.
    pub token: Token,
    /// Byte offset of its first character.
    pub offset: usize,
}

/// Tokenizes CSS source, dropping comments.
pub fn tokenize(source: &str) -> Vec<Spanned> {
    let mut tokenizer = Tokenizer {
        chars: source.char_indices().collect(),
        index: 0,
        length: source.len(),
    };
    let mut tokens = Vec::new();
    loop {
        tokenizer.skip_comments();
        let offset = tokenizer.offset();
        let Some(token) = tokenizer.next_token() else {
            return tokens;
        };
        tokens.push(Spanned { token, offset });
    }
}

/// Tokenizes CSS and returns the bare tokens.
pub fn tokens(source: &str) -> Vec<Token> {
    tokenize(source)
        .into_iter()
        .map(|spanned| spanned.token)
        .collect()
}

fn is_name_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_' || !character.is_ascii()
}

fn is_name_char(character: char) -> bool {
    is_name_start(character) || character.is_ascii_digit() || character == '-'
}

struct Tokenizer {
    chars: Vec<(usize, char)>,
    index: usize,
    length: usize,
}

impl Tokenizer {
    fn offset(&self) -> usize {
        self.chars
            .get(self.index)
            .map_or(self.length, |(offset, _)| *offset)
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars
            .get(self.index + ahead)
            .map(|(_, character)| *character)
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek(0)?;
        self.index += 1;
        Some(character)
    }

    fn skip_comments(&mut self) {
        while self.peek(0) == Some('/') && self.peek(1) == Some('*') {
            self.index += 2;
            while self.peek(0).is_some()
                && !(self.peek(0) == Some('*') && self.peek(1) == Some('/'))
            {
                self.index += 1;
            }
            self.index = (self.index + 2).min(self.chars.len());
        }
    }

    fn valid_escape(&self, ahead: usize) -> bool {
        self.peek(ahead) == Some('\\') && !matches!(self.peek(ahead + 1), None | Some('\n'))
    }

    fn starts_identifier(&self, ahead: usize) -> bool {
        match self.peek(ahead) {
            Some('-') => match self.peek(ahead + 1) {
                Some(next) if is_name_start(next) || next == '-' => true,
                _ => self.valid_escape(ahead + 1),
            },
            Some('\\') => self.valid_escape(ahead),
            Some(character) => is_name_start(character),
            None => false,
        }
    }

    fn starts_number(&self, ahead: usize) -> bool {
        let digit = |offset: usize| self.peek(offset).is_some_and(|c| c.is_ascii_digit());
        match self.peek(ahead) {
            Some('+' | '-') => {
                digit(ahead + 1) || (self.peek(ahead + 1) == Some('.') && digit(ahead + 2))
            }
            Some('.') => digit(ahead + 1),
            _ => digit(ahead),
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        let character = self.peek(0)?;
        if character.is_whitespace() {
            while self.peek(0).is_some_and(char::is_whitespace) {
                self.index += 1;
            }
            return Some(Token::Whitespace);
        }
        if self.starts_number(0) {
            return Some(self.number());
        }
        if character == '-' && self.peek(1) == Some('-') && self.peek(2) == Some('>') {
            self.index += 3;
            return Some(Token::Cdc);
        }
        if self.starts_identifier(0) {
            return Some(self.ident_like());
        }
        self.index += 1;
        Some(match character {
            '"' | '\'' => self.string(character),
            '#' if self.peek(0).is_some_and(is_name_char) || self.valid_escape(0) => {
                let id = self.starts_identifier(0);
                Token::Hash {
                    value: self.name(),
                    id,
                }
            }
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            ',' => Token::Comma,
            ':' => Token::Colon,
            ';' => Token::Semicolon,
            '<' if self.peek(0) == Some('!')
                && self.peek(1) == Some('-')
                && self.peek(2) == Some('-') =>
            {
                self.index += 3;
                Token::Cdo
            }
            '@' if self.starts_identifier(0) => Token::AtKeyword(self.name()),
            other => Token::Delim(other),
        })
    }

    fn escape(&mut self) -> char {
        let mut hex = String::new();
        while hex.len() < 6 && self.peek(0).is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(self.bump().unwrap_or_default());
        }
        if hex.is_empty() {
            return self.bump().unwrap_or('\u{fffd}');
        }
        if self.peek(0).is_some_and(char::is_whitespace) {
            self.index += 1;
        }
        let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
        match char::from_u32(code) {
            Some(character) if code != 0 => character,
            _ => '\u{fffd}',
        }
    }

    fn name(&mut self) -> String {
        let mut name = String::new();
        loop {
            match self.peek(0) {
                Some(character) if is_name_char(character) => {
                    name.push(character);
                    self.index += 1;
                }
                Some('\\') if self.valid_escape(0) => {
                    self.index += 1;
                    let character = self.escape();
                    name.push(character);
                }
                _ => return name,
            }
        }
    }

    fn ident_like(&mut self) -> Token {
        let name = self.name();
        if self.peek(0) != Some('(') {
            return Token::Ident(name);
        }
        self.index += 1;
        if !name.eq_ignore_ascii_case("url") {
            return Token::Function(name);
        }
        let mut ahead = 0;
        while self.peek(ahead).is_some_and(char::is_whitespace) {
            ahead += 1;
        }
        if matches!(self.peek(ahead), Some('"' | '\'')) {
            return Token::Function(name);
        }
        self.index += ahead;
        self.url()
    }

    fn url(&mut self) -> Token {
        let mut value = String::new();
        loop {
            match self.bump() {
                None | Some(')') => return Token::Url(value),
                Some(character) if character.is_whitespace() => {
                    while self.peek(0).is_some_and(char::is_whitespace) {
                        self.index += 1;
                    }
                    if matches!(self.peek(0), None | Some(')')) {
                        self.index = (self.index + 1).min(self.chars.len());
                        return Token::Url(value);
                    }
                    return self.bad_url();
                }
                Some('"' | '\'' | '(') => return self.bad_url(),
                Some('\\') => {
                    if matches!(self.peek(0), None | Some('\n')) {
                        return self.bad_url();
                    }
                    let character = self.escape();
                    value.push(character);
                }
                Some(character) => value.push(character),
            }
        }
    }

    fn bad_url(&mut self) -> Token {
        while let Some(character) = self.bump() {
            if character == ')' {
                break;
            }
        }
        Token::BadUrl
    }

    fn string(&mut self, quote: char) -> Token {
        let mut value = String::new();
        loop {
            match self.bump() {
                None => return Token::String(value),
                Some(character) if character == quote => return Token::String(value),
                Some('\n') => {
                    self.index -= 1;
                    return Token::BadString;
                }
                Some('\\') => match self.peek(0) {
                    None => {}
                    Some('\n') => self.index += 1,
                    Some(_) => {
                        let character = self.escape();
                        value.push(character);
                    }
                },
                Some(character) => value.push(character),
            }
        }
    }

    fn number(&mut self) -> Token {
        let mut text = String::new();
        let mut integer = true;
        if let Some(sign @ ('+' | '-')) = self.peek(0) {
            text.push(sign);
            self.index += 1;
        }
        self.digits(&mut text);
        if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            integer = false;
            text.push('.');
            self.index += 1;
            self.digits(&mut text);
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let signed = matches!(self.peek(1), Some('+' | '-'));
            let digit_at = if signed { 2 } else { 1 };
            if self.peek(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                integer = false;
                for _ in 0..digit_at {
                    text.push(self.bump().unwrap_or_default());
                }
                self.digits(&mut text);
            }
        }
        let value: f64 = text.parse().unwrap_or(0.0);
        if self.starts_identifier(0) {
            return Token::Dimension {
                value,
                unit: self.name(),
            };
        }
        if self.peek(0) == Some('%') {
            self.index += 1;
            return Token::Percentage(value);
        }
        Token::Number { value, integer }
    }

    fn digits(&mut self, text: &mut String) {
        while let Some(digit) = self.peek(0).filter(char::is_ascii_digit) {
            text.push(digit);
            self.index += 1;
        }
    }
}

#[cfg(test)]
mod tests;
