// Inspection layout adapts Node.js algorithms. Copyright Node.js contributors.
// MIT notice: ../../nativeboat-start/src/NOTICES.txt.
//! Node's `util.inspect` and `util.format`, which `console` prints with.
//! The layout follows Node's defaults (`compact: 3`, `breakLength: 80`,
//! `maxArrayLength: 100`, `maxStringLength: 10000`): short entries share a
//! line, longer ones break with two-space indentation, more than six array
//! entries are grouped into aligned columns, circular references print as
//! `<ref *1>` and `[Circular *1]`, and objects nested deeper than the depth
//! limit print as `[Object]`. Hidden properties (`showHidden`, which `%o`
//! turns on in Node) and custom inspection functions are not supported.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property, Slot, Wrapped};
use crate::ops;
use crate::realm::Realm;
use crate::string::JsStr;
use crate::value::{Thrown, Value};

/// The default nesting depth shown.
pub(crate) const DEPTH: usize = 2;
const BREAK_LENGTH: usize = 80;
const COMPACT: usize = 3;
const MAX_ARRAY_LENGTH: usize = 100;
const MAX_STRING_LENGTH: usize = 10_000;
/// Strings no longer than this never break across lines.
const MIN_LINE_WIDTH: usize = 16;

/// The length of text in UTF-16 units, as Node measures it.
fn width(text: &str) -> usize {
    text.encode_utf16().count()
}

fn plural(count: u64) -> &'static str {
    if count > 1 { "s" } else { "" }
}

/// A string literal as Node escapes it: single quotes, or double quotes or
/// backticks when that avoids escaping.
pub(crate) fn quote(text: &[u16]) -> String {
    let has = |unit: u8| text.contains(&u16::from(unit));
    let quote = if !has(b'\'') {
        '\''
    } else if !has(b'"') {
        '"'
    } else if !has(b'`')
        && !text
            .windows(2)
            .any(|pair| pair == [u16::from(b'$'), u16::from(b'{')])
    {
        '`'
    } else {
        '\''
    };
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push(quote);
    for character in char::decode_utf16(text.iter().copied()) {
        match character {
            Ok('\u{8}') => quoted.push_str("\\b"),
            Ok('\t') => quoted.push_str("\\t"),
            Ok('\n') => quoted.push_str("\\n"),
            Ok('\u{c}') => quoted.push_str("\\f"),
            Ok('\r') => quoted.push_str("\\r"),
            Ok('\\') => quoted.push_str("\\\\"),
            Ok(character) if character == quote => {
                quoted.push('\\');
                quoted.push(character);
            }
            Ok(character) if (character as u32) < 0x20 || character as u32 == 0x7f => {
                quoted.push_str(&format!("\\x{:02X}", character as u32));
            }
            Ok(character) => quoted.push(character),
            Err(lone) => quoted.push_str(&format!("\\u{:x}", lone.unpaired_surrogate())),
        }
    }
    quoted.push(quote);
    quoted
}

/// Removes terminal CSI, OSC, DCS, PM, APC and single-character escape
/// sequences while preserving all other UTF-16 code units.
pub(crate) fn strip_vt_control_characters(text: &JsStr) -> JsStr {
    let units = text.units();
    let mut output = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let start = index;
        let unit = units[index];
        let introducer = if unit == 0x1b {
            if let Some(next) = units.get(index + 1).copied() {
                index += 2;
                Some(next)
            } else {
                None
            }
        } else if matches!(unit, 0x9b | 0x9d | 0x90 | 0x9e | 0x9f) {
            index += 1;
            Some(unit)
        } else {
            output.push(unit);
            index += 1;
            continue;
        };
        let Some(kind) = introducer else {
            output.push(unit);
            index += 1;
            continue;
        };
        if kind == b'[' as u16 || kind == 0x9b {
            let mut cursor = index;
            while cursor < units.len() && !(0x40..=0x7e).contains(&units[cursor]) {
                cursor += 1;
            }
            if cursor < units.len() {
                index = cursor + 1;
            } else {
                output.extend_from_slice(&units[start..]);
                break;
            }
        } else if matches!(kind, 0x5d | 0x50 | 0x5e | 0x5f | 0x9d | 0x90 | 0x9e | 0x9f) {
            let mut cursor = index;
            while cursor < units.len() {
                if units[cursor] == 0x07 {
                    cursor += 1;
                    break;
                }
                if units[cursor] == 0x9c {
                    cursor += 1;
                    break;
                }
                if units[cursor] == 0x1b && units.get(cursor + 1) == Some(&(b'\\' as u16)) {
                    cursor += 2;
                    break;
                }
                cursor += 1;
            }
            if cursor == units.len()
                && !matches!(units.last(), Some(0x07 | 0x9c))
                && !(units.len() >= 2 && units[units.len() - 2..] == [0x1b, b'\\' as u16])
            {
                output.extend_from_slice(&units[start..]);
                break;
            }
            index = cursor;
        } else if (0x40..=0x7e).contains(&kind) {
            // A two-unit ESC sequence, such as ESC c (reset terminal).
        } else {
            output.extend_from_slice(&units[start..index]);
        }
    }
    JsStr::from_units(&output)
}

