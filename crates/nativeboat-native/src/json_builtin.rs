//! The `JSON` object (ECMAScript 2024 §25.5): `JSON.parse` over the strict
//! JSON grammar with reviver support, and `JSON.stringify` with `toJSON`,
//! replacer functions and property lists, indentation, wrapper unwrapping
//! and circular-structure errors. Recursive walks have a native depth limit.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::array_builtin::{index_key, is_callable, length_of, to_integer_or_infinity};
use crate::object::{Descriptor, Key, Kind, Object, Property, Slot, Wrapped};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// A JSON text being parsed.
struct Parser<'t> {
    text: &'t [u16],
    position: usize,
    depth: usize,
}

// Bound recursive native frames before input can exhaust the process stack:
// as deep as calls may nest, as V8 parses and serializes values nested
// thousands deep.
const MAX_DEPTH: usize = 4_000;

fn nesting_error(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Range, "Maximum JSON nesting depth exceeded")
}

impl Parser<'_> {
    fn error(&self, realm: &Realm) -> Thrown {
        let source = String::from_utf16_lossy(self.text);
        let message = match self.text.get(self.position) {
            None => "Unexpected end of JSON input".to_owned(),
            Some(&unit) => {
                let token = String::from_utf16_lossy(&[unit]);
                format!("Unexpected token '{token}', \"{source}\" is not valid JSON")
            }
        };
        realm.throw(ErrorKind::Syntax, &message)
    }

    fn skip_space(&mut self) {
        while matches!(
            self.text.get(self.position),
            Some(0x09 | 0x0A | 0x0D | 0x20)
        ) {
            self.position += 1;
        }
    }

    fn expect(&mut self, realm: &Realm, unit: u8) -> Result<(), Thrown> {
        if self.text.get(self.position) == Some(&u16::from(unit)) {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error(realm))
        }
    }

    fn literal(&mut self, realm: &Realm, word: &str, value: Value) -> Completion {
        for unit in word.encode_utf16() {
            if self.text.get(self.position) != Some(&unit) {
                return Err(self.error(realm));
            }
            self.position += 1;
        }
        Ok(value)
    }

    fn value(&mut self, realm: &mut Realm) -> Completion {
        self.skip_space();
        let container = matches!(self.text.get(self.position), Some(0x7B | 0x5B));
        if container && self.depth >= MAX_DEPTH {
            return Err(nesting_error(realm));
        }
        self.depth += usize::from(container);
        let result = self.value_inner(realm);
        self.depth -= usize::from(container);
        result
    }

    fn value_inner(&mut self, realm: &mut Realm) -> Completion {
        self.skip_space();
        match self.text.get(self.position).copied() {
            Some(0x7B) => self.object(realm),
            Some(0x5B) => self.array(realm),
            Some(0x22) => Ok(Value::String(self.string(realm)?)),
            Some(0x74) => self.literal(realm, "true", Value::Boolean(true)),
            Some(0x66) => self.literal(realm, "false", Value::Boolean(false)),
            Some(0x6E) => self.literal(realm, "null", Value::Null),
            Some(0x2D | 0x30..=0x39) => self.number(realm),
            _ => Err(self.error(realm)),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.position;
        while matches!(self.text.get(self.position), Some(0x30..=0x39)) {
            self.position += 1;
        }
        self.position - start
    }

    fn number(&mut self, realm: &Realm) -> Completion {
        let start = self.position;
        if self.text[self.position] == 0x2D {
            self.position += 1;
        }
        match self.text.get(self.position) {
            Some(0x30) => self.position += 1,
            Some(0x31..=0x39) => {
                self.digits();
            }
            _ => return Err(self.error(realm)),
        }
        if self.text.get(self.position) == Some(&0x2E) {
            self.position += 1;
            if self.digits() == 0 {
                return Err(self.error(realm));
            }
        }
        if matches!(self.text.get(self.position), Some(0x45 | 0x65)) {
            self.position += 1;
            if matches!(self.text.get(self.position), Some(0x2B | 0x2D)) {
                self.position += 1;
            }
            if self.digits() == 0 {
                return Err(self.error(realm));
            }
        }
        let literal = JsStr::from_units(&self.text[start..self.position]);
        Ok(Value::Number(crate::number::parse(&literal)))
    }

    fn hex(&mut self, realm: &Realm) -> Result<u16, Thrown> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = self
                .text
                .get(self.position)
                .and_then(|&unit| char::from_u32(u32::from(unit)))
                .and_then(|character| character.to_digit(16));
            let Some(digit) = digit else {
                return Err(self.error(realm));
            };
            value = value * 16 + digit as u16;
            self.position += 1;
        }
        Ok(value)
    }

    fn string(&mut self, realm: &Realm) -> Result<JsStr, Thrown> {
        self.position += 1;
        let mut units = Vec::new();
        loop {
            let Some(&unit) = self.text.get(self.position) else {
                return Err(self.error(realm));
            };
            match unit {
                0x22 => {
                    self.position += 1;
                    return Ok(JsStr::from_units(&units));
                }
                0x5C => {
                    self.position += 1;
                    let escape = self.text.get(self.position).copied();
                    let decoded = match escape {
                        Some(0x22) => 0x22,
                        Some(0x5C) => 0x5C,
                        Some(0x2F) => 0x2F,
                        Some(0x62) => 0x08,
                        Some(0x66) => 0x0C,
                        Some(0x6E) => 0x0A,
                        Some(0x72) => 0x0D,
                        Some(0x74) => 0x09,
                        Some(0x75) => {
                            self.position += 1;
                            units.push(self.hex(realm)?);
                            continue;
                        }
                        _ => return Err(self.error(realm)),
                    };
                    units.push(decoded);
                    self.position += 1;
                }
                0x00..=0x1F => return Err(self.error(realm)),
                _ => {
                    units.push(unit);
                    self.position += 1;
                }
            }
        }
    }

    fn array(&mut self, realm: &mut Realm) -> Completion {
        self.position += 1;
        let mut elements = Vec::new();
        self.skip_space();
        if self.text.get(self.position) == Some(&0x5D) {
            self.position += 1;
            return Ok(Value::Object(realm.array(elements)));
        }
        loop {
            elements.push(self.value(realm)?);
            self.skip_space();
            if self.text.get(self.position) == Some(&0x2C) {
                self.position += 1;
                continue;
            }
            self.expect(realm, b']')?;
            return Ok(Value::Object(realm.array(elements)));
        }
    }

    fn object(&mut self, realm: &mut Realm) -> Completion {
        self.position += 1;
        let object = realm.object();
        self.skip_space();
        if self.text.get(self.position) == Some(&0x7D) {
            self.position += 1;
            return Ok(Value::Object(object));
        }
        loop {
            self.skip_space();
            if self.text.get(self.position) != Some(&0x22) {
                return Err(self.error(realm));
            }
            let key = self.string(realm)?;
            self.skip_space();
            self.expect(realm, b':')?;
            let value = self.value(realm)?;
            // `__proto__` is an ordinary own key in JSON.
            ops::define_data(&object, Key::from_js(&key), value);
            self.skip_space();
            if self.text.get(self.position) == Some(&0x2C) {
                self.position += 1;
                continue;
            }
            self.expect(realm, b'}')?;
            return Ok(Value::Object(object));
        }
    }
}

