//! JSON for runtimes without `serde_json`: a parser into [`Json`] values,
//! which deserialize into any `serde` type the way `serde_json` reads them,
//! enums externally tagged.

use crate::prelude::*;
use serde::de::value::{Error, MapDeserializer, SeqDeserializer};
use serde::de::{
    self, DeserializeOwned, Deserializer, EnumAccess, IntoDeserializer, VariantAccess, Visitor,
};

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Members in document order.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value as JSON text.
    pub fn text(&self) -> String {
        let mut text = String::new();
        self.write(&mut text);
        text
    }

    fn write(&self, text: &mut String) {
        use core::fmt::Write;
        match self {
            Json::Null => text.push_str("null"),
            Json::Bool(value) => text.push_str(if *value { "true" } else { "false" }),
            Json::Number(value) if value.is_finite() => {
                let _ = write!(text, "{value}");
            }
            Json::Number(_) => text.push_str("null"),
            Json::String(value) => {
                text.push('"');
                for character in value.chars() {
                    match character {
                        '"' => text.push_str("\\\""),
                        '\\' => text.push_str("\\\\"),
                        '\n' => text.push_str("\\n"),
                        '\r' => text.push_str("\\r"),
                        '\t' => text.push_str("\\t"),
                        control if (control as u32) < 0x20 => {
                            let _ = write!(text, "\\u{:04x}", control as u32);
                        }
                        other => text.push(other),
                    }
                }
                text.push('"');
            }
            Json::Array(items) => {
                text.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    item.write(text);
                }
                text.push(']');
            }
            Json::Object(members) => {
                text.push('{');
                for (index, (name, value)) in members.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    Json::String(name.clone()).write(text);
                    text.push(':');
                    value.write(text);
                }
                text.push('}');
            }
        }
    }
}

/// Reads a value of a `serde` type from JSON text; `None` when the text is
/// not JSON or does not describe such a value.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Option<T> {
    T::deserialize(parse(text)?).ok()
}

/// Parses JSON text.
pub fn parse(text: &str) -> Option<Json> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value(0)?;
    parser.space();
    (parser.at == parser.bytes.len()).then_some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        let found = self.bytes.get(self.at) == Some(&byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn word(&mut self, word: &str, value: Json) -> Option<Json> {
        let end = self.at + word.len();
        (self.bytes.get(self.at..end) == Some(word.as_bytes())).then(|| {
            self.at = end;
            value
        })
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 128 {
            return None;
        }
        self.space();
        match *self.bytes.get(self.at)? {
            b'n' => self.word("null", Json::Null),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'"' => self.string().map(Json::String),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value(depth + 1)?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Some(Json::Array(items))
            }
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let name = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        members.push((name, self.value(depth + 1)?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Some(Json::Object(members))
            }
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.at;
        while matches!(
            self.bytes.get(self.at),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        let text = core::str::from_utf8(&self.bytes[start..self.at]).ok()?;
        text.parse::<f64>().ok().map(Json::Number)
    }

    fn hex(&mut self) -> Option<u32> {
        let digits = core::str::from_utf8(self.bytes.get(self.at..self.at + 4)?).ok()?;
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if self.bytes.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut text = String::new();
        loop {
            let start = self.at;
            while !matches!(self.bytes.get(self.at), Some(b'"' | b'\\') | None) {
                self.at += 1;
            }
            text.push_str(core::str::from_utf8(&self.bytes[start..self.at]).ok()?);
            match self.bytes.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    return Some(text);
                }
                _ => {
                    self.at += 1;
                    let escaped = *self.bytes.get(self.at)?;
                    self.at += 1;
                    let character = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let unit = self.hex()?;
                            if (0xD800..0xDC00).contains(&unit)
                                && self.bytes.get(self.at..self.at + 2) == Some(b"\\u")
                            {
                                self.at += 2;
                                let low = self.hex()?;
                                let code = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                                char::from_u32(code)?
                            } else {
                                char::from_u32(unit).unwrap_or('\u{fffd}')
                            }
                        }
                        _ => return None,
                    };
                    text.push(character);
                }
            }
        }
    }
}

impl<'de> IntoDeserializer<'de, Error> for Json {
    type Deserializer = Json;

    fn into_deserializer(self) -> Json {
        self
    }
}

impl<'de> Deserializer<'de> for Json {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Json::Null => visitor.visit_unit(),
            Json::Bool(value) => visitor.visit_bool(value),
            // Whole numbers read as integers, which integer fields accept.
            Json::Number(value)
                if (-9.0e15..9.0e15).contains(&value) && value == (value as i64) as f64 =>
            {
                if value < 0.0 {
                    visitor.visit_i64(value as i64)
                } else {
                    visitor.visit_u64(value as u64)
                }
            }
            Json::Number(value) => visitor.visit_f64(value),
            Json::String(value) => visitor.visit_string(value),
            Json::Array(items) => visitor.visit_seq(SeqDeserializer::new(items.into_iter())),
            Json::Object(members) => visitor.visit_map(MapDeserializer::new(members.into_iter())),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Json::Null => visitor.visit_none(),
            value => visitor.visit_some(value),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self {
            Json::String(name) => visitor.visit_enum(Variant(name, Json::Null)),
            Json::Object(mut members) if members.len() == 1 => {
                let (name, value) = members.remove(0);
                visitor.visit_enum(Variant(name, value))
            }
            _ => Err(de::Error::custom("expected an enum")),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
        byte_buf unit unit_struct newtype_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

/// An externally tagged enum variant: its name and content.
struct Variant(String, Json);

impl<'de> EnumAccess<'de> for Variant {
    type Error = Error;
    type Variant = Json;

    fn variant_seed<S: de::DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, Json), Error> {
        let name = seed.deserialize(IntoDeserializer::<Error>::into_deserializer(self.0))?;
        Ok((name, self.1))
    }
}

impl<'de> VariantAccess<'de> for Json {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }

    fn newtype_variant_seed<S: de::DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, Error> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }
}

#[cfg(test)]
mod tests;