/// A number as Node prints it, keeping the sign of zero.
pub(crate) fn number(value: f64) -> String {
    if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else {
        crate::number::to_string(value)
    }
}

/// A property name: identifiers bare, other strings quoted, symbols in
/// brackets.
fn name(key: &Key) -> String {
    if let Key::Symbol(symbol) = key {
        return format!("[{}]", symbol.descriptive().to_rust_lossy());
    }
    let text = key.to_js();
    let units = text.units();
    let letter = |unit: &u16| {
        *unit == u16::from(b'_') || u8::try_from(*unit).is_ok_and(|unit| unit.is_ascii_alphabetic())
    };
    let word =
        |unit: &u16| letter(unit) || u8::try_from(*unit).is_ok_and(|unit| unit.is_ascii_digit());
    if units.first().is_some_and(letter) && units.iter().all(word) {
        text.to_rust_lossy()
    } else {
        quote(units)
    }
}

/// `getPrefix`: the constructor name, size and tag before an object's
/// opening brace.
fn prefix(constructor: Option<&str>, tag: &str, fallback: &str, size: &str) -> String {
    match constructor {
        None if !tag.is_empty() && tag != fallback => {
            format!("[{fallback}{size}: null prototype] [{tag}] ")
        }
        None => format!("[{fallback}{size}: null prototype] "),
        Some(constructor) if !tag.is_empty() && tag != constructor => {
            format!("{constructor}{size} [{tag}] ")
        }
        Some(constructor) => format!("{constructor}{size} "),
    }
}

/// The value of an own data property, without running accessors.
fn own_data(object: &Object, key: &str) -> Option<Value> {
    match object.own_property(&Key::name(key))?.slot {
        Slot::Data { value, .. } => Some(value),
        Slot::Accessor { .. } => None,
    }
}

/// Whether `object` inherits from `prototype`.
fn inherits(object: &Object, prototype: &Object) -> bool {
    let mut current = object.data().prototype.clone();
    while let Some(object) = current {
        if &object == prototype {
            return true;
        }
        current = object.data().prototype.clone();
    }
    false
}

/// Whether a global binding of the function's own name holds the function,
/// as for the built-in constructors.
fn global_builtin(realm: &Realm, function: &Object) -> bool {
    match own_data(function, "name") {
        Some(Value::String(name)) => {
            matches!(own_data(&realm.global, &name.to_rust_lossy()), Some(Value::Object(global)) if &global == function)
        }
        _ => false,
    }
}

/// `getConstructorName`: the name of the nearest `constructor` that the
/// object is an instance of; `None` for objects without one.
fn constructor_name(realm: &mut Realm, object: &Object) -> Result<Option<String>, Thrown> {
    let mut current = Some(object.clone());
    while let Some(holder) = current {
        if let Some(Value::Object(constructor)) = own_data(&holder, "constructor") {
            let instance = matches!(
                own_data(&constructor, "prototype"),
                Some(Value::Object(prototype)) if inherits(object, &prototype)
            );
            if instance && constructor.code().is_some() {
                let name = ops::get_named(realm, &Value::Object(constructor), "name")?;
                if let Value::String(name) = name
                    && !name.is_empty()
                {
                    return Ok(Some(name.to_rust_lossy()));
                }
            }
        }
        current = holder.data().prototype.clone();
    }
    Ok(None)
}

/// The `Symbol.toStringTag` of an object, which Node shows unless it is an
/// own enumerable property (and so printed as an entry).
fn tag(realm: &mut Realm, object: &Object) -> Result<String, Thrown> {
    let key = Key::Symbol(realm.symbols.to_string_tag.clone());
    let tag = ops::get(realm, &Value::Object(object.clone()), &key)?;
    let own = object
        .own_property(&key)
        .is_some_and(|property| property.enumerable);
    Ok(match tag {
        Value::String(tag) if !own => tag.to_rust_lossy(),
        _ => String::new(),
    })
}