/// The string-keyed enumerable own properties, in property order.
pub(crate) fn enumerable_keys(realm: &mut Realm, object: &Object) -> Result<Vec<Key>, Thrown> {
    let mut keys = Vec::new();
    for key in ops::own_property_keys(realm, object)? {
        if !matches!(key, Key::Symbol(_))
            && ops::get_own_property(realm, object, &key)?
                .is_some_and(|property| property.enumerable)
        {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// `InternalizeJSONProperty`.
fn internalize(
    realm: &mut Realm,
    holder: &Value,
    key: Key,
    reviver: &Value,
    depth: usize,
) -> Completion {
    let value = ops::get(realm, holder, &key)?;
    if let Value::Object(object) = &value {
        if depth >= MAX_DEPTH {
            return Err(nesting_error(realm));
        }
        let keys: Vec<Key> = if crate::proxy_builtin::is_array(realm, object)? {
            let length = length_of(realm, &value)?;
            (0..length).map(index_key).collect()
        } else {
            enumerable_keys(realm, object)?
        };
        for key in keys {
            let element = internalize(realm, &value, key.clone(), reviver, depth + 1)?;
            // Failed deletions and definitions are ignored.
            if matches!(element, Value::Undefined) {
                ops::delete_property(realm, object, &key)?;
            } else {
                ops::try_define_property(
                    realm,
                    object,
                    &key,
                    Descriptor::of(&Property::data(element)),
                )?;
            }
        }
    }
    ops::call(
        realm,
        reviver,
        holder.clone(),
        &[Value::String(key.to_js()), value],
    )
}

pub(crate) fn parse(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let text = ops::to_string(realm, &arg(args, 0))?;
    let mut parser = Parser {
        text: text.units(),
        position: 0,
        depth: 0,
    };
    let value = parser.value(realm)?;
    parser.skip_space();
    if parser.position < parser.text.len() {
        return Err(parser.error(realm));
    }
    let reviver = arg(args, 1);
    if !is_callable(&reviver) {
        return Ok(value);
    }
    let root = realm.object();
    ops::define_data(&root, Key::name(""), value);
    internalize(realm, &Value::Object(root), Key::name(""), &reviver, 0)
}

/// The state of one `JSON.stringify` call.
struct Serializer {
    replacer: Option<Value>,
    keys: Option<Vec<Key>>,
    gap: JsStr,
    indent: JsStr,
    stack: Vec<Object>,
}

/// `QuoteJSONString`.
pub(crate) fn quote(text: &[u16]) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for decoded in char::decode_utf16(text.iter().copied()) {
        match decoded {
            Ok('"') => quoted.push_str("\\\""),
            Ok('\\') => quoted.push_str("\\\\"),
            Ok('\u{08}') => quoted.push_str("\\b"),
            Ok('\u{0C}') => quoted.push_str("\\f"),
            Ok('\n') => quoted.push_str("\\n"),
            Ok('\r') => quoted.push_str("\\r"),
            Ok('\t') => quoted.push_str("\\t"),
            Ok(control) if u32::from(control) < 0x20 => {
                quoted.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            Ok(character) => quoted.push(character),
            Err(error) => quoted.push_str(&format!("\\u{:04x}", error.unpaired_surrogate())),
        }
    }
    quoted.push('"');
    quoted
}

impl Serializer {
    /// `SerializeJSONProperty`: `None` when the value is not serializable.
    fn property(
        &mut self,
        realm: &mut Realm,
        holder: &Value,
        key: &Key,
    ) -> Result<Option<JsStr>, Thrown> {
        let mut value = ops::get(realm, holder, key)?;
        if matches!(value, Value::Object(_) | Value::BigInt(_)) {
            let to_json = ops::get_named(realm, &value, "toJSON")?;
            if is_callable(&to_json) {
                value = ops::call(realm, &to_json, value, &[Value::String(key.to_js())])?;
            }
        }
        if let Some(replacer) = &self.replacer {
            value = ops::call(
                realm,
                replacer,
                holder.clone(),
                &[Value::String(key.to_js()), value],
            )?;
        }
        if let Value::Object(object) = &value {
            let wrapped = match &object.data().kind {
                Kind::Primitive(wrapped) => Some(wrapped.clone()),
                _ => None,
            };
            match wrapped {
                Some(Wrapped::Number(_)) => value = Value::Number(ops::to_number(realm, &value)?),
                Some(Wrapped::String(_)) => value = Value::String(ops::to_string(realm, &value)?),
                Some(Wrapped::Boolean(flag)) => value = Value::Boolean(flag),
                Some(Wrapped::BigInt(big)) => value = Value::BigInt(big),
                None => {}
            }
        }
        Ok(match value {
            Value::Null => Some("null".to_owned()),
            Value::Boolean(flag) => Some(flag.to_string()),
            Value::String(text) => Some(quote(text.units())),
            Value::Number(number) if number.is_finite() => Some(crate::number::to_string(number)),
            Value::Number(_) => Some("null".to_owned()),
            Value::BigInt(_) => {
                return Err(realm.throw(ErrorKind::Type, "Do not know how to serialize a BigInt"));
            }
            Value::Object(object) if object.code().is_none() => {
                return Ok(Some(if crate::proxy_builtin::is_array(realm, &object)? {
                    self.array(realm, &object)?
                } else {
                    self.object(realm, &object)?
                }));
            }
            _ => None,
        }
        .map(|text| JsStr::from(text.as_str())))
    }

    fn enter(&mut self, realm: &Realm, object: &Object) -> Result<JsStr, Thrown> {
        if self.stack.contains(object) {
            return Err(realm.throw(ErrorKind::Type, "Converting circular structure to JSON"));
        }
        if self.stack.len() >= MAX_DEPTH {
            return Err(nesting_error(realm));
        }
        self.stack.push(object.clone());
        let stepback = self.indent.clone();
        self.indent = self.indent.concat(&self.gap);
        Ok(stepback)
    }

    fn leave(&mut self, stepback: JsStr) {
        self.stack.pop();
        self.indent = stepback;
    }

    /// Joins serialized members in brackets, on indented lines with a gap.
    fn wrap(&self, open: char, members: Vec<JsStr>, close: char, stepback: &JsStr) -> JsStr {
        if members.is_empty() {
            return format!("{open}{close}").as_str().into();
        }
        let mut units = vec![open as u16];
        let pretty = !self.gap.is_empty();
        for (index, member) in members.iter().enumerate() {
            if index != 0 {
                units.push(b',' as u16);
            }
            if pretty {
                units.push(b'\n' as u16);
                units.extend_from_slice(self.indent.units());
            }
            units.extend_from_slice(member.units());
        }
        if pretty {
            units.push(b'\n' as u16);
            units.extend_from_slice(stepback.units());
        }
        units.push(close as u16);
        JsStr::from_units(&units)
    }

    fn object(&mut self, realm: &mut Realm, object: &Object) -> Result<JsStr, Thrown> {
        let stepback = self.enter(realm, object)?;
        let keys = match &self.keys {
            Some(keys) => keys.clone(),
            None => enumerable_keys(realm, object)?,
        };
        let holder = Value::Object(object.clone());
        let mut members = Vec::new();
        for key in keys {
            if let Some(serialized) = self.property(realm, &holder, &key)? {
                let colon = if self.gap.is_empty() { ":" } else { ": " };
                let prefix = format!("{}{colon}", quote(key.to_js().units()));
                members.push(JsStr::from(prefix.as_str()).concat(&serialized));
            }
        }
        let result = self.wrap('{', members, '}', &stepback);
        self.leave(stepback);
        Ok(result)
    }

    fn array(&mut self, realm: &mut Realm, array: &Object) -> Result<JsStr, Thrown> {
        let stepback = self.enter(realm, array)?;
        let holder = Value::Object(array.clone());
        let length = length_of(realm, &holder)?;
        let mut members = Vec::new();
        for index in 0..length {
            let serialized = self.property(realm, &holder, &index_key(index))?;
            members.push(serialized.unwrap_or_else(|| "null".into()));
        }
        let result = self.wrap('[', members, ']', &stepback);
        self.leave(stepback);
        Ok(result)
    }
}

/// The property list of an array replacer: strings, numbers and their
/// wrappers, without duplicates.
fn property_list(realm: &mut Realm, list: &Value) -> Result<Vec<Key>, Thrown> {
    let length = length_of(realm, list)?;
    let mut keys: Vec<Key> = Vec::new();
    for index in 0..length {
        let element = ops::get(realm, list, &index_key(index))?;
        let named = match &element {
            Value::String(_) | Value::Number(_) => true,
            Value::Object(object) => matches!(
                object.data().kind,
                Kind::Primitive(Wrapped::String(_) | Wrapped::Number(_))
            ),
            _ => false,
        };
        if named {
            let key = Key::from_js(&ops::to_string(realm, &element)?);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    Ok(keys)
}

/// The indentation of the `space` argument.
fn gap(realm: &mut Realm, space: &Value) -> Result<JsStr, Thrown> {
    let mut space = space.clone();
    if let Value::Object(object) = &space {
        let wrapped = match &object.data().kind {
            Kind::Primitive(Wrapped::Number(_)) => Some(true),
            Kind::Primitive(Wrapped::String(_)) => Some(false),
            _ => None,
        };
        match wrapped {
            Some(true) => space = Value::Number(ops::to_number(realm, &space)?),
            Some(false) => space = Value::String(ops::to_string(realm, &space)?),
            None => {}
        }
    }
    Ok(match space {
        Value::Number(_) => {
            let count = to_integer_or_infinity(realm, &space)?.clamp(0.0, 10.0);
            " ".repeat(count as usize).as_str().into()
        }
        Value::String(text) => JsStr::from_units(&text.units()[..text.len().min(10)]),
        _ => JsStr::default(),
    })
}

pub(crate) fn stringify(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let (value, replacer) = (arg(args, 0), arg(args, 1));
    let mut serializer = Serializer {
        replacer: None,
        keys: None,
        gap: JsStr::default(),
        indent: JsStr::default(),
        stack: Vec::new(),
    };
    if let Value::Object(object) = &replacer {
        if is_callable(&replacer) {
            serializer.replacer = Some(replacer.clone());
        } else if crate::proxy_builtin::is_array(realm, object)? {
            serializer.keys = Some(property_list(realm, &replacer)?);
        }
    }
    serializer.gap = gap(realm, &arg(args, 2))?;
    let wrapper = realm.object();
    ops::define_data(&wrapper, Key::name(""), value);
    let serialized = serializer.property(realm, &Value::Object(wrapper), &Key::name(""))?;
    Ok(serialized.map_or(Value::Undefined, Value::String))
}

/// Installs the `JSON` object.
pub(crate) fn install(realm: &mut Realm) {
    let json = realm.object();
    realm.method(&json, "parse", 2, parse);
    realm.method(&json, "stringify", 3, stringify);
    json.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property {
            slot: Slot::Data {
                value: Value::str("JSON"),
                writable: false,
            },
            enumerable: false,
            configurable: true,
        },
    );
    realm
        .global
        .define(Key::name("JSON"), Property::hidden(Value::Object(json)));
}

#[cfg(test)]
mod tests;