/// Whether a constructor is a class: a native constructor with a fixed
/// `prototype` that is not one of the global built-ins.
fn is_class(realm: &Realm, function: &Object) -> bool {
    let fixed = matches!(
        function
            .own_property(&Key::name("prototype"))
            .map(|property| property.slot),
        Some(Slot::Data {
            writable: false,
            ..
        })
    );
    matches!(function.data().kind, Kind::BuiltinConstructor { .. })
        && fixed
        && !global_builtin(realm, function)
}

/// What an object is formatted as.
enum Shape {
    Array,
    Arguments,
    Error,
    /// A wrapper object: its type and its primitive's text.
    Boxed(&'static str, String),
    Collection(crate::collection_builtin::Collection),
    /// A WeakMap or WeakSet, whose entries are not listed.
    Weak(&'static str),
    /// A promise and its current state.
    Promise(crate::promise_builtin::State),
    Function,
    /// A regular expression and its literal text.
    RegExp(String),
    /// A date and its ISO text, or `Invalid Date`.
    Date(String),
    Plain,
}

fn shape(object: &Object) -> Shape {
    match &object.data().kind {
        Kind::Array(_) | Kind::TypedArray(_) => Shape::Array,
        Kind::Arguments => Shape::Arguments,
        Kind::Error => Shape::Error,
        Kind::Primitive(Wrapped::Number(value)) => Shape::Boxed("Number", number(*value)),
        Kind::Primitive(Wrapped::Boolean(value)) => Shape::Boxed("Boolean", value.to_string()),
        Kind::Primitive(Wrapped::String(text)) => Shape::Boxed("String", quote(text.units())),
        Kind::Primitive(Wrapped::BigInt(value)) => {
            Shape::Boxed("BigInt", format!("{}n", value.to_radix(10)))
        }
        Kind::SymbolObject(symbol) => Shape::Boxed("Symbol", symbol.descriptive().to_rust_lossy()),
        Kind::Collection(collection) => Shape::Collection(collection.clone()),
        Kind::WeakCollection(collection) => {
            Shape::Weak(if collection.set { "WeakSet" } else { "WeakMap" })
        }
        Kind::Promise(state) => Shape::Promise(state.borrow().state.clone()),
        Kind::RegExp(data) => Shape::RegExp(crate::regexp_builtin::literal_text(data)),
        Kind::Date(time) => Shape::Date(crate::date_builtin::inspect_text(*time)),
        _ if object.code().is_some() => Shape::Function,
        _ => Shape::Plain,
    }
}

/// One `util.inspect` call: the objects being formatted, the circular
/// references found, and the indentation and depth of the current entry.
struct Inspector {
    depth: usize,
    seen: Vec<Object>,
    circular: Vec<Object>,
    indentation: usize,
    current_depth: usize,
}

impl Inspector {
    /// `formatValue`.
    fn value(
        &mut self,
        realm: &mut Realm,
        value: &Value,
        recurse: usize,
    ) -> Result<String, Thrown> {
        Ok(match value {
            Value::Undefined => "undefined".to_owned(),
            Value::Null => "null".to_owned(),
            Value::Boolean(flag) => flag.to_string(),
            Value::Number(value) => number(*value),
            Value::String(text) => self.string(text),
            Value::Symbol(symbol) => symbol.descriptive().to_rust_lossy(),
            Value::BigInt(value) => format!("{}n", value.to_radix(10)),
            Value::Object(object) if self.seen.contains(object) => {
                let index = match self.circular.iter().position(|seen| seen == object) {
                    Some(index) => index + 1,
                    None => {
                        self.circular.push(object.clone());
                        self.circular.len()
                    }
                };
                format!("[Circular *{index}]")
            }
            Value::Object(object) => self.raw(realm, object, recurse)?,
        })
    }

    /// A quoted string, cut at the length limit, and broken after line
    /// feeds when it is too long for one line.
    fn string(&self, text: &JsStr) -> String {
        let mut units = text.units();
        let mut trailer = String::new();
        if units.len() > MAX_STRING_LENGTH {
            let remaining = (units.len() - MAX_STRING_LENGTH) as u64;
            units = &units[..MAX_STRING_LENGTH];
            trailer = format!("... {remaining} more character{}", plural(remaining));
        }
        let limit = BREAK_LENGTH.saturating_sub(self.indentation + 4);
        if units.len() > MIN_LINE_WIDTH && units.len() > limit {
            let lines: Vec<String> = units
                .split_inclusive(|&unit| unit == u16::from(b'\n'))
                .map(quote)
                .collect();
            let separator = format!(" +\n{}", " ".repeat(self.indentation + 2));
            return lines.join(&separator) + &trailer;
        }
        quote(units) + &trailer
    }

    /// `formatRaw`.
    fn raw(
        &mut self,
        realm: &mut Realm,
        object: &Object,
        recurse: usize,
    ) -> Result<String, Thrown> {
        let value = Value::Object(object.clone());
        if recurse >= 64 {
            return Err(realm.throw(
                crate::realm::ErrorKind::Range,
                "Maximum inspection nesting depth exceeded",
            ));
        }
        let constructor = constructor_name(realm, object)?;
        let constructor = constructor.as_deref();
        let tag = tag(realm, object)?;
        let mut keys: Vec<Key> = object
            .own_keys()
            .into_iter()
            .filter(|key| {
                object
                    .own_property(key)
                    .is_some_and(|property| property.enumerable)
            })
            .collect();
        let mut base = String::new();
        let mut open = "{".to_owned();
        let mut close = "}";
        let shape = shape(object);
        let array = matches!(shape, Shape::Array);
        let collection = match &shape {
            Shape::Collection(collection) => Some(collection.clone()),
            _ => None,
        };
        let promise = match &shape {
            Shape::Promise(state) => Some(state.clone()),
            _ => None,
        };
        let mut weak = false;
        match shape {
            Shape::Array => {
                keys.retain(|key| !matches!(key, Key::Index(_)));
                let length = crate::array_builtin::length_of(realm, &value)?;
                if constructor != Some("Array") || !tag.is_empty() {
                    open = prefix(constructor, &tag, "Array", &format!("({length})"));
                } else {
                    open = String::new();
                }
                open.push('[');
                close = "]";
                if length == 0 && keys.is_empty() {
                    return Ok(format!("{open}]"));
                }
            }
            Shape::Collection(collection) => {
                let size = collection.size();
                let kind = if collection.set { "Set" } else { "Map" };
                open = format!(
                    "{}{{",
                    prefix(constructor, &tag, kind, &format!("({size})"))
                );
                if size == 0 && keys.is_empty() {
                    return Ok(format!("{open}}}"));
                }
            }
            Shape::Promise(_) => {
                open = format!("{}{{", prefix(constructor, &tag, "Promise", ""));
            }
            Shape::Weak(kind) => {
                open = format!("{}{{", prefix(constructor, &tag, kind, ""));
                weak = true;
            }
            Shape::Function => {
                base = self.function_base(realm, object, constructor, &tag)?;
                if keys.is_empty() {
                    return Ok(base);
                }
            }
            Shape::Error => {
                let text = crate::builtins::error_to_string(realm, value.clone(), &[])?;
                base = format!("[{}]", ops::to_string(realm, &text)?.to_rust_lossy());
                // Node lists a cause and aggregated errors even when hidden.
                let cause = Key::name("cause");
                if !keys.contains(&cause) && ops::has_property(realm, &cause, &value)? {
                    keys.push(cause);
                }
                let errors = Key::name("errors");
                let aggregated = ops::get(realm, &value, &errors)?;
                if !keys.contains(&errors) && aggregated.as_object().is_some_and(Object::is_array) {
                    keys.push(errors);
                }
                if keys.is_empty() {
                    return Ok(base);
                }
            }
            Shape::Date(text) => {
                base = match prefix(constructor, &tag, "Date", "") {
                    start if start == "Date " => text,
                    start => start + &text,
                };
                if keys.is_empty() {
                    return Ok(base);
                }
            }
            Shape::RegExp(text) => {
                base = match prefix(constructor, &tag, "RegExp", "") {
                    start if start == "RegExp " => text,
                    start => start + &text,
                };
                if keys.is_empty() || recurse > self.depth {
                    return Ok(base);
                }
            }
            Shape::Boxed(kind, primitive) => {
                if kind == "String" {
                    // A string wrapper's characters are not listed.
                    keys.retain(|key| !matches!(key, Key::Index(_)));
                }
                base = format!("[{kind}");
                match constructor {
                    None => base.push_str(" (null prototype)"),
                    Some(constructor) if constructor != kind => {
                        base.push_str(&format!(" ({constructor})"));
                    }
                    Some(_) => {}
                }
                base.push_str(&format!(": {primitive}]"));
                if !tag.is_empty() && Some(tag.as_str()) != constructor {
                    base.push_str(&format!(" [{tag}]"));
                }
                if keys.is_empty() {
                    return Ok(base);
                }
            }
            Shape::Arguments | Shape::Plain if constructor == Some("Object") => {
                if matches!(shape, Shape::Arguments) {
                    open = "[Arguments] {".to_owned();
                } else if !tag.is_empty() {
                    open = format!("{}{{", prefix(constructor, &tag, "Object", ""));
                }
                if keys.is_empty() {
                    return Ok(format!("{open}}}"));
                }
            }
            Shape::Arguments | Shape::Plain => {
                open = format!("{}{{", prefix(constructor, &tag, "Object", ""));
                if keys.is_empty() {
                    return Ok(format!("{open}}}"));
                }
            }
        }
        if recurse > self.depth {
            let name = prefix(constructor, &tag, "Object", "");
            let name = name.trim_end();
            return Ok(match constructor {
                Some(_) => format!("[{name}]"),
                None => name.to_owned(),
            });
        }
        let recurse = recurse + 1;
        self.seen.push(object.clone());
        self.current_depth = recurse;
        let mut output = Vec::new();
        if array {
            self.elements(realm, object, recurse, &mut output)?;
        }
        // Node's `formatPromise`: the state, then the settled result.
        if let Some(state) = promise {
            let (marker, result) = match state {
                crate::promise_builtin::State::Pending => ("<pending>", None),
                crate::promise_builtin::State::Fulfilled(value) => ("", Some(value)),
                crate::promise_builtin::State::Rejected(reason) => ("<rejected> ", Some(reason)),
            };
            let mut entry = marker.trim_end().to_owned();
            if let Some(result) = result {
                self.indentation += 2;
                let text = self.value(realm, &result, recurse);
                self.indentation -= 2;
                entry = format!("{marker}{}", text?);
            }
            output.push(entry);
        }
        // Node lists a weak collection's entries only with `showHidden`.
        if weak {
            output.push("<items unknown>".to_owned());
        }
        if let Some(collection) = collection {
            let size = collection.size();
            let entries = collection.snapshot(MAX_ARRAY_LENGTH);
            for (key, value) in entries {
                self.indentation += 2;
                let entry = if collection.set {
                    self.value(realm, &key, recurse)
                } else {
                    let key = self.value(realm, &key, recurse)?;
                    self.value(realm, &value, recurse)
                        .map(|value| format!("{key} => {value}"))
                };
                self.indentation -= 2;
                output.push(entry?);
            }
            let remaining = size.saturating_sub(MAX_ARRAY_LENGTH);
            if remaining > 0 {
                output.push(format!(
                    "... {remaining} more item{}",
                    plural(remaining as u64)
                ));
            }
        }
        for key in &keys {
            let entry = self.property(realm, object, key, recurse, false)?;
            output.push(entry);
        }
        self.seen.pop();
        if let Some(index) = self.circular.iter().position(|seen| seen == object) {
            let reference = format!("<ref *{}>", index + 1);
            base = if base.is_empty() {
                reference
            } else {
                format!("{reference} {base}")
            };
        }
        let entries = output.len();
        if array && entries > 6 {
            output = self.group(object, output);
        }
        Ok(self.reduce(&output, entries, &base, &open, close, recurse))
    }

    /// `getFunctionBase` and `getClassBase`.
    fn function_base(
        &mut self,
        realm: &mut Realm,
        function: &Object,
        constructor: Option<&str>,
        tag: &str,
    ) -> Result<String, Thrown> {
        if is_class(realm, function) {
            let name = match own_data(function, "name") {
                Some(Value::String(name)) if !name.is_empty() => name.to_rust_lossy(),
                _ => "(anonymous)".to_owned(),
            };
            let mut base = format!("[class {name}");
            if let Some(constructor) = constructor.filter(|constructor| *constructor != "Function")
            {
                base.push_str(&format!(" [{constructor}]"));
            }
            if !tag.is_empty() && Some(tag) != constructor {
                base.push_str(&format!(" [{tag}]"));
            }
            let parent = function.data().prototype.clone();
            match parent {
                Some(parent) => {
                    let parent = ops::get_named(realm, &Value::Object(parent), "name")?;
                    if !matches!(parent, Value::Undefined) {
                        let parent = ops::to_string(realm, &parent)?;
                        if !parent.is_empty() {
                            base.push_str(&format!(" extends {}", parent.to_rust_lossy()));
                        }
                    }
                }
                None => base.push_str(" extends [null prototype]"),
            }
            base.push(']');
            return Ok(base);
        }
        let mut base = "[Function".to_owned();
        if constructor.is_none() {
            base.push_str(" (null prototype)");
        }
        match ops::get_named(realm, &Value::Object(function.clone()), "name")? {
            Value::String(name) if name.is_empty() => base.push_str(" (anonymous)"),
            name => base.push_str(&format!(
                ": {}",
                ops::to_string(realm, &name)?.to_rust_lossy()
            )),
        }
        base.push(']');
        if let Some(constructor) = constructor.filter(|constructor| *constructor != "Function") {
            base.push_str(&format!(" {constructor}"));
        }
        if !tag.is_empty() && Some(tag) != constructor {
            base.push_str(&format!(" [{tag}]"));
        }
        Ok(base)
    }

    /// `formatArray` and `formatSpecialArray`: elements, runs of holes and
    /// the count of elements past the limit.
    fn elements(
        &mut self,
        realm: &mut Realm,
        array: &Object,
        recurse: usize,
        output: &mut Vec<String>,
    ) -> Result<(), Thrown> {
        let length = crate::array_builtin::length_of(realm, &Value::Object(array.clone()))?;
        let present: Vec<u32> = array
            .own_keys()
            .into_iter()
            .filter_map(|key| match key {
                Key::Index(index) => Some(index),
                _ => None,
            })
            .collect();
        let mut index = 0u64;
        for element in present {
            if output.len() == MAX_ARRAY_LENGTH {
                break;
            }
            if u64::from(element) != index {
                let holes = u64::from(element) - index;
                output.push(format!("<{holes} empty item{}>", plural(holes)));
                index = u64::from(element);
                if output.len() == MAX_ARRAY_LENGTH {
                    break;
                }
            }
            let entry = self.property(realm, array, &Key::Index(element), recurse, true)?;
            output.push(entry);
            index += 1;
        }
        let remaining = length.saturating_sub(index);
        if remaining > 0 {
            output.push(if output.len() == MAX_ARRAY_LENGTH {
                format!("... {remaining} more item{}", plural(remaining))
            } else {
                format!("<{remaining} empty item{}>", plural(remaining))
            });
        }
        Ok(())
    }

    /// `formatProperty`: an entry, named unless it is an array element.
    fn property(
        &mut self,
        realm: &mut Realm,
        object: &Object,
        key: &Key,
        recurse: usize,
        element: bool,
    ) -> Result<String, Thrown> {
        let property = object
            .own_property(key)
            .unwrap_or(Property::data(Value::Undefined));
        let text = match property.slot {
            Slot::Data { value, .. } if !matches!(value, Value::Undefined) => {
                self.indentation += 2;
                let text = self.value(realm, &value, recurse);
                self.indentation -= 2;
                text?
            }
            Slot::Accessor {
                get: Some(_),
                set: Some(_),
            } => "[Getter/Setter]".to_owned(),
            Slot::Accessor {
                get: Some(_),
                set: None,
            } => "[Getter]".to_owned(),
            Slot::Accessor {
                get: None,
                set: Some(_),
            } => "[Setter]".to_owned(),
            _ => "undefined".to_owned(),
        };
        Ok(if element {
            text
        } else if property.enumerable {
            format!("{}: {text}", name(key))
        } else {
            format!("[{}]: {text}", key.to_js().to_rust_lossy())
        })
    }

    /// `reduceToSingleString`: one line when the entries are short and not
    /// nested too deeply, otherwise one entry (or group) per line.
    fn reduce(
        &self,
        output: &[String],
        entries: usize,
        base: &str,
        open: &str,
        close: &str,
        recurse: usize,
    ) -> String {
        let lead = if base.is_empty() {
            String::new()
        } else {
            format!("{base} ")
        };
        if self.current_depth - recurse < COMPACT && entries == output.len() {
            let start = output.len() + self.indentation + width(open) + width(base) + 10;
            if self.below_break_length(output, start, base) {
                let joined = output.join(", ");
                if !joined.contains('\n') {
                    return format!("{lead}{open} {joined} {close}");
                }
            }
        }
        let indentation = format!("\n{}", " ".repeat(self.indentation));
        let separator = format!(",{indentation}  ");
        format!(
            "{lead}{open}{indentation}  {}{indentation}{close}",
            output.join(&separator)
        )
    }

    fn below_break_length(&self, output: &[String], start: usize, base: &str) -> bool {
        let mut total = output.len() + start;
        if total + output.len() > BREAK_LENGTH {
            return false;
        }
        for entry in output {
            total += width(entry);
            if total > BREAK_LENGTH {
                return false;
            }
        }
        !base.contains('\n')
    }

    /// `groupArrayElements`: many short array entries in aligned columns,
    /// right-aligned when all are numbers and left-aligned otherwise.
    fn group(&self, array: &Object, output: Vec<String>) -> Vec<String> {
        let separator_space = 2;
        let mut output_length = output.len();
        if output.len() > MAX_ARRAY_LENGTH {
            // The "more items" entry is not grouped.
            output_length -= 1;
        }
        let lengths: Vec<usize> = output[..output_length]
            .iter()
            .map(|entry| width(entry))
            .collect();
        let total: usize = lengths.iter().map(|length| length + separator_space).sum();
        let max_length = lengths.iter().copied().max().unwrap_or(0);
        let actual_max = max_length + separator_space;
        let actual = actual_max as f64;
        let fits = actual_max * 3 + self.indentation < BREAK_LENGTH;
        if !fits || (total as f64 / actual <= 5.0 && max_length > 6) {
            return output;
        }
        let average_bias = crate::float::sqrt(actual - total as f64 / output.len() as f64);
        let biased_max = (actual - 3.0 - average_bias).max(1.0);
        let ideal = crate::float::sqrt(2.5 * biased_max * output_length as f64) / biased_max;
        let columns = (crate::float::floor(ideal + 0.5) as usize)
            .min((BREAK_LENGTH - self.indentation) / actual_max)
            .min(COMPACT * 4)
            .min(15);
        if columns <= 1 {
            return output;
        }
        let widths: Vec<usize> = (0..columns)
            .map(|column| {
                let widest = (column..output_length)
                    .step_by(columns)
                    .map(|index| lengths[index])
                    .max();
                widest.unwrap_or(0) + separator_space
            })
            .collect();
        let numbers = (0..output.len()).all(|index| {
            let property = u32::try_from(index)
                .ok()
                .and_then(|index| array.own_property(&Key::Index(index)));
            matches!(
                property.map(|property| property.slot),
                Some(Slot::Data {
                    value: Value::Number(_),
                    ..
                })
            )
        });
        let pad = |text: &str, width: usize| {
            let fill = " ".repeat(width.saturating_sub(self::width(text)));
            if numbers {
                fill + text
            } else {
                text.to_owned() + &fill
            }
        };
        let mut grouped = Vec::new();
        for row in (0..output_length).step_by(columns) {
            let last = (row + columns).min(output_length) - 1;
            let mut line = String::new();
            for index in row..last {
                line.push_str(&pad(&format!("{}, ", output[index]), widths[index - row]));
            }
            if numbers {
                line.push_str(&pad(&output[last], widths[last - row] - separator_space));
            } else {
                line.push_str(&output[last]);
            }
            grouped.push(line);
        }
        if output.len() > MAX_ARRAY_LENGTH {
            grouped.push(output[output_length].clone());
        }
        grouped
    }
}

/// `util.inspect(value, { depth })`.
pub(crate) fn inspect(realm: &mut Realm, value: &Value, depth: usize) -> Result<String, Thrown> {
    inspect_from(realm, value, depth, 0)
}

/// `util.inspect(value, { depth: -1 })`: objects with properties are
/// abbreviated even at the top level.
pub(crate) fn inspect_shallow(realm: &mut Realm, value: &Value) -> Result<String, Thrown> {
    inspect_from(realm, value, 0, 1)
}

fn inspect_from(
    realm: &mut Realm,
    value: &Value,
    depth: usize,
    start: usize,
) -> Result<String, Thrown> {
    let mut inspector = Inspector {
        depth,
        seen: Vec::new(),
        circular: Vec::new(),
        indentation: 0,
        current_depth: start,
    };
    inspector.value(realm, value, start)
}

/// Node's `hasBuiltInToString`: whether `%s` should inspect an object
/// rather than convert it, because its `toString` is a built-in one.
fn builtin_to_string(realm: &mut Realm, object: &Object) -> Result<bool, Thrown> {
    let key = Key::name("toString");
    let method = ops::get(realm, &Value::Object(object.clone()), &key)?;
    if !method
        .as_object()
        .is_some_and(|method| method.code().is_some())
    {
        return Ok(true);
    }
    if object.own_property(&key).is_some() {
        return Ok(false);
    }
    let mut holder = object.clone();
    loop {
        let parent = holder.data().prototype.clone();
        let Some(parent) = parent else {
            return Ok(false);
        };
        holder = parent;
        if holder.own_property(&key).is_some() {
            break;
        }
    }
    Ok(match own_data(&holder, "constructor") {
        Some(Value::Object(constructor)) => {
            let uppercase = matches!(own_data(&constructor, "name"), Some(Value::String(name)) if name.units().first().is_some_and(|first| (0x41..=0x5a).contains(first)));
            uppercase && global_builtin(realm, &constructor)
        }
        _ => false,
    })
}

/// `JSON.stringify` for `%j`, with `[Circular]` for circular structures.
fn json(realm: &mut Realm, argument: &Value) -> Result<String, Thrown> {
    match crate::json_builtin::stringify(realm, Value::Undefined, core::slice::from_ref(argument)) {
        Ok(Value::String(text)) => Ok(text.to_rust_lossy()),
        Ok(_) => Ok("undefined".to_owned()),
        Err(thrown) => {
            let circular = matches!(
                thrown.0.as_object().and_then(|error| own_data(error, "message")),
                Some(Value::String(message)) if message.to_rust_lossy() == "Converting circular structure to JSON"
            );
            if circular {
                Ok("[Circular]".to_owned())
            } else {
                Err(thrown)
            }
        }
    }
}

/// One `util.format` placeholder's text for its argument.
fn placeholder(
    realm: &mut Realm,
    kind: u8,
    argument: &Value,
    format_depth: Option<usize>,
) -> Result<String, Thrown> {
    let arguments = core::slice::from_ref(argument);
    Ok(match (kind, argument) {
        (b's', Value::Number(value)) => number(*value),
        (b's', Value::Symbol(symbol)) => symbol.descriptive().to_rust_lossy(),
        (b's', Value::Object(object))
            if object.code().is_none() && builtin_to_string(realm, object)? =>
        {
            inspect(realm, argument, 0)?
        }
        (b's', other) => ops::to_string(realm, other)?.to_rust_lossy(),
        (b'd' | b'i' | b'f', Value::Symbol(_)) => "NaN".to_owned(),
        (b'd', other) => number(ops::to_number(realm, other)?),
        (b'i', _) => {
            let value = crate::global_number::parse_int(realm, Value::Undefined, arguments)?;
            number(ops::to_number(realm, &value)?)
        }
        (b'f', _) => {
            let value = crate::global_number::parse_float(realm, Value::Undefined, arguments)?;
            number(ops::to_number(realm, &value)?)
        }
        (b'j', _) => json(realm, argument)?,
        (b'o', _) => inspect(realm, argument, format_depth.unwrap_or(4))?,
        (b'O', _) => inspect(realm, argument, format_depth.unwrap_or(DEPTH))?,
        // `%c` takes CSS, which text output ignores.
        _ => String::new(),
    })
}

/// `util.format(...args)`: placeholders in a leading string, then the
/// remaining arguments separated by spaces, strings as they are and other
/// values inspected.
pub(crate) fn format(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    format_at_depth(realm, args, None)
}

/// `util.formatWithOptions`: its `depth` option applies to all inspected
/// placeholder and trailing values, while `%o` retains its deeper default
/// when no explicit depth is supplied.
pub(crate) fn format_with_depth(
    realm: &mut Realm,
    args: &[Value],
    depth: usize,
) -> Result<String, Thrown> {
    format_at_depth(realm, args, Some(depth))
}

fn format_at_depth(
    realm: &mut Realm,
    args: &[Value],
    format_depth: Option<usize>,
) -> Result<String, Thrown> {
    let mut output = String::new();
    let mut next = 0;
    let mut join = "";
    if let Some(Value::String(first)) = args.first() {
        if args.len() == 1 {
            return Ok(first.to_rust_lossy());
        }
        let units = first.units();
        let text =
            |range: core::ops::Range<usize>| JsStr::from_units(&units[range]).to_rust_lossy();
        let mut last = 0;
        let mut index = 0;
        while index + 1 < units.len() {
            if units[index] == u16::from(b'%') {
                index += 1;
                let kind = u8::try_from(units[index]).unwrap_or(0);
                if kind == b'%' {
                    output.push_str(&text(last..index));
                    last = index + 1;
                } else if next + 1 != args.len() && b"sdifjoOc".contains(&kind) {
                    next += 1;
                    let replacement = placeholder(realm, kind, &args[next], format_depth)?;
                    output.push_str(&text(last..index - 1));
                    output.push_str(&replacement);
                    last = index + 1;
                }
            }
            index += 1;
        }
        if last != 0 {
            next += 1;
            join = " ";
            output.push_str(&text(last..units.len()));
        }
    }
    for value in &args[next..] {
        output.push_str(join);
        match value {
            Value::String(text) => output.push_str(&text.to_rust_lossy()),
            value => output.push_str(&inspect(realm, value, format_depth.unwrap_or(DEPTH))?),
        }
        join = " ";
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
